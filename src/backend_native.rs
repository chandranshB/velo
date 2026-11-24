use std::collections::HashMap;
use std::ffi::CString;
use std::fs;
use std::path::Path;
use std::process::Command;

use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::{
    types, AbiParam, Block as ClBlock, Function, InstBuilder, MemFlagsData, Signature, StackSlot,
    StackSlotData, StackSlotKind, UserFuncName, Value,
};
use cranelift_codegen::settings::{self, Configurable};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext, Variable};
use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{DataDescription, DataId, FuncId, Linkage, Module};
use cranelift_object::{ObjectBuilder, ObjectModule};

use crate::ast::{BinOp, UnaryOp};
use crate::backend_c::EMBEDDED_RT_LIB;
use crate::ir::*;
use crate::ir_opt::OptLevel;
use crate::toolchain::WindowsToolchain;
use crate::types::*;

unsafe extern "C" {
    fn velo_rt_print_i64(v: i64);
    fn velo_rt_print_u64(v: u64);
    fn velo_rt_print_f64(v: f64);
    fn velo_rt_print_bool(v: i8);
    fn velo_rt_print_str(ptr: *const u8, len: i64);
    fn velo_rt_print_cstr(s: *const i8);
    fn velo_rt_print_newline();
    fn velo_rt_print_space();
    fn velo_rt_panic(ptr: *const u8, len: i64, fp: *const u8, fl: i64, line: i64);
    fn velo_rt_bounds_fail(idx: i64, len: i64, fp: *const u8, fl: i64, line: i64);
    fn velo_rt_alloc(bytes: i64) -> *mut u8;
    fn velo_rt_realloc(p: *mut u8, bytes: i64) -> *mut u8;
    fn velo_rt_free(p: *mut u8);
    fn velo_rt_copy_mem(dst: *mut u8, src: *const u8, bytes: i64);
    fn velo_rt_set_mem(dst: *mut u8, val: u8, bytes: i64);
    fn velo_rt_time_ns() -> i64;
    fn velo_rt_sleep_ms(ms: i32);
    fn velo_rt_rand_u64() -> u64;
    fn velo_rt_rand_range(min_v: i64, max_v: i64) -> i64;
    fn velo_rt_read_int() -> i64;
    fn velo_rt_read_line(out_ptr: *mut *mut u8) -> i64;
    fn velo_rt_str_concat(
        ap: *const u8,
        al: i64,
        bp: *const u8,
        bl: i64,
        op: *mut *mut u8,
        ol: *mut i64,
    );
    fn velo_rt_i64_to_str(v: i64, op: *mut *mut u8, ol: *mut i64);
    fn velo_rt_u64_to_str(v: u64, op: *mut *mut u8, ol: *mut i64);
    fn velo_rt_f64_to_str(v: f64, prec: i32, op: *mut *mut u8, ol: *mut i64);
    fn velo_rt_bool_to_str(v: i8, op: *mut *mut u8, ol: *mut i64);
    fn velo_rt_str_eq(ap: *const u8, al: i64, bp: *const u8, bl: i64) -> i8;
    fn velo_rt_str_cmp(ap: *const u8, al: i64, bp: *const u8, bl: i64) -> i32;
    fn velo_rt_cstr_len(s: *const i8) -> i64;
    fn velo_rt_sqrt(x: f64) -> f64;
    fn velo_rt_sin(x: f64) -> f64;
    fn velo_rt_cos(x: f64) -> f64;
    fn velo_rt_tan(x: f64) -> f64;
    fn velo_rt_floor(x: f64) -> f64;
    fn velo_rt_ceil(x: f64) -> f64;
    fn velo_rt_round(x: f64) -> f64;
    fn velo_rt_pow(x: f64, y: f64) -> f64;
}

#[cfg(windows)]
unsafe extern "system" {
    fn LoadLibraryA(lpLibFileName: *const i8) -> *mut std::ffi::c_void;
    fn GetProcAddress(hModule: *mut std::ffi::c_void, lpProcName: *const i8) -> *mut std::ffi::c_void;
}

pub fn build_native_aot(
    prog: &IrProgram,
    out_exe: &Path,
    opt: OptLevel,
    tc: &WindowsToolchain,
    extra_links: &[String],
) -> Result<(), String> {
    let mut flag_builder = settings::builder();
    flag_builder
        .set("opt_level", if opt == OptLevel::Debug { "none" } else { "speed" })
        .map_err(|e| e.to_string())?;
    flag_builder
        .set("use_colocated_libcalls", "false")
        .map_err(|e| e.to_string())?;

    let isa_builder = cranelift_native::builder().map_err(|m| m.to_string())?;
    let isa = isa_builder
        .finish(settings::Flags::new(flag_builder))
        .map_err(|e| e.to_string())?;

    let obj_builder = ObjectBuilder::new(
        isa,
        out_exe
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("velo_app"),
        cranelift_module::default_libcall_names(),
    )
    .map_err(|e| e.to_string())?;

    let mut module = ObjectModule::new(obj_builder);
    compile_into_module(&mut module, prog, true)?;
    let product = module.finish();
    let obj_bytes = product.emit().map_err(|e| format!("COFF emit failed: {e}"))?;

    let tmp_dir = std::env::temp_dir().join("velo_build");
    fs::create_dir_all(&tmp_dir).map_err(|e| format!("failed to create temp dir: {e}"))?;
    let stem = out_exe
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("velo_app");
    let obj_path = tmp_dir.join(format!("{stem}_native.obj"));
    let rt_lib_path = tmp_dir.join("velo_rt.lib");
    fs::write(&obj_path, obj_bytes).map_err(|e| format!("failed to write obj: {e}"))?;
    fs::write(&rt_lib_path, EMBEDDED_RT_LIB).map_err(|e| format!("failed to write rt lib: {e}"))?;

    let linker = tc
        .lld_link_exe
        .as_ref()
        .or(tc.link_exe.as_ref())
        .ok_or_else(|| "No Windows linker available".to_string())?;

    let mut lnk_cmd = Command::new(linker);
    lnk_cmd
        .arg("/NOLOGO")
        .arg(format!("/OUT:{}", out_exe.display()))
        .arg("/OPT:REF")
        .arg("/OPT:ICF");

    match prog.subsystem {
        Subsystem::Windows => {
            lnk_cmd.arg("/SUBSYSTEM:WINDOWS").arg("/ENTRY:mainCRTStartup");
        }
        Subsystem::Console => {
            lnk_cmd.arg("/SUBSYSTEM:CONSOLE");
        }
    }

    for lp in tc.lib_paths() {
        lnk_cmd.arg(format!("/LIBPATH:{}", lp.display()));
    }

    lnk_cmd.arg(&obj_path).arg(&rt_lib_path);
    lnk_cmd
        .arg("kernel32.lib")
        .arg("ucrt.lib")
        .arg("vcruntime.lib")
        .arg("msvcrt.lib")
        .arg("legacy_stdio_definitions.lib");

    for lib in &prog.links {
        let l = if lib.ends_with(".lib") { lib.clone() } else { format!("{lib}.lib") };
        lnk_cmd.arg(l);
    }
    for lib in extra_links {
        let l = if lib.ends_with(".lib") { lib.clone() } else { format!("{lib}.lib") };
        lnk_cmd.arg(l);
    }

    let lnk_out = lnk_cmd
        .output()
        .map_err(|e| format!("failed to execute linker: {e}"))?;
    if !lnk_out.status.success() {
        return Err(format!(
            "linker failed:\n{}\n{}",
            String::from_utf8_lossy(&lnk_out.stdout),
            String::from_utf8_lossy(&lnk_out.stderr)
        ));
    }
    Ok(())
}

