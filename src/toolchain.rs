use std::path::{Path, PathBuf};
use std::process::Command;

pub struct WindowsToolchain {
    pub cl_exe: Option<PathBuf>,
    pub link_exe: Option<PathBuf>,
    pub lld_link_exe: Option<PathBuf>,
    pub msvc_include: Option<PathBuf>,
    pub msvc_lib_x64: Option<PathBuf>,
    pub sdk_include_ucrt: Option<PathBuf>,
    pub sdk_include_um: Option<PathBuf>,
    pub sdk_include_shared: Option<PathBuf>,
    pub sdk_lib_ucrt_x64: Option<PathBuf>,
    pub sdk_lib_um_x64: Option<PathBuf>,
}

impl WindowsToolchain {
    pub fn discover() -> Result<Self, String> {
        let mut tc = Self {
            cl_exe: find_in_path("cl.exe"),
            link_exe: None,
            lld_link_exe: find_in_path("lld-link.exe"),
            msvc_include: None,
            msvc_lib_x64: None,
            sdk_include_ucrt: None,
            sdk_include_um: None,
            sdk_include_shared: None,
            sdk_lib_ucrt_x64: None,
            sdk_lib_um_x64: None,
        };

        // Check rustup toolchain for lld-link.exe
        if tc.lld_link_exe.is_none() {
            if let Some(p) = find_rust_lld_link() {
                tc.lld_link_exe = Some(p);
            }
        }

        // Discover MSVC installation (via env, vswhere, or standard VS directories)
        if let Some(msvc_ver_dir) = find_latest_msvc_dir() {
            let bin_x64 = msvc_ver_dir.join("bin").join("Hostx64").join("x64");
            let cl = bin_x64.join("cl.exe");
            let lnk = bin_x64.join("link.exe");
            if cl.exists() && tc.cl_exe.is_none() {
                tc.cl_exe = Some(cl);
            }
            if lnk.exists() {
                tc.link_exe = Some(lnk);
            }
            let inc = msvc_ver_dir.join("include");
            if inc.exists() {
                tc.msvc_include = Some(inc);
            }
            let lib = msvc_ver_dir.join("lib").join("x64");
            if lib.exists() {
                tc.msvc_lib_x64 = Some(lib);
            }
        }
        if tc.link_exe.is_none() {
            tc.link_exe = find_in_path("link.exe");
        }

        // Discover Windows 10/11 SDK
        if let Some((sdk_root, ver)) = find_latest_windows_sdk() {
            let inc_ver = sdk_root.join("Include").join(&ver);
            let lib_ver = sdk_root.join("Lib").join(&ver);
            if inc_ver.join("ucrt").exists() {
                tc.sdk_include_ucrt = Some(inc_ver.join("ucrt"));
            }
            if inc_ver.join("um").exists() {
                tc.sdk_include_um = Some(inc_ver.join("um"));
            }
            if inc_ver.join("shared").exists() {
                tc.sdk_include_shared = Some(inc_ver.join("shared"));
            }
            if lib_ver.join("ucrt").join("x64").exists() {
                tc.sdk_lib_ucrt_x64 = Some(lib_ver.join("ucrt").join("x64"));
            }
            if lib_ver.join("um").join("x64").exists() {
                tc.sdk_lib_um_x64 = Some(lib_ver.join("um").join("x64"));
            }
        }

        if tc.lld_link_exe.is_none() && tc.link_exe.is_none() {
            return Err("Could not find lld-link.exe or MSVC link.exe".to_string());
        }
        Ok(tc)
    }

    pub fn lib_paths(&self) -> Vec<PathBuf> {
        let mut v = Vec::new();
        if let Some(p) = &self.msvc_lib_x64 {
            v.push(p.clone());
        }
        if let Some(p) = &self.sdk_lib_ucrt_x64 {
            v.push(p.clone());
        }
        if let Some(p) = &self.sdk_lib_um_x64 {
            v.push(p.clone());
        }
        v
    }

    pub fn include_paths(&self) -> Vec<PathBuf> {
        let mut v = Vec::new();
        if let Some(p) = &self.msvc_include {
            v.push(p.clone());
        }
        if let Some(p) = &self.sdk_include_ucrt {
            v.push(p.clone());
        }
        if let Some(p) = &self.sdk_include_um {
            v.push(p.clone());
        }
        if let Some(p) = &self.sdk_include_shared {
            v.push(p.clone());
        }
        v
    }
}

