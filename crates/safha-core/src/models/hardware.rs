//! What this machine offers the models: GPU backends llama.cpp could use, and
//! memory (spec §8: use acceleration when present, run correctly without it).
//!
//! Reported, not yet acted on: the pipeline still chooses GPU or CPU itself.
//! Detection only looks for the drivers' files, so it is cheap and needs no
//! `unsafe`; a driver that is installed but broken is still reported.

use std::path::Path;

use serde::Serialize;

/// A GPU backend llama.cpp has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Accelerator {
    /// Apple's GPUs (Apple silicon; llama.cpp's Intel Mac build leaves Metal off).
    Metal,
    /// NVIDIA's driver. The runtime reaches these GPUs through Vulkan (the CUDA
    /// builds need NVIDIA's licence-restricted runtime).
    Cuda,
    /// A Vulkan driver (AMD, Intel, NVIDIA on Windows and Linux).
    Vulkan,
}

/// What was found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hardware {
    /// `macos`, `windows`, `linux`, …
    pub os: &'static str,
    /// `aarch64`, `x86_64`, …
    pub arch: &'static str,
    /// Every backend found, best first.
    pub accelerators: Vec<Accelerator>,
    /// The backend the runtime this app installs would use, if any; `None` means
    /// OCR runs on the CPU (slowly: about 2.5 minutes a page for the 4B, ADR 0030).
    pub runtime_gpu: Option<Accelerator>,
    /// Installed memory in bytes, when it could be read.
    pub memory: Option<u64>,
}

impl Hardware {
    /// Looks at this machine.
    pub fn detect() -> Self {
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;
        let accelerators = accelerators(os, arch, &|path| path.exists());
        let runtime_gpu =
            accelerators.iter().copied().find(|a| matches!(a, Accelerator::Metal | Accelerator::Vulkan));
        Self { os, arch, accelerators, runtime_gpu, memory: memory() }
    }
}

/// The backends whose drivers are present. `exists` is the filesystem (a
/// parameter so tests can describe a machine).
fn accelerators(os: &str, arch: &str, exists: &dyn Fn(&Path) -> bool) -> Vec<Accelerator> {
    let mut found = Vec::new();
    match os {
        "macos" => {
            if arch == "aarch64" {
                found.push(Accelerator::Metal);
            }
        }
        "windows" => {
            let system = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
            let system = Path::new(&system).join("System32");
            if exists(&system.join("nvcuda.dll")) {
                found.push(Accelerator::Cuda);
            }
            if exists(&system.join("vulkan-1.dll")) {
                found.push(Accelerator::Vulkan);
            }
        }
        "linux" => {
            if exists(Path::new("/proc/driver/nvidia/version")) {
                found.push(Accelerator::Cuda);
            }
            if linux_vulkan(exists) {
                found.push(Accelerator::Vulkan);
            }
        }
        _ => {}
    }
    found
}

/// The Vulkan loader, and a driver manifest other than the CPU-only
/// llvmpipe (`lvp`), which would be no faster than llama.cpp's own CPU code.
fn linux_vulkan(exists: &dyn Fn(&Path) -> bool) -> bool {
    let loader = [
        "/usr/lib/x86_64-linux-gnu/libvulkan.so.1",
        "/usr/lib/aarch64-linux-gnu/libvulkan.so.1",
        "/usr/lib64/libvulkan.so.1",
        "/usr/lib/libvulkan.so.1",
    ]
    .iter()
    .any(|path| exists(Path::new(path)));
    let drivers = ["/usr/share/vulkan/icd.d", "/etc/vulkan/icd.d", "/usr/local/share/vulkan/icd.d"]
        .iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            name.ends_with(".json") && !name.starts_with("lvp")
        });
    loader && drivers
}

/// Installed memory, from the OS's own report.
fn memory() -> Option<u64> {
    if cfg!(target_os = "linux") {
        let info = std::fs::read_to_string("/proc/meminfo").ok()?;
        parse_meminfo(&info)
    } else if cfg!(target_os = "macos") {
        let out = std::process::Command::new("/usr/sbin/sysctl").args(["-n", "hw.memsize"]).output().ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    } else {
        windows_memory()
    }
}

#[cfg(windows)]
fn windows_memory() -> Option<u64> {
    use std::os::windows::process::CommandExt;
    /// No console window flashes up.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stdout).trim().parse().ok()
}

#[cfg(not(windows))]
fn windows_memory() -> Option<u64> {
    None
}

/// `MemTotal:       16314412 kB` → bytes.
fn parse_meminfo(info: &str) -> Option<u64> {
    let line = info.lines().find(|line| line.starts_with("MemTotal:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    kib.checked_mul(1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apple_silicon_has_metal_and_intel_macs_do_not() {
        assert_eq!(accelerators("macos", "aarch64", &|_| false), [Accelerator::Metal]);
        assert!(accelerators("macos", "x86_64", &|_| true).is_empty());
    }

    #[test]
    fn windows_drivers_by_their_files() {
        let both = accelerators("windows", "x86_64", &|path| {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name == "nvcuda.dll" || name == "vulkan-1.dll"
        });
        assert_eq!(both, [Accelerator::Cuda, Accelerator::Vulkan]);
        assert!(accelerators("windows", "x86_64", &|_| false).is_empty());
    }

    #[test]
    fn linux_cuda_from_the_driver() {
        let found = accelerators("linux", "x86_64", &|path| path == Path::new("/proc/driver/nvidia/version"));
        assert_eq!(found, [Accelerator::Cuda]);
        assert!(accelerators("freebsd", "x86_64", &|_| true).is_empty());
    }

    #[test]
    fn meminfo() {
        let info = "MemTotal:       16314412 kB\nMemFree:  1 kB\n";
        assert_eq!(parse_meminfo(info), Some(16_314_412 * 1024));
        assert_eq!(parse_meminfo("nothing"), None);
    }

    #[test]
    fn detect_describes_this_machine() {
        let found = Hardware::detect();
        assert_eq!(found.os, std::env::consts::OS);
        if let Some(gpu) = found.runtime_gpu {
            assert!(found.accelerators.contains(&gpu));
            assert_ne!(gpu, Accelerator::Cuda);
        }
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            assert!(found.memory.is_some_and(|bytes| bytes > 256 << 20), "{:?}", found.memory);
        }
    }
}
