use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=runtime/velo_rt.c");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR not set"));

    cc::Build::new()
        .file("runtime/velo_rt.c")
        .opt_level(3)
        .warnings(true)
        .compile("velo_rt");

    // Locate the compiled static library for embedding into the compiler binary
    let lib_name = if cfg!(target_env = "msvc") {
        "velo_rt.lib"
    } else {
        "libvelo_rt.a"
    };
    let lib_path = out_dir.join(lib_name);
    if !lib_path.exists() {
        panic!("Expected runtime library at {}", lib_path.display());
    }
    println!("cargo:rustc-env=VELO_RT_LIB={}", lib_path.display());

    // Ensure Windows system libs used by velo_rt are linked into velo.exe (needed for JIT)
    #[cfg(target_os = "windows")]
    {
        println!("cargo:rustc-link-lib=kernel32");
    }
}