pub fn run_jit(prog: &IrProgram, opt: OptLevel) -> Result<i32, String> {
    let mut flag_builder = settings::builder();
    flag_builder
        .set("opt_level", if opt == OptLevel::Debug { "none" } else { "speed" })
        .map_err(|e| e.to_string())?;
    flag_builder
        .set("use_colocated_libcalls", "false")
        .map_err(|e| e.to_string())?;
    let isa_builder = cranelift_native::builder().map_err(|m| m.to_string())?;
    let isa = isa_builder
        .finish(settings::Flags::new(flag_builder))
        .map_err(|e| e.to_string())?;

    let mut jit_builder = JITBuilder::with_isa(isa, cranelift_module::default_libcall_names());
    register_runtime_symbols(&mut jit_builder);

    let links = prog.links.clone();
    jit_builder.symbol_lookup_fn(Box::new(move |sym_name: &str| resolve_external_symbol(sym_name, &links)));

    let mut module = JITModule::new(jit_builder);
    let main_func_id = compile_into_module(&mut module, prog, false)?
        .ok_or_else(|| "no 'main' function defined".to_string())?;
    module.finalize_definitions().map_err(|e| e.to_string())?;

    let main_id_ir = prog.main_fn.ok_or_else(|| "no 'main' function".to_string())?;
    let ret_ty = prog.funcs[main_id_ir.0].ret;
    let code_ptr = module.get_finalized_function(main_func_id);

    unsafe {
        if ret_ty == TY_VOID {
            let f: extern "C" fn() = std::mem::transmute(code_ptr);
            f();
            Ok(0)
        } else if ret_ty == TY_I32 {
            let f: extern "C" fn() -> i32 = std::mem::transmute(code_ptr);
            Ok(f())
        } else {
            let f: extern "C" fn() -> i64 = std::mem::transmute(code_ptr);
            Ok(f() as i32)
        }
    }
}

fn register_runtime_symbols(jb: &mut JITBuilder) {
    jb.symbol("velo_rt_print_i64", velo_rt_print_i64 as *const u8);
    jb.symbol("velo_rt_print_u64", velo_rt_print_u64 as *const u8);
    jb.symbol("velo_rt_print_f64", velo_rt_print_f64 as *const u8);
    jb.symbol("velo_rt_print_bool", velo_rt_print_bool as *const u8);
    jb.symbol("velo_rt_print_str", velo_rt_print_str as *const u8);
    jb.symbol("velo_rt_print_cstr", velo_rt_print_cstr as *const u8);
    jb.symbol("velo_rt_print_newline", velo_rt_print_newline as *const u8);
    jb.symbol("velo_rt_print_space", velo_rt_print_space as *const u8);
    jb.symbol("velo_rt_panic", velo_rt_panic as *const u8);
    jb.symbol("velo_rt_bounds_fail", velo_rt_bounds_fail as *const u8);
    jb.symbol("velo_rt_alloc", velo_rt_alloc as *const u8);
    jb.symbol("velo_rt_realloc", velo_rt_realloc as *const u8);
    jb.symbol("velo_rt_free", velo_rt_free as *const u8);
    jb.symbol("velo_rt_copy_mem", velo_rt_copy_mem as *const u8);
    jb.symbol("velo_rt_set_mem", velo_rt_set_mem as *const u8);
    jb.symbol("velo_rt_time_ns", velo_rt_time_ns as *const u8);
    jb.symbol("velo_rt_sleep_ms", velo_rt_sleep_ms as *const u8);
    jb.symbol("velo_rt_rand_u64", velo_rt_rand_u64 as *const u8);
    jb.symbol("velo_rt_rand_range", velo_rt_rand_range as *const u8);
    jb.symbol("velo_rt_read_int", velo_rt_read_int as *const u8);
    jb.symbol("velo_rt_read_line", velo_rt_read_line as *const u8);
    jb.symbol("velo_rt_str_concat", velo_rt_str_concat as *const u8);
    jb.symbol("velo_rt_i64_to_str", velo_rt_i64_to_str as *const u8);
    jb.symbol("velo_rt_u64_to_str", velo_rt_u64_to_str as *const u8);
    jb.symbol("velo_rt_f64_to_str", velo_rt_f64_to_str as *const u8);
    jb.symbol("velo_rt_bool_to_str", velo_rt_bool_to_str as *const u8);
    jb.symbol("velo_rt_str_eq", velo_rt_str_eq as *const u8);
    jb.symbol("velo_rt_str_cmp", velo_rt_str_cmp as *const u8);
    jb.symbol("velo_rt_cstr_len", velo_rt_cstr_len as *const u8);
    jb.symbol("velo_rt_sqrt", velo_rt_sqrt as *const u8);
    jb.symbol("velo_rt_sin", velo_rt_sin as *const u8);
    jb.symbol("velo_rt_cos", velo_rt_cos as *const u8);
    jb.symbol("velo_rt_tan", velo_rt_tan as *const u8);
    jb.symbol("velo_rt_floor", velo_rt_floor as *const u8);
    jb.symbol("velo_rt_ceil", velo_rt_ceil as *const u8);
    jb.symbol("velo_rt_round", velo_rt_round as *const u8);
    jb.symbol("velo_rt_pow", velo_rt_pow as *const u8);
}

