//! One running copy of the app per state directory (ADR 0046).
//!
//! The state directory holds the store and the recovery files (ADR 0040),
//! which must have one writer. So the first copy to start takes an exclusive
//! operating-system lock on `<state dir>/instance.lock` and keeps it until it
//! exits; the kernel releases it even after a crash. A copy started while the
//! lock is held hands its documents to the running one and exits:
//!
//! - The running copy listens on a loopback TCP port and writes the port and
//!   a random token to `<state dir>/instance.json`, readable only by the user.
//! - A later copy connects, sends the token and its documents' absolute paths,
//!   and waits for "ok".
//!
//! Loopback TCP and an OS file lock are the same on macOS, Windows and Linux
//! and need nothing beyond `std`. The lock is keyed by directory rather than
//! by bundle identifier, so self-tests and benchmarks (each with its own
//! `SAFHA_STATE_DIR`) run beside each other and beside the owner's own copy.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const LOCK_FILE: &str = "instance.lock";
const INFO_FILE: &str = "instance.json";
/// How long a later copy keeps trying to reach the running one: across its
/// start (the lock is taken before the port is written) or its exit.
const HAND_OVER_FOR: Duration = Duration::from_secs(10);
const RETRY_EVERY: Duration = Duration::from_millis(50);
/// A running copy answers within this; it opens documents after answering.
const ANSWER_WITHIN: Duration = Duration::from_secs(5);
/// A request is a token and a few paths; anything larger is not ours.
const MAX_REQUEST: u64 = 1 << 20;

/// What starting up found.
pub enum Claim {
    /// This process is the running copy.
    Primary(Instance),
    /// Another copy runs and has taken the documents; this one should exit.
    HandedOver,
}

#[derive(Debug, thiserror::Error)]
pub enum InstanceError {
    /// The lock could not be taken or tested (an unsupported file system).
    #[error("cannot lock {0}: {1}")]
    Lock(PathBuf, io::Error),
    /// Another copy holds the lock but did not take the documents in time.
    #[error("another copy is running but did not answer: {0}")]
    NoAnswer(io::Error),
}

/// The running copy's claim on its state directory. The lock lasts as long
/// as this value (in practice, the process).
pub struct Instance {
    _lock: File,
    listener: TcpListener,
    token: String,
}

#[derive(Serialize, Deserialize)]
struct Info {
    port: u16,
    token: String,
}

#[derive(Serialize, Deserialize)]
struct Request {
    token: String,
    documents: Vec<WirePath>,
}

/// Becomes the running copy for `dir`, or hands `documents` to the copy
/// that already is. `documents` must be absolute: the running copy has its
/// own working directory.
pub fn claim(dir: &Path, documents: &[PathBuf]) -> Result<Claim, InstanceError> {
    claim_within(dir, documents, HAND_OVER_FOR)
}

fn claim_within(dir: &Path, documents: &[PathBuf], wait: Duration) -> Result<Claim, InstanceError> {
    let lock_path = dir.join(LOCK_FILE);
    let lock_error = |err| InstanceError::Lock(lock_path.clone(), err);
    std::fs::create_dir_all(dir).map_err(lock_error)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(lock_error)?;
    let deadline = Instant::now() + wait;
    loop {
        match lock.try_lock() {
            Ok(()) => return Instance::start(dir, lock).map(Claim::Primary).map_err(lock_error),
            Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(err)) => return Err(lock_error(err)),
        }
        // Held: another copy is running, starting, or exiting.
        match hand_over(dir, documents) {
            Ok(()) => return Ok(Claim::HandedOver),
            Err(err) if Instant::now() >= deadline => return Err(InstanceError::NoAnswer(err)),
            Err(_) => std::thread::sleep(RETRY_EVERY),
        }
    }
}

