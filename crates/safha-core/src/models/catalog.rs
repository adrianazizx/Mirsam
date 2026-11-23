//! What can be downloaded: the pinned OCR models and the runtime builds
//! (ADR 0030, ADR 0032, ADR 0034).
//!
//! Every file is pinned by URL (a fixed revision or release tag), size and
//! SHA-256. Bumping a model or the runtime is a deliberate change here, like
//! bumping PDFium; the installed package's receipt records the version, so an
//! older install shows as outdated.

use serde::Serialize;

/// The recogniser: what OCR needs before it can run at all (ADR 0030).
pub const PRIMARY: &str = "nuqta-4b";
/// The second reader of the ensemble, and the recogniser on machines too small
/// for the 4B.
pub const ENSEMBLE: &str = "nuqta-0.8b";
/// llama.cpp's `llama-server`, which runs the models (ADR 0032).
pub const RUNTIME: &str = "llama-cpp";
/// Tesseract's best Arabic and English models, for the quick draft on Windows
/// and Linux (owner item 23, ADR 0035). Optional.
pub const DRAFT: &str = "tesseract-best";

/// What a package does in the OCR pipeline (spec §8, ADR 0030).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    /// The best-quality recogniser, the default on every machine that can hold it.
    Primary,
    /// The second reader: re-reads low-confidence lines and every number.
    Ensemble,
    /// The quick draft pass. Built into macOS (Apple Vision); elsewhere
    /// Tesseract, with the `tesseract-best` models ([`draft_package`]).
    Draft,
    /// The program that runs the models.
    Runtime,
}

/// How a downloaded file is installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Unpack {
    /// As it is, under its own name.
    None,
    /// A gzip-compressed tar archive, unpacked into the package's folder.
    TarGz,
    /// A zip archive, unpacked into the package's folder.
    Zip,
}

/// One downloadable file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    /// The installed file's name, or the archive's (`Unpack` other than `None`).
    pub name: String,
    pub url: String,
    /// Bytes to download.
    pub size: u64,
    /// SHA-256 of the downloaded bytes, lower-case hex.
    pub sha256: String,
    pub unpack: Unpack,
}

impl Asset {
    /// An upper bound of the disk space it takes once installed. Unpacked
    /// archives are allowed four times their size (the runtime's builds unpack to
    /// about three).
    pub fn installed_size(&self) -> u64 {
        match self.unpack {
            Unpack::None => self.size,
            Unpack::TarGz | Unpack::Zip => self.size.saturating_mul(4),
        }
    }
}

/// What the user installs or removes as one item: a model with its vision
/// encoder, or the runtime for this platform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    /// Stable id: the install folder's name and the UI's key.
    pub id: String,
    pub role: Role,
    /// Its own name (a proper name: not translated).
    pub name: String,
    /// The pinned revision or release; a different installed one is outdated.
    pub version: String,
    /// SPDX expression, checked when the entry was added (spec §10).
    pub licence: String,
    /// Where it comes from, for attribution.
    pub homepage: String,
    pub files: Vec<Asset>,
    /// For a runtime, the program to start, relative to the package's folder.
    pub program: Option<String>,
}

impl Package {
    /// Bytes to download.
    pub fn download_size(&self) -> u64 {
        self.files.iter().map(|file| file.size).sum()
    }

    /// An upper bound of the disk space it takes once installed.
    pub fn installed_size(&self) -> u64 {
        self.files.iter().map(Asset::installed_size).sum()
    }
}

/// The operating system and processor a build is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Platform {
    pub os: Os,
    pub arch: Arch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    MacOs,
    Windows,
    Linux,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    Aarch64,
    X86_64,
    Other,
}

impl Platform {
    /// The platform this program was built for.
    pub fn current() -> Self {
        let os = if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_os = "linux") {
            Os::Linux
        } else {
            Os::Other
        };
        let arch = match std::env::consts::ARCH {
            "aarch64" => Arch::Aarch64,
            "x86_64" => Arch::X86_64,
            _ => Arch::Other,
        };
        Self { os, arch }
    }
}

/// The packages offered on one platform.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    packages: Vec<Package>,
}

impl Catalog {
    /// The packages for the platform this program runs on.
    pub fn builtin() -> Self {
        Self::for_platform(Platform::current())
    }

