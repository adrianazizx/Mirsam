//! OCR of a whole document through the app's own background job system (ADR 0035),
//! without the window: the M4 exit test's "a scanned document becomes searchable".
//!
//!   cargo run --release -p mirsam-core --example ocr_document -- <in.pdf> <out.pdf> \
//!       --models <model store> --state <dir> [--stop-after N]
//!
//! `--models` is a store the app's download manager filled (Settings, or
//! `ocr_smoke --models <dir> --package …`). `--state` holds the job store
//! (`state.sqlite3`), as the app's state directory does. The job runs as in the app:
//! the draft pass over every page, then the quality pass page by page, each result
//! kept in the store and written into the document as an edit. `--stop-after N`
//! quits after N quality pages, as closing the app would; running the same command
//! again resumes the job: the pages already read are written back from the store
//! without reading them again, and the rest are read. The result is saved to
//! `<out.pdf>`.
//!
//! `make-scan <in.pdf> <out.pdf> [dpi]` first turns a PDF into a scan: each page an
//! image, no text.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mirsam_core::models::ModelStore;
use mirsam_core::ocr::jobs::{
    JobBook, JobHost, JobRunner, JobState, JobStatus, LocalEngines, MAX_SIDE, RenderedPage,
};
use mirsam_core::ocr::{LayerQuality, OcrImage, PageLayer};
use mirsam_core::store::{Store, canonical_path};
use mirsam_core::{Document, Engine, PdfiumSource};

struct Host {
    path: PathBuf,
    document: Mutex<Arc<Document>>,
    started: Instant,
    stop_after: Option<usize>,
    finished: Mutex<usize>,
    quit: Mutex<Option<std::sync::mpsc::Sender<()>>>,
}

impl JobHost for Host {
    fn page_count(&self, path: &Path) -> Option<usize> {
        (path == self.path).then(|| self.document.lock().map(|d| d.page_count()).unwrap_or(0))
    }

    fn render(&self, _path: &Path, page: usize, scale: f32) -> Result<RenderedPage, String> {
        let document = self.document.lock().map_err(|_| "lock")?.clone();
        let size = document.page_size(page).map_err(|e| e.to_string())?;
        let scale = scale.min(MAX_SIDE / size.width.max(size.height));
        let bitmap = document.render_page(page, scale).map_err(|e| e.to_string())?;
        Ok(RenderedPage {
            image: OcrImage::from_bitmap(&bitmap),
            scale,
            placement: document.ocr_placement(page).map_err(|e| e.to_string())?,
        })
    }

    fn layers(&self, _path: &Path) -> Option<Vec<Option<LayerQuality>>> {
        self.document.lock().ok()?.text_layer_qualities().ok()
    }

    fn apply(&self, _path: &Path, layers: &[PageLayer]) -> Result<bool, String> {
        let mut document = self.document.lock().map_err(|_| "lock")?;
        let (next, update) = document.with_text_layers(layers).map_err(|e| e.to_string())?;
        *document = Arc::new(next);
        let pages: Vec<String> = layers.iter().map(|l| format!("{}:{:?}", l.page + 1, l.quality)).collect();
        println!(
            "{:7.1} s  wrote {} ({} bytes)",
            self.started.elapsed().as_secs_f64(),
            pages.join(" "),
            update.len()
        );
        Ok(true)
    }

