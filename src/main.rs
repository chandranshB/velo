mod ast;
mod backend_c;
mod backend_native;
mod checker;
mod error;
mod ir;
mod ir_lower;
mod ir_opt;
mod lexer;
mod loader;
mod parser;
mod toolchain;
mod types;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use ir::Subsystem;
use ir_opt::OptLevel;
use toolchain::WindowsToolchain;

#[derive(Clone, Copy, PartialEq, Eq)]
enum BackendKind {
    Native, // Cranelift direct native machine code (.obj -> .exe)
    Max,    // Velo IR Opt + strict __restrict + MSVC AVX2 /O2
    Jit,    // In-memory Cranelift JIT
}

struct CliOpts {
    cmd: String,
    input: Option<PathBuf>,
    output: Option<PathBuf>,
    backend: BackendKind,
    opt: OptLevel,
    gui: bool,
    emit_c: bool,
    show_stats: bool,
    links: Vec<String>,
    program_args: Vec<String>,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "--help" || args[0] == "-h" {
        print_usage();
        return;
    }

    let opts = match parse_args(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("velo: {e}");
            std::process::exit(1);
        }
    };

    match opts.cmd.as_str() {
        "check" => cmd_check(&opts),
        "build" => cmd_build(&opts),
        "run" => cmd_run(&opts),
        "version" | "--version" | "-V" => {
            println!("velo 0.1.0 (x86_64-windows-msvc | Cranelift + Max-Opt IR)");
        }
        other => {
            eprintln!("velo: unknown command '{other}'");
            print_usage();
            std::process::exit(1);
        }
    }
}

fn print_usage() {
    println!(
        "Velo Programming Language Compiler v0.1.0\n\n\
         USAGE:\n  \
           velo run <file.velo> [options] [-- args...]\n  \
           velo build <file.velo> [-o out.exe] [options]\n  \
           velo check <file.velo>\n\n\
         OPTIONS:\n  \
           --backend=<native|max|jit>   Select compilation engine (default: native; max for --release)\n  \
           --release                    Enable full optimization + Max AVX2 backend\n  \
           --debug                      Disable IR optimizations for fastest debug build\n  \
           --jit                        Execute directly in memory via Cranelift JIT\n  \
           --gui                        Use Windows GUI subsystem (hides console window)\n  \
           --link=<lib>                 Link extra system/C library (e.g. --link=user32)\n  \
           --emit-c                     Print generated optimizing C IR (with --backend=max)\n  \
           --stats                      Print compiler phase timings and optimizer statistics"
    );
}

fn parse_args(args: &[String]) -> Result<CliOpts, String> {
    let cmd = args[0].clone();
    let mut input = None;
    let mut output = None;
    let mut backend = None;
    let mut opt = OptLevel::Speed;
    let mut gui = false;
    let mut emit_c = false;
    let mut show_stats = false;
    let mut links = Vec::new();
    let mut program_args = Vec::new();

    let mut i = 1;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            program_args.extend_from_slice(&args[i + 1..]);
            break;
        } else if a == "-o" || a == "--out" {
            i += 1;
            if i >= args.len() {
                return Err("expected output path after -o".to_string());
            }
            output = Some(PathBuf::from(&args[i]));
        } else if let Some(val) = a.strip_prefix("--backend=") {
            backend = Some(match val {
                "native" => BackendKind::Native,
                "max" => BackendKind::Max,
                "jit" => BackendKind::Jit,
                _ => return Err(format!("unknown backend '{val}'")),
            });
        } else if a == "--release" {
            opt = OptLevel::Max;
            if backend.is_none() {
                backend = Some(BackendKind::Max);
            }
        } else if a == "--debug" {
            opt = OptLevel::Debug;
        } else if a == "--jit" {
            backend = Some(BackendKind::Jit);
        } else if a == "--gui" {
            gui = true;
        } else if a == "--emit-c" {
            emit_c = true;
        } else if a == "--stats" {
            show_stats = true;
        } else if let Some(lib) = a.strip_prefix("--link=") {
            links.push(lib.to_string());
        } else if a.starts_with('-') {
            return Err(format!("unknown flag '{a}'"));
        } else if input.is_none() {
            input = Some(PathBuf::from(a));
        } else {
            program_args.push(a.clone());
        }
        i += 1;
    }

    Ok(CliOpts {
        cmd,
        input,
        output,
        backend: backend.unwrap_or(BackendKind::Native),
        opt,
        gui,
        emit_c,
        show_stats,
        links,
        program_args,
    })
}

