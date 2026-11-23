//! Nuqta-Arabic-OCR through llama.cpp's `llama-server`, run as a child process
//! (ADR 0032).
//!
//! The server listens on `127.0.0.1` on a free port and accepts only requests that
//! carry a random key made for this run. It is stopped when [`LlamaServer`] is
//! dropped, which gives the model's memory (about 11 GiB for the 4B) back to the
//! system.

use std::ffi::OsString;
use std::fs::File;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine as _;
use serde_json::{Value, json};

use super::image::{OcrImage, prepare_nuqta};
use super::text::{Token, to_text, word_confidences};
use super::{OcrEngine, OcrError, Recognition, Timings};

/// The model card's prompt ("as written": tashkeel only where it is printed).
pub const NUQTA_PROMPT: &str = "Transcribe the Arabic text in this image exactly as written. \
     Include only the diacritics (tashkeel) that are visible in the image.";

/// The longest answer allowed; a full page is about 130–1,000 tokens.
const MAX_TOKENS: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NuqtaSize {
    /// Nuqta-Arabic-OCR 4B: the recogniser (ADR 0030).
    B4,
    /// Nuqta-Arabic-OCR 0.8B: the second reader, and the recogniser where memory
    /// cannot hold the 4B.
    B0_8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NuqtaQuant {
    /// 8-bit: the same text as the full model on ADR 0030's set. The one to ship.
    Q8_0,
    /// 4-bit: smaller, less accurate on degraded mixed text and digits.
    Q4KM,
}

/// One of the authors' GGUF builds (`SalahALHaismawi/Nuqta-Arabic-OCR-*-GGUF`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NuqtaModel {
    pub size: NuqtaSize,
    pub quant: NuqtaQuant,
}

impl NuqtaModel {
    /// The recogniser.
    pub const DEFAULT: Self = Self { size: NuqtaSize::B4, quant: NuqtaQuant::Q8_0 };
    /// The second reader.
    pub const PARTNER: Self = Self { size: NuqtaSize::B0_8, quant: NuqtaQuant::Q8_0 };

    fn size_tag(self) -> &'static str {
        match self.size {
            NuqtaSize::B4 => "4b",
            NuqtaSize::B0_8 => "0.8b",
        }
    }

    fn quant_tag(self) -> &'static str {
        match self.quant {
            NuqtaQuant::Q8_0 => "Q8_0",
            NuqtaQuant::Q4KM => "Q4_K_M",
        }
    }

    /// `nuqta-4b-q8_0`: the engine's name in reports and baselines.
    pub fn name(self) -> String {
        format!("nuqta-{}-{}", self.size_tag(), self.quant_tag().to_ascii_lowercase())
    }

    /// Parses [`NuqtaModel::name`].
    pub fn from_name(name: &str) -> Option<Self> {
        [NuqtaSize::B4, NuqtaSize::B0_8]
            .into_iter()
            .flat_map(|size| [NuqtaQuant::Q8_0, NuqtaQuant::Q4KM].map(|quant| Self { size, quant }))
            .find(|model| model.name() == name)
    }

    /// The language model's file name in the GGUF repository.
    pub fn model_file(self) -> String {
        format!("nuqta-ocr-{}-{}.gguf", self.size_tag(), self.quant_tag())
    }

    /// The vision encoder's file name (always needed beside the model).
    pub fn mmproj_file(self) -> String {
        format!("nuqta-ocr-{}-mmproj-f16.gguf", self.size_tag())
    }
}

/// Where the model runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    /// The GPU where llama.cpp has a backend for it (Metal on macOS).
    Gpu,
    /// The CPU only, vision encoder included.
    Cpu,
}

/// How to start `llama-server`.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// The `llama-server` program: a path, or a bare name looked up on `PATH`.
    pub program: PathBuf,
    pub model: PathBuf,
    pub mmproj: PathBuf,
    pub device: Device,
    /// CPU threads; `None` lets llama.cpp choose.
    pub threads: Option<u32>,
    /// Context size in tokens: enough for a 2.4 MP page and its text.
    pub context: u32,
    /// The most tokens an image may take, so llama.cpp does not shrink the page again.
    pub image_max_tokens: u32,
    /// How long loading may take (several GB from disk).
    pub startup_timeout: Duration,
    /// How long one page may take (minutes on a slow CPU).
    pub request_timeout: Duration,
    /// Where the server's own output goes; discarded when `None`.
    pub log: Option<PathBuf>,
}

