//! The quick draft readers (spec §8 "Progressive results", ADR 0030, ADR 0035):
//! the platform's own OCR where there is one (Apple Vision on macOS), Tesseract
//! elsewhere. They make a page searchable within moments, marked as a draft
//! until the quality pipeline replaces it, and they give each line and word a
//! box, which the quality result's word boxes are aligned to ([`super::boxes`]).

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use super::{OcrError, OcrImage};

/// A box in an image's pixels, origin top-left: `[x0, y0, x1, y1]`.
pub type PixelBox = [f32; 4];

/// A word a draft reader found, with its box when the reader gives one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DraftWord {
    pub text: String,
    #[serde(rename = "box", default, skip_serializing_if = "Option::is_none")]
    pub bounds: Option<PixelBox>,
}

/// A line (or a piece of one) a draft reader found: its text in logical order,
/// its box, and its words in logical order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DraftLine {
    pub text: String,
    #[serde(default)]
    pub confidence: f32,
    #[serde(rename = "box")]
    pub bounds: PixelBox,
    #[serde(default)]
    pub words: Vec<DraftWord>,
}

/// A page as a draft reader read it, in the reader's own line order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DraftPage {
    pub lines: Vec<DraftLine>,
}

impl DraftPage {
    /// The page's text, one line per line.
    pub fn text(&self) -> String {
        self.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n")
    }
}

/// A quick reader that finds lines and words with boxes.
pub trait DraftReader: Send {
    /// A short stable name (`apple-vision`, `tesseract`).
    fn name(&self) -> &str;
    fn read(&mut self, image: &OcrImage) -> Result<DraftPage, OcrError>;
}

/// Whether `text`'s first strong character is right-to-left (`None`: no strong character).
pub fn first_strong_rtl(text: &str) -> Option<bool> {
    use unicode_bidi::{BidiClass, bidi_class};
    text.chars().find_map(|c| match bidi_class(c) {
        BidiClass::R | BidiClass::AL => Some(true),
        BidiClass::L => Some(false),
        _ => None,
    })
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_png(image: &OcrImage) -> Result<PathBuf, OcrError> {
    let path = std::env::temp_dir().join(format!(
        "mirsam-draft-{}-{}.png",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, image.encode_png()?)
        .map_err(|err| OcrError::StartFailed(format!("temp image: {err}")))?;
    Ok(path)
}

// ---------- Apple Vision ----------

/// The Vision helper's source; built on first use ([`vision_helper`]).
pub const VISION_SOURCE: &str = include_str!("vision.swift");

/// The Vision helper program, built from [`VISION_SOURCE`] into `cache` the first
/// time (with the Swift compiler of Xcode or its command-line tools) and reused
/// after. `SAFHA_VISION_HELPER` names a ready-made helper instead (a signed one
/// shipped with the app, M7).
pub fn vision_helper(cache: &Path) -> Result<PathBuf, OcrError> {
    if let Some(path) = std::env::var_os("SAFHA_VISION_HELPER") {
        return Ok(PathBuf::from(path));
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in VISION_SOURCE.bytes() {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    let program = cache.join(format!("mirsam-vision-{hash:016x}"));
    if program.is_file() {
        return Ok(program);
    }
    std::fs::create_dir_all(cache).map_err(|err| OcrError::StartFailed(format!("helper folder: {err}")))?;
    let source = cache.join(format!("mirsam-vision-{hash:016x}.swift"));
    std::fs::write(&source, VISION_SOURCE)
        .map_err(|err| OcrError::StartFailed(format!("helper source: {err}")))?;
    let building = cache.join(format!("mirsam-vision-{hash:016x}.{}.part", std::process::id()));
    let output = Command::new("xcrun")
        .args(["swiftc", "-O", "-o"])
        .arg(&building)
        .arg(&source)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => OcrError::EngineMissing(PathBuf::from("xcrun")),
            _ => OcrError::StartFailed(err.to_string()),
        })?;
    let _ = std::fs::remove_file(&source);
    if !output.status.success() {
        let _ = std::fs::remove_file(&building);
        let message: String = String::from_utf8_lossy(&output.stderr).chars().take(300).collect();
        return Err(OcrError::StartFailed(format!("building the Vision helper: {message}")));
    }
    std::fs::rename(&building, &program).map_err(|err| OcrError::StartFailed(format!("helper: {err}")))?;
    Ok(program)
}

/// Apple Vision through the helper, which stays running for the pages of a job.
pub struct VisionReader {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl VisionReader {
    pub fn start(program: &Path) -> Result<Self, OcrError> {
        let mut child = Command::new(program)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|err| match err.kind() {
                std::io::ErrorKind::NotFound => OcrError::EngineMissing(program.to_owned()),
                _ => OcrError::StartFailed(err.to_string()),
            })?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            let _ = child.kill();
            return Err(OcrError::StartFailed("the Vision helper has no pipes".into()));
        };
        Ok(Self { child, stdin, stdout: BufReader::new(stdout) })
    }
}

impl Drop for VisionReader {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Deserialize)]
struct VisionAnswer {
    #[serde(default)]
    lines: Vec<DraftLine>,
    error: Option<String>,
}