fn compile_frontend(entry: &Path, opt: OptLevel, force_gui: bool, show_stats: bool) -> Result<ir::IrProgram, ()> {
    let t0 = Instant::now();
    let loaded = match loader::load_project(entry) {
        Ok(p) => p,
        Err((files, diag)) => {
            eprint!("{}", diag.render(&files));
            return Err(());
        }
    };
    let checked = match checker::check_project(loaded) {
        Ok(p) => p,
        Err((files, diag)) => {
            eprint!("{}", diag.render(&files));
            return Err(());
        }
    };
    let mut ir_prog = ir_lower::lower_program(checked);
    if force_gui {
        ir_prog.subsystem = Subsystem::Windows;
    }
    let stats = ir_opt::optimize_program(&mut ir_prog, opt);
    if show_stats {
        eprintln!(
            "[velo] frontend+opt: {:.2} ms | folded={}, copies={}, cse={}, bounds_elim={}, inlined={}, dce={}, dead_blocks={}",
            t0.elapsed().as_secs_f64() * 1000.0,
            stats.folded_consts,
            stats.propagated_copies,
            stats.cse_eliminated,
            stats.bounds_eliminated,
            stats.inlined_calls,
            stats.dce_removed,
            stats.blocks_removed
        );
    }
    Ok(ir_prog)
}

fn cmd_check(opts: &CliOpts) {
    let Some(entry) = &opts.input else {
        eprintln!("velo check: missing input file");
        std::process::exit(1);
    };
    if compile_frontend(entry, opts.opt, opts.gui, opts.show_stats).is_ok() {
        println!("OK: '{}' type-checked cleanly.", entry.display());
    } else {
        std::process::exit(1);
    }
}

fn default_out_exe(entry: &Path) -> PathBuf {
    entry.with_extension("exe")
}

fn cmd_build(opts: &CliOpts) {
    let Some(entry) = &opts.input else {
        eprintln!("velo build: missing input file");
        std::process::exit(1);
    };
    let Ok(ir_prog) = compile_frontend(entry, opts.opt, opts.gui, opts.show_stats) else {
        std::process::exit(1);
    };
    if ir_prog.main_fn.is_none() {
        eprintln!("velo build: no 'main()' function found in '{}'", entry.display());
        std::process::exit(1);
    }
    if opts.emit_c {
        println!("{}", backend_c::emit_c_source(&ir_prog));
        return;
    }
    let out_exe = opts.output.clone().unwrap_or_else(|| default_out_exe(entry));
    let tc = match WindowsToolchain::discover() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("velo toolchain error: {e}");
            std::process::exit(1);
        }
    };
    let t_be = Instant::now();
    let res = match opts.backend {
        BackendKind::Max => backend_c::build_with_msvc_max(&ir_prog, &out_exe, &tc, &opts.links),
        BackendKind::Native | BackendKind::Jit => {
            backend_native::build_native_aot(&ir_prog, &out_exe, opts.opt, &tc, &opts.links)
        }
    };
    if let Err(e) = res {
        eprintln!("velo build error: {e}");
        std::process::exit(1);
    }
    if opts.show_stats {
        eprintln!(
            "[velo] backend+link: {:.2} ms -> {}",
            t_be.elapsed().as_secs_f64() * 1000.0,
            out_exe.display()
        );
    } else {
        println!("Built {}", out_exe.display());
    }
}

fn cmd_run(opts: &CliOpts) {
    let Some(entry) = &opts.input else {
        eprintln!("velo run: missing input file");
        std::process::exit(1);
    };
    let Ok(ir_prog) = compile_frontend(entry, opts.opt, opts.gui, opts.show_stats) else {
        std::process::exit(1);
    };
    if ir_prog.main_fn.is_none() {
        eprintln!("velo run: no 'main()' function found in '{}'", entry.display());
        std::process::exit(1);
    }

    if opts.backend == BackendKind::Jit {
        match backend_native::run_jit(&ir_prog, opts.opt) {
            Ok(code) => std::process::exit(code),
            Err(e) => {
                eprintln!("velo jit error: {e}");
                std::process::exit(1);
            }
        }
    }

    let tc = match WindowsToolchain::discover() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("velo toolchain error: {e}");
            std::process::exit(1);
        }
    };
    let tmp_exe = std::env::temp_dir()
        .join("velo_build")
        .join(format!(
            "{}_run.exe",
            entry.file_stem().and_then(|s| s.to_str()).unwrap_or("velo")
        ));
    let _ = std::fs::create_dir_all(tmp_exe.parent().unwrap());

    let t_be = Instant::now();
    let res = match opts.backend {
        BackendKind::Max => backend_c::build_with_msvc_max(&ir_prog, &tmp_exe, &tc, &opts.links),
        _ => backend_native::build_native_aot(&ir_prog, &tmp_exe, opts.opt, &tc, &opts.links),
    };
    if let Err(e) = res {
        eprintln!("velo build error: {e}");
        std::process::exit(1);
    }
    if opts.show_stats {
        eprintln!("[velo] backend+link: {:.2} ms", t_be.elapsed().as_secs_f64() * 1000.0);
    }

    let status = Command::new(&tmp_exe)
        .args(&opts.program_args)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("velo run error: failed to execute '{}': {e}", tmp_exe.display());
            std::process::exit(1);
        });
    std::process::exit(status.code().unwrap_or(1));
}