    fn changed(&self, status: &JobStatus) {
        let finished = status.finished;
        let mut last = self.finished.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if finished != *last {
            *last = finished;
            if self.stop_after.is_some_and(|n| finished >= n)
                && let Some(quit) = self.quit.lock().ok().and_then(|mut q| q.take())
            {
                let _ = quit.send(());
            }
        }
        println!(
            "{:7.1} s  job {} {:?}: {}/{} drafted, {}/{} final{}",
            self.started.elapsed().as_secs_f64(),
            status.id,
            status.state,
            status.drafted,
            status.pages,
            status.finished,
            status.pages,
            status.current.map(|(p, pass)| format!(", page {} {pass:?}", p + 1)).unwrap_or_default()
        );
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let engine = Engine::global(&PdfiumSource::Auto)?;
    if args.first().map(String::as_str) == Some("make-scan") {
        let (input, output) = (args.get(1).ok_or("make-scan <in> <out> [dpi]")?, args.get(2).ok_or("out")?);
        let dpi: f32 = args.get(3).map(|d| d.parse()).transpose()?.unwrap_or(200.0);
        return make_scan(engine, Path::new(input), Path::new(output), dpi);
    }
    let (mut positional, mut models, mut state, mut stop_after) = (Vec::new(), None, None, None);
    let mut rest = args.into_iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--models" => models = rest.next().map(PathBuf::from),
            "--state" => state = rest.next().map(PathBuf::from),
            "--stop-after" => stop_after = rest.next().map(|n| n.parse()).transpose()?,
            _ => positional.push(PathBuf::from(arg)),
        }
    }
    let [input, output] = positional.as_slice() else { return Err("<in.pdf> <out.pdf> required".into()) };
    let state = state.ok_or("--state <dir> required")?;
    std::fs::create_dir_all(&state)?;
    let book = Arc::new(Store::open(&state.join("state.sqlite3"))?);
    let document = engine.open_bytes(std::fs::read(input)?, None)?;
    let path = canonical_path(input);
    let page_count = document.page_count();
    let (quit, quitting) = std::sync::mpsc::channel();
    let host = Arc::new(Host {
        path: path.clone(),
        document: Mutex::new(Arc::new(document)),
        started: Instant::now(),
        stop_after,
        finished: Mutex::new(usize::MAX),
        quit: Mutex::new(Some(quit)),
    });
    let engines = LocalEngines::new(
        ModelStore::new(models.ok_or("--models <dir> required")?),
        std::env::temp_dir().join("mirsam-helpers"),
        Some(state.join("ocr-logs")),
    );
    std::fs::create_dir_all(state.join("ocr-logs"))?;
    let runner = JobRunner::start(book.clone(), host.clone(), Box::new(engines))?;
    let job = match runner
        .jobs(&path)
        .into_iter()
        .find(|j| !matches!(j.state, JobState::Done | JobState::Failed))
    {
        Some(job) => {
            println!("resuming job {} ({:?}, {} of {} final)", job.id, job.state, job.finished, job.pages);
            if job.state != JobState::Queued {
                runner.resume(job.id);
            }
            runner.wake();
            job
        }
        None => runner.submit(
            &path,
            "document",
            page_count,
            &(0..page_count).collect::<Vec<_>>(),
            mirsam_core::ocr::jobs::JobOptions::default(),
        )?,
    };
    loop {
        if quitting.recv_timeout(Duration::from_millis(200)).is_ok() {
            println!("stopping after {} final pages, as the app quitting would", stop_after.unwrap_or(0));
            runner.shutdown();
            break;
        }
        let status = runner.status(job.id).ok_or("job vanished")?;
        if matches!(status.state, JobState::Done | JobState::Failed | JobState::Cancelled) {
            println!("job {:?} {}", status.state, status.error.unwrap_or_default());
            break;
        }
    }
    let bytes = host.document.lock().map_err(|_| "lock")?.bytes();
    std::fs::write(output, &*bytes)?;
    let states = host.document.lock().map_err(|_| "lock")?.text_layer_states()?;
    let review = runner.review(&path, &states, mirsam_core::ocr::ReviewFilter::default())?;
    println!("saved {} ({} bytes); {} pages on the review list", output.display(), bytes.len(), review.len());
    for page in review {
        println!(
            "  page {}: {} unsure, {} corrected, {} reordered, {:.0}% confident",
            page.page + 1,
            page.uncertain,
            page.corrected,
            page.reordered,
            page.confidence * 100.0
        );
    }
    let jobs = book.jobs()?;
    println!(
        "store: {} job(s), {} page results",
        jobs.len(),
        jobs.iter().map(|j| book.pages(j.id).map(|p| p.len()).unwrap_or(0)).sum::<usize>()
    );
    Ok(())
}

/// Each page of `input` rendered at `dpi` and put on a page of its own as one
/// image: a scan, with no text to extract.
fn make_scan(
    engine: &'static Engine,
    input: &Path,
    output: &Path,
    dpi: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    let source = engine.open_bytes(std::fs::read(input)?, None)?;
    let mut objects: Vec<Vec<u8>> = vec![Vec::new(), Vec::new()];
    let mut kids = Vec::new();
    for page in 0..source.page_count() {
        let size = source.page_size(page)?;
        let image = OcrImage::from_bitmap(&source.render_page(page, dpi / 72.0)?);
        let data = miniz_oxide::deflate::compress_to_vec_zlib(image.rgb(), 6);
        let image_number = objects.len() + 1;
        let mut xobject = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /FlateDecode /Length {} >>\nstream\n",
            image.width(),
            image.height(),
            data.len()
        )
        .into_bytes();
        xobject.extend_from_slice(&data);
        xobject.extend_from_slice(b"\nendstream");
        objects.push(xobject);
        let content = format!("q {:.3} 0 0 {:.3} 0 0 cm /Im0 Do Q\n", size.width, size.height);
        objects.push(format!("<< /Length {} >>\nstream\n{content}endstream", content.len()).into_bytes());
        let content_number = objects.len();
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.3} {:.3}] /Resources << /XObject << /Im0 {image_number} 0 R >> >> /Contents {content_number} 0 R >>",
                size.width, size.height
            )
            .into_bytes(),
        );
        kids.push(format!("{} 0 R", objects.len()));
    }
    objects[0] = b"<< /Type /Catalog /Pages 2 0 R >>".to_vec();
    objects[1] = format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), kids.len()).into_bytes();
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f\r\n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n\r\n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1)
            .as_bytes(),
    );
    std::fs::write(output, out)?;
    println!("{} pages at {dpi} dpi -> {}", source.page_count(), output.display());
    Ok(())
}