impl DraftReader for VisionReader {
    fn name(&self) -> &str {
        "apple-vision"
    }

    fn read(&mut self, image: &OcrImage) -> Result<DraftPage, OcrError> {
        let path = temp_png(image)?;
        let result = (|| {
            let line = path.to_str().ok_or(OcrError::Image("temporary path is not UTF-8"))?;
            writeln!(self.stdin, "{line}")
                .and_then(|()| self.stdin.flush())
                .map_err(|err| OcrError::Exited(format!("the Vision helper stopped: {err}")))?;
            let mut answer = String::new();
            let read = self.stdout.read_line(&mut answer).map_err(|err| OcrError::Exited(err.to_string()))?;
            if read == 0 {
                return Err(OcrError::Exited("the Vision helper stopped".into()));
            }
            let answer: VisionAnswer = serde_json::from_str(&answer)
                .map_err(|err| OcrError::BadResponse(format!("Vision helper: {err}")))?;
            if let Some(error) = answer.error {
                return Err(OcrError::BadResponse(format!("Vision: {error}")));
            }
            Ok(DraftPage { lines: answer.lines })
        })();
        let _ = std::fs::remove_file(&path);
        result
    }
}

// ---------- Tesseract ----------

/// Tesseract (`ara+eng`), one process per page, read as TSV.
pub struct TesseractReader {
    pub program: PathBuf,
    /// `ara+eng`.
    pub languages: String,
    /// Where `ara.traineddata` and `eng.traineddata` are, if not Tesseract's own folder.
    pub data: Option<PathBuf>,
    /// The page image's resolution, which Tesseract uses to size its analysis.
    pub dpi: u32,
}

impl DraftReader for TesseractReader {
    fn name(&self) -> &str {
        "tesseract"
    }

    fn read(&mut self, image: &OcrImage) -> Result<DraftPage, OcrError> {
        let path = temp_png(image)?;
        let mut command = Command::new(&self.program);
        // Options before the config name: Tesseract reads every argument after
        // the first config ("tsv") as another config file.
        command.arg(&path).arg("stdout");
        if let Some(data) = &self.data {
            command.arg("--tessdata-dir").arg(data);
        }
        command.args(["-l", &self.languages, "--dpi", &self.dpi.to_string(), "tsv"]);
        let output = command.stdin(Stdio::null()).output();
        let _ = std::fs::remove_file(&path);
        let output = output.map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => OcrError::EngineMissing(self.program.clone()),
            _ => OcrError::StartFailed(err.to_string()),
        })?;
        if !output.status.success() {
            let message: String = String::from_utf8_lossy(&output.stderr).chars().take(300).collect();
            return Err(OcrError::Exited(format!("{}: {message}", output.status)));
        }
        let text = String::from_utf8(output.stdout).map_err(|_| OcrError::BadResponse("not UTF-8".into()))?;
        Ok(parse_tesseract_tsv(&text))
    }
}

impl TesseractReader {
    /// Arabic and English (`ara+eng`) at 200 dpi, from `data` when given.
    pub fn new(program: PathBuf, data: Option<PathBuf>) -> Self {
        Self { program, languages: "ara+eng".into(), data, dpi: 200 }
    }
}

/// The models a Tesseract draft reads, as the catalog's `tesseract-best`
/// package installs them (ADR 0034).
pub const TESSERACT_MODELS: [&str; 2] = ["ara.traineddata", "eng.traineddata"];