impl Instance {
    fn start(dir: &Path, lock: File) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).map_err(|err| io::Error::other(err.to_string()))?;
        let token = bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        let info = Info { port: listener.local_addr()?.port(), token: token.clone() };
        write_private(&dir.join(INFO_FILE), &serde_json::to_vec(&info).map_err(io::Error::other)?)?;
        Ok(Self { _lock: lock, listener, token })
    }

    /// Answers later copies on a thread of its own, for the life of the
    /// process: `open` gets each one's documents (possibly none: it was
    /// started without any, and the running copy should come forward).
    /// While `accepting` is false (the app is quitting) requests go
    /// unanswered, and the later copy waits to take over the lock instead.
    pub fn serve(
        self,
        accepting: impl Fn() -> bool + Send + 'static,
        open: impl Fn(Vec<PathBuf>) + Send + 'static,
    ) {
        std::thread::Builder::new()
            .name("instance".into())
            .spawn(move || {
                // The lock must outlive every answer.
                let Self { _lock, listener, token } = self;
                for stream in listener.incoming() {
                    if !accepting() {
                        continue;
                    }
                    match stream.and_then(|stream| answer(stream, &token)) {
                        Ok(documents) => open(documents),
                        Err(err) => eprintln!("safha: ignored a request from another copy: {err}"),
                    }
                }
            })
            .map(drop)
            .unwrap_or_else(|err| eprintln!("safha: cannot answer other copies: {err}"));
    }
}

/// Reads one request, checks its token, and acknowledges it.
fn answer(stream: TcpStream, token: &str) -> io::Result<Vec<PathBuf>> {
    stream.set_read_timeout(Some(ANSWER_WITHIN))?;
    stream.set_write_timeout(Some(ANSWER_WITHIN))?;
    let mut line = Vec::new();
    BufReader::new((&stream).take(MAX_REQUEST)).read_until(b'\n', &mut line)?;
    let request: Request = serde_json::from_slice(&line).map_err(invalid)?;
    if request.token != token {
        return Err(invalid("wrong token"));
    }
    let documents: Vec<PathBuf> = request.documents.into_iter().map(WirePath::into_path).collect();
    if documents.iter().any(|path| !path.is_absolute()) {
        return Err(invalid("a relative path"));
    }
    (&stream).write_all(b"ok\n")?;
    Ok(documents)
}

/// Sends `documents` to the running copy and waits for its "ok".
fn hand_over(dir: &Path, documents: &[PathBuf]) -> io::Result<()> {
    let info: Info = serde_json::from_slice(&std::fs::read(dir.join(INFO_FILE))?).map_err(invalid)?;
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, info.port));
    let stream = TcpStream::connect_timeout(&address, ANSWER_WITHIN)?;
    stream.set_read_timeout(Some(ANSWER_WITHIN))?;
    stream.set_write_timeout(Some(ANSWER_WITHIN))?;
    let request = Request { token: info.token, documents: documents.iter().map(WirePath::from).collect() };
    let mut line = serde_json::to_vec(&request).map_err(invalid)?;
    line.push(b'\n');
    (&stream).write_all(&line)?;
    let mut reply = String::new();
    BufReader::new(&stream).read_line(&mut reply)?;
    if reply.trim_end() == "ok" { Ok(()) } else { Err(invalid("no acknowledgement")) }
}

/// Writes `bytes` to `path` through a temporary file, readable by the user only.
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temp = path.with_extension("tmp");
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temp, path)
}

fn invalid(err: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
}