impl ServerConfig {
    /// The model card's settings for `model` and `mmproj`.
    pub fn new(program: impl Into<PathBuf>, model: impl Into<PathBuf>, mmproj: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            model: model.into(),
            mmproj: mmproj.into(),
            device: Device::Gpu,
            threads: None,
            context: 8192,
            image_max_tokens: 4096,
            startup_timeout: Duration::from_secs(600),
            request_timeout: Duration::from_secs(3600),
            log: None,
        }
    }

    /// The files of `model` in `dir`.
    pub fn for_model(program: impl Into<PathBuf>, dir: &Path, model: NuqtaModel) -> Self {
        Self::new(program, dir.join(model.model_file()), dir.join(model.mmproj_file()))
    }

    /// The command line for one run.
    pub fn arguments(&self, port: u16, api_key: &str) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec![
            "-m".into(),
            self.model.clone().into(),
            "--mmproj".into(),
            self.mmproj.clone().into(),
            "-c".into(),
            self.context.to_string().into(),
            "--image-max-tokens".into(),
            self.image_max_tokens.to_string().into(),
            "--temp".into(),
            "0".into(),
            "--host".into(),
            "127.0.0.1".into(),
            "--port".into(),
            port.to_string().into(),
            "--api-key".into(),
            api_key.into(),
            "--parallel".into(),
            "1".into(),
            "--no-webui".into(),
        ];
        if self.device == Device::Cpu {
            args.extend(["-ngl", "0", "--no-mmproj-offload"].map(OsString::from));
        }
        if let Some(threads) = self.threads {
            args.extend(["-t".into(), threads.to_string().into()]);
        }
        args
    }
}

/// A running `llama-server`, stopped on drop.
pub struct LlamaServer {
    child: Mutex<Child>,
    base: String,
    authorization: String,
    agent: ureq::Agent,
}

impl std::fmt::Debug for LlamaServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlamaServer").field("base", &self.base).finish_non_exhaustive()
    }
}

impl LlamaServer {
    /// Starts the server and waits until the model is loaded.
    pub fn start(config: &ServerConfig) -> Result<Self, OcrError> {
        for file in [&config.model, &config.mmproj] {
            if !file.is_file() {
                return Err(OcrError::ModelMissing(file.clone()));
            }
        }
        if config.program.components().count() > 1 && !config.program.is_file() {
            return Err(OcrError::EngineMissing(config.program.clone()));
        }
        // A free port can be taken between choosing it and the server binding it;
        // a server that stops at once is retried on another port.
        let mut last = None;
        for _ in 0..3 {
            match Self::start_once(config) {
                Err(OcrError::Exited(reason)) if last.is_none() || reason.contains("bind") => {
                    last = Some(OcrError::Exited(reason));
                }
                other => return other,
            }
        }
        Err(last.unwrap_or_else(|| OcrError::StartFailed("no attempt made".into())))
    }

