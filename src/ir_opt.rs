use std::collections::{HashMap, HashSet, VecDeque};

use crate::ast::{BinOp, UnaryOp};
use crate::ir::*;
use crate::types::{TypeArena, TypeId, TY_BOOL, TY_F32, TY_F64, TY_I64, TY_U64};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptLevel {
    Debug,
    Speed,
    Max,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ConstVal {
    Int(i128),
    Float(f64),
    Bool(bool),
}

pub fn optimize_program(prog: &mut IrProgram, level: OptLevel) -> OptStats {
    let mut stats = OptStats::new();
    if level == OptLevel::Debug {
        return stats;
    }

    // First pass: local optimization on functions so small functions shrink before inlining
    for func in &mut prog.funcs {
        optimize_function(func, &prog.arena, &mut stats);
    }

    // Inline small single-block functions
    inline_small_functions(prog, level, &mut stats);

    // Re-optimize after inlining to fold newly exposed constants and dead code
    for func in &mut prog.funcs {
        for _ in 0..3 {
            let before = stats.folded_consts + stats.dce_removed + stats.blocks_removed + stats.cse_eliminated;
            optimize_function(func, &prog.arena, &mut stats);
            let after = stats.folded_consts + stats.dce_removed + stats.blocks_removed + stats.cse_eliminated;
            if before == after {
                break;
            }
        }
    }

    stats
}

fn inline_small_functions(prog: &mut IrProgram, level: OptLevel, stats: &mut OptStats) {
    let threshold = if level == OptLevel::Max { 36 } else { 18 };
    let mut inlineables: HashMap<FnId, IrFunction> = HashMap::new();
    for f in &prog.funcs {
        if f.blocks.len() == 1 && matches!(f.blocks[0].term, IrTerm::Return(_)) {
            let has_self_call = f.blocks[0].insts.iter().any(|i| matches!(i, IrInst::Call { func, .. } if *func == f.id));
            let no_addr_taken_param = f.params.iter().all(|p| !f.slots[p.slot.0 as usize].addr_taken);
            let limit = if f.inline_hint { threshold * 2 } else { threshold };
            if !has_self_call && no_addr_taken_param && f.blocks[0].insts.len() <= limit {
                inlineables.insert(f.id, f.clone());
            }
        }
    }

    for f in &mut prog.funcs {
        for b in &mut f.blocks {
            let mut new_insts = Vec::with_capacity(b.insts.len());
            for inst in &b.insts {
                if let IrInst::Call { dst, func: callee_id, args, .. } = inst {
                    if *callee_id != f.id {
                        if let Some(callee) = inlineables.get(callee_id) {
                            stats.inlined_calls += 1;
                            let slot_base = f.slots.len() as u32;
                            for cs in &callee.slots {
                                f.slots.push(IrSlot {
                                    id: SlotId(slot_base + cs.id.0),
                                    name: format!("inl_{}", cs.name),
                                    ty: cs.ty,
                                    addr_taken: cs.addr_taken,
                                    is_param: false,
                                });
                            }
                            let vreg_base = f.vreg_types.len() as u32;
                            for &vt in &callee.vreg_types {
                                f.vreg_types.push(vt);
                            }
                            for (p, &arg_v) in callee.params.iter().zip(args.iter()) {
                                let mapped_slot = SlotId(slot_base + p.slot.0);
                                new_insts.push(IrInst::StoreSlot {
                                    slot: mapped_slot,
                                    val: arg_v,
                                    ty: p.ty,
                                });
                            }
                            for ci in &callee.blocks[0].insts {
                                new_insts.push(remap_inst(ci, slot_base, vreg_base));
                            }
                            if let (Some(d), IrTerm::Return(Some(ret_v))) = (dst, &callee.blocks[0].term) {
                                new_insts.push(IrInst::Copy {
                                    dst: *d,
                                    src: VReg(vreg_base + ret_v.0),
                                });
                            }
                            continue;
                        }
                    }
                }
                new_insts.push(inst.clone());
            }
            b.insts = new_insts;
        }
    }
}

fn remap_inst(inst: &IrInst, sb: u32, vb: u32) -> IrInst {
    let rv = |v: VReg| VReg(vb + v.0);
    let rs = |s: SlotId| SlotId(sb + s.0);
    match inst {
        IrInst::ConstInt { dst, ty, val } => IrInst::ConstInt { dst: rv(*dst), ty: *ty, val: *val },
        IrInst::ConstFloat { dst, ty, val } => IrInst::ConstFloat { dst: rv(*dst), ty: *ty, val: *val },
        IrInst::ConstBool { dst, val } => IrInst::ConstBool { dst: rv(*dst), val: *val },
        IrInst::ConstNull { dst, ty } => IrInst::ConstNull { dst: rv(*dst), ty: *ty },
        IrInst::ConstStr { dst, lit } => IrInst::ConstStr { dst: rv(*dst), lit: *lit },
        IrInst::ConstCStr { dst, lit } => IrInst::ConstCStr { dst: rv(*dst), lit: *lit },
        IrInst::ConstWStr { dst, lit } => IrInst::ConstWStr { dst: rv(*dst), lit: *lit },
        IrInst::FnAddr { dst, func, ty } => IrInst::FnAddr { dst: rv(*dst), func: *func, ty: *ty },
        IrInst::Copy { dst, src } => IrInst::Copy { dst: rv(*dst), src: rv(*src) },
        IrInst::Unary { dst, op, src, ty } => IrInst::Unary { dst: rv(*dst), op: *op, src: rv(*src), ty: *ty },
        IrInst::Binary { dst, op, left, right, ty } => IrInst::Binary {
            dst: rv(*dst), op: *op, left: rv(*left), right: rv(*right), ty: *ty,
        },
        IrInst::Cast { dst, src, from, to } => IrInst::Cast { dst: rv(*dst), src: rv(*src), from: *from, to: *to },
        IrInst::SlotAddr { dst, slot, ptr_ty } => IrInst::SlotAddr { dst: rv(*dst), slot: rs(*slot), ptr_ty: *ptr_ty },
        IrInst::LoadSlot { dst, slot, ty } => IrInst::LoadSlot { dst: rv(*dst), slot: rs(*slot), ty: *ty },
        IrInst::StoreSlot { slot, val, ty } => IrInst::StoreSlot { slot: rs(*slot), val: rv(*val), ty: *ty },
        IrInst::ZeroSlot { slot, ty } => IrInst::ZeroSlot { slot: rs(*slot), ty: *ty },
        IrInst::LoadPtr { dst, ptr, ty, trusted } => IrInst::LoadPtr { dst: rv(*dst), ptr: rv(*ptr), ty: *ty, trusted: *trusted },
        IrInst::StorePtr { ptr, val, ty, trusted } => IrInst::StorePtr { ptr: rv(*ptr), val: rv(*val), ty: *ty, trusted: *trusted },
        IrInst::FieldPtr { dst, base_ptr, offset, field_ty } => IrInst::FieldPtr {
            dst: rv(*dst), base_ptr: rv(*base_ptr), offset: *offset, field_ty: *field_ty,
        },
        IrInst::IndexPtr { dst, base_ptr, index, elem_ty, elem_size } => IrInst::IndexPtr {
            dst: rv(*dst), base_ptr: rv(*base_ptr), index: rv(*index), elem_ty: *elem_ty, elem_size: *elem_size,
        },
        IrInst::BoundsCheck { index, len, file_lit, line } => IrInst::BoundsCheck {
            index: rv(*index), len: rv(*len), file_lit: *file_lit, line: *line,
        },
        IrInst::MemCopy { dst_ptr, src_ptr, bytes } => IrInst::MemCopy {
            dst_ptr: rv(*dst_ptr), src_ptr: rv(*src_ptr), bytes: *bytes,
        },
        IrInst::MemZero { dst_ptr, bytes } => IrInst::MemZero { dst_ptr: rv(*dst_ptr), bytes: *bytes },
        IrInst::Call { dst, func, args, ret } => IrInst::Call {
            dst: dst.map(rv), func: *func, args: args.iter().copied().map(rv).collect(), ret: *ret,
        },
        IrInst::CallExtern { dst, ext, args, ret } => IrInst::CallExtern {
            dst: dst.map(rv), ext: *ext, args: args.iter().copied().map(rv).collect(), ret: *ret,
        },
        IrInst::CallIndirect { dst, callee, args, fn_ty, ret } => IrInst::CallIndirect {
            dst: dst.map(rv), callee: rv(*callee), args: args.iter().copied().map(rv).collect(), fn_ty: *fn_ty, ret: *ret,
        },
        IrInst::RuntimeCall { dst, op, args, ret } => IrInst::RuntimeCall {
            dst: dst.map(rv), op: *op, args: args.iter().copied().map(rv).collect(), ret: *ret,
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum CseKey {
    Unary(u8, u32, usize),
    Binary(u8, u32, u32, usize),
    FieldPtr(u32, u32, usize),
    IndexPtr(u32, u32, usize, u32),
}

fn optimize_function(func: &mut IrFunction, arena: &TypeArena, stats: &mut OptStats) {
    let mut consts: HashMap<VReg, ConstVal> = HashMap::new();
    let mut copies: HashMap<VReg, VReg> = HashMap::new();

    // Pass 1: Global copy propagation + constant folding + local store-to-load & CSE
    for bb in &mut func.blocks {
        let mut slot_vals: HashMap<SlotId, VReg> = HashMap::new();
        let mut cse: HashMap<CseKey, VReg> = HashMap::new();
        let mut out = Vec::with_capacity(bb.insts.len());

        for mut inst in std::mem::take(&mut bb.insts) {
            rewrite_inst_uses(&mut inst, &copies);

            match &inst {
                IrInst::ConstInt { dst, val, .. } => {
                    consts.insert(*dst, ConstVal::Int(*val));
                }
                IrInst::ConstFloat { dst, val, .. } => {
                    consts.insert(*dst, ConstVal::Float(*val));
                }
                IrInst::ConstBool { dst, val } => {
                    consts.insert(*dst, ConstVal::Bool(*val));
                }
                IrInst::Copy { dst, src } => {
                    let root = resolve_copy(*src, &copies);
                    copies.insert(*dst, root);
                    if let Some(c) = consts.get(&root).copied() {
                        consts.insert(*dst, c);
                    }
                    stats.propagated_copies += 1;
                    continue;
                }
                IrInst::Unary { dst, op, src, ty } => {
                    if let Some(c) = consts.get(src).copied() {
                        if let Some(folded) = fold_unary(*op, c, *ty, *dst) {
                            match &folded {
                                IrInst::ConstInt { val, .. } => { consts.insert(*dst, ConstVal::Int(*val)); }
                                IrInst::ConstFloat { val, .. } => { consts.insert(*dst, ConstVal::Float(*val)); }
                                IrInst::ConstBool { val, .. } => { consts.insert(*dst, ConstVal::Bool(*val)); }
                                _ => {}
                            }
                            stats.folded_consts += 1;
                            out.push(folded);
                            continue;
                        }
                    }
                    let key = CseKey::Unary(*op as u8, src.0, ty.0);
                    if let Some(&prev) = cse.get(&key) {
                        copies.insert(*dst, prev);
                        stats.cse_eliminated += 1;
                        continue;
                    }
                    cse.insert(key, *dst);
                }
                IrInst::Binary { dst, op, left, right, ty } => {
                    if let (Some(lc), Some(rc)) = (consts.get(left).copied(), consts.get(right).copied()) {
                        if let Some(folded) = fold_binary(*op, lc, rc, *ty, *dst, arena) {
                            match &folded {
                                IrInst::ConstInt { val, .. } => { consts.insert(*dst, ConstVal::Int(*val)); }
                                IrInst::ConstFloat { val, .. } => { consts.insert(*dst, ConstVal::Float(*val)); }
                                IrInst::ConstBool { val, .. } => { consts.insert(*dst, ConstVal::Bool(*val)); }
                                _ => {}
                            }
                            stats.folded_consts += 1;
                            out.push(folded);
                            continue;
                        }
                    }
                    // Algebraic identities & strength reduction
                    if let Some(simplified) = simplify_binary(*dst, *op, *left, *right, *ty, &consts, arena) {
                        match &simplified {
                            IrInst::Copy { src, .. } => {
                                let root = resolve_copy(*src, &copies);
                                copies.insert(*dst, root);
                                stats.folded_consts += 1;
                                continue;
                            }
                            IrInst::ConstInt { val, .. } => {
                                consts.insert(*dst, ConstVal::Int(*val));
                            }
                            IrInst::ConstBool { val, .. } => {
                                consts.insert(*dst, ConstVal::Bool(*val));
                            }
                            _ => {}
                        }
                        stats.folded_consts += 1;
                        out.push(simplified);
                        continue;
                    }
                    let key = CseKey::Binary(*op as u8, left.0, right.0, ty.0);
                    if let Some(&prev) = cse.get(&key) {
                        copies.insert(*dst, prev);
                        stats.cse_eliminated += 1;
                        continue;
                    }
                    cse.insert(key, *dst);
                }
                IrInst::Cast { dst, src, from, to } => {
                    if from == to {
                        copies.insert(*dst, *src);
                        stats.propagated_copies += 1;
                        continue;
                    }
                    if let Some(ConstVal::Int(n)) = consts.get(src).copied() {
                        if arena.is_int(*to) {
                            consts.insert(*dst, ConstVal::Int(n));
                            stats.folded_consts += 1;
                            out.push(IrInst::ConstInt { dst: *dst, ty: *to, val: n });
                            continue;
                        } else if arena.is_float(*to) {
                            let fv = n as f64;
                            consts.insert(*dst, ConstVal::Float(fv));
                            stats.folded_consts += 1;
                            out.push(IrInst::ConstFloat { dst: *dst, ty: *to, val: fv });
                            continue;
                        }
                    }
                }
                IrInst::StoreSlot { slot, val, .. } => {
                    if !func.slots[slot.0 as usize].addr_taken {
                        slot_vals.insert(*slot, *val);
                    }
                }
                IrInst::LoadSlot { dst, slot, .. } => {
                    if !func.slots[slot.0 as usize].addr_taken {
                        if let Some(&val) = slot_vals.get(slot) {
                            let root = resolve_copy(val, &copies);
                            copies.insert(*dst, root);
                            if let Some(c) = consts.get(&root).copied() {
                                consts.insert(*dst, c);
                            }
                            stats.propagated_copies += 1;
                            continue;
                        }
                        slot_vals.insert(*slot, *dst);
                    }
                }
                IrInst::FieldPtr { dst, base_ptr, offset, field_ty } => {
                    let key = CseKey::FieldPtr(base_ptr.0, *offset, field_ty.0);
                    if let Some(&prev) = cse.get(&key) {
                        copies.insert(*dst, prev);
                        stats.cse_eliminated += 1;
                        continue;
                    }
                    cse.insert(key, *dst);
                }
                IrInst::IndexPtr { dst, base_ptr, index, elem_ty, elem_size } => {
                    let key = CseKey::IndexPtr(base_ptr.0, index.0, elem_ty.0, *elem_size);
                    if let Some(&prev) = cse.get(&key) {
                        copies.insert(*dst, prev);
                        stats.cse_eliminated += 1;
                        continue;
                    }
                    cse.insert(key, *dst);
                }
                IrInst::BoundsCheck { index, len, .. } => {
                    if let (Some(ConstVal::Int(i)), Some(ConstVal::Int(n))) = (consts.get(index), consts.get(len)) {
                        if *i >= 0 && *i < *n {
                            stats.bounds_eliminated += 1;
                            continue;
                        }
                    }
                }
                _ => {}
            }
            out.push(inst);
        }
        bb.insts = out;
        rewrite_term_uses(&mut bb.term, &copies);

        if let IrTerm::BranchIf { cond, then_bb, else_bb } = bb.term {
            if then_bb == else_bb {
                bb.term = IrTerm::Jump(then_bb);
                stats.blocks_removed += 1;
            } else if let Some(ConstVal::Bool(b)) = consts.get(&cond).copied() {
                bb.term = IrTerm::Jump(if b { then_bb } else { else_bb });
                stats.folded_consts += 1;
            }
        }
    }

    // Pass 2: Dead block elimination + jump-chain cleanup
    simplify_cfg(func, stats);

    // Pass 3: Dead pure instruction elimination
    dce_function(func, stats);
}

fn resolve_copy(mut v: VReg, copies: &HashMap<VReg, VReg>) -> VReg {
    while let Some(&next) = copies.get(&v) {
        if next == v {
            break;
        }
        v = next;
    }
    v
}

fn fold_unary(op: UnaryOp, val: ConstVal, ty: TypeId, dst: VReg) -> Option<IrInst> {
    match (op, val) {
        (UnaryOp::Neg, ConstVal::Int(x)) => Some(IrInst::ConstInt { dst, ty, val: x.wrapping_neg() }),
        (UnaryOp::Neg, ConstVal::Float(x)) => Some(IrInst::ConstFloat { dst, ty, val: -x }),
        (UnaryOp::BitNot, ConstVal::Int(x)) => Some(IrInst::ConstInt { dst, ty, val: !x }),
        (UnaryOp::Not, ConstVal::Bool(b)) => Some(IrInst::ConstBool { dst, val: !b }),
        _ => None,
    }
}

fn fold_binary(
    op: BinOp,
    l: ConstVal,
    r: ConstVal,
    out_ty: TypeId,
    dst: VReg,
    arena: &TypeArena,
) -> Option<IrInst> {
    match (l, r) {
        (ConstVal::Int(a), ConstVal::Int(b)) => {
            if matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge) {
                let res = match op {
                    BinOp::Eq => a == b,
                    BinOp::Ne => a != b,
                    BinOp::Lt => a < b,
                    BinOp::Le => a <= b,
                    BinOp::Gt => a > b,
                    BinOp::Ge => a >= b,
                    _ => unreachable!(),
                };
                return Some(IrInst::ConstBool { dst, val: res });
            }
            if !arena.is_int(out_ty) {
                return None;
            }
            let val = match op {
                BinOp::Add => a.wrapping_add(b),
                BinOp::Sub => a.wrapping_sub(b),
                BinOp::Mul => a.wrapping_mul(b),
                BinOp::Div if b != 0 => a.wrapping_div(b),
                BinOp::Mod if b != 0 => a.wrapping_rem(b),
                BinOp::BitAnd => a & b,
                BinOp::BitOr => a | b,
                BinOp::BitXor => a ^ b,
                BinOp::Shl if b >= 0 && b < 64 => a.wrapping_shl(b as u32),
                BinOp::Shr if b >= 0 && b < 64 => a.wrapping_shr(b as u32),
                _ => return None,
            };
            Some(IrInst::ConstInt { dst, ty: out_ty, val })
        }
        (ConstVal::Float(a), ConstVal::Float(b)) => {
            if matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge) {
                let res = match op {
                    BinOp::Eq => a == b,
                    BinOp::Ne => a != b,
                    BinOp::Lt => a < b,
                    BinOp::Le => a <= b,
                    BinOp::Gt => a > b,
                    BinOp::Ge => a >= b,
                    _ => unreachable!(),
                };
                return Some(IrInst::ConstBool { dst, val: res });
            }
            let val = match op {
                BinOp::Add => a + b,
                BinOp::Sub => a - b,
                BinOp::Mul => a * b,
                BinOp::Div => a / b,
                _ => return None,
            };
            Some(IrInst::ConstFloat { dst, ty: out_ty, val })
        }
        (ConstVal::Bool(a), ConstVal::Bool(b)) => {
            let res = match op {
                BinOp::Eq => a == b,
                BinOp::Ne => a != b,
                BinOp::And => a && b,
                BinOp::Or => a || b,
                _ => return None,
            };
            Some(IrInst::ConstBool { dst, val: res })
        }
        _ => None,
    }
}

fn simplify_binary(
    dst: VReg,
    op: BinOp,
    left: VReg,
    right: VReg,
    ty: TypeId,
    consts: &HashMap<VReg, ConstVal>,
    arena: &TypeArena,
) -> Option<IrInst> {
    if left == right {
        match op {
            BinOp::Sub | BinOp::BitXor if arena.is_int(ty) => {
                return Some(IrInst::ConstInt { dst, ty, val: 0 });
            }
            BinOp::Eq | BinOp::Le | BinOp::Ge if ty == TY_BOOL => {
                return Some(IrInst::ConstBool { dst, val: true });
            }
            BinOp::Ne | BinOp::Lt | BinOp::Gt if ty == TY_BOOL => {
                return Some(IrInst::ConstBool { dst, val: false });
            }
            _ => {}
        }
    }
    let rc = consts.get(&right).copied();
    let lc = consts.get(&left).copied();
    if arena.is_int(ty) {
        if let Some(ConstVal::Int(r)) = rc {
            match op {
                BinOp::Add | BinOp::Sub | BinOp::BitOr | BinOp::BitXor | BinOp::Shl | BinOp::Shr if r == 0 => {
                    return Some(IrInst::Copy { dst, src: left });
                }
                BinOp::Mul if r == 1 => return Some(IrInst::Copy { dst, src: left }),
                BinOp::Mul | BinOp::BitAnd if r == 0 => return Some(IrInst::ConstInt { dst, ty, val: 0 }),
                BinOp::Div if r == 1 => return Some(IrInst::Copy { dst, src: left }),
                BinOp::Mul if r > 1 && (r as u128).is_power_of_two() && r <= (1 << 30) => {
                    // Notice: strength reduction to shift doesn't need extra VReg if backend also handles, skip creating orphan VReg
                }
                _ => {}
            }
        }
        if let Some(ConstVal::Int(l)) = lc {
            match op {
                BinOp::Add | BinOp::BitOr | BinOp::BitXor if l == 0 => {
                    return Some(IrInst::Copy { dst, src: right });
                }
                BinOp::Mul if l == 1 => return Some(IrInst::Copy { dst, src: right }),
                BinOp::Mul | BinOp::BitAnd if l == 0 => return Some(IrInst::ConstInt { dst, ty, val: 0 }),
                _ => {}
            }
        }
    }
    let _ = (TY_F32, TY_F64, TY_I64, TY_U64);
    None
}

fn simplify_cfg(func: &mut IrFunction, stats: &mut OptStats) {
    if func.blocks.is_empty() {
        return;
    }
    let mut reachable = HashSet::new();
    let mut q = VecDeque::new();
    reachable.insert(BlockId(0));
    q.push_back(BlockId(0));
    while let Some(b) = q.pop_front() {
        let term = &func.blocks[b.0 as usize].term;
        match *term {
            IrTerm::Jump(t) => {
                if reachable.insert(t) { q.push_back(t); }
            }
            IrTerm::BranchIf { then_bb, else_bb, .. } => {
                if reachable.insert(then_bb) { q.push_back(then_bb); }
                if reachable.insert(else_bb) { q.push_back(else_bb); }
            }
            IrTerm::Return(_) | IrTerm::Unreachable => {}
        }
    }

    if reachable.len() < func.blocks.len() {
        let mut old_to_new: HashMap<BlockId, BlockId> = HashMap::new();
        let mut new_blocks = Vec::with_capacity(reachable.len());
        for b in &func.blocks {
            if reachable.contains(&b.id) {
                let nid = BlockId(new_blocks.len() as u32);
                old_to_new.insert(b.id, nid);
                let mut nb = b.clone();
                nb.id = nid;
                new_blocks.push(nb);
            } else {
                stats.blocks_removed += 1;
            }
        }
        for b in &mut new_blocks {
            match &mut b.term {
                IrTerm::Jump(t) => *t = old_to_new[t],
                IrTerm::BranchIf { then_bb, else_bb, .. } => {
                    *then_bb = old_to_new[then_bb];
                    *else_bb = old_to_new[else_bb];
                }
                _ => {}
            }
        }
        func.blocks = new_blocks;
    }
}

fn dce_function(func: &mut IrFunction, stats: &mut OptStats) {
    loop {
        let mut used: HashSet<VReg> = HashSet::new();
        for b in &func.blocks {
            for inst in &b.insts {
                collect_uses(inst, &mut used);
            }
            match b.term {
                IrTerm::BranchIf { cond, .. } => { used.insert(cond); }
                IrTerm::Return(Some(v)) => { used.insert(v); }
                _ => {}
            }
        }
        let mut removed_any = false;
        for b in &mut func.blocks {
            let before = b.insts.len();
            b.insts.retain(|inst| match inst_pure_dst(inst) {
                Some(d) => used.contains(&d),
                None => true,
            });
            let diff = before - b.insts.len();
            if diff > 0 {
                stats.dce_removed += diff;
                removed_any = true;
            }
        }
        if !removed_any {
            break;
        }
    }
}

fn inst_pure_dst(inst: &IrInst) -> Option<VReg> {
    match inst {
        IrInst::ConstInt { dst, .. }
        | IrInst::ConstFloat { dst, .. }
        | IrInst::ConstBool { dst, .. }
        | IrInst::ConstNull { dst, .. }
        | IrInst::ConstStr { dst, .. }
        | IrInst::ConstCStr { dst, .. }
        | IrInst::ConstWStr { dst, .. }
        | IrInst::FnAddr { dst, .. }
        | IrInst::Copy { dst, .. }
        | IrInst::Unary { dst, .. }
        | IrInst::Binary { dst, .. }
        | IrInst::Cast { dst, .. }
        | IrInst::SlotAddr { dst, .. }
        | IrInst::LoadSlot { dst, .. }
        | IrInst::FieldPtr { dst, .. }
        | IrInst::IndexPtr { dst, .. } => Some(*dst),
        _ => None,
    }
}

fn rewrite_inst_uses(inst: &mut IrInst, copies: &HashMap<VReg, VReg>) {
    let r = |v: &mut VReg| {
        *v = resolve_copy(*v, copies);
    };
    match inst {
        IrInst::Copy { src, .. }
        | IrInst::Unary { src, .. }
        | IrInst::Cast { src, .. } => r(src),
        IrInst::Binary { left, right, .. } => { r(left); r(right); }
        IrInst::StoreSlot { val, .. } => r(val),
        IrInst::LoadPtr { ptr, .. } => r(ptr),
        IrInst::StorePtr { ptr, val, .. } => { r(ptr); r(val); }
        IrInst::FieldPtr { base_ptr, .. } => r(base_ptr),
        IrInst::IndexPtr { base_ptr, index, .. } => { r(base_ptr); r(index); }
        IrInst::BoundsCheck { index, len, .. } => { r(index); r(len); }
        IrInst::MemCopy { dst_ptr, src_ptr, .. } => { r(dst_ptr); r(src_ptr); }
        IrInst::MemZero { dst_ptr, .. } => r(dst_ptr),
        IrInst::Call { args, .. }
        | IrInst::CallExtern { args, .. }
        | IrInst::RuntimeCall { args, .. } => {
            for a in args { r(a); }
        }
        IrInst::CallIndirect { callee, args, .. } => {
            r(callee);
            for a in args { r(a); }
        }
        _ => {}
    }
}

fn rewrite_term_uses(term: &mut IrTerm, copies: &HashMap<VReg, VReg>) {
    match term {
        IrTerm::BranchIf { cond, .. } => *cond = resolve_copy(*cond, copies),
        IrTerm::Return(Some(v)) => *v = resolve_copy(*v, copies),
        _ => {}
    }
}

fn collect_uses(inst: &IrInst, used: &mut HashSet<VReg>) {
    match inst {
        IrInst::Copy { src, .. }
        | IrInst::Unary { src, .. }
        | IrInst::Cast { src, .. } => { used.insert(*src); }
        IrInst::Binary { left, right, .. } => { used.insert(*left); used.insert(*right); }
        IrInst::StoreSlot { val, .. } => { used.insert(*val); }
        IrInst::LoadPtr { ptr, .. } => { used.insert(*ptr); }
        IrInst::StorePtr { ptr, val, .. } => { used.insert(*ptr); used.insert(*val); }
        IrInst::FieldPtr { base_ptr, .. } => { used.insert(*base_ptr); }
        IrInst::IndexPtr { base_ptr, index, .. } => { used.insert(*base_ptr); used.insert(*index); }
        IrInst::BoundsCheck { index, len, .. } => { used.insert(*index); used.insert(*len); }
        IrInst::MemCopy { dst_ptr, src_ptr, .. } => { used.insert(*dst_ptr); used.insert(*src_ptr); }
        IrInst::MemZero { dst_ptr, .. } => { used.insert(*dst_ptr); }
        IrInst::Call { args, .. }
        | IrInst::CallExtern { args, .. }
        | IrInst::RuntimeCall { args, .. } => {
            for a in args { used.insert(*a); }
        }
        IrInst::CallIndirect { callee, args, .. } => {
            used.insert(*callee);
            for a in args { used.insert(*a); }
        }
        _ => {}
    }
}
