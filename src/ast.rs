use crate::error::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TypeExpr {
    Named(String, Span),
    Ptr(Box<TypeExpr>, Span),
    OpaquePtr(Span),
    Array(Box<TypeExpr>, u64, Span),
    Slice(Box<TypeExpr>, Span),
    Fn(Vec<TypeExpr>, Box<TypeExpr>, Span),
    Void(Span),
}

impl TypeExpr {
    pub fn span(&self) -> Span {
        match self {
            TypeExpr::Named(_, s)
            | TypeExpr::Ptr(_, s)
            | TypeExpr::OpaquePtr(s)
            | TypeExpr::Array(_, _, s)
            | TypeExpr::Slice(_, s)
            | TypeExpr::Fn(_, _, s)
            | TypeExpr::Void(s) => *s,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Program {
    pub modules: Vec<ModuleAst>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ModuleAst {
    pub file_id: usize,
    pub imports: Vec<ImportDecl>,
    pub links: Vec<(String, Span)>,
    pub subsystem: Option<(String, Span)>,
    pub consts: Vec<ConstDecl>,
    pub enums: Vec<EnumDecl>,
    pub structs: Vec<StructDecl>,
    pub externs: Vec<ExternDecl>,
    pub funcs: Vec<FnDecl>,
}

#[derive(Debug, Clone)]
pub struct ImportDecl {
    pub path: String,
    pub is_file: bool,
    pub alias: Option<String>,
    pub from_items: Option<Vec<String>>, // None => module import, Some(["*"]) => glob, Some([...]) => specific
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ConstDecl {
    pub name: String,
    pub ty: Option<TypeExpr>,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EnumVariant {
    pub name: String,
    pub value: Option<i64>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EnumDecl {
    pub name: String,
    pub repr: Option<TypeExpr>,
    pub variants: Vec<EnumVariant>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FieldDecl {
    pub name: String,
    pub ty: TypeExpr,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct StructDecl {
    pub name: String,
    pub fields: Vec<FieldDecl>,
    pub methods: Vec<FnDecl>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct ParamDecl {
    pub name: String,
    pub is_var: bool,
    pub is_self: bool,
    pub ty: Option<TypeExpr>,
    pub span: Span,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ExternDecl {
    pub name: String,
    pub params: Vec<ParamDecl>,
    pub ret: TypeExpr,
    pub variadic: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct FnDecl {
    pub name: String,
    pub params: Vec<ParamDecl>,
    pub ret: TypeExpr,
    pub inline_hint: bool,
    pub body: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    Let {
        name: String,
        ty: Option<TypeExpr>,
        value: Expr,
    },
    Var {
        name: String,
        ty: Option<TypeExpr>,
        value: Expr,
    },
    Const(ConstDecl),
    Assign {
        target: Expr,
        op: Option<BinOp>,
        value: Expr,
    },
    Expr(Expr),
    If {
        branches: Vec<(Expr, Vec<Stmt>)>,
        else_body: Option<Vec<Stmt>>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    ForRange {
        var_name: String,
        start: Expr,
        end: Expr,
        inclusive: bool,
        body: Vec<Stmt>,
    },
    ForIter {
        var_name: String,
        iter: Expr,
        body: Vec<Stmt>,
    },
    Match {
        expr: Expr,
        cases: Vec<(Vec<Expr>, Vec<Stmt>)>,
        else_body: Option<Vec<Stmt>>,
    },
    Return(Option<Expr>),
    Break,
    Continue,
    Defer(Box<Stmt>),
    Pass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone)]
pub enum FStrSeg {
    Lit(String),
    Interp { expr: Expr, prec: Option<i32> },
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    Int(i128),
    Float(f64),
    Bool(bool),
    Str(String),
    CStr(String),
    WStr(String),
    FStr(Vec<FStrSeg>),
    Null,
    Ident(String),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Cast {
        expr: Box<Expr>,
        ty: TypeExpr,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    Field {
        obj: Box<Expr>,
        field: String,
    },
    Index {
        obj: Box<Expr>,
        index: Box<Expr>,
    },
    Slice {
        obj: Box<Expr>,
        start: Option<Box<Expr>>,
        end: Option<Box<Expr>>,
    },
    StructLit {
        name: String,
        fields: Vec<(String, Expr)>,
    },
    ArrayLit(Vec<Expr>),
    ArrayRepeat {
        value: Box<Expr>,
        count: u64,
    },
    Ternary {
        then_expr: Box<Expr>,
        cond: Box<Expr>,
        else_expr: Box<Expr>,
    },
    AddrOf(Box<Expr>),
    Deref(Box<Expr>),
    SizeOf(TypeExpr),
    Zeroed(TypeExpr),
}
