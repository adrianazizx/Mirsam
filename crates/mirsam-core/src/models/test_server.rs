//! A small HTTP/1.1 file server on 127.0.0.1 for the download tests: ranges,
//! redirects, and the trouble real servers cause (dropped connections, 503s,
//! no range support). No real model is ever downloaded in tests.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex, PoisonError};

/// Misbehaviour to put on.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Trouble {
    None,
    /// Ignore `Range` and always send the whole file.
    NoRanges,
    /// Close the connection after `bytes` of the body, `times` times.
    CutAfter {
        bytes: usize,
        times: usize,
    },
    /// Answer `status` with no body, `times` times.
    Status {
        status: u16,
        times: usize,
    },
}

struct Shared {
    files: HashMap<String, Vec<u8>>,
    /// Path and `Range` header of every request for a file (redirects excluded).
    requests: Vec<(String, Option<String>)>,
    trouble: Trouble,
}

pub(crate) struct Server {
    addr: SocketAddr,
    shared: Arc<Mutex<Shared>>,
}

impl Server {
    pub(crate) fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
        let addr = listener.local_addr().expect("local address");
        let shared = Arc::new(Mutex::new(Shared {
            files: HashMap::new(),
            requests: Vec::new(),
            trouble: Trouble::None,
        }));
        let accept = Arc::clone(&shared);
        // Serves until the test process ends.
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let shared = Arc::clone(&accept);
                std::thread::spawn(move || {
                    let _ = handle(stream, &shared);
                });
            }
        });
        Self { addr, shared }
    }

    pub(crate) fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }

    pub(crate) fn add(&self, path: &str, bytes: &[u8]) {
        self.lock().files.insert(path.to_owned(), bytes.to_vec());
    }

    pub(crate) fn trouble(&self, trouble: Trouble) {
        self.lock().trouble = trouble;
    }

    pub(crate) fn requests(&self) -> Vec<(String, Option<String>)> {
        self.lock().requests.clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn handle(mut stream: TcpStream, shared: &Mutex<Shared>) -> std::io::Result<()> {
    let mut head = Vec::new();
    let mut byte = [0; 1];
    while !head.ends_with(b"\r\n\r\n") && head.len() < 16 * 1024 {
        if stream.read(&mut byte)? == 0 {
            return Ok(());
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head);
    let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
    let range = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("range").then(|| value.trim().to_owned())
    });

    if let Some(rest) = path.strip_prefix("/redirect") {
        let location = format!("http://{}{rest}", stream.local_addr()?);
        return write!(
            stream,
            "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
    }

    let (file, trouble) = {
        let mut shared = shared.lock().unwrap_or_else(PoisonError::into_inner);
        shared.requests.push((path.clone(), range.clone()));
        let trouble = shared.trouble;
        // Count down the trouble that has a count.
        shared.trouble = match trouble {
            Trouble::CutAfter { bytes, times } if times > 1 => Trouble::CutAfter { bytes, times: times - 1 },
            Trouble::Status { status, times } if times > 1 => Trouble::Status { status, times: times - 1 },
            Trouble::CutAfter { .. } | Trouble::Status { .. } => Trouble::None,
            other => other,
        };
        (shared.files.get(&path).cloned(), trouble)
    };
    if let Trouble::Status { status, .. } = trouble {
        return write!(stream, "HTTP/1.1 {status} Trouble\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    }
    let Some(file) = file else {
        return write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    };
    let start = match (&range, trouble) {
        (_, Trouble::NoRanges) | (None, _) => None,
        (Some(range), _) => {
            range.strip_prefix("bytes=").and_then(|r| r.strip_suffix('-')).and_then(|n| n.parse().ok())
        }
    };
    let body = match start {
        Some(start) if start >= file.len() => {
            return write!(
                stream,
                "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                file.len()
            );
        }
        Some(start) => {
            write!(
                stream,
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                file.len() - 1,
                file.len(),
                file.len() - start
            )?;
            &file[start..]
        }
        None => {
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                file.len()
            )?;
            &file[..]
        }
    };
    match trouble {
        Trouble::CutAfter { bytes, .. } => stream.write_all(&body[..bytes.min(body.len())]),
        _ => stream.write_all(body),
    }
}