    /// The models, which are the same everywhere, the quick draft's models
    /// where Apple Vision is not there to draft ([`draft_package`]), then the
    /// llama.cpp runtime built for `platform` ([`runtime_package`]) where it
    /// publishes one.
    pub fn for_platform(platform: Platform) -> Self {
        let mut packages = vec![nuqta_4b(), nuqta_0_8b()];
        packages.extend(draft_package(platform));
        packages.extend(runtime_package(platform));
        Self::new(packages)
    }

    /// A catalog of the given packages (tests, or a mirror).
    pub fn new(packages: Vec<Package>) -> Self {
        Self { packages }
    }

    /// In the order the UI lists them: the recogniser first.
    pub fn packages(&self) -> &[Package] {
        &self.packages
    }

    pub fn get(&self, id: &str) -> Option<&Package> {
        self.packages.iter().find(|package| package.id == id)
    }

    /// What OCR needs before it can run: the recogniser, and the runtime when
    /// this catalog offers one.
    pub fn required(&self) -> Vec<&Package> {
        self.packages.iter().filter(|package| matches!(package.role, Role::Primary | Role::Runtime)).collect()
    }
}

/// The authors' own GGUF conversions on Hugging Face, pinned to a revision
/// (ADR 0030's follow-up measured these exact files). Apache-2.0: the repo's
/// LICENSE file and card, and the Qwen3.5 base model's card (checked
/// 2026-11-22). The file names are the ones `ocr::NuqtaModel` loads.
fn nuqta(
    id: &str,
    role: Role,
    name: &str,
    repo: &str,
    revision: &str,
    files: [(&str, u64, &str); 2],
) -> Package {
    Package {
        id: id.to_owned(),
        role,
        name: name.to_owned(),
        version: revision[..7].to_owned(),
        licence: "Apache-2.0".to_owned(),
        homepage: format!("https://huggingface.co/{repo}"),
        files: files
            .into_iter()
            .map(|(file, size, sha256)| Asset {
                name: file.to_owned(),
                url: format!("https://huggingface.co/{repo}/resolve/{revision}/{file}"),
                size,
                sha256: sha256.to_owned(),
                unpack: Unpack::None,
            })
            .collect(),
        program: None,
    }
}

fn nuqta_4b() -> Package {
    nuqta(
        PRIMARY,
        Role::Primary,
        "Nuqta-Arabic-OCR 4B",
        "SalahALHaismawi/Nuqta-Arabic-OCR-4B-GGUF",
        "f4fdd78f8bfce76caac5f9e9b6f12d5b0bede140",
        [
            (
                "nuqta-ocr-4b-Q8_0.gguf",
                4_839_019_360,
                "759dce9aa4a9efcc3880c938d78ae1765d5a1a53f8f0bcaa5893c7c95dd43a7a",
            ),
            (
                "nuqta-ocr-4b-mmproj-f16.gguf",
                672_423_072,
                "69499fb23498321a16c6467084e55cf9a294d69b49809c8b962df5db4ab11026",
            ),
        ],
    )
}

fn nuqta_0_8b() -> Package {
    nuqta(
        ENSEMBLE,
        Role::Ensemble,
        "Nuqta-Arabic-OCR 0.8B",
        "SalahALHaismawi/Nuqta-Arabic-OCR-0.8B-GGUF",
        "4f720541afdf85461f78bdc705509fe4382a1806",
        [
            (
                "nuqta-ocr-0.8b-Q8_0.gguf",
                872_607_744,
                "9e668a31aaaa0262134737262298d53f8240f390004f5ff72270334acc90532c",
            ),
            (
                "nuqta-ocr-0.8b-mmproj-f16.gguf",
                204_986_688,
                "ca24aaf3a572147676411ba661f754eeadb66aa1daf8d133dc001e556370ed41",
            ),
        ],
    )
}

/// `tessdata_best` at its last release tag, `4.1.0` (the commit the tag
/// points to; `ara` and `eng` are unchanged on `main` since). Apache-2.0: the
/// repository's LICENSE (checked 2026-11-23). Each file's SHA-256 was taken
/// from the download, whose git blob hash matched GitHub's for the tag.
const TESSDATA_BEST_COMMIT: &str = "e2aad9b983032bb1beff9133104a67cdbb87ca4d";