fn find_in_path(exe: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&paths) {
        let p = dir.join(exe);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn find_rust_lld_link() -> Option<PathBuf> {
    if let Ok(out) = Command::new("rustc").args(["--print", "sysroot"]).output() {
        if out.status.success() {
            let sysroot = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let cand = PathBuf::from(&sysroot)
                .join("lib")
                .join("rustlib")
                .join("x86_64-pc-windows-msvc")
                .join("bin")
                .join("gcc-ld")
                .join("lld-link.exe");
            if cand.is_file() {
                return Some(cand);
            }
            let cand2 = PathBuf::from(&sysroot)
                .join("lib")
                .join("rustlib")
                .join("x86_64-pc-windows-msvc")
                .join("bin")
                .join("lld-link.exe");
            if cand2.is_file() {
                return Some(cand2);
            }
        }
    }
    if let Ok(home) = std::env::var("RUSTUP_HOME") {
        let tc_root = PathBuf::from(home).join("toolchains");
        if let Ok(entries) = std::fs::read_dir(tc_root) {
            for e in entries.flatten() {
                let p = e
                    .path()
                    .join("lib/rustlib/x86_64-pc-windows-msvc/bin/gcc-ld/lld-link.exe");
                if p.is_file() {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn find_latest_msvc_dir() -> Option<PathBuf> {
    if let Ok(vc_dir) = std::env::var("VCToolsInstallDir") {
        let p = PathBuf::from(vc_dir);
        if p.exists() {
            return Some(p);
        }
    }

    let mut vs_roots = Vec::new();
    let vswhere = PathBuf::from(r"C:\Program Files (x86)\Microsoft Visual Studio\Installer\vswhere.exe");
    if vswhere.is_file() {
        if let Ok(out) = Command::new(&vswhere)
            .args(["-all", "-products", "*", "-property", "installationPath"])
            .output()
        {
            if out.status.success() {
                for line in String::from_utf8_lossy(&out.stdout).lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        vs_roots.push(PathBuf::from(trimmed));
                    }
                }
            }
        }
    }
    for base in [
        r"C:\Program Files (x86)\Microsoft Visual Studio",
        r"C:\Program Files\Microsoft Visual Studio",
    ] {
        if let Ok(years) = std::fs::read_dir(base) {
            for y in years.flatten() {
                if let Ok(editions) = std::fs::read_dir(y.path()) {
                    for ed in editions.flatten() {
                        vs_roots.push(ed.path());
                    }
                }
            }
        }
    }

    let mut best: Option<(String, PathBuf)> = None;
    for root in vs_roots {
        let msvc_tools = root.join("VC").join("Tools").join("MSVC");
        if let Ok(entries) = std::fs::read_dir(msvc_tools) {
            for e in entries.flatten() {
                let p = e.path();
                if p.join("bin").join("Hostx64").join("x64").join("link.exe").is_file() {
                    let ver = e.file_name().to_string_lossy().to_string();
                    if best.as_ref().map(|(bv, _)| ver > *bv).unwrap_or(true) {
                        best = Some((ver, p));
                    }
                }
            }
        }
    }
    best.map(|(_, p)| p)
}

fn find_latest_windows_sdk() -> Option<(PathBuf, String)> {
    let candidates = [
        Path::new(r"C:\Program Files (x86)\Windows Kits\10").to_path_buf(),
        Path::new(r"C:\Program Files\Windows Kits\10").to_path_buf(),
    ];
    let mut best: Option<(String, PathBuf)> = None;
    for root in candidates {
        let lib_dir = root.join("Lib");
        if let Ok(entries) = std::fs::read_dir(&lib_dir) {
            for e in entries.flatten() {
                let ver = e.file_name().to_string_lossy().to_string();
                if ver.starts_with("10.") && e.path().join("um").join("x64").join("kernel32.lib").is_file() {
                    if best.as_ref().map(|(bv, _)| ver > *bv).unwrap_or(true) {
                        best = Some((ver, root.clone()));
                    }
                }
            }
        }
    }
    best.map(|(v, r)| (r, v))
}