#[cfg(windows)]
fn resolve_external_symbol(name: &str, links: &[String]) -> Option<*const u8> {
    let cname = CString::new(name).ok()?;
    let mut libs = vec![
        "kernel32.dll".to_string(),
        "user32.dll".to_string(),
        "gdi32.dll".to_string(),
        "ucrtbase.dll".to_string(),
        "msvcrt.dll".to_string(),
    ];
    for l in links {
        let dll = if l.ends_with(".dll") { l.clone() } else { format!("{l}.dll") };
        libs.push(dll);
    }
    for lib in libs {
        if let Ok(clib) = CString::new(lib) {
            unsafe {
                let h = LoadLibraryA(clib.as_ptr());
                if !h.is_null() {
                    let p = GetProcAddress(h, cname.as_ptr());
                    if !p.is_null() {
                        return Some(p as *const u8);
                    }
                }
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn resolve_external_symbol(_name: &str, _links: &[String]) -> Option<*const u8> {
    None
}

#[derive(Clone, Copy)]
enum AggAbi {
    ByReg(types::Type), // size 1, 2, 4, 8
    ByPtr(u32),         // > 8 bytes (or 3/5/6/7)
}

fn agg_abi(size: u32) -> AggAbi {
    match size {
        1 => AggAbi::ByReg(types::I8),
        2 => AggAbi::ByReg(types::I16),
        4 => AggAbi::ByReg(types::I32),
        8 => AggAbi::ByReg(types::I64),
        _ => AggAbi::ByPtr(size),
    }
}

fn cl_scalar_type(arena: &TypeArena, ty: TypeId) -> types::Type {
    match arena.kind(ty) {
        TypeKind::Bool | TypeKind::I8 | TypeKind::U8 => types::I8,
        TypeKind::I16 | TypeKind::U16 => types::I16,
        TypeKind::I32 | TypeKind::U32 => types::I32,
        TypeKind::F32 => types::F32,
        TypeKind::F64 => types::F64,
        TypeKind::Enum(eid) => cl_scalar_type(arena, arena.enums[eid.0].repr),
        _ => types::I64,
    }
}

fn make_sig<M: Module>(
    module: &M,
    arena: &TypeArena,
    param_tys: &[TypeId],
    ret_ty: TypeId,
) -> (Signature, bool) {
    let mut sig = module.make_signature();
    let mut sret = false;
    if ret_ty != TY_VOID {
        if arena.is_aggregate(ret_ty) {
            let sz = arena.layout(ret_ty).size;
            match agg_abi(sz) {
                AggAbi::ByReg(t) => sig.returns.push(AbiParam::new(t)),
                AggAbi::ByPtr(_) => {
                    sret = true;
                    sig.params.push(AbiParam::new(types::I64));
                    sig.returns.push(AbiParam::new(types::I64));
                }
            }
        } else {
            sig.returns.push(AbiParam::new(cl_scalar_type(arena, ret_ty)));
        }
    }
    for &pt in param_tys {
        if arena.is_aggregate(pt) {
            let sz = arena.layout(pt).size;
            match agg_abi(sz) {
                AggAbi::ByReg(t) => sig.params.push(AbiParam::new(t)),
                AggAbi::ByPtr(_) => sig.params.push(AbiParam::new(types::I64)),
            }
        } else {
            sig.params.push(AbiParam::new(cl_scalar_type(arena, pt)));
        }
    }
    (sig, sret)
}

fn compile_into_module<M: Module>(
    module: &mut M,
    prog: &IrProgram,
    emit_c_main: bool,
) -> Result<Option<FuncId>, String> {
    // 1. Emit static string literals and str descriptors
    let mut cstr_ids: Vec<DataId> = Vec::with_capacity(prog.str_lits.len());
    let mut str_desc_ids: Vec<DataId> = Vec::with_capacity(prog.str_lits.len());
    for (i, s) in prog.str_lits.iter().enumerate() {
        let mut bytes = s.as_bytes().to_vec();
        bytes.push(0);
        let did = module
            .declare_data(&format!("velo_lit_{i}"), Linkage::Local, false, false)
            .map_err(|e| e.to_string())?;
        let mut desc = DataDescription::new();
        desc.define(bytes.into_boxed_slice());
        module.define_data(did, &desc).map_err(|e| e.to_string())?;
        cstr_ids.push(did);

        let desc_id = module
            .declare_data(&format!("velo_str_{i}"), Linkage::Local, false, false)
            .map_err(|e| e.to_string())?;
        let mut sdesc = DataDescription::new();
        let mut raw = [0u8; 16];
        raw[8..16].copy_from_slice(&(s.len() as i64).to_le_bytes());
        sdesc.define(raw.into());
        sdesc.set_align(8);
        let gv = module.declare_data_in_data(did, &mut sdesc);
        sdesc.write_data_addr(0, gv, 0);
        module.define_data(desc_id, &sdesc).map_err(|e| e.to_string())?;
        str_desc_ids.push(desc_id);
    }

    // Emit wide UTF-16 literals
    let mut wstr_ids: Vec<DataId> = Vec::with_capacity(prog.wstr_lits.len());
    for (i, ws) in prog.wstr_lits.iter().enumerate() {
        let mut bytes = Vec::with_capacity(ws.len() * 2);
        for &u in ws {
            bytes.extend_from_slice(&u.to_le_bytes());
        }
        let did = module
            .declare_data(&format!("velo_wlit_{i}"), Linkage::Local, false, false)
            .map_err(|e| e.to_string())?;
        let mut desc = DataDescription::new();
        desc.set_align(2);
        desc.define(bytes.into_boxed_slice());
        module.define_data(did, &desc).map_err(|e| e.to_string())?;
        wstr_ids.push(did);
    }

    // 2. Declare runtime helpers
    let rt_funcs = declare_runtime_funcs(module)?;

    // 3. Declare extern C functions
    let mut ext_ids = Vec::with_capacity(prog.externs.len());
    for ex in &prog.externs {
        let (sig, _) = make_sig(module, &prog.arena, &ex.params, ex.ret);
        let fid = module
            .declare_function(&ex.name, Linkage::Import, &sig)
            .map_err(|e| e.to_string())?;
        ext_ids.push(fid);
    }

    // 4. Declare internal Velo functions
    let mut fn_ids = Vec::with_capacity(prog.funcs.len());
    let mut fn_srets = Vec::with_capacity(prog.funcs.len());
    for f in &prog.funcs {
        let param_tys: Vec<TypeId> = f.params.iter().map(|p| p.ty).collect();
        let (sig, sret) = make_sig(module, &prog.arena, &param_tys, f.ret);
        let fid = module
            .declare_function(&f.c_name, Linkage::Local, &sig)
            .map_err(|e| e.to_string())?;
        fn_ids.push(fid);
        fn_srets.push(sret);
    }

    // 5. Compile function bodies
    let mut fb_ctx = FunctionBuilderContext::new();
    let mut ctx = module.make_context();

    let fcfg = module.target_config();
    for f in &prog.funcs {
        let param_tys: Vec<TypeId> = f.params.iter().map(|p| p.ty).collect();
        let (sig, sret) = make_sig(module, &prog.arena, &param_tys, f.ret);
        ctx.func = Function::with_name_signature(UserFuncName::user(0, f.id.0 as u32), sig);
        {
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
            {
                let mut compiler = FnCompiler {
                    module,
                    prog,
                    func: f,
                    builder: &mut builder,
                    cstr_ids: &cstr_ids,
                    str_desc_ids: &str_desc_ids,
                    wstr_ids: &wstr_ids,
                    rt_funcs: &rt_funcs,
                    ext_ids: &ext_ids,
                    fn_ids: &fn_ids,
                    fn_srets: &fn_srets,
                    sret,
                    sret_ptr: None,
                    vars: Vec::new(),
                    stack_slots: Vec::new(),
                    vregs: HashMap::new(),
                    blocks: Vec::new(),
                };
                compiler.compile();
            }
            builder.finalize(fcfg);
        }
        module
            .define_function(fn_ids[f.id.0], &mut ctx)
            .map_err(|e| format!("cranelift compile error in '{}': {e}", f.name))?;
        module.clear_context(&mut ctx);
    }

    // 6. Optional C `main(argc, argv) -> i32` entry wrapper
    if emit_c_main {
        if let Some(main_ir_id) = prog.main_fn {
            let mut main_sig = module.make_signature();
            main_sig.params.push(AbiParam::new(types::I32));
            main_sig.params.push(AbiParam::new(types::I64));
            main_sig.returns.push(AbiParam::new(types::I32));

            let main_fid = module
                .declare_function("main", Linkage::Export, &main_sig)
                .map_err(|e| e.to_string())?;
            ctx.func = Function::with_name_signature(UserFuncName::user(0, 99999), main_sig);
            {
                let mut builder = FunctionBuilder::new(&mut ctx.func, &mut fb_ctx);
                let bb = builder.create_block();
                builder.append_block_params_for_function_params(bb);
                builder.switch_to_block(bb);
                builder.seal_block(bb);

                let target_ref = module.declare_func_in_func(fn_ids[main_ir_id.0], builder.func);
                let call = builder.ins().call(target_ref, &[]);
                let mf_ret = prog.funcs[main_ir_id.0].ret;
                let exit_code = if mf_ret == TY_VOID {
                    builder.ins().iconst(types::I32, 0)
                } else {
                    let r = builder.inst_results(call)[0];
                    if mf_ret == TY_I32 || mf_ret == TY_U32 {
                        r
                    } else {
                        builder.ins().ireduce(types::I32, r)
                    }
                };
                builder.ins().return_(&[exit_code]);
                let fcfg = module.target_config();
                builder.finalize(fcfg);
            }
            module.define_function(main_fid, &mut ctx).map_err(|e| e.to_string())?;
            module.clear_context(&mut ctx);
        }
    }

    Ok(prog.main_fn.map(|id| fn_ids[id.0]))
}

struct RtFuncs {
    map: HashMap<RtOp, FuncId>,
    bounds_fail: FuncId,
}

fn declare_runtime_funcs<M: Module>(module: &mut M) -> Result<RtFuncs, String> {
    let mut map = HashMap::new();
    let specs: &[(RtOp, &str, &[types::Type], Option<types::Type>)] = &[
        (RtOp::PrintI64, "velo_rt_print_i64", &[types::I64], None),
        (RtOp::PrintU64, "velo_rt_print_u64", &[types::I64], None),
        (RtOp::PrintF64, "velo_rt_print_f64", &[types::F64], None),
        (RtOp::PrintBool, "velo_rt_print_bool", &[types::I8], None),
        (RtOp::PrintStr, "velo_rt_print_str", &[types::I64, types::I64], None),
        (RtOp::PrintCStr, "velo_rt_print_cstr", &[types::I64], None),
        (RtOp::PrintNewline, "velo_rt_print_newline", &[], None),
        (RtOp::PrintSpace, "velo_rt_print_space", &[], None),
        (RtOp::Panic, "velo_rt_panic", &[types::I64, types::I64, types::I64, types::I64, types::I64], None),
        (RtOp::Alloc, "velo_rt_alloc", &[types::I64], Some(types::I64)),
        (RtOp::Realloc, "velo_rt_realloc", &[types::I64, types::I64], Some(types::I64)),
        (RtOp::Free, "velo_rt_free", &[types::I64], None),
        (RtOp::CopyMem, "velo_rt_copy_mem", &[types::I64, types::I64, types::I64], None),
        (RtOp::SetMem, "velo_rt_set_mem", &[types::I64, types::I8, types::I64], None),
        (RtOp::TimeNs, "velo_rt_time_ns", &[], Some(types::I64)),
        (RtOp::SleepMs, "velo_rt_sleep_ms", &[types::I32], None),
        (RtOp::RandU64, "velo_rt_rand_u64", &[], Some(types::I64)),
        (RtOp::RandRange, "velo_rt_rand_range", &[types::I64, types::I64], Some(types::I64)),
        (RtOp::ReadInt, "velo_rt_read_int", &[], Some(types::I64)),
        (RtOp::ReadLine, "velo_rt_read_line", &[types::I64], Some(types::I64)),
        (RtOp::StrConcat, "velo_rt_str_concat", &[types::I64, types::I64, types::I64, types::I64, types::I64, types::I64], None),
        (RtOp::I64ToStr, "velo_rt_i64_to_str", &[types::I64, types::I64, types::I64], None),
        (RtOp::U64ToStr, "velo_rt_u64_to_str", &[types::I64, types::I64, types::I64], None),
        (RtOp::F64ToStr, "velo_rt_f64_to_str", &[types::F64, types::I32, types::I64, types::I64], None),
        (RtOp::BoolToStr, "velo_rt_bool_to_str", &[types::I8, types::I64, types::I64], None),
        (RtOp::StrEq, "velo_rt_str_eq", &[types::I64, types::I64, types::I64, types::I64], Some(types::I8)),
        (RtOp::StrCmp, "velo_rt_str_cmp", &[types::I64, types::I64, types::I64, types::I64], Some(types::I32)),
        (RtOp::CStrLen, "velo_rt_cstr_len", &[types::I64], Some(types::I64)),
        (RtOp::SqrtF64, "velo_rt_sqrt", &[types::F64], Some(types::F64)),
        (RtOp::SinF64, "velo_rt_sin", &[types::F64], Some(types::F64)),
        (RtOp::CosF64, "velo_rt_cos", &[types::F64], Some(types::F64)),
        (RtOp::TanF64, "velo_rt_tan", &[types::F64], Some(types::F64)),
        (RtOp::FloorF64, "velo_rt_floor", &[types::F64], Some(types::F64)),
        (RtOp::CeilF64, "velo_rt_ceil", &[types::F64], Some(types::F64)),
        (RtOp::RoundF64, "velo_rt_round", &[types::F64], Some(types::F64)),
        (RtOp::PowF64, "velo_rt_pow", &[types::F64, types::F64], Some(types::F64)),
    ];

    for &(op, name, ps, ret) in specs {
        let mut sig = module.make_signature();
        for &pt in ps {
            sig.params.push(AbiParam::new(pt));
        }
        if let Some(rt) = ret {
            sig.returns.push(AbiParam::new(rt));
        }
        let fid = module
            .declare_function(name, Linkage::Import, &sig)
            .map_err(|e| e.to_string())?;
        map.insert(op, fid);
    }

    let mut bf_sig = module.make_signature();
    for _ in 0..5 {
        bf_sig.params.push(AbiParam::new(types::I64));
    }
    let bounds_fail = module
        .declare_function("velo_rt_bounds_fail", Linkage::Import, &bf_sig)
        .map_err(|e| e.to_string())?;

    Ok(RtFuncs { map, bounds_fail })
}

struct FnCompiler<'a, 'b, M: Module> {
    module: &'a mut M,
    prog: &'a IrProgram,
    func: &'a IrFunction,
    builder: &'b mut FunctionBuilder<'a>,
    cstr_ids: &'a [DataId],
    str_desc_ids: &'a [DataId],
    wstr_ids: &'a [DataId],
    rt_funcs: &'a RtFuncs,
    ext_ids: &'a [FuncId],
    fn_ids: &'a [FuncId],
    fn_srets: &'a [bool],
    sret: bool,
    sret_ptr: Option<Value>,
    vars: Vec<Option<Variable>>,
    stack_slots: Vec<Option<StackSlot>>,
    vregs: HashMap<VReg, Value>,
    blocks: Vec<ClBlock>,
}

impl<'a, 'b, M: Module> FnCompiler<'a, 'b, M> {
    fn compile(&mut self) {
        for _ in &self.func.blocks {
            self.blocks.push(self.builder.create_block());
        }

        let mut needs_stack_slot = vec![false; self.func.slots.len()];
        for (idx, sl) in self.func.slots.iter().enumerate() {
            if sl.addr_taken || self.prog.arena.is_aggregate(sl.ty) {
                needs_stack_slot[idx] = true;
            }
        }
        for bb in &self.func.blocks {
            for inst in &bb.insts {
                if let IrInst::SlotAddr { slot, .. } = inst {
                    needs_stack_slot[slot.0 as usize] = true;
                }
            }
        }

        // Allocate Cranelift Variable for scalar non-addr-taken slots, StackSlot for memory/aggregate slots
        for (idx, sl) in self.func.slots.iter().enumerate() {
            if !needs_stack_slot[idx] {
                let var = self.builder.declare_var(cl_scalar_type(&self.prog.arena, sl.ty));
                self.vars.push(Some(var));
                self.stack_slots.push(None);
            } else {
                let layout = self.prog.arena.layout(sl.ty);
                let align_shift = layout.align.max(1).trailing_zeros() as u8;
                let ss = self.builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    layout.size.max(1),
                    align_shift,
                ));
                self.vars.push(None);
                self.stack_slots.push(Some(ss));
            }
        }

        // Setup entry block parameters
        let entry = self.blocks[0];
        self.builder.append_block_params_for_function_params(entry);
        self.builder.switch_to_block(entry);

        let entry_params: Vec<Value> = self.builder.block_params(entry).to_vec();
        let mut pidx = 0usize;
        if self.sret {
            self.sret_ptr = Some(entry_params[0]);
            pidx = 1;
        }
        for p in &self.func.params {
            let pval = entry_params[pidx];
            pidx += 1;
            let s_idx = p.slot.0 as usize;
            if self.prog.arena.is_aggregate(p.ty) {
                let sz = self.prog.arena.layout(p.ty).size;
                let dst_ptr = self.slot_addr_val(p.slot);
                match agg_abi(sz) {
                    AggAbi::ByReg(_) => {
                        self.builder.ins().store(MemFlagsData::new(), pval, dst_ptr, 0);
                    }
                    AggAbi::ByPtr(_) => {
                        self.emit_memcpy(dst_ptr, pval, sz);
                    }
                }
            } else if let Some(var) = self.vars[s_idx] {
                self.builder.def_var(var, pval);
            } else {
                let ptr = self.slot_addr_val(p.slot);
                self.builder.ins().store(MemFlagsData::new(), pval, ptr, 0);
            }
        }

        // Compile every basic block
        for (bi, ir_bb) in self.func.blocks.iter().enumerate() {
            let cl_bb = self.blocks[bi];
            if bi != 0 {
                self.builder.switch_to_block(cl_bb);
            }
            for inst in &ir_bb.insts {
                self.compile_inst(inst);
            }
            self.compile_term(&ir_bb.term);
        }

        self.builder.seal_all_blocks();
    }

    fn slot_addr_val(&mut self, slot: SlotId) -> Value {
        let ss = self.stack_slots[slot.0 as usize].expect("slot must have StackSlot");
        self.builder.ins().stack_addr(types::I64, ss, 0)
    }

    fn val(&self, v: VReg) -> Value {
        self.vregs[&v]
    }

    fn emit_memcpy(&mut self, dst: Value, src: Value, bytes: u32) {
        if bytes <= 64 {
            let mut off = 0i32;
            let mut rem = bytes;
            while rem >= 8 {
                let v = self.builder.ins().load(types::I64, MemFlagsData::new(), src, off);
                self.builder.ins().store(MemFlagsData::new(), v, dst, off);
                off += 8;
                rem -= 8;
            }
            while rem >= 4 {
                let v = self.builder.ins().load(types::I32, MemFlagsData::new(), src, off);
                self.builder.ins().store(MemFlagsData::new(), v, dst, off);
                off += 4;
                rem -= 4;
            }
            while rem >= 2 {
                let v = self.builder.ins().load(types::I16, MemFlagsData::new(), src, off);
                self.builder.ins().store(MemFlagsData::new(), v, dst, off);
                off += 2;
                rem -= 2;
            }
            while rem >= 1 {
                let v = self.builder.ins().load(types::I8, MemFlagsData::new(), src, off);
                self.builder.ins().store(MemFlagsData::new(), v, dst, off);
                off += 1;
                rem -= 1;
            }
        } else {
            let fid = self.rt_funcs.map[&RtOp::CopyMem];
            let fref = self.module.declare_func_in_func(fid, self.builder.func);
            let b = self.builder.ins().iconst(types::I64, bytes as i64);
            self.builder.ins().call(fref, &[dst, src, b]);
        }
    }

    fn emit_memzero(&mut self, dst: Value, bytes: u32) {
        if bytes <= 64 {
            let z64 = self.builder.ins().iconst(types::I64, 0);
            let z8 = self.builder.ins().iconst(types::I8, 0);
            let mut off = 0i32;
            let mut rem = bytes;
            while rem >= 8 {
                self.builder.ins().store(MemFlagsData::new(), z64, dst, off);
                off += 8;
                rem -= 8;
            }
            while rem >= 1 {
                self.builder.ins().store(MemFlagsData::new(), z8, dst, off);
                off += 1;
                rem -= 1;
            }
        } else {
            let fid = self.rt_funcs.map[&RtOp::SetMem];
            let fref = self.module.declare_func_in_func(fid, self.builder.func);
            let z = self.builder.ins().iconst(types::I8, 0);
            let b = self.builder.ins().iconst(types::I64, bytes as i64);
            self.builder.ins().call(fref, &[dst, z, b]);
        }
    }

    fn compile_inst(&mut self, inst: &IrInst) {
        match inst {
            IrInst::ConstInt { dst, ty, val } => {
                let cty = cl_scalar_type(&self.prog.arena, *ty);
                let v = self.builder.ins().iconst(cty, *val as i64);
                self.vregs.insert(*dst, v);
            }
            IrInst::ConstFloat { dst, ty, val } => {
                let v = if *ty == TY_F32 {
                    self.builder.ins().f32const(*val as f32)
                } else {
                    self.builder.ins().f64const(*val)
                };
                self.vregs.insert(*dst, v);
            }
            IrInst::ConstBool { dst, val } => {
                let v = self.builder.ins().iconst(types::I8, if *val { 1 } else { 0 });
                self.vregs.insert(*dst, v);
            }
            IrInst::ConstNull { dst, .. } => {
                let v = self.builder.ins().iconst(types::I64, 0);
                self.vregs.insert(*dst, v);
            }
            IrInst::ConstStr { dst, lit } => {
                let gv = self.module.declare_data_in_func(self.str_desc_ids[lit.0], self.builder.func);
                let v = self.builder.ins().symbol_value(types::I64, gv);
                self.vregs.insert(*dst, v);
            }
            IrInst::ConstCStr { dst, lit } => {
                let gv = self.module.declare_data_in_func(self.cstr_ids[lit.0], self.builder.func);
                let v = self.builder.ins().symbol_value(types::I64, gv);
                self.vregs.insert(*dst, v);
            }
            IrInst::ConstWStr { dst, lit } => {
                let gv = self.module.declare_data_in_func(self.wstr_ids[*lit], self.builder.func);
                let v = self.builder.ins().symbol_value(types::I64, gv);
                self.vregs.insert(*dst, v);
            }
            IrInst::FnAddr { dst, func, .. } => {
                let fref = self.module.declare_func_in_func(self.fn_ids[func.0], self.builder.func);
                let v = self.builder.ins().func_addr(types::I64, fref);
                self.vregs.insert(*dst, v);
            }
            IrInst::Copy { dst, src } => {
                if let Some(&v) = self.vregs.get(src) {
                    self.vregs.insert(*dst, v);
                }
            }
            IrInst::Unary { dst, op, src, ty } => {
                let sv = self.val(*src);
                let res = match op {
                    UnaryOp::Neg => {
                        if self.prog.arena.is_float(*ty) {
                            self.builder.ins().fneg(sv)
                        } else {
                            self.builder.ins().ineg(sv)
                        }
                    }
                    UnaryOp::Not => self.builder.ins().bxor_imm_u(sv, 1),
                    UnaryOp::BitNot => self.builder.ins().bnot(sv),
                };
                self.vregs.insert(*dst, res);
            }
            IrInst::Binary { dst, op, left, right, ty } => {
                let lv = self.val(*left);
                let rv = self.val(*right);
                let op_ty = self.func.vreg_types[left.0 as usize];
                let is_float = self.prog.arena.is_float(op_ty);
                let is_signed = self.prog.arena.is_signed_int(op_ty)
                    || matches!(self.prog.arena.kind(op_ty), TypeKind::Enum(_));
                let _ = ty;

                let res = if is_float {
                    match op {
                        BinOp::Add => self.builder.ins().fadd(lv, rv),
                        BinOp::Sub => self.builder.ins().fsub(lv, rv),
                        BinOp::Mul => self.builder.ins().fmul(lv, rv),
                        BinOp::Div => self.builder.ins().fdiv(lv, rv),
                        BinOp::Eq => self.builder.ins().fcmp(FloatCC::Equal, lv, rv),
                        BinOp::Ne => self.builder.ins().fcmp(FloatCC::NotEqual, lv, rv),
                        BinOp::Lt => self.builder.ins().fcmp(FloatCC::LessThan, lv, rv),
                        BinOp::Le => self.builder.ins().fcmp(FloatCC::LessThanOrEqual, lv, rv),
                        BinOp::Gt => self.builder.ins().fcmp(FloatCC::GreaterThan, lv, rv),
                        BinOp::Ge => self.builder.ins().fcmp(FloatCC::GreaterThanOrEqual, lv, rv),
                        _ => unreachable!(),
                    }
                } else {
                    match op {
                        BinOp::Add => self.builder.ins().iadd(lv, rv),
                        BinOp::Sub => self.builder.ins().isub(lv, rv),
                        BinOp::Mul => self.builder.ins().imul(lv, rv),
                        BinOp::Div => {
                            if is_signed { self.builder.ins().sdiv(lv, rv) } else { self.builder.ins().udiv(lv, rv) }
                        }
                        BinOp::Mod => {
                            if is_signed { self.builder.ins().srem(lv, rv) } else { self.builder.ins().urem(lv, rv) }
                        }
                        BinOp::BitAnd | BinOp::And => self.builder.ins().band(lv, rv),
                        BinOp::BitOr | BinOp::Or => self.builder.ins().bor(lv, rv),
                        BinOp::BitXor => self.builder.ins().bxor(lv, rv),
                        BinOp::Shl => self.builder.ins().ishl(lv, rv),
                        BinOp::Shr => {
                            if is_signed { self.builder.ins().sshr(lv, rv) } else { self.builder.ins().ushr(lv, rv) }
                        }
                        BinOp::Eq => self.builder.ins().icmp(IntCC::Equal, lv, rv),
                        BinOp::Ne => self.builder.ins().icmp(IntCC::NotEqual, lv, rv),
                        BinOp::Lt => self.builder.ins().icmp(
                            if is_signed { IntCC::SignedLessThan } else { IntCC::UnsignedLessThan }, lv, rv
                        ),
                        BinOp::Le => self.builder.ins().icmp(
                            if is_signed { IntCC::SignedLessThanOrEqual } else { IntCC::UnsignedLessThanOrEqual }, lv, rv
                        ),
                        BinOp::Gt => self.builder.ins().icmp(
                            if is_signed { IntCC::SignedGreaterThan } else { IntCC::UnsignedGreaterThan }, lv, rv
                        ),
                        BinOp::Ge => self.builder.ins().icmp(
                            if is_signed { IntCC::SignedGreaterThanOrEqual } else { IntCC::UnsignedGreaterThanOrEqual }, lv, rv
                        ),
                    }
                };
                self.vregs.insert(*dst, res);
            }
            IrInst::Cast { dst, src, from, to } => {
                let sv = self.val(*src);
                let res = self.emit_cast(sv, *from, *to);
                self.vregs.insert(*dst, res);
            }
            IrInst::SlotAddr { dst, slot, .. } => {
                let p = self.slot_addr_val(*slot);
                self.vregs.insert(*dst, p);
            }
            IrInst::LoadSlot { dst, slot, ty } => {
                let s_idx = slot.0 as usize;
                let v = if let Some(var) = self.vars[s_idx] {
                    self.builder.use_var(var)
                } else {
                    let p = self.slot_addr_val(*slot);
                    let cty = cl_scalar_type(&self.prog.arena, *ty);
                    self.builder.ins().load(cty, MemFlagsData::new(), p, 0)
                };
                self.vregs.insert(*dst, v);
            }
            IrInst::StoreSlot { slot, val, .. } => {
                let s_idx = slot.0 as usize;
                let v = self.val(*val);
                if let Some(var) = self.vars[s_idx] {
                    self.builder.def_var(var, v);
                } else {
                    let p = self.slot_addr_val(*slot);
                    self.builder.ins().store(MemFlagsData::new(), v, p, 0);
                }
            }
            IrInst::ZeroSlot { slot, ty } => {
                let s_idx = slot.0 as usize;
                if let Some(var) = self.vars[s_idx] {
                    let cty = cl_scalar_type(&self.prog.arena, *ty);
                    let z = if cty == types::F32 {
                        self.builder.ins().f32const(0.0)
                    } else if cty == types::F64 {
                        self.builder.ins().f64const(0.0)
                    } else {
                        self.builder.ins().iconst(cty, 0)
                    };
                    self.builder.def_var(var, z);
                } else {
                    let p = self.slot_addr_val(*slot);
                    let sz = self.prog.arena.layout(*ty).size;
                    self.emit_memzero(p, sz);
                }
            }
            IrInst::LoadPtr { dst, ptr, ty, trusted } => {
                let pv = self.val(*ptr);
                let cty = cl_scalar_type(&self.prog.arena, *ty);
                let flags = if *trusted { MemFlagsData::trusted() } else { MemFlagsData::new() };
                let v = self.builder.ins().load(cty, flags, pv, 0);
                self.vregs.insert(*dst, v);
            }
            IrInst::StorePtr { ptr, val, trusted, .. } => {
                let pv = self.val(*ptr);
                let vv = self.val(*val);
                let flags = if *trusted { MemFlagsData::trusted() } else { MemFlagsData::new() };
                self.builder.ins().store(flags, vv, pv, 0);
            }
            IrInst::FieldPtr { dst, base_ptr, offset, .. } => {
                let bp = self.val(*base_ptr);
                let p = if *offset == 0 {
                    bp
                } else {
                    self.builder.ins().iadd_imm_s(bp, *offset as i64)
                };
                self.vregs.insert(*dst, p);
            }
            IrInst::IndexPtr { dst, base_ptr, index, elem_size, .. } => {
                let bp = self.val(*base_ptr);
                let idx = self.val(*index);
                let off = if *elem_size == 1 {
                    idx
                } else {
                    self.builder.ins().imul_imm_s(idx, *elem_size as i64)
                };
                let p = self.builder.ins().iadd(bp, off);
                self.vregs.insert(*dst, p);
            }
            IrInst::BoundsCheck { index, len, file_lit, line } => {
                let iv = self.val(*index);
                let lv = self.val(*len);
                let ok_bb = self.builder.create_block();
                let fail_bb = self.builder.create_block();
                let cond = self.builder.ins().icmp(IntCC::UnsignedLessThan, iv, lv);
                self.builder.ins().brif(cond, ok_bb, &[], fail_bb, &[]);

                self.builder.switch_to_block(fail_bb);
                let fref = self.module.declare_func_in_func(self.rt_funcs.bounds_fail, self.builder.func);
                let gv = self.module.declare_data_in_func(self.cstr_ids[file_lit.0], self.builder.func);
                let fp = self.builder.ins().symbol_value(types::I64, gv);
                let fl = self.builder.ins().iconst(types::I64, self.prog.str_lits[file_lit.0].len() as i64);
                let ln = self.builder.ins().iconst(types::I64, *line);
                self.builder.ins().call(fref, &[iv, lv, fp, fl, ln]);
                self.builder.ins().trap(cranelift_codegen::ir::TrapCode::unwrap_user(1));

                self.builder.switch_to_block(ok_bb);
            }
            IrInst::MemCopy { dst_ptr, src_ptr, bytes } => {
                let d = self.val(*dst_ptr);
                let s = self.val(*src_ptr);
                self.emit_memcpy(d, s, *bytes);
            }
            IrInst::MemZero { dst_ptr, bytes } => {
                let d = self.val(*dst_ptr);
                self.emit_memzero(d, *bytes);
            }
            IrInst::Call { dst, func, args, ret } => {
                let param_tys: Vec<TypeId> = self.prog.funcs[func.0].params.iter().map(|p| p.ty).collect();
                let sret = self.fn_srets[func.0];
                let fref = self.module.declare_func_in_func(self.fn_ids[func.0], self.builder.func);
                self.emit_abi_call(Some(fref), None, &param_tys, *ret, sret, args, *dst);
            }
            IrInst::CallExtern { dst, ext, args, ret } => {
                let ex = &self.prog.externs[ext.0];
                let sret = *ret != TY_VOID
                    && self.prog.arena.is_aggregate(*ret)
                    && matches!(agg_abi(self.prog.arena.layout(*ret).size), AggAbi::ByPtr(_));
                let fref = self.module.declare_func_in_func(self.ext_ids[ext.0], self.builder.func);
                self.emit_abi_call(Some(fref), None, &ex.params, *ret, sret, args, *dst);
            }
            IrInst::CallIndirect { dst, callee, args, fn_ty, ret } => {
                let param_tys = match self.prog.arena.kind(*fn_ty) {
                    TypeKind::Fn(ps, _) => ps.clone(),
                    _ => Vec::new(),
                };
                let (sig, sret) = make_sig(self.module, &self.prog.arena, &param_tys, *ret);
                let sig_ref = self.builder.import_signature(sig);
                let cval = self.val(*callee);
                self.emit_abi_call(None, Some((cval, sig_ref)), &param_tys, *ret, sret, args, *dst);
            }
            IrInst::RuntimeCall { dst, op, args, .. } => {
                let fid = self.rt_funcs.map[op];
                let fref = self.module.declare_func_in_func(fid, self.builder.func);
                let vals: Vec<Value> = args.iter().map(|a| self.val(*a)).collect();
                let call = self.builder.ins().call(fref, &vals);
                if let Some(d) = dst {
                    let r = self.builder.inst_results(call)[0];
                    self.vregs.insert(*d, r);
                }
            }
        }
    }

    fn emit_abi_call(
        &mut self,
        direct: Option<cranelift_codegen::ir::FuncRef>,
        indirect: Option<(Value, cranelift_codegen::ir::SigRef)>,
        param_tys: &[TypeId],
        ret_ty: TypeId,
        sret: bool,
        args: &[VReg],
        dst: Option<VReg>,
    ) {
        let mut call_args = Vec::new();
        let ret_ss = if sret {
            let layout = self.prog.arena.layout(ret_ty);
            let ss = self.builder.create_sized_stack_slot(StackSlotData::new(
                StackSlotKind::ExplicitSlot,
                layout.size.max(1),
                layout.align.max(1).trailing_zeros() as u8,
            ));
            let sret_addr = self.builder.ins().stack_addr(types::I64, ss, 0);
            call_args.push(sret_addr);
            Some((ss, sret_addr))
        } else {
            None
        };

        for (i, &arg) in args.iter().enumerate() {
            let v = self.val(arg);
            if let Some(&pty) = param_tys.get(i) {
                if self.prog.arena.is_aggregate(pty) {
                    let layout = self.prog.arena.layout(pty);
                    match agg_abi(layout.size) {
                        AggAbi::ByReg(t) => {
                            let loaded = self.builder.ins().load(t, MemFlagsData::new(), v, 0);
                            call_args.push(loaded);
                        }
                        AggAbi::ByPtr(sz) => {
                            let copy_ss = self.builder.create_sized_stack_slot(StackSlotData::new(
                                StackSlotKind::ExplicitSlot,
                                sz.max(1),
                                layout.align.max(1).trailing_zeros() as u8,
                            ));
                            let copy_ptr = self.builder.ins().stack_addr(types::I64, copy_ss, 0);
                            self.emit_memcpy(copy_ptr, v, sz);
                            call_args.push(copy_ptr);
                        }
                    }
                    continue;
                }
            }
            call_args.push(v);
        }

        let call = if let Some(fref) = direct {
            self.builder.ins().call(fref, &call_args)
        } else if let Some((cval, sig_ref)) = indirect {
            self.builder.ins().call_indirect(sig_ref, cval, &call_args)
        } else {
            unreachable!()
        };

        if let Some(d) = dst {
            if ret_ty == TY_VOID {
                return;
            }
            if let Some((_, sret_addr)) = ret_ss {
                self.vregs.insert(d, sret_addr);
            } else if self.prog.arena.is_aggregate(ret_ty) {
                let layout = self.prog.arena.layout(ret_ty);
                let ss = self.builder.create_sized_stack_slot(StackSlotData::new(
                    StackSlotKind::ExplicitSlot,
                    layout.size.max(1),
                    layout.align.max(1).trailing_zeros() as u8,
                ));
                let addr = self.builder.ins().stack_addr(types::I64, ss, 0);
                let res = self.builder.inst_results(call)[0];
                self.builder.ins().store(MemFlagsData::new(), res, addr, 0);
                self.vregs.insert(d, addr);
            } else {
                let res = self.builder.inst_results(call)[0];
                self.vregs.insert(d, res);
            }
        }
    }

    fn emit_cast(&mut self, v: Value, from: TypeId, to: TypeId) -> Value {
        if from == to {
            return v;
        }
        let from_f = self.prog.arena.is_float(from);
        let to_f = self.prog.arena.is_float(to);
        let from_s = self.prog.arena.is_signed_int(from)
            || matches!(self.prog.arena.kind(from), TypeKind::Enum(_));
        let to_s = self.prog.arena.is_signed_int(to)
            || matches!(self.prog.arena.kind(to), TypeKind::Enum(_));
        let cl_from = cl_scalar_type(&self.prog.arena, from);
        let cl_to = cl_scalar_type(&self.prog.arena, to);

        if to == TY_BOOL {
            let zero = self.builder.ins().iconst(cl_from, 0);
            return self.builder.ins().icmp(IntCC::NotEqual, v, zero);
        }
        if !from_f && !to_f {
            let from_bits = cl_from.bits();
            let to_bits = cl_to.bits();
            if from_bits == to_bits {
                v
            } else if to_bits < from_bits {
                self.builder.ins().ireduce(cl_to, v)
            } else if from_s {
                self.builder.ins().sextend(cl_to, v)
            } else {
                self.builder.ins().uextend(cl_to, v)
            }
        } else if from_f && to_f {
            if cl_from == cl_to {
                v
            } else if cl_to == types::F64 {
                self.builder.ins().fpromote(types::F64, v)
            } else {
                self.builder.ins().fdemote(types::F32, v)
            }
        } else if !from_f && to_f {
            if from_s {
                self.builder.ins().fcvt_from_sint(cl_to, v)
            } else {
                self.builder.ins().fcvt_from_uint(cl_to, v)
            }
        } else {
            if to_s {
                self.builder.ins().fcvt_to_sint_sat(cl_to, v)
            } else {
                self.builder.ins().fcvt_to_uint_sat(cl_to, v)
            }
        }
    }

    fn compile_term(&mut self, term: &IrTerm) {
        match term {
            IrTerm::Jump(t) => {
                self.builder.ins().jump(self.blocks[t.0 as usize], &[]);
            }
            IrTerm::BranchIf { cond, then_bb, else_bb } => {
                let c = self.val(*cond);
                self.builder.ins().brif(
                    c,
                    self.blocks[then_bb.0 as usize],
                    &[],
                    self.blocks[else_bb.0 as usize],
                    &[],
                );
            }
            IrTerm::Return(opt) => match opt {
                None => {
                    self.builder.ins().return_(&[]);
                }
                Some(v) => {
                    let val = self.val(*v);
                    if self.func.ret == TY_VOID {
                        self.builder.ins().return_(&[]);
                    } else if self.prog.arena.is_aggregate(self.func.ret) {
                        let sz = self.prog.arena.layout(self.func.ret).size;
                        match agg_abi(sz) {
                            AggAbi::ByReg(t) => {
                                let loaded = self.builder.ins().load(t, MemFlagsData::new(), val, 0);
                                self.builder.ins().return_(&[loaded]);
                            }
                            AggAbi::ByPtr(_) => {
                                let sret = self.sret_ptr.expect("sret missing");
                                self.emit_memcpy(sret, val, sz);
                                self.builder.ins().return_(&[sret]);
                            }
                        }
                    } else {
                        self.builder.ins().return_(&[val]);
                    }
                }
            },
            IrTerm::Unreachable => {
                self.builder.ins().trap(cranelift_codegen::ir::TrapCode::unwrap_user(2));
            }
        }
    }
}