    fn start_once(config: &ServerConfig) -> Result<Self, OcrError> {
        let port = free_port().map_err(|err| OcrError::StartFailed(format!("no free port: {err}")))?;
        let api_key = random_key().map_err(|err| OcrError::StartFailed(format!("no random key: {err}")))?;
        let (stdout, stderr) = match &config.log {
            Some(path) => {
                let file =
                    File::create(path).map_err(|err| OcrError::StartFailed(format!("log file: {err}")))?;
                let copy =
                    file.try_clone().map_err(|err| OcrError::StartFailed(format!("log file: {err}")))?;
                (Stdio::from(file), Stdio::from(copy))
            }
            None => (Stdio::null(), Stdio::null()),
        };
        let child = Command::new(&config.program)
            .args(config.arguments(port, &api_key))
            .stdin(Stdio::null())
            .stdout(stdout)
            .stderr(stderr)
            .spawn()
            .map_err(|err| match err.kind() {
                ErrorKind::NotFound => OcrError::EngineMissing(config.program.clone()),
                _ => OcrError::StartFailed(err.to_string()),
            })?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(config.request_timeout))
            .http_status_as_error(false)
            .build()
            .into();
        let server = Self {
            child: Mutex::new(child),
            base: format!("http://127.0.0.1:{port}"),
            authorization: format!("Bearer {api_key}"),
            agent,
        };
        server.wait_ready(config.startup_timeout, config.log.as_deref())?;
        Ok(server)
    }

    fn wait_ready(&self, timeout: Duration, log: Option<&Path>) -> Result<(), OcrError> {
        let started = Instant::now();
        let probe: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(2)))
            .http_status_as_error(false)
            .build()
            .into();
        loop {
            if let Some(reason) = self.exited() {
                return Err(OcrError::Exited(with_log_tail(reason, log)));
            }
            // 503 while the model loads, 200 once it can answer.
            if let Ok(response) = probe.get(format!("{}/health", self.base)).call()
                && response.status().as_u16() == 200
            {
                return Ok(());
            }
            if started.elapsed() > timeout {
                return Err(OcrError::Timeout("the model did not finish loading"));
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    /// Stops the server now, from any thread: a request in flight fails
    /// (`ocr_engine_stopped`). Used to cancel an OCR job mid-page.
    pub fn stop(&self) {
        let mut child = self.child.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = child.kill();
    }

    /// Whether the server is still running.
    pub fn is_running(&self) -> bool {
        self.exited().is_none()
    }

    /// `Some(reason)` once the process has stopped.
    fn exited(&self) -> Option<String> {
        let mut child = self.child.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        match child.try_wait() {
            Ok(Some(status)) => Some(status.to_string()),
            Ok(None) => None,
            Err(err) => Some(err.to_string()),
        }
    }

    /// Sends one OpenAI-style chat request and returns the parsed answer.
    pub fn chat(&self, body: &Value) -> Result<Value, OcrError> {
        let result = self
            .agent
            .post(format!("{}/v1/chat/completions", self.base))
            .header("Authorization", &self.authorization)
            .header("Content-Type", "application/json")
            .send(body.to_string());
        let mut response = match result {
            Ok(response) => response,
            Err(err) => {
                if let Some(reason) = self.exited() {
                    return Err(OcrError::Exited(reason));
                }
                return Err(match err {
                    ureq::Error::Timeout(_) => OcrError::Timeout("no answer for the page"),
                    other => OcrError::BadResponse(other.to_string()),
                });
            }
        };
        let status = response.status().as_u16();
        let text = response
            .body_mut()
            .with_config()
            .limit(256 * 1024 * 1024)
            .read_to_string()
            .map_err(|err| OcrError::BadResponse(err.to_string()))?;
        if status != 200 {
            let excerpt: String = text.chars().take(300).collect();
            return Err(OcrError::BadResponse(format!("HTTP {status}: {excerpt}")));
        }
        serde_json::from_str(&text).map_err(|err| OcrError::BadResponse(format!("not JSON: {err}")))
    }
}

impl Drop for LlamaServer {
    fn drop(&mut self) {
        let mut child = self.child.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn free_port() -> std::io::Result<u16> {
    Ok(TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port())
}

fn random_key() -> Result<String, getrandom::Error> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// The last lines of the server's log, which say why it stopped.
fn with_log_tail(reason: String, log: Option<&Path>) -> String {
    let Some(text) = log.and_then(|path| std::fs::read_to_string(path).ok()) else { return reason };
    let tail: Vec<&str> = text.lines().rev().take(5).collect();
    let tail: Vec<&str> = tail.into_iter().rev().collect();
    format!("{reason}; log: {}", tail.join(" | "))
}

/// An answer from `/v1/chat/completions`.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatAnswer {
    pub content: String,
    /// `finish_reason` was `length`: cut off at the token limit.
    pub truncated: bool,
    /// The generated tokens with their probabilities (empty without `logprobs`).
    pub tokens: Vec<Token>,
    pub prompt_tokens: u32,
    pub prompt: Duration,
    pub generated_tokens: u32,
    pub generation: Duration,
}

/// Reads an OpenAI-style chat answer as `llama-server` writes it: the message,
/// `logprobs.content[].bytes`/`logprob`, and its own `timings`.
pub fn parse_chat_response(value: &Value) -> Result<ChatAnswer, OcrError> {
    let choice = value
        .get("choices")
        .and_then(|choices| choices.get(0))
        .ok_or_else(|| OcrError::BadResponse("no choices in the answer".into()))?;
    let content = choice
        .pointer("/message/content")
        .and_then(Value::as_str)
        .ok_or_else(|| OcrError::BadResponse("no message content".into()))?
        .to_owned();
    let truncated = choice.get("finish_reason").and_then(Value::as_str) == Some("length");
    let tokens = choice
        .pointer("/logprobs/content")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let bytes = entry
                        .get("bytes")
                        .and_then(Value::as_array)?
                        .iter()
                        .map(|b| b.as_u64().and_then(|b| u8::try_from(b).ok()))
                        .collect::<Option<Vec<u8>>>()?;
                    let logprob = entry.get("logprob").and_then(Value::as_f64)?;
                    Some(Token { bytes, probability: logprob.exp().clamp(0.0, 1.0) as f32 })
                })
                .collect()
        })
        .unwrap_or_default();
    let timings = value.get("timings");
    let number = |key: &str| timings.and_then(|t| t.get(key)).and_then(Value::as_f64).unwrap_or(0.0);
    let millis = |key: &str| Duration::from_secs_f64(number(key).max(0.0) / 1000.0);
    Ok(ChatAnswer {
        content,
        truncated,
        tokens,
        prompt_tokens: number("prompt_n") as u32,
        prompt: millis("prompt_ms"),
        generated_tokens: number("predicted_n") as u32,
        generation: millis("predicted_ms"),
    })
}

