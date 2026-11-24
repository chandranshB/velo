use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::error::{Diagnostic, SourceFile, Span};
use crate::ir::{ExternId, FnId, Subsystem};
use crate::loader::LoadedProject;
use crate::types::*;

#[derive(Debug, Clone)]
pub enum ConstValue {
    Int(i128),
    Float(f64),
    Bool(bool),
    Str(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalId(pub u32);

#[derive(Debug, Clone)]
pub struct CheckedLocal {
    pub id: LocalId,
    pub name: String,
    pub ty: TypeId,
    pub is_mut: bool,
    pub is_param: bool,
    pub noalias: bool,
    pub addr_taken: bool,
}

#[derive(Debug, Clone)]
pub struct CheckedExtern {
    pub id: ExternId,
    pub name: String,
    pub params: Vec<TypeId>,
    pub ret: TypeId,
    pub variadic: bool,
}

#[derive(Debug, Clone)]
pub struct CheckedFn {
    pub id: FnId,
    pub name: String,
    pub c_name: String,
    pub params: Vec<LocalId>,
    pub ret: TypeId,
    pub inline_hint: bool,
    pub locals: Vec<CheckedLocal>,
    pub body: Vec<TStmt>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct TStmt {
    pub kind: TStmtKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TStmtKind {
    VarInit {
        local: LocalId,
        value: TExpr,
    },
    Assign {
        target: TExpr,
        op: Option<BinOp>,
        value: TExpr,
    },
    Expr(TExpr),
    If {
        branches: Vec<(TExpr, Vec<TStmt>)>,
        else_body: Option<Vec<TStmt>>,
    },
    While {
        cond: TExpr,
        body: Vec<TStmt>,
    },
    ForRange {
        var_local: LocalId,
        start: TExpr,
        end: TExpr,
        inclusive: bool,
        body: Vec<TStmt>,
    },
    ForIter {
        var_local: LocalId,
        iter: TExpr,
        body: Vec<TStmt>,
    },
    Match {
        expr: TExpr,
        cases: Vec<(Vec<TExpr>, Vec<TStmt>)>,
        else_body: Option<Vec<TStmt>>,
    },
    Return(Option<TExpr>),
    Break,
    Continue,
    Defer(Box<TStmt>),
    Pass,
}

#[derive(Debug, Clone)]
pub struct TExpr {
    pub kind: TExprKind,
    pub ty: TypeId,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum BuiltinCall {
    Print { newline: bool },
    Assert,
    Panic,
    Alloc,
    Realloc,
    Free,
    CopyMem,
    SetMem,
    TimeNs,
    SleepMs,
    RandU64,
    RandRange,
    ReadInt,
    ReadLine,
    Min,
    Max,
    Abs,
    Clamp,
    Sqrt,
    Sin,
    Cos,
    Tan,
    Floor,
    Ceil,
    Round,
    Pow,
    CStrLen,
    StrFromCStr,
}

#[derive(Debug, Clone)]
pub enum TExprKind {
    Int(i128),
    Float(f64),
    Bool(bool),
    Str(String),
    CStr(String),
    WStr(String),
    FStr(Vec<TFStrSeg>),
    Null,
    Local(LocalId),
    FnRef(FnId),
    Unary {
        op: UnaryOp,
        expr: Box<TExpr>,
    },
    Binary {
        op: BinOp,
        left: Box<TExpr>,
        right: Box<TExpr>,
    },
    Cast {
        expr: Box<TExpr>,
        to: TypeId,
    },
    CallFn {
        func: FnId,
        args: Vec<TExpr>,
    },
    CallExtern {
        ext: ExternId,
        args: Vec<TExpr>,
    },
    CallIndirect {
        callee: Box<TExpr>,
        args: Vec<TExpr>,
    },
    CallBuiltin {
        builtin: BuiltinCall,
        args: Vec<TExpr>,
    },
    Field {
        obj: Box<TExpr>,
        offset: u32,
        through_ptr: bool,
    },
    StrLen(Box<TExpr>),
    StrPtr(Box<TExpr>),
    SliceLen(Box<TExpr>),
    SlicePtr(Box<TExpr>),
    Index {
        obj: Box<TExpr>,
        index: Box<TExpr>,
        proved_in_bounds: bool,
    },
    Slice {
        obj: Box<TExpr>,
        start: Option<Box<TExpr>>,
        end: Option<Box<TExpr>>,
    },
    StructLit {
        #[allow(dead_code)]
        sid: StructId,
        fields: Vec<(u32, TypeId, TExpr)>,
    },
    ArrayLit(Vec<TExpr>),
    ArrayRepeat {
        value: Box<TExpr>,
        count: u64,
    },
    Ternary {
        then_expr: Box<TExpr>,
        cond: Box<TExpr>,
        else_expr: Box<TExpr>,
    },
    AddrOf(Box<TExpr>),
    Deref(Box<TExpr>),
    Zeroed,
}

#[derive(Debug, Clone)]
pub enum TFStrSeg {
    Lit(String),
    Interp { expr: TExpr, prec: Option<i32> },
}

pub struct CheckedProgram {
    pub files: Vec<SourceFile>,
    pub arena: TypeArena,
    pub links: Vec<String>,
    pub subsystem: Subsystem,
    pub externs: Vec<CheckedExtern>,
    pub funcs: Vec<CheckedFn>,
    pub main_fn: Option<FnId>,
}

#[derive(Debug, Clone)]
enum ModItem {
    Const(ConstValue, TypeId),
    Enum(EnumId),
    Struct(StructId),
    Extern(ExternId),
    Fn(FnId),
}

#[derive(Default)]
struct ModuleEnv {
    items: HashMap<String, ModItem>,
    submodules: HashMap<String, usize>,
}

#[derive(Debug, Clone)]
enum BoundsUpper {
    Const(i128),
    ArrayLenOf(LocalId),
}

pub fn check_project(project: LoadedProject) -> Result<CheckedProgram, (Vec<SourceFile>, Diagnostic)> {
    let mut checker = Checker {
        files: project.files,
        modules: project.modules,
        entry_mod: project.entry_mod,
        arena: TypeArena::new(),
        mod_envs: Vec::new(),
        struct_methods: HashMap::new(),
        struct_statics: HashMap::new(),
        externs: Vec::new(),
        extern_by_name: HashMap::new(),
        funcs: Vec::new(),
        fn_Asts: Vec::new(),
        links: Vec::new(),
        subsystem: Subsystem::Console,
    };

    if let Err(d) = checker.run() {
        return Err((checker.files, d));
    }

    let main_fn = checker.mod_envs[checker.entry_mod]
        .items
        .get("main")
        .and_then(|it| match it {
            ModItem::Fn(id) => Some(*id),
            _ => None,
        });

    Ok(CheckedProgram {
        files: checker.files,
        arena: checker.arena,
        links: checker.links,
        subsystem: checker.subsystem,
        externs: checker.externs,
        funcs: checker.funcs,
        main_fn,
    })
}

struct PendingFn {
    mod_idx: usize,
    fn_id: FnId,
    decl: FnDecl,
    self_struct: Option<StructId>,
    locals: Vec<CheckedLocal>,
    param_ids: Vec<LocalId>,
    ret: TypeId,
}

#[allow(non_snake_case)]
struct Checker {
    files: Vec<SourceFile>,
    modules: Vec<crate::loader::LoadedModule>,
    entry_mod: usize,
    arena: TypeArena,
    mod_envs: Vec<ModuleEnv>,
    struct_methods: HashMap<(StructId, String), FnId>,
    struct_statics: HashMap<(StructId, String), FnId>,
    externs: Vec<CheckedExtern>,
    extern_by_name: HashMap<String, ExternId>,
    funcs: Vec<CheckedFn>,
    fn_Asts: Vec<PendingFn>,
    links: Vec<String>,
    subsystem: Subsystem,
}

impl Checker {
    fn run(&mut self) -> Result<(), Diagnostic> {
        self.mod_envs = (0..self.modules.len()).map(|_| ModuleEnv::default()).collect();

        // Collect links and subsystem
        let mut link_set = HashSet::new();
        for (idx, m) in self.modules.iter().enumerate() {
            for (lib, _) in &m.ast.links {
                if link_set.insert(lib.clone()) {
                    self.links.push(lib.clone());
                }
            }
            if idx == self.entry_mod {
                if let Some((sub, sp)) = &m.ast.subsystem {
                    self.subsystem = match sub.as_str() {
                        "windows" => Subsystem::Windows,
                        "console" => Subsystem::Console,
                        _ => {
                            return Err(Diagnostic::error(
                                *sp,
                                format!("invalid subsystem '{sub}'; expected \"console\" or \"windows\""),
                            ))
                        }
                    };
                }
            }
        }

        // Phase 1: Register all structs and enums
        for m_idx in 0..self.modules.len() {
            let mod_name = self.modules[m_idx].name.clone();
            let struct_decls = self.modules[m_idx].ast.structs.clone();
            for st in struct_decls {
                if self.mod_envs[m_idx].items.contains_key(&st.name) {
                    return Err(Diagnostic::error(st.span, format!("duplicate symbol '{}'", st.name)));
                }
                let c_name = format!("velo_st_{}_{}", sanitize_ident(&mod_name), sanitize_ident(&st.name));
                let sid = self.arena.new_struct(st.name.clone(), c_name);
                self.mod_envs[m_idx].items.insert(st.name, ModItem::Struct(sid));
            }

            let enum_decls = self.modules[m_idx].ast.enums.clone();
            for en in enum_decls {
                if self.mod_envs[m_idx].items.contains_key(&en.name) {
                    return Err(Diagnostic::error(en.span, format!("duplicate symbol '{}'", en.name)));
                }
                let repr = if let Some(r) = &en.repr {
                    let t = self.resolve_type(m_idx, r)?;
                    if !self.arena.is_int(t) {
                        return Err(Diagnostic::error(r.span(), "enum base type must be an integer type"));
                    }
                    t
                } else {
                    TY_I32
                };
                let mut next_val = 0i64;
                let mut map = HashMap::new();
                for v in &en.variants {
                    let val = v.value.unwrap_or(next_val);
                    next_val = val + 1;
                    if map.insert(v.name.clone(), val).is_some() {
                        return Err(Diagnostic::error(v.span, format!("duplicate enum variant '{}'", v.name)));
                    }
                }
                let eid = self.arena.new_enum(en.name.clone(), repr, map);
                self.mod_envs[m_idx].items.insert(en.name, ModItem::Enum(eid));
            }
        }

        // Phase 1b: Wire import aliases / from-imports (for types & symbols)
        self.wire_imports()?;

        // Phase 2: Resolve struct fields and check for recursive value cycles
        let mut struct_map: Vec<(usize, StructDecl, StructId)> = Vec::new();
        for m_idx in 0..self.modules.len() {
            let structs = self.modules[m_idx].ast.structs.clone();
            for st in structs {
                if let Some(ModItem::Struct(sid)) = self.mod_envs[m_idx].items.get(&st.name).cloned() {
                    struct_map.push((m_idx, st, sid));
                }
            }
        }
        let mut struct_deps: HashMap<StructId, Vec<(String, TypeId, Span)>> = HashMap::new();
        for (m_idx, st, sid) in &struct_map {
            let mut fields = Vec::new();
            let mut seen = HashSet::new();
            for f in &st.fields {
                if !seen.insert(f.name.clone()) {
                    return Err(Diagnostic::error(f.span, format!("duplicate field '{}'", f.name)));
                }
                let fty = self.resolve_type(*m_idx, &f.ty)?;
                fields.push((f.name.clone(), fty, f.span));
            }
            struct_deps.insert(*sid, fields);
        }
        // Topological layout computation
        let mut laid = HashSet::new();
        let mut visiting = HashSet::new();
        for (_, st, sid) in &struct_map {
            self.compute_struct_layout(*sid, &struct_deps, &mut laid, &mut visiting, st.span)?;
        }

        // Phase 3: Evaluate module consts
        for m_idx in 0..self.modules.len() {
            let consts = self.modules[m_idx].ast.consts.clone();
            for c in consts {
                if self.mod_envs[m_idx].items.contains_key(&c.name) {
                    return Err(Diagnostic::error(c.span, format!("duplicate symbol '{}'", c.name)));
                }
                let expected = match &c.ty {
                    Some(te) => Some(self.resolve_type(m_idx, te)?),
                    None => None,
                };
                let (val, ty) = self.eval_const_expr(m_idx, &c.value, expected)?;
                self.mod_envs[m_idx].items.insert(c.name, ModItem::Const(val, ty));
            }
        }
        // Re-wire imports so from-imports pick up consts too
        self.wire_imports()?;

        // Phase 4: Register externs, functions, and struct methods
        for m_idx in 0..self.modules.len() {
            let ext_decls = self.modules[m_idx].ast.externs.clone();
            for ex in ext_decls {
                let mut params = Vec::new();
                for p in &ex.params {
                    let t = self.resolve_type(m_idx, p.ty.as_ref().unwrap())?;
                    params.push(t);
                }
                let ret = self.resolve_type(m_idx, &ex.ret)?;
                let eid = if let Some(&existing) = self.extern_by_name.get(&ex.name) {
                    existing
                } else {
                    let id = ExternId(self.externs.len());
                    self.externs.push(CheckedExtern {
                        id,
                        name: ex.name.clone(),
                        params,
                        ret,
                        variadic: ex.variadic,
                    });
                    self.extern_by_name.insert(ex.name.clone(), id);
                    id
                };
                self.mod_envs[m_idx].items.insert(ex.name, ModItem::Extern(eid));
            }

            let fn_decls = self.modules[m_idx].ast.funcs.clone();
            for f in fn_decls {
                self.register_fn(m_idx, f, None)?;
            }

            let structs = self.modules[m_idx].ast.structs.clone();
            for st in structs {
                let Some(ModItem::Struct(sid)) = self.mod_envs[m_idx].items.get(&st.name).cloned() else {
                    continue;
                };
                for m in st.methods {
                    self.register_fn(m_idx, m, Some(sid))?;
                }
            }
        }

        // Re-wire imports so from-imports pick up functions and externs
        self.wire_imports()?;

        // Phase 5: Type-check all function bodies
        let pending = std::mem::take(&mut self.fn_Asts);
        self.funcs = Vec::with_capacity(pending.len());
        // Pre-populate function skeletons so recursive / cross-calls know parameter types
        for p in &pending {
            self.funcs.push(CheckedFn {
                id: p.fn_id,
                name: p.decl.name.clone(),
                c_name: self.make_fn_cname(p),
                params: p.param_ids.clone(),
                ret: p.ret,
                inline_hint: p.decl.inline_hint,
                locals: p.locals.clone(),
                body: Vec::new(),
            });
        }

        for p in pending {
            let mut fn_chk = FnChecker {
                checker: self,
                mod_idx: p.mod_idx,
                ret_ty: p.ret,
                locals: p.locals,
                scopes: vec![HashMap::new()],
                bounds: HashMap::new(),
                loop_depth: 0,
            };
            for &pid in &p.param_ids {
                let name = fn_chk.locals[pid.0 as usize].name.clone();
                fn_chk.scopes[0].insert(name, pid);
            }
            let body = fn_chk.check_block(&p.decl.body)?;
            if p.ret != TY_VOID && !definitely_returns(&body) {
                return Err(Diagnostic::error(
                    p.decl.span,
                    format!(
                        "function '{}' returns '{}' but not all paths return a value",
                        p.decl.name,
                        fn_chk.checker.arena.display(p.ret)
                    ),
                ));
            }
            let fid = p.fn_id.0;
            self.funcs[fid].locals = fn_chk.locals;
            self.funcs[fid].body = body;
        }

        Ok(())
    }

    fn make_fn_cname(&self, p: &PendingFn) -> String {
        if p.mod_idx == self.entry_mod && p.self_struct.is_none() && p.decl.name == "main" {
            return "velo_user_main".to_string();
        }
        let mod_name = sanitize_ident(&self.modules[p.mod_idx].name);
        if let Some(sid) = p.self_struct {
            let st_name = sanitize_ident(&self.arena.structs[sid.0].name);
            format!("velo_m_{}_{}_{}_{}", p.fn_id.0, mod_name, st_name, sanitize_ident(&p.decl.name))
        } else {
            format!("velo_fn_{}_{}_{}", p.fn_id.0, mod_name, sanitize_ident(&p.decl.name))
        }
    }

    fn wire_imports(&mut self) -> Result<(), Diagnostic> {
        for m_idx in 0..self.modules.len() {
            let imps = self.modules[m_idx].imports.clone();
            for imp in imps {
                self.mod_envs[m_idx]
                    .submodules
                    .insert(imp.bind_name.clone(), imp.target_mod);
                if let Some(items) = imp.from_items {
                    let target_items = self.mod_envs[imp.target_mod].items.clone();
                    if items.len() == 1 && items[0] == "*" {
                        for (k, v) in target_items {
                            self.mod_envs[m_idx].items.entry(k).or_insert(v);
                        }
                    } else {
                        for name in items {
                            if let Some(item) = target_items.get(&name).cloned() {
                                self.mod_envs[m_idx].items.insert(name, item);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn compute_struct_layout(
        &mut self,
        sid: StructId,
        deps: &HashMap<StructId, Vec<(String, TypeId, Span)>>,
        laid: &mut HashSet<StructId>,
        visiting: &mut HashSet<StructId>,
        span: Span,
    ) -> Result<(), Diagnostic> {
        if laid.contains(&sid) {
            return Ok(());
        }
        if !visiting.insert(sid) {
            return Err(Diagnostic::error(
                span,
                format!("recursive value struct '{}'; use a pointer '*{}' instead", self.arena.structs[sid.0].name, self.arena.structs[sid.0].name),
            ));
        }
        let fields = deps.get(&sid).cloned().unwrap_or_default();
        let mut resolved = Vec::with_capacity(fields.len());
        for (fname, fty, fsp) in fields {
            self.ensure_type_laid_out(fty, deps, laid, visiting, fsp)?;
            resolved.push((fname, fty));
        }
        visiting.remove(&sid);
        self.arena.finalize_struct(sid, resolved);
        laid.insert(sid);
        Ok(())
    }

    fn ensure_type_laid_out(
        &mut self,
        ty: TypeId,
        deps: &HashMap<StructId, Vec<(String, TypeId, Span)>>,
        laid: &mut HashSet<StructId>,
        visiting: &mut HashSet<StructId>,
        span: Span,
    ) -> Result<(), Diagnostic> {
        match self.arena.kind(ty).clone() {
            TypeKind::Struct(sid) => self.compute_struct_layout(sid, deps, laid, visiting, span),
            TypeKind::Array(inner, _) => self.ensure_type_laid_out(inner, deps, laid, visiting, span),
            _ => Ok(()),
        }
    }

    fn register_fn(
        &mut self,
        mod_idx: usize,
        decl: FnDecl,
        self_struct: Option<StructId>,
    ) -> Result<(), Diagnostic> {
        let fn_id = FnId(self.fn_Asts.len());
        let mut locals = Vec::new();
        let mut param_ids = Vec::new();
        let mut has_self = false;

        for (idx, p) in decl.params.iter().enumerate() {
            let ty = if p.is_self {
                let Some(sid) = self_struct else {
                    return Err(Diagnostic::error(p.span, "'self' is only valid inside struct methods"));
                };
                if idx != 0 {
                    return Err(Diagnostic::error(p.span, "'self' must be the first parameter"));
                }
                has_self = true;
                let st_ty = self.arena.intern(TypeKind::Struct(sid));
                self.arena.intern(TypeKind::Ptr(st_ty))
            } else {
                self.resolve_type(mod_idx, p.ty.as_ref().unwrap())?
            };
            let lid = LocalId(locals.len() as u32);
            let is_mut = p.is_var;
            let noalias = self.arena.is_ptr_like(ty) || matches!(self.arena.kind(ty), TypeKind::Slice(_));
            locals.push(CheckedLocal {
                id: lid,
                name: p.name.clone(),
                ty,
                is_mut,
                is_param: true,
                noalias,
                addr_taken: false,
            });
            param_ids.push(lid);
        }

        let ret = self.resolve_type(mod_idx, &decl.ret)?;
        if let Some(sid) = self_struct {
            if has_self {
                self.struct_methods.insert((sid, decl.name.clone()), fn_id);
            } else {
                self.struct_statics.insert((sid, decl.name.clone()), fn_id);
            }
        } else {
            if self.mod_envs[mod_idx].items.contains_key(&decl.name) {
                return Err(Diagnostic::error(decl.span, format!("duplicate function '{}'", decl.name)));
            }
            self.mod_envs[mod_idx].items.insert(decl.name.clone(), ModItem::Fn(fn_id));
        }

        self.fn_Asts.push(PendingFn {
            mod_idx,
            fn_id,
            decl,
            self_struct,
            locals,
            param_ids,
            ret,
        });
        Ok(())
    }

    fn resolve_type(&mut self, mod_idx: usize, te: &TypeExpr) -> Result<TypeId, Diagnostic> {
        match te {
            TypeExpr::Void(_) => Ok(TY_VOID),
            TypeExpr::OpaquePtr(_) => Ok(TY_PTR),
            TypeExpr::Ptr(inner, _) => {
                let it = self.resolve_type(mod_idx, inner)?;
                Ok(self.arena.intern(TypeKind::Ptr(it)))
            }
            TypeExpr::Array(inner, len, _) => {
                let it = self.resolve_type(mod_idx, inner)?;
                Ok(self.arena.intern(TypeKind::Array(it, *len)))
            }
            TypeExpr::Slice(inner, _) => {
                let it = self.resolve_type(mod_idx, inner)?;
                Ok(self.arena.intern(TypeKind::Slice(it)))
            }
            TypeExpr::Fn(params, ret, _) => {
                let mut ps = Vec::with_capacity(params.len());
                for p in params {
                    ps.push(self.resolve_type(mod_idx, p)?);
                }
                let r = self.resolve_type(mod_idx, ret)?;
                Ok(self.arena.intern(TypeKind::Fn(ps, r)))
            }
            TypeExpr::Named(name, sp) => match name.as_str() {
                "bool" => Ok(TY_BOOL),
                "i8" => Ok(TY_I8),
                "i16" => Ok(TY_I16),
                "i32" => Ok(TY_I32),
                "i64" | "int" => Ok(TY_I64),
                "u8" | "byte" => Ok(TY_U8),
                "u16" => Ok(TY_U16),
                "u32" => Ok(TY_U32),
                "u64" | "usize" => Ok(TY_U64),
                "f32" => Ok(TY_F32),
                "f64" | "float" => Ok(TY_F64),
                "ptr" => Ok(TY_PTR),
                "cstr" => Ok(TY_CSTR),
                "str" => Ok(TY_STR),
                other => {
                    let item = self.lookup_item_path(mod_idx, other, *sp)?;
                    match item {
                        ModItem::Struct(sid) => Ok(self.arena.intern(TypeKind::Struct(sid))),
                        ModItem::Enum(eid) => Ok(self.arena.intern(TypeKind::Enum(eid))),
                        _ => Err(Diagnostic::error(*sp, format!("'{other}' is not a type"))),
                    }
                }
            },
        }
    }

    fn lookup_item_path(&self, mod_idx: usize, path: &str, sp: Span) -> Result<ModItem, Diagnostic> {
        if let Some((m, rest)) = path.split_once('.') {
            if let Some(&target_mod) = self.mod_envs[mod_idx].submodules.get(m) {
                return self.mod_envs[target_mod]
                    .items
                    .get(rest)
                    .cloned()
                    .ok_or_else(|| Diagnostic::error(sp, format!("no symbol '{rest}' in module '{m}'")));
            }
        }
        self.mod_envs[mod_idx]
            .items
            .get(path)
            .cloned()
            .ok_or_else(|| Diagnostic::error(sp, format!("unknown symbol '{path}'")))
    }

    fn eval_const_expr(
        &mut self,
        mod_idx: usize,
        expr: &Expr,
        expected: Option<TypeId>,
    ) -> Result<(ConstValue, TypeId), Diagnostic> {
        match &expr.kind {
            ExprKind::Int(v) => {
                let ty = match expected {
                    Some(t) if self.arena.is_int(t) => {
                        let (lo, hi) = self.arena.int_bounds(t).unwrap();
                        if *v < lo || *v > hi {
                            return Err(Diagnostic::error(expr.span, format!("integer {v} out of range for {}", self.arena.display(t))));
                        }
                        t
                    }
                    _ => TY_I64,
                };
                Ok((ConstValue::Int(*v), ty))
            }
            ExprKind::Float(v) => {
                let ty = match expected {
                    Some(t) if t == TY_F32 => TY_F32,
                    _ => TY_F64,
                };
                Ok((ConstValue::Float(*v), ty))
            }
            ExprKind::Bool(b) => Ok((ConstValue::Bool(*b), TY_BOOL)),
            ExprKind::Str(s) => Ok((ConstValue::Str(s.clone()), TY_STR)),
            ExprKind::SizeOf(te) => {
                let t = self.resolve_type(mod_idx, te)?;
                let sz = self.arena.layout(t).size as i128;
                Ok((ConstValue::Int(sz), expected.filter(|e| self.arena.is_int(*e)).unwrap_or(TY_I64)))
            }
            ExprKind::Ident(name) => match self.mod_envs[mod_idx].items.get(name) {
                Some(ModItem::Const(val, ty)) => Ok((val.clone(), *ty)),
                _ => Err(Diagnostic::error(expr.span, format!("'{name}' is not a compile-time constant"))),
            },
            ExprKind::Field { obj, field } => {
                if let ExprKind::Ident(base) = &obj.kind {
                    if let Some(&target_mod) = self.mod_envs[mod_idx].submodules.get(base) {
                        if let Some(ModItem::Const(val, ty)) = self.mod_envs[target_mod].items.get(field) {
                            return Ok((val.clone(), *ty));
                        }
                    }
                    if let Some(ModItem::Enum(eid)) = self.mod_envs[mod_idx].items.get(base) {
                        let en = &self.arena.enums[eid.0];
                        if let Some(&v) = en.variants.get(field) {
                            let ety = self.arena.intern(TypeKind::Enum(*eid));
                            return Ok((ConstValue::Int(v as i128), ety));
                        }
                    }
                }
                Err(Diagnostic::error(expr.span, "unsupported constant field expression"))
            }
            ExprKind::Unary { op: UnaryOp::Neg, expr: inner } => {
                if let ExprKind::Int(v) = inner.kind {
                    let neg_v = -v;
                    let ty = match expected {
                        Some(t) if self.arena.is_int(t) => {
                            let (lo, hi) = self.arena.int_bounds(t).unwrap();
                            if neg_v < lo || neg_v > hi {
                                return Err(Diagnostic::error(expr.span, format!("integer {neg_v} out of range for {}", self.arena.display(t))));
                            }
                            t
                        }
                        _ => TY_I64,
                    };
                    return Ok((ConstValue::Int(neg_v), ty));
                }
                let (v, t) = self.eval_const_expr(mod_idx, inner, expected)?;
                match v {
                    ConstValue::Int(x) => Ok((ConstValue::Int(-x), t)),
                    ConstValue::Float(x) => Ok((ConstValue::Float(-x), t)),
                    _ => Err(Diagnostic::error(expr.span, "invalid unary '-' in constant expression")),
                }
            }
            ExprKind::Unary { op, expr: inner } => {
                let (v, t) = self.eval_const_expr(mod_idx, inner, expected)?;
                match (op, v) {
                    (UnaryOp::Neg, ConstValue::Int(x)) => Ok((ConstValue::Int(-x), t)),
                    (UnaryOp::Neg, ConstValue::Float(x)) => Ok((ConstValue::Float(-x), t)),
                    (UnaryOp::BitNot, ConstValue::Int(x)) => Ok((ConstValue::Int(!x), t)),
                    (UnaryOp::Not, ConstValue::Bool(b)) => Ok((ConstValue::Bool(!b), TY_BOOL)),
                    _ => Err(Diagnostic::error(expr.span, "invalid unary operator in constant expression")),
                }
            }
            ExprKind::Cast { expr: inner, ty: te } => {
                let to = self.resolve_type(mod_idx, te)?;
                let (v, _) = self.eval_const_expr(mod_idx, inner, Some(to))?;
                match v {
                    ConstValue::Int(x) if self.arena.is_int(to) => Ok((ConstValue::Int(x), to)),
                    ConstValue::Int(x) if self.arena.is_float(to) => Ok((ConstValue::Float(x as f64), to)),
                    ConstValue::Float(x) if self.arena.is_int(to) => Ok((ConstValue::Int(x as i128), to)),
                    ConstValue::Float(x) if self.arena.is_float(to) => Ok((ConstValue::Float(x), to)),
                    _ => Err(Diagnostic::error(expr.span, "unsupported cast in constant expression")),
                }
            }
            ExprKind::Binary { op, left, right } => {
                let (lv, lt) = self.eval_const_expr(mod_idx, left, expected)?;
                let (rv, _rt) = self.eval_const_expr(mod_idx, right, Some(lt))?;
                match (lv, rv) {
                    (ConstValue::Int(a), ConstValue::Int(b)) => {
                        let res = match op {
                            BinOp::Add => ConstValue::Int(a.wrapping_add(b)),
                            BinOp::Sub => ConstValue::Int(a.wrapping_sub(b)),
                            BinOp::Mul => ConstValue::Int(a.wrapping_mul(b)),
                            BinOp::Div if b != 0 => ConstValue::Int(a / b),
                            BinOp::Mod if b != 0 => ConstValue::Int(a % b),
                            BinOp::BitAnd => ConstValue::Int(a & b),
                            BinOp::BitOr => ConstValue::Int(a | b),
                            BinOp::BitXor => ConstValue::Int(a ^ b),
                            BinOp::Shl => ConstValue::Int(a << (b as u32)),
                            BinOp::Shr => ConstValue::Int(a >> (b as u32)),
                            BinOp::Eq => return Ok((ConstValue::Bool(a == b), TY_BOOL)),
                            BinOp::Ne => return Ok((ConstValue::Bool(a != b), TY_BOOL)),
                            BinOp::Lt => return Ok((ConstValue::Bool(a < b), TY_BOOL)),
                            BinOp::Le => return Ok((ConstValue::Bool(a <= b), TY_BOOL)),
                            BinOp::Gt => return Ok((ConstValue::Bool(a > b), TY_BOOL)),
                            BinOp::Ge => return Ok((ConstValue::Bool(a >= b), TY_BOOL)),
                            _ => return Err(Diagnostic::error(expr.span, "invalid constant integer operation")),
                        };
                        Ok((res, lt))
                    }
                    (ConstValue::Float(a), ConstValue::Float(b)) => {
                        let res = match op {
                            BinOp::Add => ConstValue::Float(a + b),
                            BinOp::Sub => ConstValue::Float(a - b),
                            BinOp::Mul => ConstValue::Float(a * b),
                            BinOp::Div => ConstValue::Float(a / b),
                            BinOp::Eq => return Ok((ConstValue::Bool(a == b), TY_BOOL)),
                            BinOp::Ne => return Ok((ConstValue::Bool(a != b), TY_BOOL)),
                            BinOp::Lt => return Ok((ConstValue::Bool(a < b), TY_BOOL)),
                            BinOp::Le => return Ok((ConstValue::Bool(a <= b), TY_BOOL)),
                            BinOp::Gt => return Ok((ConstValue::Bool(a > b), TY_BOOL)),
                            BinOp::Ge => return Ok((ConstValue::Bool(a >= b), TY_BOOL)),
                            _ => return Err(Diagnostic::error(expr.span, "invalid constant float operation")),
                        };
                        Ok((res, lt))
                    }
                    (ConstValue::Bool(a), ConstValue::Bool(b)) => {
                        let res = match op {
                            BinOp::And => a && b,
                            BinOp::Or => a || b,
                            BinOp::Eq => a == b,
                            BinOp::Ne => a != b,
                            _ => return Err(Diagnostic::error(expr.span, "invalid constant bool operation")),
                        };
                        Ok((ConstValue::Bool(res), TY_BOOL))
                    }
                    _ => Err(Diagnostic::error(expr.span, "mismatched constant operand types")),
                }
            }
            _ => Err(Diagnostic::error(expr.span, "expression is not a compile-time constant")),
        }
    }
}

struct FnChecker<'a> {
    checker: &'a mut Checker,
    mod_idx: usize,
    ret_ty: TypeId,
    locals: Vec<CheckedLocal>,
    scopes: Vec<HashMap<String, LocalId>>,
    bounds: HashMap<LocalId, BoundsUpper>,
    loop_depth: usize,
}

impl<'a> FnChecker<'a> {
    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn add_local(&mut self, name: String, ty: TypeId, is_mut: bool) -> LocalId {
        let id = LocalId(self.locals.len() as u32);
        self.locals.push(CheckedLocal {
            id,
            name: name.clone(),
            ty,
            is_mut,
            is_param: false,
            noalias: false,
            addr_taken: false,
        });
        self.scopes.last_mut().unwrap().insert(name, id);
        id
    }

    fn lookup_local(&self, name: &str) -> Option<LocalId> {
        for sc in self.scopes.iter().rev() {
            if let Some(&id) = sc.get(name) {
                return Some(id);
            }
        }
        None
    }

    fn check_block(&mut self, stmts: &[Stmt]) -> Result<Vec<TStmt>, Diagnostic> {
        self.push_scope();
        let mut out = Vec::with_capacity(stmts.len());
        for s in stmts {
            out.push(self.check_stmt(s)?);
        }
        self.pop_scope();
        Ok(out)
    }

    fn check_stmt(&mut self, stmt: &Stmt) -> Result<TStmt, Diagnostic> {
        let sp = stmt.span;
        let kind = match &stmt.kind {
            StmtKind::Pass => TStmtKind::Pass,
            StmtKind::Const(c) => {
                let expected = match &c.ty {
                    Some(te) => Some(self.checker.resolve_type(self.mod_idx, te)?),
                    None => None,
                };
                let (val, ty) = self.checker.eval_const_expr(self.mod_idx, &c.value, expected)?;
                // Materialize block-local const as immutable local
                let lit = match val {
                    ConstValue::Int(n) => TExprKind::Int(n),
                    ConstValue::Float(f) => TExprKind::Float(f),
                    ConstValue::Bool(b) => TExprKind::Bool(b),
                    ConstValue::Str(s) => TExprKind::Str(s),
                };
                let lid = self.add_local(c.name.clone(), ty, false);
                TStmtKind::VarInit {
                    local: lid,
                    value: TExpr { kind: lit, ty, span: sp },
                }
            }
            StmtKind::Let { name, ty, value } | StmtKind::Var { name, ty, value } => {
                let is_mut = matches!(stmt.kind, StmtKind::Var { .. });
                let expected = match ty {
                    Some(te) => Some(self.checker.resolve_type(self.mod_idx, te)?),
                    None => None,
                };
                let val = self.check_expr(value, expected)?;
                if let Some(exp) = expected {
                    self.ensure_assignable(exp, val.ty, value.span)?;
                }
                if val.ty == TY_VOID {
                    return Err(Diagnostic::error(value.span, "cannot bind a 'void' expression"));
                }
                let lid = self.add_local(name.clone(), expected.unwrap_or(val.ty), is_mut);
                TStmtKind::VarInit { local: lid, value: val }
            }
            StmtKind::Assign { target, op, value } => {
                let lhs = self.check_expr(target, None)?;
                self.check_lvalue_mutable(&lhs, target.span)?;
                let rhs = self.check_expr(value, Some(lhs.ty))?;
                if let Some(bop) = op {
                    self.check_binop_types(*bop, lhs.ty, rhs.ty, sp)?;
                } else {
                    self.ensure_assignable(lhs.ty, rhs.ty, value.span)?;
                }
                TStmtKind::Assign {
                    target: lhs,
                    op: *op,
                    value: rhs,
                }
            }
            StmtKind::Expr(e) => {
                let te = self.check_expr(e, None)?;
                TStmtKind::Expr(te)
            }
            StmtKind::If { branches, else_body } => {
                let mut tbranches = Vec::with_capacity(branches.len());
                for (cond, body) in branches {
                    let tcond = self.check_expr(cond, Some(TY_BOOL))?;
                    if tcond.ty != TY_BOOL {
                        return Err(Diagnostic::error(cond.span, "condition must be 'bool'"));
                    }
                    let tbody = self.check_block(body)?;
                    tbranches.push((tcond, tbody));
                }
                let telse = match else_body {
                    Some(eb) => Some(self.check_block(eb)?),
                    None => None,
                };
                TStmtKind::If {
                    branches: tbranches,
                    else_body: telse,
                }
            }
            StmtKind::While { cond, body } => {
                let tcond = self.check_expr(cond, Some(TY_BOOL))?;
                if tcond.ty != TY_BOOL {
                    return Err(Diagnostic::error(cond.span, "while condition must be 'bool'"));
                }
                self.loop_depth += 1;
                let tbody = self.check_block(body)?;
                self.loop_depth -= 1;
                TStmtKind::While {
                    cond: tcond,
                    body: tbody,
                }
            }
            StmtKind::ForRange {
                var_name,
                start,
                end,
                inclusive,
                body,
            } => {
                let tstart = self.check_expr(start, Some(TY_I64))?;
                let tend = self.check_expr(end, Some(tstart.ty))?;
                if !self.checker.arena.is_int(tstart.ty) || tstart.ty != tend.ty {
                    return Err(Diagnostic::error(sp, "for-range bounds must have matching integer types"));
                }
                self.push_scope();
                let var_local = self.add_local(var_name.clone(), tstart.ty, false);
                // Record bounds fact if start >= 0
                let mut added_bound = false;
                if let TExprKind::Int(0) = tstart.kind {
                    if !*inclusive {
                        match &tend.kind {
                            TExprKind::Int(n) if *n >= 0 => {
                                self.bounds.insert(var_local, BoundsUpper::Const(*n));
                                added_bound = true;
                            }
                            TExprKind::SliceLen(arr_expr) => {
                                if let TExprKind::Local(arr_lid) = arr_expr.kind {
                                    self.bounds.insert(var_local, BoundsUpper::ArrayLenOf(arr_lid));
                                    added_bound = true;
                                }
                            }
                            _ => {}
                        }
                    } else if let TExprKind::Int(n) = tend.kind {
                        if n >= 0 {
                            self.bounds.insert(var_local, BoundsUpper::Const(n + 1));
                            added_bound = true;
                        }
                    }
                }
                self.loop_depth += 1;
                let mut tbody = Vec::with_capacity(body.len());
                for bs in body {
                    tbody.push(self.check_stmt(bs)?);
                }
                self.loop_depth -= 1;
                if added_bound {
                    self.bounds.remove(&var_local);
                }
                self.pop_scope();
                TStmtKind::ForRange {
                    var_local,
                    start: tstart,
                    end: tend,
                    inclusive: *inclusive,
                    body: tbody,
                }
            }
            StmtKind::ForIter {
                var_name,
                iter,
                body,
            } => {
                let titer = self.check_expr(iter, None)?;
                let elem_ty = match self.checker.arena.kind(titer.ty).clone() {
                    TypeKind::Array(el, _) | TypeKind::Slice(el) => el,
                    _ => {
                        return Err(Diagnostic::error(
                            iter.span,
                            format!("cannot iterate over type '{}'", self.checker.arena.display(titer.ty)),
                        ))
                    }
                };
                self.push_scope();
                let var_local = self.add_local(var_name.clone(), elem_ty, false);
                self.loop_depth += 1;
                let mut tbody = Vec::with_capacity(body.len());
                for bs in body {
                    tbody.push(self.check_stmt(bs)?);
                }
                self.loop_depth -= 1;
                self.pop_scope();
                TStmtKind::ForIter {
                    var_local,
                    iter: titer,
                    body: tbody,
                }
            }
            StmtKind::Match {
                expr,
                cases,
                else_body,
            } => {
                let tex = self.check_expr(expr, None)?;
                let is_valid = self.checker.arena.is_int(tex.ty)
                    || tex.ty == TY_BOOL
                    || matches!(self.checker.arena.kind(tex.ty), TypeKind::Enum(_))
                    || tex.ty == TY_STR;
                if !is_valid {
                    return Err(Diagnostic::error(expr.span, "match expression must be integer, bool, enum, or str"));
                }
                let mut tcases = Vec::new();
                for (pats, cbody) in cases {
                    let mut tpats = Vec::new();
                    for p in pats {
                        let tp = self.check_expr(p, Some(tex.ty))?;
                        self.ensure_assignable(tex.ty, tp.ty, p.span)?;
                        tpats.push(tp);
                    }
                    let tb = self.check_block(cbody)?;
                    tcases.push((tpats, tb));
                }
                let telse = match else_body {
                    Some(eb) => Some(self.check_block(eb)?),
                    None => None,
                };
                TStmtKind::Match {
                    expr: tex,
                    cases: tcases,
                    else_body: telse,
                }
            }
            StmtKind::Return(opt_expr) => {
                if let Some(e) = opt_expr {
                    let te = self.check_expr(e, Some(self.ret_ty))?;
                    if self.ret_ty == TY_VOID {
                        return Err(Diagnostic::error(e.span, "void function cannot return a value"));
                    }
                    self.ensure_assignable(self.ret_ty, te.ty, e.span)?;
                    TStmtKind::Return(Some(te))
                } else {
                    if self.ret_ty != TY_VOID {
                        return Err(Diagnostic::error(
                            sp,
                            format!("expected return value of type '{}'", self.checker.arena.display(self.ret_ty)),
                        ));
                    }
                    TStmtKind::Return(None)
                }
            }
            StmtKind::Break => {
                if self.loop_depth == 0 {
                    return Err(Diagnostic::error(sp, "'break' outside loop"));
                }
                TStmtKind::Break
            }
            StmtKind::Continue => {
                if self.loop_depth == 0 {
                    return Err(Diagnostic::error(sp, "'continue' outside loop"));
                }
                TStmtKind::Continue
            }
            StmtKind::Defer(inner) => {
                let ti = self.check_stmt(inner)?;
                TStmtKind::Defer(Box::new(ti))
            }
        };
        Ok(TStmt { kind, span: sp })
    }

    fn check_lvalue_mutable(&self, expr: &TExpr, sp: Span) -> Result<(), Diagnostic> {
        match &expr.kind {
            TExprKind::Local(lid) => {
                let l = &self.locals[lid.0 as usize];
                if !l.is_mut {
                    // Exception: if local is *Struct / *T parameter and we're writing through pointer,
                    // wait — assigning `x = ...` mutates the local, which requires `is_mut`!
                    return Err(Diagnostic::error(
                        sp,
                        format!("cannot mutate immutable variable '{}'; declare with 'var'", l.name),
                    ));
                }
                Ok(())
            }
            TExprKind::Deref(_) => Ok(()),
            TExprKind::Field { obj, through_ptr, .. } => {
                if *through_ptr {
                    // Through *Struct (including `var self` or raw `*T`)
                    if let TExprKind::Local(lid) = obj.kind {
                        let l = &self.locals[lid.0 as usize];
                        if l.name == "self" && !l.is_mut {
                            return Err(Diagnostic::error(
                                sp,
                                "cannot mutate fields of immutable 'self'; use 'var self' in method signature",
                            ));
                        }
                    }
                    Ok(())
                } else {
                    self.check_lvalue_mutable(obj, sp)
                }
            }
            TExprKind::Index { obj, .. } => {
                if matches!(self.checker.arena.kind(obj.ty), TypeKind::Ptr(_) | TypeKind::Slice(_)) {
                    Ok(())
                } else {
                    self.check_lvalue_mutable(obj, sp)
                }
            }
            _ => Err(Diagnostic::error(sp, "expression is not assignable")),
        }
    }

    fn mark_addr_taken(&mut self, expr: &TExpr) {
        match &expr.kind {
            TExprKind::Local(lid) => {
                self.locals[lid.0 as usize].addr_taken = true;
            }
            TExprKind::Field { obj, through_ptr, .. } if !*through_ptr => self.mark_addr_taken(obj),
            TExprKind::Index { obj, .. } if !matches!(self.checker.arena.kind(obj.ty), TypeKind::Ptr(_) | TypeKind::Slice(_)) => {
                self.mark_addr_taken(obj);
            }
            _ => {}
        }
    }

    fn ensure_assignable(&self, expected: TypeId, actual: TypeId, sp: Span) -> Result<(), Diagnostic> {
        if expected == actual {
            return Ok(());
        }
        // Array -> Slice coercion handled in check_expr, also allow *T <-> ptr?
        // Allow opaque `ptr` interoperability with `*T`? Keep `ptr` interop convenient for C/Win32!
        if expected == TY_PTR && matches!(self.checker.arena.kind(actual), TypeKind::Ptr(_)) {
            return Ok(());
        }
        if matches!(self.checker.arena.kind(expected), TypeKind::Ptr(_)) && actual == TY_PTR {
            return Ok(());
        }
        Err(Diagnostic::error(
            sp,
            format!(
                "type mismatch: expected '{}', found '{}'",
                self.checker.arena.display(expected),
                self.checker.arena.display(actual)
            ),
        ))
    }

    fn check_expr(&mut self, expr: &Expr, expected: Option<TypeId>) -> Result<TExpr, Diagnostic> {
        let sp = expr.span;
        let res = match &expr.kind {
            ExprKind::Int(v) => {
                let ty = match expected {
                    Some(t) if self.checker.arena.is_int(t) => {
                        let (lo, hi) = self.checker.arena.int_bounds(t).unwrap();
                        if *v < lo || *v > hi {
                            return Err(Diagnostic::error(
                                sp,
                                format!("integer literal {v} out of range for '{}'", self.checker.arena.display(t)),
                            ));
                        }
                        t
                    }
                    _ => TY_I64,
                };
                TExpr { kind: TExprKind::Int(*v), ty, span: sp }
            }
            ExprKind::Float(v) => {
                let ty = match expected {
                    Some(t) if t == TY_F32 => TY_F32,
                    _ => TY_F64,
                };
                TExpr { kind: TExprKind::Float(*v), ty, span: sp }
            }
            ExprKind::Bool(b) => TExpr { kind: TExprKind::Bool(*b), ty: TY_BOOL, span: sp },
            ExprKind::Str(s) => {
                if expected == Some(TY_CSTR) {
                    TExpr { kind: TExprKind::CStr(s.clone()), ty: TY_CSTR, span: sp }
                } else {
                    TExpr { kind: TExprKind::Str(s.clone()), ty: TY_STR, span: sp }
                }
            }
            ExprKind::CStr(s) => TExpr { kind: TExprKind::CStr(s.clone()), ty: TY_CSTR, span: sp },
            ExprKind::WStr(s) => {
                let u16_ptr = self.checker.arena.intern(TypeKind::Ptr(TY_U16));
                TExpr { kind: TExprKind::WStr(s.clone()), ty: u16_ptr, span: sp }
            }
            ExprKind::FStr(segs) => {
                let mut tsegs = Vec::with_capacity(segs.len());
                for seg in segs {
                    match seg {
                        FStrSeg::Lit(l) => tsegs.push(TFStrSeg::Lit(l.clone())),
                        FStrSeg::Interp { expr: ie, prec } => {
                            let tie = self.check_expr(ie, None)?;
                            let valid = self.checker.arena.is_int(tie.ty)
                                || self.checker.arena.is_float(tie.ty)
                                || tie.ty == TY_BOOL
                                || tie.ty == TY_STR
                                || tie.ty == TY_CSTR;
                            if !valid {
                                return Err(Diagnostic::error(
                                    ie.span,
                                    format!("cannot format type '{}' in f-string", self.checker.arena.display(tie.ty)),
                                ));
                            }
                            tsegs.push(TFStrSeg::Interp { expr: tie, prec: *prec });
                        }
                    }
                }
                TExpr { kind: TExprKind::FStr(tsegs), ty: TY_STR, span: sp }
            }
            ExprKind::Null => {
                let ty = match expected {
                    Some(t) if self.checker.arena.is_ptr_like(t) => t,
                    _ => TY_PTR,
                };
                TExpr { kind: TExprKind::Null, ty, span: sp }
            }
            ExprKind::SizeOf(te) => {
                let t = self.checker.resolve_type(self.mod_idx, te)?;
                let sz = self.checker.arena.layout(t).size as i128;
                let ty = expected.filter(|e| self.checker.arena.is_int(*e)).unwrap_or(TY_I64);
                TExpr { kind: TExprKind::Int(sz), ty, span: sp }
            }
            ExprKind::Zeroed(te) => {
                let t = self.checker.resolve_type(self.mod_idx, te)?;
                TExpr { kind: TExprKind::Zeroed, ty: t, span: sp }
            }
            ExprKind::Ident(name) => {
                if let Some(lid) = self.lookup_local(name) {
                    let ty = self.locals[lid.0 as usize].ty;
                    TExpr { kind: TExprKind::Local(lid), ty, span: sp }
                } else if let Some(item) = self.checker.mod_envs[self.mod_idx].items.get(name).cloned() {
                    self.item_to_expr(item, expected, sp)?
                } else {
                    return Err(Diagnostic::error(sp, format!("undefined identifier '{name}'")));
                }
            }
            ExprKind::Unary { op: UnaryOp::Neg, expr: inner } => {
                if let ExprKind::Int(v) = inner.kind {
                    let neg_v = -v;
                    let ty = match expected {
                        Some(t) if self.checker.arena.is_int(t) => {
                            let (lo, hi) = self.checker.arena.int_bounds(t).unwrap();
                            if neg_v < lo || neg_v > hi {
                                return Err(Diagnostic::error(
                                    sp,
                                    format!("integer literal {neg_v} out of range for '{}'", self.checker.arena.display(t)),
                                ));
                            }
                            t
                        }
                        _ => TY_I64,
                    };
                    if !self.checker.arena.is_signed_int(ty) {
                        return Err(Diagnostic::error(sp, "unary '-' requires signed integer or float"));
                    }
                    return Ok(TExpr { kind: TExprKind::Int(neg_v), ty, span: sp });
                }
                let ti = self.check_expr(inner, expected)?;
                if !self.checker.arena.is_signed_int(ti.ty) && !self.checker.arena.is_float(ti.ty) {
                    return Err(Diagnostic::error(sp, "unary '-' requires signed integer or float"));
                }
                let ty = ti.ty;
                TExpr { kind: TExprKind::Unary { op: UnaryOp::Neg, expr: Box::new(ti) }, ty, span: sp }
            }
            ExprKind::Unary { op, expr: inner } => {
                let ti = self.check_expr(inner, expected)?;
                match op {
                    UnaryOp::Neg => {
                        if !self.checker.arena.is_signed_int(ti.ty) && !self.checker.arena.is_float(ti.ty) {
                            return Err(Diagnostic::error(sp, "unary '-' requires signed integer or float"));
                        }
                        let ty = ti.ty;
                        TExpr { kind: TExprKind::Unary { op: *op, expr: Box::new(ti) }, ty, span: sp }
                    }
                    UnaryOp::Not => {
                        if ti.ty != TY_BOOL {
                            return Err(Diagnostic::error(sp, "'not' requires bool operand"));
                        }
                        TExpr { kind: TExprKind::Unary { op: *op, expr: Box::new(ti) }, ty: TY_BOOL, span: sp }
                    }
                    UnaryOp::BitNot => {
                        if !self.checker.arena.is_int(ti.ty) {
                            return Err(Diagnostic::error(sp, "'~' requires integer operand"));
                        }
                        let ty = ti.ty;
                        TExpr { kind: TExprKind::Unary { op: *op, expr: Box::new(ti) }, ty, span: sp }
                    }
                }
            }
            ExprKind::Binary { op, left, right } => {
                if matches!(op, BinOp::And | BinOp::Or) {
                    let tl = self.check_expr(left, Some(TY_BOOL))?;
                    let tr = self.check_expr(right, Some(TY_BOOL))?;
                    if tl.ty != TY_BOOL || tr.ty != TY_BOOL {
                        return Err(Diagnostic::error(sp, "logical 'and'/'or' require bool operands"));
                    }
                    TExpr {
                        kind: TExprKind::Binary { op: *op, left: Box::new(tl), right: Box::new(tr) },
                        ty: TY_BOOL,
                        span: sp,
                    }
                } else {
                    let hint = if matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge) {
                        None
                    } else {
                        expected
                    };
                    // Peek left or right to propagate literal type hints cleanly
                    let (tl, tr) = if matches!(left.kind, ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Null) {
                        let tr = self.check_expr(right, hint)?;
                        let tl = self.check_expr(left, Some(tr.ty))?;
                        (tl, tr)
                    } else {
                        let tl = self.check_expr(left, hint)?;
                        let tr = self.check_expr(right, Some(tl.ty))?;
                        (tl, tr)
                    };
                    let out_ty = self.check_binop_types(*op, tl.ty, tr.ty, sp)?;
                    TExpr {
                        kind: TExprKind::Binary { op: *op, left: Box::new(tl), right: Box::new(tr) },
                        ty: out_ty,
                        span: sp,
                    }
                }
            }
            ExprKind::Cast { expr: inner, ty: te } => {
                let to = self.checker.resolve_type(self.mod_idx, te)?;
                let ti = self.check_expr(inner, Some(to))?;
                let ok = (self.checker.arena.is_numeric(ti.ty) && self.checker.arena.is_numeric(to))
                    || (matches!(self.checker.arena.kind(ti.ty), TypeKind::Enum(_)) && self.checker.arena.is_int(to))
                    || (self.checker.arena.is_int(ti.ty) && matches!(self.checker.arena.kind(to), TypeKind::Enum(_)))
                    || (ti.ty == TY_BOOL && self.checker.arena.is_int(to))
                    || (self.checker.arena.is_ptr_like(ti.ty) && self.checker.arena.is_ptr_like(to))
                    || (self.checker.arena.is_ptr_like(ti.ty) && (to == TY_U64 || to == TY_I64))
                    || ((ti.ty == TY_U64 || ti.ty == TY_I64) && self.checker.arena.is_ptr_like(to))
                    || (ti.ty == TY_STR && to == TY_CSTR);
                if !ok {
                    return Err(Diagnostic::error(
                        sp,
                        format!("cannot cast '{}' as '{}'", self.checker.arena.display(ti.ty), self.checker.arena.display(to)),
                    ));
                }
                TExpr { kind: TExprKind::Cast { expr: Box::new(ti), to }, ty: to, span: sp }
            }
            ExprKind::AddrOf(inner) => {
                let ti = self.check_expr(inner, None)?;
                self.mark_addr_taken(&ti);
                let pty = self.checker.arena.intern(TypeKind::Ptr(ti.ty));
                TExpr { kind: TExprKind::AddrOf(Box::new(ti)), ty: pty, span: sp }
            }
            ExprKind::Deref(inner) => {
                let ti = self.check_expr(inner, None)?;
                let inner_ty = match self.checker.arena.kind(ti.ty) {
                    TypeKind::Ptr(t) => *t,
                    _ => return Err(Diagnostic::error(sp, "cannot dereference non-pointer type")),
                };
                TExpr { kind: TExprKind::Deref(Box::new(ti)), ty: inner_ty, span: sp }
            }
            ExprKind::Field { obj, field } => {
                // Check module namespace access (e.g., `win32.WS_OVERLAPPED`)
                if let ExprKind::Ident(mod_alias) = &obj.kind {
                    if let Some(&target_mod) = self.checker.mod_envs[self.mod_idx].submodules.get(mod_alias) {
                        if self.lookup_local(mod_alias).is_none() {
                            let item = self.checker.mod_envs[target_mod]
                                .items
                                .get(field)
                                .cloned()
                                .ok_or_else(|| {
                                    Diagnostic::error(sp, format!("no symbol '{field}' in module '{mod_alias}'"))
                                })?;
                            return self.item_to_expr(item, expected, sp);
                        }
                    }
                }
                // Check Enum.Variant (including mod.Enum.Variant)
                if let Some(enum_res) = self.try_resolve_enum_variant(obj, field, sp)? {
                    return Ok(enum_res);
                }

                let tobj = self.check_expr(obj, None)?;
                match self.checker.arena.kind(tobj.ty).clone() {
                    TypeKind::Str => match field.as_str() {
                        "len" => TExpr { kind: TExprKind::StrLen(Box::new(tobj)), ty: TY_I64, span: sp },
                        "ptr" => {
                            let pty = self.checker.arena.intern(TypeKind::Ptr(TY_U8));
                            TExpr { kind: TExprKind::StrPtr(Box::new(tobj)), ty: pty, span: sp }
                        }
                        _ => return Err(Diagnostic::error(sp, format!("str has no field '{field}'"))),
                    },
                    TypeKind::Slice(el) => match field.as_str() {
                        "len" => TExpr { kind: TExprKind::SliceLen(Box::new(tobj)), ty: TY_I64, span: sp },
                        "ptr" => {
                            let pty = self.checker.arena.intern(TypeKind::Ptr(el));
                            TExpr { kind: TExprKind::SlicePtr(Box::new(tobj)), ty: pty, span: sp }
                        }
                        _ => return Err(Diagnostic::error(sp, format!("slice has no field '{field}'"))),
                    },
                    TypeKind::Array(_, len) => match field.as_str() {
                        "len" => TExpr { kind: TExprKind::Int(len as i128), ty: TY_I64, span: sp },
                        _ => return Err(Diagnostic::error(sp, format!("array has no field '{field}'"))),
                    },
                    TypeKind::Struct(sid) => self.resolve_struct_field(tobj, sid, field, false, sp)?,
                    TypeKind::Ptr(inner) => match self.checker.arena.kind(inner) {
                        TypeKind::Struct(sid) => {
                            let sid = *sid;
                            self.resolve_struct_field(tobj, sid, field, true, sp)?
                        }
                        _ => return Err(Diagnostic::error(sp, format!("pointer to non-struct has no field '{field}'"))),
                    },
                    other => {
                        let ot = self.checker.arena.intern(other);
                        return Err(Diagnostic::error(
                            sp,
                            format!("type '{}' has no field '{field}'", self.checker.arena.display(ot)),
                        ))
                    }
                }
            }
            ExprKind::Call { callee, args } => self.check_call(callee, args, expected, sp)?,
            ExprKind::Index { obj, index } => {
                let tobj = self.check_expr(obj, None)?;
                let tidx = self.check_expr(index, Some(TY_I64))?;
                if !self.checker.arena.is_int(tidx.ty) {
                    return Err(Diagnostic::error(index.span, "index must be an integer"));
                }
                let elem_ty = match self.checker.arena.kind(tobj.ty) {
                    TypeKind::Array(el, _) | TypeKind::Slice(el) | TypeKind::Ptr(el) => *el,
                    TypeKind::Str => TY_U8,
                    _ => return Err(Diagnostic::error(sp, format!("cannot index type '{}'", self.checker.arena.display(tobj.ty)))),
                };
                let proved = self.is_index_proved(&tobj, &tidx);
                TExpr {
                    kind: TExprKind::Index {
                        obj: Box::new(tobj),
                        index: Box::new(tidx),
                        proved_in_bounds: proved,
                    },
                    ty: elem_ty,
                    span: sp,
                }
            }
            ExprKind::Slice { obj, start, end } => {
                let tobj = self.check_expr(obj, None)?;
                let elem_ty = match self.checker.arena.kind(tobj.ty) {
                    TypeKind::Array(el, _) | TypeKind::Slice(el) => *el,
                    TypeKind::Str => TY_U8,
                    _ => return Err(Diagnostic::error(sp, "slicing requires array, slice, or str")),
                };
                let tstart = match start {
                    Some(s) => Some(Box::new(self.check_expr(s, Some(TY_I64))?)),
                    None => None,
                };
                let tend = match end {
                    Some(e) => Some(Box::new(self.check_expr(e, Some(TY_I64))?)),
                    None => None,
                };
                let out_ty = if tobj.ty == TY_STR {
                    TY_STR
                } else {
                    self.checker.arena.intern(TypeKind::Slice(elem_ty))
                };
                TExpr {
                    kind: TExprKind::Slice {
                        obj: Box::new(tobj),
                        start: tstart,
                        end: tend,
                    },
                    ty: out_ty,
                    span: sp,
                }
            }
            ExprKind::StructLit { name, fields } => {
                let item = self.checker.lookup_item_path(self.mod_idx, name, sp)?;
                let ModItem::Struct(sid) = item else {
                    return Err(Diagnostic::error(sp, format!("'{name}' is not a struct")));
                };
                let st_info = self.checker.arena.structs[sid.0].clone();
                let mut seen = HashSet::new();
                let mut checked_fields = Vec::with_capacity(fields.len());
                for (fname, fexpr) in fields {
                    if !seen.insert(fname.clone()) {
                        return Err(Diagnostic::error(fexpr.span, format!("duplicate field '{fname}' in struct literal")));
                    }
                    let finfo = st_info.fields.iter().find(|f| &f.name == fname).ok_or_else(|| {
                        Diagnostic::error(fexpr.span, format!("struct '{}' has no field '{fname}'", st_info.name))
                    })?;
                    let val = self.check_expr(fexpr, Some(finfo.ty))?;
                    self.ensure_assignable(finfo.ty, val.ty, fexpr.span)?;
                    checked_fields.push((finfo.offset, finfo.ty, val));
                }
                if seen.len() != st_info.fields.len() {
                    let missing: Vec<&str> = st_info
                        .fields
                        .iter()
                        .filter(|f| !seen.contains(&f.name))
                        .map(|f| f.name.as_str())
                        .collect();
                    return Err(Diagnostic::error(
                        sp,
                        format!("missing field(s) in '{}' literal: {} (hint: use zeroed({}) for zero-initialization)", st_info.name, missing.join(", "), st_info.name),
                    ));
                }
                let st_ty = self.checker.arena.intern(TypeKind::Struct(sid));
                TExpr {
                    kind: TExprKind::StructLit { sid, fields: checked_fields },
                    ty: st_ty,
                    span: sp,
                }
            }
            ExprKind::ArrayLit(elems) => {
                let elem_hint = match expected.map(|e| self.checker.arena.kind(e).clone()) {
                    Some(TypeKind::Array(el, _)) | Some(TypeKind::Slice(el)) => Some(el),
                    _ => None,
                };
                if elems.is_empty() {
                    let el = elem_hint.ok_or_else(|| {
                        Diagnostic::error(sp, "cannot infer element type of empty array '[]'")
                    })?;
                    let arr_ty = self.checker.arena.intern(TypeKind::Array(el, 0));
                    TExpr { kind: TExprKind::ArrayLit(Vec::new()), ty: arr_ty, span: sp }
                } else {
                    let mut telems = Vec::with_capacity(elems.len());
                    let first = self.check_expr(&elems[0], elem_hint)?;
                    let el_ty = first.ty;
                    telems.push(first);
                    for e in &elems[1..] {
                        let te = self.check_expr(e, Some(el_ty))?;
                        self.ensure_assignable(el_ty, te.ty, e.span)?;
                        telems.push(te);
                    }
                    let arr_ty = self.checker.arena.intern(TypeKind::Array(el_ty, telems.len() as u64));
                    TExpr { kind: TExprKind::ArrayLit(telems), ty: arr_ty, span: sp }
                }
            }
            ExprKind::ArrayRepeat { value, count } => {
                let elem_hint = match expected.map(|e| self.checker.arena.kind(e).clone()) {
                    Some(TypeKind::Array(el, _)) | Some(TypeKind::Slice(el)) => Some(el),
                    _ => None,
                };
                let tv = self.check_expr(value, elem_hint)?;
                let arr_ty = self.checker.arena.intern(TypeKind::Array(tv.ty, *count));
                TExpr {
                    kind: TExprKind::ArrayRepeat { value: Box::new(tv), count: *count },
                    ty: arr_ty,
                    span: sp,
                }
            }
            ExprKind::Ternary { then_expr, cond, else_expr } => {
                let tc = self.check_expr(cond, Some(TY_BOOL))?;
                if tc.ty != TY_BOOL {
                    return Err(Diagnostic::error(cond.span, "ternary condition must be 'bool'"));
                }
                let is_lit = |e: &Expr| matches!(
                    &e.kind,
                    ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Null
                ) || matches!(&e.kind, ExprKind::Unary { op: UnaryOp::Neg, expr: inner } if matches!(inner.kind, ExprKind::Int(_) | ExprKind::Float(_)));
                let (tt, te) = if expected.is_none() && is_lit(then_expr) && !is_lit(else_expr) {
                    let te = self.check_expr(else_expr, None)?;
                    let tt = self.check_expr(then_expr, Some(te.ty))?;
                    (tt, te)
                } else {
                    let tt = self.check_expr(then_expr, expected)?;
                    let te = self.check_expr(else_expr, Some(tt.ty))?;
                    (tt, te)
                };
                self.ensure_assignable(tt.ty, te.ty, else_expr.span)?;
                let ty = tt.ty;
                TExpr {
                    kind: TExprKind::Ternary {
                        then_expr: Box::new(tt),
                        cond: Box::new(tc),
                        else_expr: Box::new(te),
                    },
                    ty,
                    span: sp,
                }
            }
        };

        // Coerce [T; N] -> []T automatically when expected
        if let Some(exp) = expected {
            if let (TypeKind::Array(a_el, _), TypeKind::Slice(s_el)) =
                (self.checker.arena.kind(res.ty), self.checker.arena.kind(exp))
            {
                if a_el == s_el {
                    let end_len = match self.checker.arena.kind(res.ty) {
                        TypeKind::Array(_, n) => *n as i128,
                        _ => 0,
                    };
                    return Ok(TExpr {
                        kind: TExprKind::Slice {
                            obj: Box::new(res),
                            start: Some(Box::new(TExpr { kind: TExprKind::Int(0), ty: TY_I64, span: sp })),
                            end: Some(Box::new(TExpr { kind: TExprKind::Int(end_len), ty: TY_I64, span: sp })),
                        },
                        ty: exp,
                        span: sp,
                    });
                }
            }
        }
        Ok(res)
    }

    fn is_index_proved(&self, obj: &TExpr, idx: &TExpr) -> bool {
        let arr_len_opt = match self.checker.arena.kind(obj.ty) {
            TypeKind::Array(_, len) => Some(*len as i128),
            _ => None,
        };
        if let TExprKind::Int(k) = idx.kind {
            if let Some(len) = arr_len_opt {
                return k >= 0 && k < len;
            }
        }
        if let TExprKind::Local(idx_lid) = idx.kind {
            if let Some(bound) = self.bounds.get(&idx_lid) {
                match bound {
                    BoundsUpper::Const(n) => {
                        if let Some(len) = arr_len_opt {
                            return *n <= len;
                        }
                    }
                    BoundsUpper::ArrayLenOf(arr_lid) => {
                        if let TExprKind::Local(obj_lid) = obj.kind {
                            return *arr_lid == obj_lid;
                        }
                    }
                }
            }
        }
        false
    }

    fn try_resolve_enum_variant(&mut self, obj: &Expr, variant: &str, sp: Span) -> Result<Option<TExpr>, Diagnostic> {
        let enum_item = match &obj.kind {
            ExprKind::Ident(name) => self.checker.mod_envs[self.mod_idx].items.get(name).cloned(),
            ExprKind::Field { obj: mod_obj, field: ename } => {
                if let ExprKind::Ident(m) = &mod_obj.kind {
                    if let Some(&tmod) = self.checker.mod_envs[self.mod_idx].submodules.get(m) {
                        self.checker.mod_envs[tmod].items.get(ename).cloned()
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(ModItem::Enum(eid)) = enum_item {
            let en = &self.checker.arena.enums[eid.0];
            let Some(&val) = en.variants.get(variant) else {
                return Err(Diagnostic::error(sp, format!("enum '{}' has no variant '{variant}'", en.name)));
            };
            let ety = self.checker.arena.intern(TypeKind::Enum(eid));
            return Ok(Some(TExpr {
                kind: TExprKind::Int(val as i128),
                ty: ety,
                span: sp,
            }));
        }
        Ok(None)
    }

    fn item_to_expr(&mut self, item: ModItem, expected: Option<TypeId>, sp: Span) -> Result<TExpr, Diagnostic> {
        match item {
            ModItem::Const(val, mut ty) => {
                let kind = match val {
                    ConstValue::Int(n) => {
                        if let Some(exp) = expected {
                            if self.checker.arena.is_int(exp) && ty == TY_I64 {
                                if let Some((lo, hi)) = self.checker.arena.int_bounds(exp) {
                                    if n >= lo && n <= hi {
                                        ty = exp;
                                    }
                                }
                            }
                        }
                        TExprKind::Int(n)
                    }
                    ConstValue::Float(f) => TExprKind::Float(f),
                    ConstValue::Bool(b) => TExprKind::Bool(b),
                    ConstValue::Str(s) => TExprKind::Str(s),
                };
                Ok(TExpr { kind, ty, span: sp })
            }
            ModItem::Fn(fid) => {
                let f = &self.checker.funcs[fid.0];
                let param_tys: Vec<TypeId> = f.params.iter().map(|l| f.locals[l.0 as usize].ty).collect();
                let fn_ty = self.checker.arena.intern(TypeKind::Fn(param_tys, f.ret));
                Ok(TExpr { kind: TExprKind::FnRef(fid), ty: fn_ty, span: sp })
            }
            _ => Err(Diagnostic::error(sp, "type/module cannot be used directly as a value")),
        }
    }

    fn resolve_struct_field(
        &self,
        obj: TExpr,
        sid: StructId,
        field: &str,
        through_ptr: bool,
        sp: Span,
    ) -> Result<TExpr, Diagnostic> {
        let st = &self.checker.arena.structs[sid.0];
        let finfo = st.fields.iter().find(|f| f.name == field).ok_or_else(|| {
            Diagnostic::error(sp, format!("struct '{}' has no field '{field}'", st.name))
        })?;
        Ok(TExpr {
            kind: TExprKind::Field {
                obj: Box::new(obj),
                offset: finfo.offset,
                through_ptr,
            },
            ty: finfo.ty,
            span: sp,
        })
    }

    fn check_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        expected: Option<TypeId>,
        sp: Span,
    ) -> Result<TExpr, Diagnostic> {
        // 1. Builtin functions
        if let ExprKind::Ident(name) = &callee.kind {
            if self.lookup_local(name).is_none() && !self.checker.mod_envs[self.mod_idx].items.contains_key(name) {
                if let Some(res) = self.try_check_builtin(name, args, expected, sp)? {
                    return Ok(res);
                }
            }
        }

        // 2. Method calls or module-qualified calls or static struct calls
        if let ExprKind::Field { obj, field } = &callee.kind {
            // Module-qualified: `mod.fn(...)`
            if let ExprKind::Ident(mod_alias) = &obj.kind {
                if self.lookup_local(mod_alias).is_none() {
                    if let Some(&tmod) = self.checker.mod_envs[self.mod_idx].submodules.get(mod_alias) {
                        let item = self.checker.mod_envs[tmod]
                            .items
                            .get(field)
                            .cloned()
                            .ok_or_else(|| {
                                Diagnostic::error(sp, format!("no symbol '{field}' in module '{mod_alias}'"))
                            })?;
                        return self.check_item_call(item, args, sp);
                    }
                    // Static method on Struct: `Board.new(...)` or `mod.Board.new(...)`
                    if let Some(ModItem::Struct(sid)) = self.checker.mod_envs[self.mod_idx].items.get(mod_alias).cloned() {
                        if let Some(&fid) = self.checker.struct_statics.get(&(sid, field.clone())) {
                            return self.check_fn_call(fid, args, None, sp);
                        }
                    }
                }
            }

            // Receiver method call: `obj.method(args...)`
            let tobj = self.check_expr(obj, None)?;
            let (sid, is_ptr) = match self.checker.arena.kind(tobj.ty) {
                TypeKind::Struct(s) => (*s, false),
                TypeKind::Ptr(inner) => match self.checker.arena.kind(*inner) {
                    TypeKind::Struct(s) => (*s, true),
                    _ => return Err(Diagnostic::error(sp, format!("type has no method '{field}'"))),
                },
                _ => {
                    // Maybe field is a fn ptr? No, non-struct has no fn field
                    return Err(Diagnostic::error(
                        sp,
                        format!("type '{}' has no method '{field}'", self.checker.arena.display(tobj.ty)),
                    ));
                }
            };
            if let Some(&fid) = self.checker.struct_methods.get(&(sid, field.clone())) {
                let self_param_mut = self.checker.funcs[fid.0].locals[0].is_mut;
                if self_param_mut && !is_ptr {
                    self.check_lvalue_mutable(&tobj, obj.span)?;
                }
                let self_arg = if is_ptr {
                    tobj
                } else {
                    self.mark_addr_taken(&tobj);
                    let ptr_ty = self.checker.arena.intern(TypeKind::Ptr(tobj.ty));
                    TExpr {
                        kind: TExprKind::AddrOf(Box::new(tobj)),
                        ty: ptr_ty,
                        span: obj.span,
                    }
                };
                return self.check_fn_call(fid, args, Some(self_arg), sp);
            }
            // Fallthrough: maybe struct has a field of `fn(...)` type!
            let fexpr = self.resolve_struct_field(tobj, sid, field, is_ptr, sp)?;
            return self.check_indirect_call(fexpr, args, sp);
        }

        // 3. Direct identifier call (local fn ptr vs module fn vs extern)
        if let ExprKind::Ident(name) = &callee.kind {
            if let Some(lid) = self.lookup_local(name) {
                let ty = self.locals[lid.0 as usize].ty;
                return self.check_indirect_call(TExpr { kind: TExprKind::Local(lid), ty, span: callee.span }, args, sp);
            }
            if let Some(item) = self.checker.mod_envs[self.mod_idx].items.get(name).cloned() {
                return self.check_item_call(item, args, sp);
            }
            return Err(Diagnostic::error(sp, format!("undefined function '{name}'")));
        }

        // 4. General indirect expression call
        let tcallee = self.check_expr(callee, None)?;
        self.check_indirect_call(tcallee, args, sp)
    }

    fn check_item_call(&mut self, item: ModItem, args: &[Expr], sp: Span) -> Result<TExpr, Diagnostic> {
        match item {
            ModItem::Fn(fid) => self.check_fn_call(fid, args, None, sp),
            ModItem::Extern(eid) => self.check_extern_call(eid, args, sp),
            _ => Err(Diagnostic::error(sp, "symbol is not callable")),
        }
    }

    fn check_fn_call(
        &mut self,
        fid: FnId,
        args: &[Expr],
        self_arg: Option<TExpr>,
        sp: Span,
    ) -> Result<TExpr, Diagnostic> {
        let param_tys: Vec<TypeId> = {
            let f = &self.checker.funcs[fid.0];
            f.params.iter().map(|lid| f.locals[lid.0 as usize].ty).collect()
        };
        let ret = self.checker.funcs[fid.0].ret;
        let fname = self.checker.funcs[fid.0].name.clone();
        let expected_user_args = if self_arg.is_some() { param_tys.len() - 1 } else { param_tys.len() };
        if args.len() != expected_user_args {
            return Err(Diagnostic::error(
                sp,
                format!("function '{fname}' expects {expected_user_args} argument(s), got {}", args.len()),
            ));
        }
        let mut targs = Vec::with_capacity(param_tys.len());
        let mut offset = 0;
        if let Some(sa) = self_arg {
            targs.push(sa);
            offset = 1;
        }
        for (i, a) in args.iter().enumerate() {
            let exp_ty = param_tys[i + offset];
            let ta = self.check_expr(a, Some(exp_ty))?;
            self.ensure_assignable(exp_ty, ta.ty, a.span)?;
            targs.push(ta);
        }
        Ok(TExpr {
            kind: TExprKind::CallFn { func: fid, args: targs },
            ty: ret,
            span: sp,
        })
    }

    fn check_extern_call(&mut self, eid: ExternId, args: &[Expr], sp: Span) -> Result<TExpr, Diagnostic> {
        let ext = self.checker.externs[eid.0].clone();
        if (!ext.variadic && args.len() != ext.params.len()) || (ext.variadic && args.len() < ext.params.len()) {
            return Err(Diagnostic::error(
                sp,
                format!("extern '{}' expects {} argument(s), got {}", ext.name, ext.params.len(), args.len()),
            ));
        }
        let mut targs = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            let exp_ty = ext.params.get(i).copied();
            let ta = self.check_expr(a, exp_ty)?;
            if let Some(et) = exp_ty {
                self.ensure_assignable(et, ta.ty, a.span)?;
            }
            targs.push(ta);
        }
        Ok(TExpr {
            kind: TExprKind::CallExtern { ext: eid, args: targs },
            ty: ext.ret,
            span: sp,
        })
    }

    fn check_indirect_call(&mut self, callee: TExpr, args: &[Expr], sp: Span) -> Result<TExpr, Diagnostic> {
        let TypeKind::Fn(param_tys, ret) = self.checker.arena.kind(callee.ty).clone() else {
            return Err(Diagnostic::error(sp, "expression is not callable"));
        };
        if args.len() != param_tys.len() {
            return Err(Diagnostic::error(
                sp,
                format!("function pointer expects {} argument(s), got {}", param_tys.len(), args.len()),
            ));
        }
        let mut targs = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            let ta = self.check_expr(a, Some(param_tys[i]))?;
            self.ensure_assignable(param_tys[i], ta.ty, a.span)?;
            targs.push(ta);
        }
        Ok(TExpr {
            kind: TExprKind::CallIndirect { callee: Box::new(callee), args: targs },
            ty: ret,
            span: sp,
        })
    }

    fn try_check_builtin(
        &mut self,
        name: &str,
        args: &[Expr],
        expected: Option<TypeId>,
        sp: Span,
    ) -> Result<Option<TExpr>, Diagnostic> {
        let builtin = match name {
            "print" | "print_raw" => {
                let newline = name == "print";
                let mut targs = Vec::with_capacity(args.len());
                for a in args {
                    targs.push(self.check_expr(a, None)?);
                }
                return Ok(Some(TExpr {
                    kind: TExprKind::CallBuiltin {
                        builtin: BuiltinCall::Print { newline },
                        args: targs,
                    },
                    ty: TY_VOID,
                    span: sp,
                }));
            }
            "assert" => {
                if args.is_empty() || args.len() > 2 {
                    return Err(Diagnostic::error(sp, "assert expects 1 or 2 arguments"));
                }
                let cond = self.check_expr(&args[0], Some(TY_BOOL))?;
                if cond.ty != TY_BOOL {
                    return Err(Diagnostic::error(args[0].span, "assert condition must be bool"));
                }
                let mut targs = vec![cond];
                if args.len() == 2 {
                    let msg = self.check_expr(&args[1], Some(TY_STR))?;
                    if msg.ty != TY_STR {
                        return Err(Diagnostic::error(args[1].span, "assert message must be str"));
                    }
                    targs.push(msg);
                }
                return Ok(Some(TExpr {
                    kind: TExprKind::CallBuiltin { builtin: BuiltinCall::Assert, args: targs },
                    ty: TY_VOID,
                    span: sp,
                }));
            }
            "panic" => {
                if args.len() != 1 {
                    return Err(Diagnostic::error(sp, "panic expects 1 str argument"));
                }
                let msg = self.check_expr(&args[0], Some(TY_STR))?;
                if msg.ty != TY_STR {
                    return Err(Diagnostic::error(args[0].span, "panic message must be str"));
                }
                return Ok(Some(TExpr {
                    kind: TExprKind::CallBuiltin { builtin: BuiltinCall::Panic, args: vec![msg] },
                    ty: TY_VOID,
                    span: sp,
                }));
            }
            "alloc" => (BuiltinCall::Alloc, vec![TY_I64], expected.filter(|t| self.checker.arena.is_ptr_like(*t)).unwrap_or(TY_PTR)),
            "realloc" => (BuiltinCall::Realloc, vec![TY_PTR, TY_I64], TY_PTR),
            "free" => (BuiltinCall::Free, vec![TY_PTR], TY_VOID),
            "copy_mem" => (BuiltinCall::CopyMem, vec![TY_PTR, TY_PTR, TY_I64], TY_VOID),
            "set_mem" => (BuiltinCall::SetMem, vec![TY_PTR, TY_U8, TY_I64], TY_VOID),
            "time_ns" => (BuiltinCall::TimeNs, vec![], TY_I64),
            "sleep_ms" => (BuiltinCall::SleepMs, vec![TY_I32], TY_VOID),
            "rand_u64" => (BuiltinCall::RandU64, vec![], TY_U64),
            "rand_range" => (BuiltinCall::RandRange, vec![TY_I64, TY_I64], TY_I64),
            "read_int" => (BuiltinCall::ReadInt, vec![], TY_I64),
            "read_line" => (BuiltinCall::ReadLine, vec![], TY_STR),
            "cstr_len" => (BuiltinCall::CStrLen, vec![TY_CSTR], TY_I64),
            "str_from_cstr" => (BuiltinCall::StrFromCStr, vec![TY_CSTR], TY_STR),
            "sqrt" => (BuiltinCall::Sqrt, vec![TY_F64], TY_F64),
            "sin" => (BuiltinCall::Sin, vec![TY_F64], TY_F64),
            "cos" => (BuiltinCall::Cos, vec![TY_F64], TY_F64),
            "tan" => (BuiltinCall::Tan, vec![TY_F64], TY_F64),
            "floor" => (BuiltinCall::Floor, vec![TY_F64], TY_F64),
            "ceil" => (BuiltinCall::Ceil, vec![TY_F64], TY_F64),
            "round" => (BuiltinCall::Round, vec![TY_F64], TY_F64),
            "pow" => (BuiltinCall::Pow, vec![TY_F64, TY_F64], TY_F64),
            "min" | "max" | "abs" | "clamp" => {
                return self.check_polymorphic_math_builtin(name, args, expected, sp).map(Some);
            }
            _ => return Ok(None),
        };

        let (op, param_tys, ret_ty) = builtin;
        if args.len() != param_tys.len() {
            return Err(Diagnostic::error(
                sp,
                format!("builtin '{name}' expects {} argument(s), got {}", param_tys.len(), args.len()),
            ));
        }
        let mut targs = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            let ta = self.check_expr(a, Some(param_tys[i]))?;
            self.ensure_assignable(param_tys[i], ta.ty, a.span)?;
            targs.push(ta);
        }
        Ok(Some(TExpr {
            kind: TExprKind::CallBuiltin { builtin: op, args: targs },
            ty: ret_ty,
            span: sp,
        }))
    }

    fn check_polymorphic_math_builtin(
        &mut self,
        name: &str,
        args: &[Expr],
        expected: Option<TypeId>,
        sp: Span,
    ) -> Result<TExpr, Diagnostic> {
        let want_args = match name {
            "abs" => 1,
            "min" | "max" => 2,
            "clamp" => 3,
            _ => unreachable!(),
        };
        if args.len() != want_args {
            return Err(Diagnostic::error(sp, format!("'{name}' expects {want_args} arguments")));
        }
        let first = self.check_expr(&args[0], expected)?;
        if !self.checker.arena.is_numeric(first.ty) {
            return Err(Diagnostic::error(args[0].span, format!("'{name}' requires numeric arguments")));
        }
        let ty = first.ty;
        let mut targs = vec![first];
        for a in &args[1..] {
            let ta = self.check_expr(a, Some(ty))?;
            self.ensure_assignable(ty, ta.ty, a.span)?;
            targs.push(ta);
        }
        let op = match name {
            "min" => BuiltinCall::Min,
            "max" => BuiltinCall::Max,
            "abs" => BuiltinCall::Abs,
            "clamp" => BuiltinCall::Clamp,
            _ => unreachable!(),
        };
        Ok(TExpr {
            kind: TExprKind::CallBuiltin { builtin: op, args: targs },
            ty,
            span: sp,
        })
    }

    fn check_binop_types(&self, op: BinOp, lt: TypeId, rt: TypeId, sp: Span) -> Result<TypeId, Diagnostic> {
        if matches!(op, BinOp::Add) && lt == TY_STR && rt == TY_STR {
            return Ok(TY_STR);
        }
        if matches!(op, BinOp::Eq | BinOp::Ne) {
            if lt == rt && (self.checker.arena.is_numeric(lt) || lt == TY_BOOL || lt == TY_STR || self.checker.arena.is_ptr_like(lt) || matches!(self.checker.arena.kind(lt), TypeKind::Enum(_))) {
                return Ok(TY_BOOL);
            }
            if self.checker.arena.is_ptr_like(lt) && self.checker.arena.is_ptr_like(rt) {
                return Ok(TY_BOOL);
            }
        }
        if matches!(op, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge) {
            if lt == rt && (self.checker.arena.is_numeric(lt) || lt == TY_STR) {
                return Ok(TY_BOOL);
            }
        }
        if matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div) {
            if lt == rt && self.checker.arena.is_numeric(lt) {
                return Ok(lt);
            }
        }
        if matches!(op, BinOp::Mod | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr) {
            if lt == rt && self.checker.arena.is_int(lt) {
                return Ok(lt);
            }
        }
        Err(Diagnostic::error(
            sp,
            format!(
                "invalid binary operator {:?} for '{}' and '{}'",
                op,
                self.checker.arena.display(lt),
                self.checker.arena.display(rt)
            ),
        ))
    }
}

fn definitely_returns(stmts: &[TStmt]) -> bool {
    for s in stmts {
        match &s.kind {
            TStmtKind::Return(_) => return true,
            TStmtKind::If { branches, else_body } => {
                if let Some(eb) = else_body {
                    if branches.iter().all(|(_, b)| definitely_returns(b)) && definitely_returns(eb) {
                        return true;
                    }
                }
            }
            TStmtKind::Match { cases, else_body, .. } => {
                if let Some(eb) = else_body {
                    if cases.iter().all(|(_, b)| definitely_returns(b)) && definitely_returns(eb) {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

fn sanitize_ident(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
