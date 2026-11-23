//! The operating system's own record of a native crash (ADR 0061).
//!
//! No Rust code runs when PDFium or the webview crash the process, and an
//! in-process signal handler would need `unsafe` code that must stay
//! async-signal-safe. The systems already write a crash log for every crashed
//! process, so the next start reads that instead:
//!
//! - **macOS** writes `~/Library/Logs/DiagnosticReports/<process>-<time>.ips`:
//!   a JSON header line, then a JSON body with the pid, the exception, and
//!   every thread's frames as (image, offset, symbol).
//! - **Windows** Error Reporting archives `Report.wer` (UTF-16 `key=value`
//!   lines) under `…\Microsoft\Windows\WER\ReportArchive\AppCrash_<exe>_…`: the
//!   faulting module, the exception code and its offset.
//! - **Linux** has no common place (systemd-coredump, apport, or nothing), so
//!   native crashes there are not reported.
//!
//! Only the exception, the crashed thread's name, and frames as `image +
//! offset symbol` are taken: the logs also hold the executable's path (which
//! can contain the user's name), registers and other threads, none of which
//! is kept.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use super::RunRecord;

/// Whether this system's crash logs can be read.
pub const SUPPORTED: bool = cfg!(any(target_os = "macos", target_os = "windows"));

/// What a system crash log says about a crash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeCrash {
    /// `EXC_BAD_ACCESS (SIGSEGV)`, `exception c0000005 in pdfium.dll`.
    pub exception: String,
    pub thread: Option<String>,
    /// `libpdfium.dylib + 0x1a2b3c FPDF_LoadPage`, innermost first.
    pub frames: Vec<String>,
}

/// Crash logs bigger than this are not read.
const MAX_LOG_BYTES: u64 = 4 * 1024 * 1024;
/// Crash log files looked at, newest first.
const MAX_LOGS: usize = 200;

/// Finds the system's crash log of `run`, if the system wrote one.
pub fn find_crash(run: &RunRecord) -> Option<NativeCrash> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")?;
        let dir = Path::new(&home).join("Library/Logs/DiagnosticReports");
        recent_files(&[dir], run.started, |path| path.extension().is_some_and(|ext| ext == "ips"))
            .into_iter()
            .find_map(|path| parse_ips(&fs::read_to_string(path).ok()?, run.pid, &run.exe))
    }
    #[cfg(target_os = "windows")]
    {
        let mut roots = Vec::new();
        for (key, sub) in
            [("LOCALAPPDATA", "Microsoft/Windows/WER"), ("ProgramData", "Microsoft/Windows/WER")]
        {
            if let Some(base) = std::env::var_os(key) {
                for kind in ["ReportArchive", "ReportQueue"] {
                    roots.push(Path::new(&base).join(sub).join(kind));
                }
            }
        }
        recent_files(&roots, run.started, |path| {
            path.file_name().and_then(|name| name.to_str()).is_some_and(|name| name.starts_with("AppCrash_"))
        })
        .into_iter()
        .find_map(|dir| parse_wer(&read_wer(&dir.join("Report.wer"))?, &run.exe))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = run;
        None
    }
}

/// Entries of `dirs` modified since `since` (Unix seconds, less a minute's
/// slack for clocks), newest first, that `keep` accepts.
#[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
fn recent_files(dirs: &[PathBuf], since: i64, keep: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut found: Vec<(i64, PathBuf)> = dirs
        .iter()
        .filter_map(|dir| fs::read_dir(dir).ok())
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
            let modified = i64::try_from(modified).ok()?;
            let path = entry.path();
            (modified + 60 >= since && keep(&path) && (meta.is_dir() || meta.len() <= MAX_LOG_BYTES))
                .then_some((modified, path))
        })
        .collect();
    found.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    found.into_iter().take(MAX_LOGS).map(|(_, path)| path).collect()
}

/// Reads a `Report.wer`: UTF-16 little-endian with a byte-order mark, or UTF-8.
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
fn read_wer(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    if bytes.len() as u64 > MAX_LOG_BYTES {
        return None;
    }
    decode_wer(&bytes)
}

fn decode_wer(bytes: &[u8]) -> Option<String> {
    match bytes {
        [0xFF, 0xFE, rest @ ..] => {
            let units: Vec<u16> =
                rest.as_chunks::<2>().0.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
            String::from_utf16(&units).ok()
        }
        _ => String::from_utf8(bytes.to_vec()).ok(),
    }
}