/// Nuqta-Arabic-OCR behind a running `llama-server`.
#[derive(Debug)]
pub struct NuqtaEngine {
    server: LlamaServer,
    name: String,
}

impl NuqtaEngine {
    /// Starts `llama-server` for `model` (the paths in `config` must be its files).
    pub fn start(config: &ServerConfig, model: NuqtaModel) -> Result<Self, OcrError> {
        Ok(Self { server: LlamaServer::start(config)?, name: model.name() })
    }

    /// Stops its server now (see [`LlamaServer::stop`]).
    pub fn stop(&self) {
        self.server.stop();
    }

    /// Whether its server is still running.
    pub fn is_running(&self) -> bool {
        self.server.is_running()
    }

    /// The request for one image, as the model card prescribes.
    pub fn request(image: &OcrImage) -> Result<Value, OcrError> {
        let png = prepare_nuqta(image)?.encode_png()?;
        let url = format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png));
        Ok(json!({
            "messages": [{
                "role": "user",
                "content": [
                    { "type": "image_url", "image_url": { "url": url } },
                    { "type": "text", "text": NUQTA_PROMPT },
                ],
            }],
            "temperature": 0,
            "max_tokens": MAX_TOKENS,
            "logprobs": true,
            "chat_template_kwargs": { "enable_thinking": false },
        }))
    }
}

impl OcrEngine for NuqtaEngine {
    fn name(&self) -> &str {
        &self.name
    }