/// Where a `tesseract` program may be, the first tried first: `SAFHA_TESSERACT`
/// alone when it is set; else the `PATH` (Linux distributions' packages), and
/// on Windows the folders Tesseract's official installer uses, for all users
/// and for one.
pub fn tesseract_programs() -> Vec<PathBuf> {
    if let Some(program) = std::env::var_os("SAFHA_TESSERACT") {
        return vec![PathBuf::from(program)];
    }
    let mut programs = vec![PathBuf::from("tesseract")];
    if cfg!(windows) {
        for (variable, folder) in [("ProgramFiles", ""), ("LOCALAPPDATA", "Programs")] {
            if let Some(base) = std::env::var_os(variable) {
                programs.push(PathBuf::from(base).join(folder).join("Tesseract-OCR").join("tesseract.exe"));
            }
        }
    }
    programs
}

/// The first of `programs` that runs and reads Arabic and English: from
/// `models` (the downloaded `tessdata_best` models) when that folder holds
/// both, else from the program's own data when it has both. `None` when no
/// program can.
pub fn find_tesseract(programs: &[PathBuf], models: Option<&Path>) -> Option<TesseractReader> {
    let models = models.filter(|dir| TESSERACT_MODELS.iter().all(|file| dir.join(file).is_file()));
    programs.iter().find_map(|program| {
        let mut command = Command::new(program);
        if let Some(dir) = models {
            command.arg("--tessdata-dir").arg(dir);
        }
        let listed = command.arg("--list-langs").stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
        let listed = String::from_utf8_lossy(&listed.stdout).into_owned();
        let has = |code: &str| listed.lines().any(|line| line.trim() == code);
        (has("ara") && has("eng"))
            .then(|| TesseractReader::new(program.clone(), models.map(Path::to_path_buf)))
    })
}

/// Block, paragraph and line number of a Tesseract row.
type LineKey = (u32, u32, u32);