/// The quick draft's models on Windows and Linux (owner item 23, decided
/// 2026-11-23): Tesseract's `tessdata_best` Arabic and English LSTM models,
/// read by a `tesseract` program 4.0 or later. macOS drafts with Apple Vision
/// and is offered none.
///
/// **The `tesseract` program is not in the catalog:** the user installs it
/// (owner item 26, decided 2026-11-23). The reasons:
/// - **Linux:** Tesseract publishes no Linux builds, only source; the
///   distributions package it (`tesseract-ocr`, `tesseract`), and the draft
///   uses that program.
/// - **Windows:** the one official build (release `5.5.3`,
///   `tesseract-ocr-w64-setup-5.5.3.20260724.exe`, SHA-256
///   `bee9e343…0ae4`) is an NSIS installer, not an archive, and it installs
///   every DLL its tools need: GLib, Pango, Cairo, FriBidi, gettext's libintl,
///   libiconv, libidn2 and libunistring are LGPL, and the GCC runtime is
///   GPL-3.0 with the runtime exception. Spec §10 asks before any of those,
///   and the owner gave no approval. The draft uses a `tesseract.exe` the user
///   installed. Tesseract itself (Apache-2.0) and Leptonica (BSD-2-Clause)
///   would be fine.
pub fn draft_package(platform: Platform) -> Option<Package> {
    if !matches!(platform.os, Os::Windows | Os::Linux) {
        return None;
    }
    let files = [
        ("ara.traineddata", 12_603_724, "ab9d157d8e38ca00e7e39c7d5363a5239e053f5b0dbdb3167dde9d8124335896"),
        ("eng.traineddata", 15_400_601, "8280aed0782fe27257a68ea10fe7ef324ca0f8d85bd2fd145d1c2b560bcb66ba"),
    ];
    Some(Package {
        id: DRAFT.to_owned(),
        role: Role::Draft,
        name: "Tesseract tessdata_best (ara + eng)".to_owned(),
        version: "4.1.0".to_owned(),
        licence: "Apache-2.0".to_owned(),
        homepage: "https://github.com/tesseract-ocr/tessdata_best".to_owned(),
        files: files
            .into_iter()
            .map(|(file, size, sha256)| Asset {
                name: file.to_owned(),
                url: format!(
                    "https://raw.githubusercontent.com/tesseract-ocr/tessdata_best/{TESSDATA_BEST_COMMIT}/{file}"
                ),
                size,
                sha256: sha256.to_owned(),
                unpack: Unpack::None,
            })
            .collect(),
        program: None,
    })
}

/// The pinned llama.cpp release (ADR 0032): the same build the benchmark ran
/// through Homebrew.
const LLAMA_CPP_TAG: &str = "b12359";