    fn recognize(&self, image: &OcrImage) -> Result<Recognition, OcrError> {
        let body = Self::request(image)?;
        let started = Instant::now();
        let answer = parse_chat_response(&self.server.chat(&body)?)?;
        let total = started.elapsed();
        let words = word_confidences(&answer.tokens)
            .into_iter()
            .filter(|w| w.word != "<think>" && w.word != "</think>")
            .collect();
        Ok(Recognition {
            text: to_text(&answer.content),
            raw: answer.content,
            words,
            truncated: answer.truncated,
            timings: Timings {
                total,
                prompt_tokens: answer.prompt_tokens,
                prompt: answer.prompt,
                generated_tokens: answer.generated_tokens,
                generation: answer.generation,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_names_and_files() {
        assert_eq!(NuqtaModel::DEFAULT.name(), "nuqta-4b-q8_0");
        assert_eq!(NuqtaModel::DEFAULT.model_file(), "nuqta-ocr-4b-Q8_0.gguf");
        assert_eq!(NuqtaModel::PARTNER.mmproj_file(), "nuqta-ocr-0.8b-mmproj-f16.gguf");
        let q4 = NuqtaModel { size: NuqtaSize::B0_8, quant: NuqtaQuant::Q4KM };
        assert_eq!(q4.name(), "nuqta-0.8b-q4_k_m");
        assert_eq!(q4.model_file(), "nuqta-ocr-0.8b-Q4_K_M.gguf");
        assert_eq!(NuqtaModel::from_name("nuqta-0.8b-q4_k_m"), Some(q4));
        assert_eq!(NuqtaModel::from_name("nuqta-9b"), None);
    }

    #[test]
    fn command_line_keeps_the_server_private_and_follows_the_card() {
        let config = ServerConfig::new("llama-server", "/m/model.gguf", "/m/mmproj.gguf");
        let args: Vec<String> =
            config.arguments(4321, "k3y").into_iter().map(|a| a.to_string_lossy().into_owned()).collect();
        let joined = args.join(" ");
        assert!(joined.contains("-m /m/model.gguf --mmproj /m/mmproj.gguf"));
        assert!(joined.contains("-c 8192 --image-max-tokens 4096 --temp 0"));
        assert!(joined.contains("--host 127.0.0.1 --port 4321 --api-key k3y"));
        assert!(!joined.contains("-ngl"));

        let cpu = ServerConfig { device: Device::Cpu, threads: Some(6), ..config };
        let joined =
            cpu.arguments(1, "k").into_iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>();
        let joined = joined.join(" ");
        assert!(joined.ends_with("-ngl 0 --no-mmproj-offload -t 6"), "{joined}");
    }

    #[test]
    fn missing_files_are_reported_before_starting() {
        let dir = std::env::temp_dir();
        let config =
            ServerConfig::new("llama-server", dir.join("no-such-model.gguf"), dir.join("no-such.gguf"));
        let err = LlamaServer::start(&config).expect_err("model missing");
        assert_eq!(err.code(), "ocr_model_missing");
    }

    #[test]
    fn a_missing_program_is_reported() {
        // Real files for the model paths, so the check reaches the program.
        let model = std::env::current_exe().expect("test binary");
        let config = ServerConfig::new("/no/such/dir/llama-server", &model, &model);
        assert_eq!(LlamaServer::start(&config).expect_err("program missing").code(), "ocr_engine_missing");
        let config = ServerConfig::new("safha-no-such-llama-server-program", &model, &model);
        assert_eq!(LlamaServer::start(&config).expect_err("program missing").code(), "ocr_engine_missing");
    }

    #[test]
    fn a_server_that_stops_at_once_is_reported() {
        // The test binary itself rejects llama-server's arguments and exits.
        let program = std::env::current_exe().expect("test binary");
        let mut config = ServerConfig::new(&program, &program, &program);
        config.startup_timeout = Duration::from_secs(60);
        let err = LlamaServer::start(&config).expect_err("server stops");
        assert_eq!(err.code(), "ocr_engine_stopped", "{err}");
    }

    #[test]
    fn parses_an_answer_with_token_probabilities() {
        let value = json!({
            "choices": [{
                "finish_reason": "stop",
                "message": { "role": "assistant", "content": "لا بأس" },
                "logprobs": { "content": [
                    { "token": "لا", "bytes": [217, 132, 216, 167], "logprob": -0.01 },
                    { "token": " بأ", "bytes": [32, 216, 168, 216, 163], "logprob": -0.5 },
                    { "token": "س", "bytes": [216, 179], "logprob": 0.0 },
                ]},
            }],
            "timings": { "prompt_n": 2420, "prompt_ms": 1500.0, "predicted_n": 3, "predicted_ms": 30.5 },
        });
        let answer = parse_chat_response(&value).expect("parsed");
        assert_eq!(answer.content, "لا بأس");
        assert!(!answer.truncated);
        assert_eq!(answer.tokens.len(), 3);
        assert_eq!((answer.prompt_tokens, answer.generated_tokens), (2420, 3));
        assert_eq!(answer.prompt, Duration::from_millis(1500));
        let words = word_confidences(&answer.tokens);
        assert_eq!(words.iter().map(|w| w.word.as_str()).collect::<Vec<_>>(), ["لا", "بأس"]);
        assert!((words[1].probability - (-0.5f64).exp() as f32).abs() < 1e-6);
    }

    #[test]
    fn a_cut_off_answer_is_marked_and_garbage_is_an_error() {
        let value = json!({ "choices": [{ "finish_reason": "length", "message": { "content": "x" } }] });
        assert!(parse_chat_response(&value).expect("parsed").truncated);
        assert_eq!(parse_chat_response(&json!({})).expect_err("bad").code(), "ocr_bad_response");
    }

    #[test]
    fn the_request_follows_the_card() {
        let image = OcrImage::from_rgb(4, 2, vec![255; 24]).expect("valid");
        let body = NuqtaEngine::request(&image).expect("request");
        assert_eq!(body["temperature"], 0);
        assert_eq!(body["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(body["messages"][0]["content"][1]["text"], NUQTA_PROMPT);
        let url = body["messages"][0]["content"][0]["image_url"]["url"].as_str().expect("url");
        assert!(url.starts_with("data:image/png;base64,iVBORw0KGgo"));
    }
}