/// The crash in a macOS `.ips` crash log, if it is the crash of process `pid`
/// running `exe`.
pub fn parse_ips(text: &str, pid: u32, exe: &str) -> Option<NativeCrash> {
    let (header, body) = text.split_once('\n')?;
    let header: serde_json::Value = serde_json::from_str(header).ok()?;
    // 309: a crash. Other kinds (hangs, resource reports) are not crashes.
    if header.get("bug_type").and_then(|kind| kind.as_str()) != Some("309") {
        return None;
    }
    let body: serde_json::Value = serde_json::from_str(body).ok()?;
    if body.get("pid").and_then(serde_json::Value::as_u64) != Some(u64::from(pid)) {
        return None;
    }
    if body.get("procName").and_then(|name| name.as_str()).is_some_and(|name| name != exe) {
        return None;
    }

    let exception = body.get("exception");
    let field = |name: &str| exception.and_then(|e| e.get(name)).and_then(|v| v.as_str()).map(str::to_owned);
    let exception = match (field("type"), field("signal")) {
        (Some(kind), Some(signal)) => format!("{kind} ({signal})"),
        (Some(kind), None) => kind,
        (None, Some(signal)) => signal,
        (None, None) => "crash".to_owned(),
    };
    let indicator = body
        .get("termination")
        .and_then(|t| t.get("indicator"))
        .and_then(|v| v.as_str())
        .filter(|indicator| indicator.chars().all(|c| c.is_ascii() && !c.is_control()));
    let exception = match indicator {
        Some(indicator) => format!("{exception}; {}", indicator.chars().take(80).collect::<String>()),
        None => exception,
    };

    let images: Vec<String> = body
        .get("usedImages")
        .and_then(|images| images.as_array())
        .map(|images| {
            images
                .iter()
                .map(|image| {
                    image
                        .get("name")
                        .and_then(|name| name.as_str())
                        .map(str::to_owned)
                        .or_else(|| {
                            let path = image.get("path")?.as_str()?;
                            Some(path.rsplit('/').next()?.to_owned())
                        })
                        .unwrap_or_else(|| "???".to_owned())
                })
                .collect()
        })
        .unwrap_or_default();
    let threads = body.get("threads").and_then(|threads| threads.as_array());
    let faulting = body.get("faultingThread").and_then(serde_json::Value::as_u64);
    let thread = threads.and_then(|threads| {
        faulting.and_then(|index| threads.get(usize::try_from(index).ok()?)).or_else(|| {
            threads
                .iter()
                .find(|thread| thread.get("triggered").and_then(serde_json::Value::as_bool) == Some(true))
        })
    });
    let thread_name = thread.and_then(|thread| {
        let name = thread.get("name").or_else(|| thread.get("queue"))?.as_str()?;
        name.chars().all(|c| c.is_ascii() && !c.is_control()).then(|| name.chars().take(64).collect())
    });
    let frames = thread
        .and_then(|thread| thread.get("frames")?.as_array())
        .map(|frames| {
            frames
                .iter()
                .take(super::MAX_FRAMES)
                .map(|frame| {
                    let image = frame
                        .get("imageIndex")
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|index| images.get(usize::try_from(index).ok()?))
                        .map_or("???", String::as_str);
                    let offset = frame.get("imageOffset").and_then(serde_json::Value::as_u64).unwrap_or(0);
                    match frame.get("symbol").and_then(|symbol| symbol.as_str()) {
                        Some(symbol) => {
                            format!("{image} + 0x{offset:x} {}", super::redact::function_name(symbol))
                        }
                        None => format!("{image} + 0x{offset:x}"),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Some(NativeCrash { exception, thread: thread_name, frames })
}

/// The crash in a Windows Error Reporting `Report.wer`, if it is a crash of
/// `exe`. WER records one frame: the faulting module and the offset in it.
pub fn parse_wer(text: &str, exe: &str) -> Option<NativeCrash> {
    let mut event = None;
    let mut names: Vec<(usize, String)> = Vec::new();
    let mut values: Vec<(usize, String)> = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = line.trim().split_once('=') else { continue };
        if key == "EventType" {
            event = Some(value.to_owned());
        } else if let Some(rest) = key.strip_prefix("Sig[") {
            let Some((index, field)) = rest.split_once("].") else { continue };
            let Ok(index) = index.parse::<usize>() else { continue };
            match field {
                "Name" => names.push((index, value.to_owned())),
                "Value" => values.push((index, value.to_owned())),
                _ => {}
            }
        }
    }
    if !matches!(event.as_deref(), Some("APPCRASH" | "BEX" | "BEX64")) {
        return None;
    }
    let signature = |name: &str| {
        let index = names.iter().find(|(_, n)| n.eq_ignore_ascii_case(name))?.0;
        values.iter().find(|(i, _)| *i == index).map(|(_, value)| value.clone())
    };
    if !signature("Application Name")?.eq_ignore_ascii_case(exe) {
        return None;
    }
    // Module names are of the app's own files and the system's; anything else
    // is dropped rather than risk a path.
    let safe = |value: String| {
        value.chars().all(|c| c.is_ascii_alphanumeric() || "._-+".contains(c)).then_some(value)
    };
    let module = signature("Fault Module Name").and_then(safe).unwrap_or_else(|| "???".to_owned());
    let version = signature("Fault Module Version").and_then(safe);
    let code = signature("Exception Code").and_then(safe).unwrap_or_else(|| "?".to_owned());
    let offset = signature("Exception Offset").and_then(safe).unwrap_or_else(|| "?".to_owned());
    let module_with_version = match version {
        Some(version) => format!("{module} {version}"),
        None => module.clone(),
    };
    Some(NativeCrash {
        exception: format!("exception {code} in {module_with_version}"),
        thread: None,
        frames: vec![format!("{module} + 0x{}", offset.trim_start_matches("0x"))],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const IPS: &str = r#"{"app_name":"mirsam-desktop","timestamp":"2026-11-23 10:42:10.00 +0000","app_version":"0.0.1","bug_type":"309","os_version":"macOS 26.1 (25B78)","name":"mirsam-desktop"}
{
  "pid" : 4242,
  "procName" : "mirsam-desktop",
  "procPath" : "/Users/someone/Applications/Mirsam.app/Contents/MacOS/mirsam-desktop",
  "exception" : {"codes":"0x0000000000000001, 0x0000000000000000","type":"EXC_BAD_ACCESS","signal":"SIGSEGV","subtype":"KERN_INVALID_ADDRESS at 0x0000000000000000"},
  "termination" : {"indicator":"Segmentation fault: 11"},
  "asi" : {"libsystem_c.dylib":["secret document title"]},
  "faultingThread" : 1,
  "threads" : [
    {"id":1,"queue":"com.apple.main-thread","frames":[{"imageOffset":10,"imageIndex":0}]},
    {"id":2,"name":"mirsam-render","triggered":true,"frames":[
      {"imageOffset":1715004,"symbol":"CPDF_Parser::LoadPage","symbolLocation":12,"imageIndex":1},
      {"imageOffset":4096,"imageIndex":0},
      {"imageOffset":255,"symbol":"_pthread_start","imageIndex":2}
    ]}
  ],
  "usedImages" : [
    {"base":4294967296,"name":"mirsam-desktop","path":"/Users/someone/Applications/Mirsam.app/Contents/MacOS/mirsam-desktop"},
    {"base":4300000000,"path":"/Users/someone/Applications/Mirsam.app/Contents/Frameworks/libpdfium.dylib"},
    {"base":4400000000,"name":"libsystem_pthread.dylib","path":"/usr/lib/system/libsystem_pthread.dylib"}
  ]
}"#;

    #[test]
    fn reads_the_crashed_thread_of_a_macos_crash_log() {
        let crash = parse_ips(IPS, 4242, "mirsam-desktop").expect("the crash");
        assert_eq!(crash.exception, "EXC_BAD_ACCESS (SIGSEGV); Segmentation fault: 11");
        assert_eq!(crash.thread.as_deref(), Some("mirsam-render"));
        assert_eq!(
            crash.frames,
            [
                "libpdfium.dylib + 0x1a2b3c CPDF_Parser::LoadPage",
                "mirsam-desktop + 0x1000",
                "libsystem_pthread.dylib + 0xff _pthread_start",
            ]
        );
        // Nothing from paths or application-specific information.
        let all = format!("{crash:?}");
        assert!(!all.contains("someone") && !all.contains("secret"), "{all}");
    }

    #[test]
    fn ignores_other_processes_and_other_kinds_of_log() {
        assert!(parse_ips(IPS, 4243, "mirsam-desktop").is_none());
        assert!(parse_ips(IPS, 4242, "Mirsam").is_none());
        let hang = IPS.replacen("\"309\"", "\"288\"", 1);
        assert!(parse_ips(&hang, 4242, "mirsam-desktop").is_none());
        assert!(parse_ips("not json\n{}", 4242, "mirsam-desktop").is_none());
    }

    const WER: &str = "Version=1\r\nEventType=APPCRASH\r\nEventTime=134080000000000000\r\n\
Sig[0].Name=Application Name\r\nSig[0].Value=mirsam-desktop.exe\r\n\
Sig[1].Name=Application Version\r\nSig[1].Value=0.0.1.0\r\n\
Sig[3].Name=Fault Module Name\r\nSig[3].Value=pdfium.dll\r\n\
Sig[4].Name=Fault Module Version\r\nSig[4].Value=0.0.0.0\r\n\
Sig[6].Name=Exception Code\r\nSig[6].Value=c0000005\r\n\
Sig[7].Name=Exception Offset\r\nSig[7].Value=00000000001a2b3c\r\n\
AppPath=C:\\Users\\Some One\\AppData\\Local\\Mirsam\\mirsam-desktop.exe\r\n";

    #[test]
    fn reads_the_faulting_module_of_a_windows_error_report() {
        let crash = parse_wer(WER, "mirsam-desktop.exe").expect("the crash");
        assert_eq!(crash.exception, "exception c0000005 in pdfium.dll 0.0.0.0");
        assert_eq!(crash.frames, ["pdfium.dll + 0x00000000001a2b3c"]);
        assert!(!format!("{crash:?}").contains("Some One"));
        assert!(parse_wer(WER, "other.exe").is_none());
        assert!(parse_wer(&WER.replace("APPCRASH", "AppHangB1"), "mirsam-desktop.exe").is_none());
    }

    #[test]
    fn decodes_utf16_error_reports() {
        let mut bytes = vec![0xFF, 0xFE];
        for unit in WER.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        assert_eq!(decode_wer(&bytes).as_deref(), Some(WER));
        assert_eq!(decode_wer(WER.as_bytes()).as_deref(), Some(WER));
    }
}
