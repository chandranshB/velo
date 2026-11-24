use crate::ast::{BinOp, UnaryOp};
use crate::error::Span;
use crate::types::{TypeArena, TypeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VReg(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SlotId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FnId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ExternId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StrLitId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subsystem {
    Console,
    Windows,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct IrExtern {
    pub id: ExternId,
    pub name: String,
    pub params: Vec<TypeId>,
    pub ret: TypeId,
    pub variadic: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct IrSlot {
    pub id: SlotId,
    pub name: String,
    pub ty: TypeId,
    pub addr_taken: bool,
    pub is_param: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct IrParam {
    pub name: String,
    pub ty: TypeId,
    pub noalias: bool,
    pub slot: SlotId,
}

#[derive(Debug, Clone)]
pub struct IrFunction {
    pub id: FnId,
    pub name: String,
    pub c_name: String,
    pub params: Vec<IrParam>,
    pub ret: TypeId,
    pub inline_hint: bool,
    pub slots: Vec<IrSlot>,
    pub vreg_types: Vec<TypeId>,
    pub blocks: Vec<IrBlock>,
}

#[derive(Debug, Clone)]
pub struct IrBlock {
    pub id: BlockId,
    pub insts: Vec<IrInst>,
    pub term: IrTerm,
}

#[derive(Debug, Clone)]
pub enum IrInst {
    ConstInt { dst: VReg, ty: TypeId, val: i128 },
    ConstFloat { dst: VReg, ty: TypeId, val: f64 },
    ConstBool { dst: VReg, val: bool },
    ConstNull { dst: VReg, ty: TypeId },
    ConstStr { dst: VReg, lit: StrLitId },     // builds `str { ptr, len }` in slot or value
    ConstCStr { dst: VReg, lit: StrLitId },    // *u8 pointer
    ConstWStr { dst: VReg, lit: usize },       // *u16 pointer into wstr_lits
    FnAddr { dst: VReg, func: FnId, ty: TypeId },
    Copy { dst: VReg, src: VReg },
    Unary { dst: VReg, op: UnaryOp, src: VReg, ty: TypeId },
    Binary { dst: VReg, op: BinOp, left: VReg, right: VReg, ty: TypeId },
    Cast { dst: VReg, src: VReg, from: TypeId, to: TypeId },
    SlotAddr { dst: VReg, slot: SlotId, ptr_ty: TypeId },
    LoadSlot { dst: VReg, slot: SlotId, ty: TypeId },
    StoreSlot { slot: SlotId, val: VReg, ty: TypeId },
    ZeroSlot { slot: SlotId, ty: TypeId },
    LoadPtr { dst: VReg, ptr: VReg, ty: TypeId, trusted: bool },
    StorePtr { ptr: VReg, val: VReg, ty: TypeId, trusted: bool },
    FieldPtr { dst: VReg, base_ptr: VReg, offset: u32, field_ty: TypeId },
    IndexPtr { dst: VReg, base_ptr: VReg, index: VReg, elem_ty: TypeId, elem_size: u32 },
    BoundsCheck {
        index: VReg,
        len: VReg,
        file_lit: StrLitId,
        line: i64,
    },
    MemCopy { dst_ptr: VReg, src_ptr: VReg, bytes: u32 },
    MemZero { dst_ptr: VReg, bytes: u32 },
    Call { dst: Option<VReg>, func: FnId, args: Vec<VReg>, ret: TypeId },
    CallExtern { dst: Option<VReg>, ext: ExternId, args: Vec<VReg>, ret: TypeId },
    CallIndirect { dst: Option<VReg>, callee: VReg, args: Vec<VReg>, fn_ty: TypeId, ret: TypeId },
    RuntimeCall { dst: Option<VReg>, op: RtOp, args: Vec<VReg>, ret: TypeId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RtOp {
    PrintI64,
    PrintU64,
    PrintF64,
    PrintBool,
    PrintStr,
    PrintCStr,
    PrintNewline,
    PrintSpace,
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
    StrConcat,
    I64ToStr,
    U64ToStr,
    F64ToStr,
    BoolToStr,
    StrEq,
    StrCmp,
    CStrLen,
    SqrtF64,
    SinF64,
    CosF64,
    TanF64,
    FloorF64,
    CeilF64,
    RoundF64,
    PowF64,
}

#[derive(Debug, Clone)]
pub enum IrTerm {
    Jump(BlockId),
    BranchIf {
        cond: VReg,
        then_bb: BlockId,
        else_bb: BlockId,
    },
    Return(Option<VReg>),
    Unreachable,
}

pub struct IrProgram {
    pub arena: TypeArena,
    pub links: Vec<String>,
    pub subsystem: Subsystem,
    pub str_lits: Vec<String>,
    pub wstr_lits: Vec<Vec<u16>>,
    pub externs: Vec<IrExtern>,
    pub funcs: Vec<IrFunction>,
    pub main_fn: Option<FnId>,
}

#[derive(Debug, Clone)]
pub struct OptStats {
    pub folded_consts: usize,
    pub propagated_copies: usize,
    pub cse_eliminated: usize,
    pub bounds_eliminated: usize,
    pub dce_removed: usize,
    pub inlined_calls: usize,
    pub blocks_removed: usize,
}

impl OptStats {
    pub fn new() -> Self {
        Self {
            folded_consts: 0,
            propagated_copies: 0,
            cse_eliminated: 0,
            bounds_eliminated: 0,
            dce_removed: 0,
            inlined_calls: 0,
            blocks_removed: 0,
        }
    }
}

#[allow(dead_code)]
pub fn span_line(sp: Span) -> i64 {
    sp.line as i64
}