/// llama.cpp's own prebuilt `llama-server` for `platform`, or `None` where it
/// publishes none we can use.
///
/// - **macOS:** Metal on Apple silicon; the Intel build has Metal off.
/// - **Windows and Linux:** the Vulkan builds. They carry the CPU backends too
///   and load the Vulkan one only where a driver is present, so one build
///   serves every machine. NVIDIA GPUs are reached through Vulkan: the CUDA
///   builds need NVIDIA's CUDA runtime, whose licence is outside spec §10.
///
/// Licences (checked 2026-11-22 against the release workflow and `vendor/`):
/// llama.cpp, cpp-httplib, nlohmann/json and stb_image are MIT; BoringSSL
/// (macOS, Windows) is Apache-2.0; LLVM's OpenMP runtime (Windows, shipped as
/// `libomp.dll` with its licence) is Apache-2.0 WITH LLVM-exception. Outside
/// spec §10, and approved by the owner on 2026-11-22 (ADR 0034): sheredom's
/// `subprocess.h` (Unlicense) and miniaudio (Unlicense OR MIT-0), both compiled
/// in; and the built-in web UI's npm packages, not audited. That approval
/// holds only while the app starts the server with `--no-webui`
/// (`ocr::ServerConfig` does) and ships no CUDA build.
pub fn runtime_package(platform: Platform) -> Option<Package> {
    let (file, size, sha256, unpack) = match (platform.os, platform.arch) {
        (Os::MacOs, Arch::Aarch64) => (
            "bin-macos-arm64.tar.gz",
            16_907_811,
            "82b00abc740160e1b9980e6cc79c117345fa34af058c80e6c0a69aa8ddccde75",
            Unpack::TarGz,
        ),
        (Os::MacOs, Arch::X86_64) => (
            "bin-macos-x64.tar.gz",
            12_661_621,
            "25320d529077ef5a3d16e7b5da26438f221d88c8c0930a32b0c79457f5eede56",
            Unpack::TarGz,
        ),
        (Os::Linux, Arch::X86_64) => (
            "bin-ubuntu-vulkan-x64.tar.gz",
            36_153_377,
            "19b60f00395a3f4954e46a5037695bc1d843404fc9d3124f82a612ecd8fbe9f0",
            Unpack::TarGz,
        ),
        (Os::Linux, Arch::Aarch64) => (
            "bin-ubuntu-vulkan-arm64.tar.gz",
            29_435_221,
            "3f8ae812a58ac92e5a596ca9fbc988f52854d147c5a7fe1264dddc90c1d085b4",
            Unpack::TarGz,
        ),
        (Os::Windows, Arch::X86_64) => (
            "bin-win-vulkan-x64.zip",
            38_090_643,
            "e8d18c6e4210cca87e7307c2ea78363ac8d158e4e1397358ec96ab70b53a0d5e",
            Unpack::Zip,
        ),
        (Os::Windows, Arch::Aarch64) => (
            "bin-win-vulkan-arm64.zip",
            26_027_698,
            "62cd50089cffdd825524a91c5a8819041eb75d8e2beacd6eca68194dba50574f",
            Unpack::Zip,
        ),
        _ => return None,
    };
    let name = format!("llama-{LLAMA_CPP_TAG}-{file}");
    // The tarballs hold one top folder named after the tag; the zips are flat.
    let program = match platform.os {
        Os::Windows => "llama-server.exe".to_owned(),
        _ => format!("llama-{LLAMA_CPP_TAG}/llama-server"),
    };
    Some(Package {
        id: RUNTIME.to_owned(),
        role: Role::Runtime,
        name: "llama.cpp".to_owned(),
        version: LLAMA_CPP_TAG.to_owned(),
        licence:
            "MIT AND Apache-2.0 AND Apache-2.0 WITH LLVM-exception AND (Unlicense OR MIT-0) AND Unlicense"
                .to_owned(),
        homepage: "https://github.com/ggml-org/llama.cpp".to_owned(),
        files: vec![Asset {
            url: format!("https://github.com/ggml-org/llama.cpp/releases/download/{LLAMA_CPP_TAG}/{name}"),
            name,
            size,
            sha256: sha256.to_owned(),
            unpack,
        }],
        program: Some(program),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_platforms() -> Vec<Platform> {
        let mut out = Vec::new();
        for os in [Os::MacOs, Os::Windows, Os::Linux, Os::Other] {
            for arch in [Arch::Aarch64, Arch::X86_64, Arch::Other] {
                out.push(Platform { os, arch });
            }
        }
        out
    }

    fn check(package: &Package) {
        assert!(!package.files.is_empty(), "{}", package.id);
        for file in &package.files {
            assert!(file.url.starts_with("https://"), "{}: pinned files come over HTTPS", file.url);
            assert!(file.url.ends_with(&file.name), "{}", file.url);
            assert_eq!(file.sha256.len(), 64, "{}", file.name);
            assert!(file.sha256.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
            assert!(file.size > 0);
            assert!(!file.name.contains(['/', '\\']), "{}: a plain file name", file.name);
        }
        assert!(!package.licence.contains("GPL"), "{}", package.id);
    }

    #[test]
    fn every_platform_offers_the_models_in_order() {
        for platform in all_platforms() {
            let catalog = Catalog::for_platform(platform);
            let ids: Vec<&str> = catalog.packages().iter().map(|p| p.id.as_str()).collect();
            assert_eq!(ids[..2], [PRIMARY, ENSEMBLE]);
            for package in catalog.packages() {
                check(package);
            }
            let required: Vec<&str> = catalog.required().iter().map(|p| p.id.as_str()).collect();
            assert!(required.contains(&PRIMARY));
            assert!(!required.contains(&ENSEMBLE));
            // The quick draft's models: optional, after the models, and not on macOS.
            assert!(!required.contains(&DRAFT));
            let drafts = matches!(platform.os, Os::Windows | Os::Linux);
            assert_eq!(catalog.get(DRAFT).is_some(), drafts, "{platform:?}");
            if drafts {
                assert_eq!(ids[2], DRAFT);
            }
            // The runtime comes last, and only where llama.cpp publishes a build.
            let runtime = runtime_package(platform);
            assert_eq!(catalog.get(RUNTIME).cloned(), runtime);
            assert_eq!(required.contains(&RUNTIME), runtime.is_some(), "{platform:?}");
            if runtime.is_some() {
                assert_eq!(ids.last(), Some(&RUNTIME));
            }
        }
    }

    #[test]
    fn models_are_the_benchmarked_files() {
        let catalog = Catalog::builtin();
        let four = catalog.get(PRIMARY).expect("4B");
        assert_eq!(four.role, Role::Primary);
        assert_eq!(four.licence, "Apache-2.0");
        assert_eq!(four.version, "f4fdd78");
        let names: Vec<&str> = four.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["nuqta-ocr-4b-Q8_0.gguf", "nuqta-ocr-4b-mmproj-f16.gguf"]);
        assert_eq!(four.download_size(), 5_511_442_432);
        assert_eq!(four.installed_size(), four.download_size());
        let small = catalog.get(ENSEMBLE).expect("0.8B");
        assert_eq!(small.version, "4f72054");
        assert_eq!(small.download_size(), 1_077_594_432);
        assert!(catalog.get("nope").is_none());
    }

    /// The owner approved the runtime's licences only with its web UI off
    /// (ADR 0034): every server the OCR pipeline starts must say so.
    #[test]
    fn the_runtime_always_runs_without_its_web_ui() {
        for device in [crate::ocr::Device::Gpu, crate::ocr::Device::Cpu] {
            let mut config = crate::ocr::ServerConfig::new("llama-server", "m.gguf", "mmproj.gguf");
            config.device = device;
            let arguments = config.arguments(8080, "key");
            assert!(arguments.iter().any(|arg| arg == "--no-webui"), "{device:?}: {arguments:?}");
        }
    }

    #[test]
    fn the_draft_models_are_tessdata_best_pinned_to_its_release() {
        let platform = Platform { os: Os::Windows, arch: Arch::X86_64 };
        let package = draft_package(platform).expect("Windows drafts with Tesseract");
        check(&package);
        assert_eq!(package.role, Role::Draft);
        assert_eq!(package.licence, "Apache-2.0");
        assert_eq!(package.version, "4.1.0");
        assert_eq!(package.program, None, "the program is the user's (owner item 26)");
        let names: Vec<&str> = package.files.iter().map(|f| f.name.as_str()).collect();
        // `TesseractReader` reads `ara+eng` from this folder.
        assert_eq!(names, ["ara.traineddata", "eng.traineddata"]);
        for file in &package.files {
            assert!(file.url.contains(TESSDATA_BEST_COMMIT), "{}", file.url);
            assert!(file.url.starts_with("https://raw.githubusercontent.com/tesseract-ocr/tessdata_best/"));
            assert_eq!(file.unpack, Unpack::None);
        }
        assert_eq!(package.download_size(), 28_004_325);
        assert_eq!(draft_package(Platform { os: Os::Linux, arch: Arch::Aarch64 }).as_ref(), Some(&package));
        assert_eq!(
            draft_package(Platform { os: Os::MacOs, arch: Arch::Aarch64 }),
            None,
            "Apple Vision drafts"
        );
        assert_eq!(draft_package(Platform { os: Os::Other, arch: Arch::X86_64 }), None);
    }

    #[test]
    fn runtime_builds_cover_the_desktop_platforms() {
        for platform in all_platforms() {
            let package = runtime_package(platform);
            let supported = platform.os != Os::Other && platform.arch != Arch::Other;
            assert_eq!(package.is_some(), supported, "{platform:?}");
            let Some(package) = package else { continue };
            check(&package);
            assert_eq!(package.role, Role::Runtime);
            let file = &package.files[0];
            assert!(file.name.starts_with("llama-b12359-bin-"), "{}", file.name);
            let program = package.program.as_deref().expect("program");
            if platform.os == Os::Windows {
                assert_eq!(file.unpack, Unpack::Zip);
                assert_eq!(program, "llama-server.exe");
            } else {
                assert_eq!(file.unpack, Unpack::TarGz);
                assert_eq!(program, "llama-b12359/llama-server");
            }
            assert!(!file.name.contains("cuda"), "CUDA builds need NVIDIA's runtime (licence)");
            assert_eq!(package.installed_size(), file.size * 4);
        }
    }
}