/// Tesseract's TSV: one row per word (level 5), grouped into lines by block,
/// paragraph and line number. Words come left to right; a right-to-left line's
/// words are put back in logical order.
pub fn parse_tesseract_tsv(tsv: &str) -> DraftPage {
    let mut lines: Vec<(LineKey, Vec<(DraftWord, f32)>)> = Vec::new();
    for row in tsv.lines().skip(1) {
        let cells: Vec<&str> = row.split('\t').collect();
        if cells.len() < 12 || cells[0] != "5" {
            continue;
        }
        let number = |i: usize| cells[i].trim().parse::<f32>().unwrap_or(0.0);
        let text = cells[11].trim();
        if text.is_empty() {
            continue;
        }
        let key = (number(2) as u32, number(3) as u32, number(4) as u32);
        let (left, top, width, height) = (number(6), number(7), number(8), number(9));
        let word = DraftWord { text: text.to_owned(), bounds: Some([left, top, left + width, top + height]) };
        let confidence = (number(10) / 100.0).clamp(0.0, 1.0);
        match lines.last_mut() {
            Some((last, words)) if *last == key => words.push((word, confidence)),
            _ => lines.push((key, vec![(word, confidence)])),
        }
    }
    DraftPage {
        lines: lines
            .into_iter()
            .map(|(_, mut words)| {
                let joined: String = words.iter().map(|(w, _)| w.text.as_str()).collect::<Vec<_>>().join(" ");
                if first_strong_rtl(&joined) == Some(true) {
                    words.reverse();
                }
                let boxes: Vec<PixelBox> = words.iter().filter_map(|(w, _)| w.bounds).collect();
                let bounds = [
                    boxes.iter().map(|b| b[0]).fold(f32::INFINITY, f32::min),
                    boxes.iter().map(|b| b[1]).fold(f32::INFINITY, f32::min),
                    boxes.iter().map(|b| b[2]).fold(f32::NEG_INFINITY, f32::max),
                    boxes.iter().map(|b| b[3]).fold(f32::NEG_INFINITY, f32::max),
                ];
                let confidence = words.iter().map(|(_, c)| *c).sum::<f32>() / words.len().max(1) as f32;
                let words: Vec<DraftWord> = words.into_iter().map(|(w, _)| w).collect();
                DraftLine {
                    text: words.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" "),
                    confidence,
                    bounds,
                    words,
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tesseract_rows_become_lines_in_logical_order() {
        let tsv = "level\tpage_num\tblock_num\tpar_num\tline_num\tword_num\tleft\ttop\twidth\theight\tconf\ttext\n\
                   1\t1\t0\t0\t0\t0\t0\t0\t600\t800\t-1\t\n\
                   5\t1\t1\t1\t1\t1\t300\t100\t50\t20\t91\tعليكم\n\
                   5\t1\t1\t1\t1\t2\t360\t100\t40\t20\t95\tالسلام\n\
                   5\t1\t1\t1\t2\t1\t50\t140\t60\t20\t80\tHello\n\
                   5\t1\t1\t1\t2\t2\t120\t140\t60\t20\t70\tworld\n";
        let page = parse_tesseract_tsv(tsv);
        assert_eq!(page.text(), "السلام عليكم\nHello world");
        assert_eq!(page.lines[0].bounds, [300.0, 100.0, 400.0, 120.0]);
        assert_eq!(page.lines[0].words[0].bounds, Some([360.0, 100.0, 400.0, 120.0]));
        assert!((page.lines[1].confidence - 0.75).abs() < 1e-6);
    }

    /// A stand-in `tesseract`: it notes its arguments, lists Arabic only from
    /// its own data and both languages from a `--tessdata-dir`, and reads every
    /// page as one Arabic word.
    #[cfg(unix)]
    fn fake_tesseract(dir: &Path) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let program = dir.join("tesseract");
        std::fs::write(
            &program,
            "#!/bin/sh\n\
             echo \"$@\" >> \"$0.args\"\n\
             case \"$*\" in\n\
             *--list-langs*) echo 'List of available languages (2):'; echo ara;\n\
               case \"$*\" in *--tessdata-dir*) echo eng;; esac;;\n\
             *) printf 'level\\tpage_num\\tblock_num\\tpar_num\\tline_num\\tword_num\\tleft\\ttop\\twidth\\theight\\tconf\\ttext\\n';\n\
                printf '5\\t1\\t1\\t1\\t1\\t1\\t4\\t2\\t10\\t6\\t90\\tمسودة\\n';;\n\
             esac\n",
        )
        .expect("write");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        program
    }

    #[cfg(unix)]
    #[test]
    fn tesseract_reads_the_downloaded_models_when_they_are_installed() {
        let dir = std::env::temp_dir().join(format!("mirsam-fake-tesseract-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let models = dir.join("tesseract-best");
        std::fs::create_dir_all(&models).expect("dir");
        let program = fake_tesseract(&dir);
        let missing = dir.join("no-such-tesseract");

        // Its own data lacks English, and the models are not all there yet: no draft.
        std::fs::write(models.join("ara.traineddata"), b"model").expect("write");
        assert!(find_tesseract(&[missing.clone(), program.clone()], Some(&models)).is_none());
        // Installed: the first program that runs reads them.
        std::fs::write(models.join("eng.traineddata"), b"model").expect("write");
        let mut reader = find_tesseract(&[missing, program.clone()], Some(&models)).expect("a reader");
        assert_eq!(reader.program, program);
        assert_eq!(reader.data.as_deref(), Some(models.as_path()));
        assert_eq!(reader.languages, "ara+eng");

        let image = OcrImage::from_rgb(2, 2, vec![255; 12]).expect("valid");
        let page = reader.read(&image).expect("reads");
        assert_eq!(page.text(), "مسودة");
        let args = std::fs::read_to_string(dir.join("tesseract.args")).expect("args");
        let read = args.lines().last().expect("a page read");
        // The data folder comes before the config, or Tesseract takes it for one.
        let expected = format!("stdout --tessdata-dir {} -l ara+eng --dpi 200 tsv", models.display());
        assert!(read.ends_with(&expected), "{read}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn vision_answers_parse() {
        let answer: VisionAnswer = serde_json::from_str(
            r#"{"lines":[{"text":"نص عربي","confidence":0.5,"box":[1,2,3,4],"words":[{"text":"نص","box":[2,2,3,4]},{"text":"عربي"}]}]}"#,
        )
        .expect("parses");
        assert_eq!(answer.lines[0].words[1].bounds, None);
        assert_eq!(first_strong_rtl("123 نص"), Some(true));
        assert_eq!(first_strong_rtl("(A) نص"), Some(false));
        assert_eq!(first_strong_rtl("123"), None);
    }
}