/// A path exactly as the operating system spells it: bytes on Unix, UTF-16
/// on Windows. A path that is not valid Unicode survives the trip.
#[derive(Serialize, Deserialize)]
#[serde(transparent)]
struct WirePath(#[cfg(unix)] Vec<u8>, #[cfg(windows)] Vec<u16>);

impl From<&PathBuf> for WirePath {
    fn from(path: &PathBuf) -> Self {
        #[cfg(unix)]
        return Self(std::os::unix::ffi::OsStrExt::as_bytes(path.as_os_str()).to_vec());
        #[cfg(windows)]
        return Self(std::os::windows::ffi::OsStrExt::encode_wide(path.as_os_str()).collect());
    }
}

impl WirePath {
    fn into_path(self) -> PathBuf {
        #[cfg(unix)]
        return PathBuf::from(<std::ffi::OsString as std::os::unix::ffi::OsStringExt>::from_vec(self.0));
        #[cfg(windows)]
        return PathBuf::from(<std::ffi::OsString as std::os::windows::ffi::OsStringExt>::from_wide(&self.0));
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("safha-instance-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn primary(dir: &Path) -> Instance {
        match claim(dir, &[]).expect("claims") {
            Claim::Primary(instance) => instance,
            Claim::HandedOver => panic!("nothing else holds {}", dir.display()),
        }
    }

    #[test]
    fn a_second_copy_hands_its_documents_to_the_first() {
        let dir = temp_dir("handover");
        let (sent, received) = mpsc::channel();
        primary(&dir).serve(|| true, move |documents| sent.send(documents).expect("receiver alive"));

        let documents = vec![dir.join("a.pdf"), dir.join("نص عربي.pdf")];
        assert!(matches!(claim(&dir, &documents).expect("hands over"), Claim::HandedOver));
        assert_eq!(received.recv_timeout(Duration::from_secs(5)).expect("delivered"), documents);

        // Started without documents: the running copy still hears of it.
        assert!(matches!(claim(&dir, &[]).expect("hands over"), Claim::HandedOver));
        assert_eq!(received.recv_timeout(Duration::from_secs(5)).expect("delivered"), Vec::<PathBuf>::new());
    }

    #[test]
    fn the_lock_passes_on_when_the_running_copy_ends() {
        let dir = temp_dir("release");
        let first = primary(&dir);
        drop(first);
        // A stale instance.json is left behind; the next copy just takes over.
        assert!(dir.join(INFO_FILE).is_file());
        let _second = primary(&dir);
    }

    #[test]
    fn copies_with_different_state_directories_do_not_meet() {
        let (one, two) = (temp_dir("one"), temp_dir("two"));
        let _first = primary(&one);
        let _second = primary(&two);
    }

    #[test]
    fn a_request_without_the_token_is_refused() {
        let dir = temp_dir("token");
        let (sent, received) = mpsc::channel();
        primary(&dir).serve(|| true, move |documents| sent.send(documents).expect("receiver alive"));
        let info: Info =
            serde_json::from_slice(&std::fs::read(dir.join(INFO_FILE)).expect("info")).expect("json");
        let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, info.port)).expect("connects");
        let request = Request { token: "0".repeat(32), documents: vec![WirePath::from(&dir.join("x.pdf"))] };
        let mut line = serde_json::to_vec(&request).expect("json");
        line.push(b'\n');
        (&stream).write_all(&line).expect("sends");
        let mut reply = String::new();
        BufReader::new(&stream).read_line(&mut reply).expect("closed");
        assert_eq!(reply, "");
        assert!(received.recv_timeout(Duration::from_millis(200)).is_err());
    }

    #[test]
    fn a_copy_that_is_quitting_lets_the_next_one_take_over() {
        let dir = temp_dir("quitting");
        let (sent, received) = mpsc::channel();
        let quitting = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let first = primary(&dir);
        let refusing = std::sync::Arc::clone(&quitting);
        first.serve(
            move || !refusing.load(std::sync::atomic::Ordering::SeqCst),
            move |documents| sent.send(documents).expect("receiver alive"),
        );
        quitting.store(true, std::sync::atomic::Ordering::SeqCst);
        // Unanswered until the deadline: the later copy reports it rather
        // than writing beside the running one.
        let wait = Duration::from_millis(500);
        let started = Instant::now();
        assert!(matches!(claim_within(&dir, &[dir.join("late.pdf")], wait), Err(InstanceError::NoAnswer(_))));
        assert!(started.elapsed() >= wait);
        assert!(received.try_recv().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn the_port_and_token_are_readable_by_the_user_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = temp_dir("private");
        let _instance = primary(&dir);
        let mode = std::fs::metadata(dir.join(INFO_FILE)).expect("info").permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn a_path_that_is_not_unicode_survives() {
        use std::os::unix::ffi::OsStrExt;
        let dir = temp_dir("bytes");
        let (sent, received) = mpsc::channel();
        primary(&dir).serve(|| true, move |documents| sent.send(documents).expect("receiver alive"));
        let odd = dir.join(std::ffi::OsStr::from_bytes(b"caf\xe9.pdf"));
        assert!(matches!(claim(&dir, std::slice::from_ref(&odd)).expect("hands over"), Claim::HandedOver));
        assert_eq!(received.recv_timeout(Duration::from_secs(5)).expect("delivered"), vec![odd]);
    }
}
