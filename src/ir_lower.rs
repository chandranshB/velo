use std::collections::HashMap;

use crate::ast::{BinOp, UnaryOp};
use crate::checker::*;
use crate::error::Span;
use crate::ir::*;
use crate::types::*;

pub fn lower_program(prog: CheckedProgram) -> IrProgram {
    let mut ctx = LowerProgramCtx {
        files: prog.files,
        arena: prog.arena,
        str_lits: Vec::new(),
        str_map: HashMap::new(),
        wstr_lits: Vec::new(),
        wstr_map: HashMap::new(),
    };

    let externs = prog
        .externs
        .into_iter()
        .map(|e| IrExtern {
            id: e.id,
            name: e.name,
            params: e.params,
            ret: e.ret,
            variadic: e.variadic,
        })
        .collect();

    let mut funcs = Vec::with_capacity(prog.funcs.len());
    for f in prog.funcs {
        funcs.push(ctx.lower_function(f));
    }

    IrProgram {
        arena: ctx.arena,
        links: prog.links,
        subsystem: prog.subsystem,
        str_lits: ctx.str_lits,
        wstr_lits: ctx.wstr_lits,
        externs,
        funcs,
        main_fn: prog.main_fn,
    }
}

struct LowerProgramCtx {
    files: Vec<crate::error::SourceFile>,
    arena: TypeArena,
    str_lits: Vec<String>,
    str_map: HashMap<String, StrLitId>,
    wstr_lits: Vec<Vec<u16>>,
    wstr_map: HashMap<String, usize>,
}

impl LowerProgramCtx {
    fn intern_str(&mut self, s: &str) -> StrLitId {
        if let Some(&id) = self.str_map.get(s) {
            return id;
        }
        let id = StrLitId(self.str_lits.len());
        self.str_lits.push(s.to_string());
        self.str_map.insert(s.to_string(), id);
        id
    }

    fn intern_wstr(&mut self, s: &str) -> usize {
        if let Some(&id) = self.wstr_map.get(s) {
            return id;
        }
        let mut units: Vec<u16> = s.encode_utf16().collect();
        units.push(0);
        let id = self.wstr_lits.len();
        self.wstr_lits.push(units);
        self.wstr_map.insert(s.to_string(), id);
        id
    }

    fn file_lit(&mut self, file_id: usize) -> StrLitId {
        let p = self
            .files
            .get(file_id)
            .map(|f| f.path.display().to_string())
            .unwrap_or_else(|| "<unknown>".to_string());
        self.intern_str(&p)
    }

    fn lower_function(&mut self, f: CheckedFn) -> IrFunction {
        let mut slots = Vec::with_capacity(f.locals.len());
        let mut local_to_slot = HashMap::new();
        for l in &f.locals {
            let sid = SlotId(slots.len() as u32);
            let force_mem = self.arena.is_aggregate(l.ty);
            slots.push(IrSlot {
                id: sid,
                name: l.name.clone(),
                ty: l.ty,
                addr_taken: l.addr_taken || force_mem,
                is_param: l.is_param,
            });
            local_to_slot.insert(l.id, sid);
        }

        let mut params = Vec::with_capacity(f.params.len());
        for pid in &f.params {
            let l = &f.locals[pid.0 as usize];
            params.push(IrParam {
                name: l.name.clone(),
                ty: l.ty,
                noalias: l.noalias,
                slot: local_to_slot[pid],
            });
        }

        let mut builder = FnLower {
            ctx: self,
            slots,
            local_to_slot,
            vreg_types: Vec::new(),
            blocks: Vec::new(),
            cur_bb: BlockId(0),
            cur_insts: Vec::new(),
            terminated: false,
            defer_scopes: vec![Vec::new()],
            loop_stack: Vec::new(),
        };
        builder.blocks.push(IrBlock {
            id: BlockId(0),
            insts: Vec::new(),
            term: IrTerm::Unreachable,
        });

        builder.lower_stmts(&f.body);
        if !builder.terminated {
            builder.run_defers_from(0);
            if f.ret == TY_VOID {
                builder.terminate(IrTerm::Return(None));
            } else {
                builder.terminate(IrTerm::Unreachable);
            }
        }

        IrFunction {
            id: f.id,
            name: f.name,
            c_name: f.c_name,
            params,
            ret: f.ret,
            inline_hint: f.inline_hint,
            slots: builder.slots,
            vreg_types: builder.vreg_types,
            blocks: builder.blocks,
        }
    }
}

#[derive(Clone)]
struct LoopCtx {
    break_bb: BlockId,
    continue_bb: BlockId,
    defer_depth: usize,
}

struct FnLower<'a> {
    ctx: &'a mut LowerProgramCtx,
    slots: Vec<IrSlot>,
    local_to_slot: HashMap<LocalId, SlotId>,
    vreg_types: Vec<TypeId>,
    blocks: Vec<IrBlock>,
    cur_bb: BlockId,
    cur_insts: Vec<IrInst>,
    terminated: bool,
    defer_scopes: Vec<Vec<TStmt>>,
    loop_stack: Vec<LoopCtx>,
}

impl<'a> FnLower<'a> {
    fn new_vreg(&mut self, ty: TypeId) -> VReg {
        let id = VReg(self.vreg_types.len() as u32);
        self.vreg_types.push(ty);
        id
    }

    fn new_temp_slot(&mut self, name: &str, ty: TypeId, addr_taken: bool) -> SlotId {
        let id = SlotId(self.slots.len() as u32);
        let force_mem = addr_taken || self.ctx.arena.is_aggregate(ty);
        self.slots.push(IrSlot {
            id,
            name: format!("_tmp_{}_{}", id.0, name),
            ty,
            addr_taken: force_mem,
            is_param: false,
        });
        id
    }

    fn new_block(&mut self) -> BlockId {
        let id = BlockId(self.blocks.len() as u32);
        self.blocks.push(IrBlock {
            id,
            insts: Vec::new(),
            term: IrTerm::Unreachable,
        });
        id
    }

    fn switch_to(&mut self, bb: BlockId) {
        self.cur_bb = bb;
        self.cur_insts = Vec::new();
        self.terminated = false;
    }

    fn emit(&mut self, inst: IrInst) {
        if !self.terminated {
            self.cur_insts.push(inst);
        }
    }

    fn terminate(&mut self, term: IrTerm) {
        if !self.terminated {
            let bb = &mut self.blocks[self.cur_bb.0 as usize];
            bb.insts = std::mem::take(&mut self.cur_insts);
            bb.term = term;
            self.terminated = true;
        }
    }

    fn push_defer_scope(&mut self) {
        self.defer_scopes.push(Vec::new());
    }

    fn pop_defer_scope(&mut self) {
        let defers = self.defer_scopes.pop().unwrap_or_default();
        if !self.terminated {
            for d in defers.into_iter().rev() {
                self.lower_stmt(&d);
            }
        }
    }

    fn run_defers_from(&mut self, depth: usize) {
        let total = self.defer_scopes.len();
        for i in (depth..total).rev() {
            let list = self.defer_scopes[i].clone();
            for d in list.into_iter().rev() {
                if self.terminated {
                    return;
                }
                self.lower_stmt(&d);
            }
        }
    }

    fn lower_stmts(&mut self, stmts: &[TStmt]) {
        self.push_defer_scope();
        for s in stmts {
            if self.terminated {
                break;
            }
            self.lower_stmt(s);
        }
        self.pop_defer_scope();
    }

    fn lower_stmt(&mut self, stmt: &TStmt) {
        match &stmt.kind {
            TStmtKind::Pass => {}
            TStmtKind::Defer(inner) => {
                if let Some(top) = self.defer_scopes.last_mut() {
                    top.push((**inner).clone());
                }
            }
            TStmtKind::VarInit { local, value } => {
                let slot = self.local_to_slot[local];
                let ty = self.slots[slot.0 as usize].ty;
                self.store_expr_into_slot(slot, ty, value);
            }
            TStmtKind::Assign { target, op, value } => {
                let ptr = self.lower_lvalue(target);
                let ty = target.ty;
                if let Some(bop) = op {
                    let cur = self.load_from_ptr(ptr, ty);
                    let rhs = self.lower_expr(value);
                    let res = self.emit_binary(*bop, cur, rhs, ty);
                    self.store_value_to_ptr(ptr, res, ty);
                } else {
                    self.store_expr_to_ptr(ptr, ty, value);
                }
            }
            TStmtKind::Expr(e) => {
                let _ = self.lower_expr(e);
            }
            TStmtKind::Return(opt) => {
                let ret_val = opt.as_ref().map(|e| {
                    let v = self.lower_expr(e);
                    if self.ctx.arena.is_aggregate(e.ty) {
                        // Copy into a dedicated return temp so deferred mutations don't clobber return value
                        let tmp = self.new_temp_slot("ret", e.ty, true);
                        let pty = self.ctx.arena.intern(TypeKind::Ptr(e.ty));
                        let dst_ptr = self.new_vreg(pty);
                        self.emit(IrInst::SlotAddr { dst: dst_ptr, slot: tmp, ptr_ty: pty });
                        let sz = self.ctx.arena.layout(e.ty).size;
                        self.emit(IrInst::MemCopy { dst_ptr, src_ptr: v, bytes: sz });
                        dst_ptr
                    } else {
                        v
                    }
                });
                self.run_defers_from(0);
                self.terminate(IrTerm::Return(ret_val));
            }
            TStmtKind::Break => {
                if let Some(lc) = self.loop_stack.last().cloned() {
                    self.run_defers_from(lc.defer_depth);
                    self.terminate(IrTerm::Jump(lc.break_bb));
                }
            }
            TStmtKind::Continue => {
                if let Some(lc) = self.loop_stack.last().cloned() {
                    self.run_defers_from(lc.defer_depth);
                    self.terminate(IrTerm::Jump(lc.continue_bb));
                }
            }
            TStmtKind::If { branches, else_body } => {
                let merge_bb = self.new_block();
                let mut any_reaches_merge = false;

                for (i, (cond, body)) in branches.iter().enumerate() {
                    let c = self.lower_expr(cond);
                    let then_bb = self.new_block();
                    let next_bb = if i + 1 < branches.len() || else_body.is_some() {
                        self.new_block()
                    } else {
                        any_reaches_merge = true;
                        merge_bb
                    };
                    self.terminate(IrTerm::BranchIf {
                        cond: c,
                        then_bb,
                        else_bb: next_bb,
                    });

                    self.switch_to(then_bb);
                    self.lower_stmts(body);
                    if !self.terminated {
                        any_reaches_merge = true;
                        self.terminate(IrTerm::Jump(merge_bb));
                    }

                    if i + 1 < branches.len() || else_body.is_some() {
                        self.switch_to(next_bb);
                    }
                }

                if let Some(eb) = else_body {
                    self.lower_stmts(eb);
                    if !self.terminated {
                        any_reaches_merge = true;
                        self.terminate(IrTerm::Jump(merge_bb));
                    }
                }

                if any_reaches_merge {
                    self.switch_to(merge_bb);
                } else {
                    self.terminated = true;
                }
            }
            TStmtKind::While { cond, body } => {
                let head_bb = self.new_block();
                let body_bb = self.new_block();
                let exit_bb = self.new_block();

                self.terminate(IrTerm::Jump(head_bb));
                self.switch_to(head_bb);
                let c = self.lower_expr(cond);
                self.terminate(IrTerm::BranchIf {
                    cond: c,
                    then_bb: body_bb,
                    else_bb: exit_bb,
                });

                self.switch_to(body_bb);
                self.loop_stack.push(LoopCtx {
                    break_bb: exit_bb,
                    continue_bb: head_bb,
                    defer_depth: self.defer_scopes.len(),
                });
                self.lower_stmts(body);
                self.loop_stack.pop();
                if !self.terminated {
                    self.terminate(IrTerm::Jump(head_bb));
                }

                self.switch_to(exit_bb);
            }
            TStmtKind::ForRange {
                var_local,
                start,
                end,
                inclusive,
                body,
            } => {
                let var_slot = self.local_to_slot[var_local];
                let idx_ty = self.slots[var_slot.0 as usize].ty;
                let s_val = self.lower_expr(start);
                let e_val = self.lower_expr(end);
                let end_slot = self.new_temp_slot("for_end", idx_ty, false);
                self.emit(IrInst::StoreSlot { slot: var_slot, val: s_val, ty: idx_ty });
                self.emit(IrInst::StoreSlot { slot: end_slot, val: e_val, ty: idx_ty });

                let head_bb = self.new_block();
                let body_bb = self.new_block();
                let step_bb = self.new_block();
                let exit_bb = self.new_block();

                self.terminate(IrTerm::Jump(head_bb));
                self.switch_to(head_bb);
                let cur = self.new_vreg(idx_ty);
                self.emit(IrInst::LoadSlot { dst: cur, slot: var_slot, ty: idx_ty });
                let lim = self.new_vreg(idx_ty);
                self.emit(IrInst::LoadSlot { dst: lim, slot: end_slot, ty: idx_ty });
                let cmp_op = if *inclusive { BinOp::Le } else { BinOp::Lt };
                let cond = self.new_vreg(TY_BOOL);
                self.emit(IrInst::Binary {
                    dst: cond,
                    op: cmp_op,
                    left: cur,
                    right: lim,
                    ty: TY_BOOL,
                });
                self.terminate(IrTerm::BranchIf {
                    cond,
                    then_bb: body_bb,
                    else_bb: exit_bb,
                });

                self.switch_to(body_bb);
                self.loop_stack.push(LoopCtx {
                    break_bb: exit_bb,
                    continue_bb: step_bb,
                    defer_depth: self.defer_scopes.len(),
                });
                self.lower_stmts(body);
                self.loop_stack.pop();
                if !self.terminated {
                    self.terminate(IrTerm::Jump(step_bb));
                }

                self.switch_to(step_bb);
                let cur2 = self.new_vreg(idx_ty);
                self.emit(IrInst::LoadSlot { dst: cur2, slot: var_slot, ty: idx_ty });
                let one = self.new_vreg(idx_ty);
                self.emit(IrInst::ConstInt { dst: one, ty: idx_ty, val: 1 });
                let next = self.new_vreg(idx_ty);
                self.emit(IrInst::Binary {
                    dst: next,
                    op: BinOp::Add,
                    left: cur2,
                    right: one,
                    ty: idx_ty,
                });
                self.emit(IrInst::StoreSlot { slot: var_slot, val: next, ty: idx_ty });
                self.terminate(IrTerm::Jump(head_bb));

                self.switch_to(exit_bb);
            }
            TStmtKind::ForIter { var_local, iter, body } => {
                let var_slot = self.local_to_slot[var_local];
                let elem_ty = self.slots[var_slot.0 as usize].ty;
                let elem_sz = self.ctx.arena.layout(elem_ty).size;
                let iter_ptr_or_agg = self.lower_expr(iter);
                let elem_ptr_ty = self.ctx.arena.intern(TypeKind::Ptr(elem_ty));

                let (base_ptr, len_val) = match self.ctx.arena.kind(iter.ty).clone() {
                    TypeKind::Array(_, n) => {
                        let base = self.new_vreg(elem_ptr_ty);
                        self.emit(IrInst::FieldPtr {
                            dst: base,
                            base_ptr: iter_ptr_or_agg,
                            offset: 0,
                            field_ty: elem_ptr_ty,
                        });
                        let len = self.new_vreg(TY_I64);
                        self.emit(IrInst::ConstInt { dst: len, ty: TY_I64, val: n as i128 });
                        (base, len)
                    }
                    TypeKind::Slice(_) => {
                        let base = self.load_field(iter_ptr_or_agg, 0, elem_ptr_ty);
                        let len = self.load_field(iter_ptr_or_agg, 8, TY_I64);
                        (base, len)
                    }
                    _ => unreachable!(),
                };

                let idx_slot = self.new_temp_slot("iter_i", TY_I64, false);
                let zero = self.new_vreg(TY_I64);
                self.emit(IrInst::ConstInt { dst: zero, ty: TY_I64, val: 0 });
                self.emit(IrInst::StoreSlot { slot: idx_slot, val: zero, ty: TY_I64 });

                let head_bb = self.new_block();
                let body_bb = self.new_block();
                let step_bb = self.new_block();
                let exit_bb = self.new_block();

                self.terminate(IrTerm::Jump(head_bb));
                self.switch_to(head_bb);
                let idx = self.new_vreg(TY_I64);
                self.emit(IrInst::LoadSlot { dst: idx, slot: idx_slot, ty: TY_I64 });
                let cond = self.new_vreg(TY_BOOL);
                self.emit(IrInst::Binary {
                    dst: cond,
                    op: BinOp::Lt,
                    left: idx,
                    right: len_val,
                    ty: TY_BOOL,
                });
                self.terminate(IrTerm::BranchIf {
                    cond,
                    then_bb: body_bb,
                    else_bb: exit_bb,
                });

                self.switch_to(body_bb);
                let idx_b = self.new_vreg(TY_I64);
                self.emit(IrInst::LoadSlot { dst: idx_b, slot: idx_slot, ty: TY_I64 });
                let el_ptr = self.new_vreg(elem_ptr_ty);
                self.emit(IrInst::IndexPtr {
                    dst: el_ptr,
                    base_ptr,
                    index: idx_b,
                    elem_ty,
                    elem_size: elem_sz,
                });
                let val = self.load_from_ptr(el_ptr, elem_ty);
                self.store_value_to_slot(var_slot, val, elem_ty);

                self.loop_stack.push(LoopCtx {
                    break_bb: exit_bb,
                    continue_bb: step_bb,
                    defer_depth: self.defer_scopes.len(),
                });
                self.lower_stmts(body);
                self.loop_stack.pop();
                if !self.terminated {
                    self.terminate(IrTerm::Jump(step_bb));
                }

                self.switch_to(step_bb);
                let cur = self.new_vreg(TY_I64);
                self.emit(IrInst::LoadSlot { dst: cur, slot: idx_slot, ty: TY_I64 });
                let one = self.new_vreg(TY_I64);
                self.emit(IrInst::ConstInt { dst: one, ty: TY_I64, val: 1 });
                let next = self.new_vreg(TY_I64);
                self.emit(IrInst::Binary {
                    dst: next,
                    op: BinOp::Add,
                    left: cur,
                    right: one,
                    ty: TY_I64,
                });
                self.emit(IrInst::StoreSlot { slot: idx_slot, val: next, ty: TY_I64 });
                self.terminate(IrTerm::Jump(head_bb));

                self.switch_to(exit_bb);
            }
            TStmtKind::Match {
                expr,
                cases,
                else_body,
            } => {
                let scrut = self.lower_expr(expr);
                let scrut_ty = expr.ty;
                let merge_bb = self.new_block();
                let mut any_reaches = false;

                for (pats, body) in cases {
                    let body_bb = self.new_block();
                    let next_case_bb = self.new_block();

                    for (pi, p) in pats.iter().enumerate() {
                        let pv = self.lower_expr(p);
                        let eq = self.emit_binary(BinOp::Eq, scrut, pv, scrut_ty);
                        let next_pat_bb = if pi + 1 < pats.len() {
                            self.new_block()
                        } else {
                            next_case_bb
                        };
                        self.terminate(IrTerm::BranchIf {
                            cond: eq,
                            then_bb: body_bb,
                            else_bb: next_pat_bb,
                        });
                        if pi + 1 < pats.len() {
                            self.switch_to(next_pat_bb);
                        }
                    }

                    self.switch_to(body_bb);
                    self.lower_stmts(body);
                    if !self.terminated {
                        any_reaches = true;
                        self.terminate(IrTerm::Jump(merge_bb));
                    }

                    self.switch_to(next_case_bb);
                }

                if let Some(eb) = else_body {
                    self.lower_stmts(eb);
                    if !self.terminated {
                        any_reaches = true;
                        self.terminate(IrTerm::Jump(merge_bb));
                    }
                } else {
                    any_reaches = true;
                    self.terminate(IrTerm::Jump(merge_bb));
                }

                if any_reaches {
                    self.switch_to(merge_bb);
                } else {
                    self.terminated = true;
                }
            }
        }
    }

    fn store_expr_into_slot(&mut self, slot: SlotId, ty: TypeId, expr: &TExpr) {
        if matches!(expr.kind, TExprKind::Zeroed) {
            self.emit(IrInst::ZeroSlot { slot, ty });
            return;
        }
        if self.ctx.arena.is_aggregate(ty) {
            let pty = self.ctx.arena.intern(TypeKind::Ptr(ty));
            let ptr = self.new_vreg(pty);
            self.emit(IrInst::SlotAddr { dst: ptr, slot, ptr_ty: pty });
            self.store_expr_to_ptr(ptr, ty, expr);
        } else {
            let v = self.lower_expr(expr);
            self.emit(IrInst::StoreSlot { slot, val: v, ty });
        }
    }

    fn store_value_to_slot(&mut self, slot: SlotId, val: VReg, ty: TypeId) {
        if self.ctx.arena.is_aggregate(ty) {
            let pty = self.ctx.arena.intern(TypeKind::Ptr(ty));
            let dst_ptr = self.new_vreg(pty);
            self.emit(IrInst::SlotAddr { dst: dst_ptr, slot, ptr_ty: pty });
            let bytes = self.ctx.arena.layout(ty).size;
            self.emit(IrInst::MemCopy { dst_ptr, src_ptr: val, bytes });
        } else {
            self.emit(IrInst::StoreSlot { slot, val, ty });
        }
    }

    fn store_expr_to_ptr(&mut self, ptr: VReg, ty: TypeId, expr: &TExpr) {
        if matches!(expr.kind, TExprKind::Zeroed) {
            let bytes = self.ctx.arena.layout(ty).size;
            self.emit(IrInst::MemZero { dst_ptr: ptr, bytes });
            return;
        }
        let val = self.lower_expr(expr);
        self.store_value_to_ptr(ptr, val, ty);
    }

    fn store_value_to_ptr(&mut self, ptr: VReg, val: VReg, ty: TypeId) {
        if self.ctx.arena.is_aggregate(ty) {
            let bytes = self.ctx.arena.layout(ty).size;
            self.emit(IrInst::MemCopy { dst_ptr: ptr, src_ptr: val, bytes });
        } else {
            self.emit(IrInst::StorePtr { ptr, val, ty, trusted: false });
        }
    }

    fn load_from_ptr(&mut self, ptr: VReg, ty: TypeId) -> VReg {
        if self.ctx.arena.is_aggregate(ty) {
            ptr
        } else {
            let dst = self.new_vreg(ty);
            self.emit(IrInst::LoadPtr { dst, ptr, ty, trusted: false });
            dst
        }
    }

    fn load_field(&mut self, base_ptr: VReg, offset: u32, field_ty: TypeId) -> VReg {
        let pty = self.ctx.arena.intern(TypeKind::Ptr(field_ty));
        let fptr = self.new_vreg(pty);
        self.emit(IrInst::FieldPtr {
            dst: fptr,
            base_ptr,
            offset,
            field_ty,
        });
        self.load_from_ptr(fptr, field_ty)
    }

    fn lower_lvalue(&mut self, expr: &TExpr) -> VReg {
        match &expr.kind {
            TExprKind::Local(lid) => {
                let slot = self.local_to_slot[lid];
                let pty = self.ctx.arena.intern(TypeKind::Ptr(expr.ty));
                let dst = self.new_vreg(pty);
                self.emit(IrInst::SlotAddr { dst, slot, ptr_ty: pty });
                dst
            }
            TExprKind::Deref(inner) => self.lower_expr(inner),
            TExprKind::Field { obj, offset, through_ptr } => {
                let base = if *through_ptr {
                    self.lower_expr(obj)
                } else {
                    self.lower_lvalue(obj)
                };
                let pty = self.ctx.arena.intern(TypeKind::Ptr(expr.ty));
                let dst = self.new_vreg(pty);
                self.emit(IrInst::FieldPtr {
                    dst,
                    base_ptr: base,
                    offset: *offset,
                    field_ty: expr.ty,
                });
                dst
            }
            TExprKind::Index { obj, index, proved_in_bounds } => {
                self.lower_index_addr(obj, index, *proved_in_bounds, expr.ty, expr.span)
            }
            _ => self.lower_expr(expr),
        }
    }

    fn lower_index_addr(
        &mut self,
        obj: &TExpr,
        index: &TExpr,
        proved: bool,
        elem_ty: TypeId,
        span: Span,
    ) -> VReg {
        let idx = self.lower_expr(index);
        let idx64 = self.cast_to_i64(idx, index.ty);
        let elem_ptr_ty = self.ctx.arena.intern(TypeKind::Ptr(elem_ty));
        let elem_size = self.ctx.arena.layout(elem_ty).size;

        let (base_ptr, len_opt) = match self.ctx.arena.kind(obj.ty).clone() {
            TypeKind::Array(_, n) => {
                let arr_ptr = self.lower_lvalue(obj);
                let base = self.new_vreg(elem_ptr_ty);
                self.emit(IrInst::FieldPtr {
                    dst: base,
                    base_ptr: arr_ptr,
                    offset: 0,
                    field_ty: elem_ptr_ty,
                });
                let len = self.new_vreg(TY_I64);
                self.emit(IrInst::ConstInt { dst: len, ty: TY_I64, val: n as i128 });
                (base, Some(len))
            }
            TypeKind::Slice(_) | TypeKind::Str => {
                let sl_ptr = self.lower_expr(obj);
                let base = self.load_field(sl_ptr, 0, elem_ptr_ty);
                let len = self.load_field(sl_ptr, 8, TY_I64);
                (base, Some(len))
            }
            TypeKind::Ptr(_) => (self.lower_expr(obj), None),
            _ => unreachable!(),
        };

        if !proved {
            if let Some(len) = len_opt {
                let file_lit = self.ctx.file_lit(span.file_id);
                self.emit(IrInst::BoundsCheck {
                    index: idx64,
                    len,
                    file_lit,
                    line: span.line as i64,
                });
            }
        }

        let dst = self.new_vreg(elem_ptr_ty);
        self.emit(IrInst::IndexPtr {
            dst,
            base_ptr,
            index: idx64,
            elem_ty,
            elem_size,
        });
        dst
    }

    fn cast_to_i64(&mut self, v: VReg, from: TypeId) -> VReg {
        if from == TY_I64 {
            return v;
        }
        let dst = self.new_vreg(TY_I64);
        self.emit(IrInst::Cast { dst, src: v, from, to: TY_I64 });
        dst
    }

    fn lower_expr(&mut self, expr: &TExpr) -> VReg {
        let ty = expr.ty;
        match &expr.kind {
            TExprKind::Int(v) => {
                let dst = self.new_vreg(ty);
                self.emit(IrInst::ConstInt { dst, ty, val: *v });
                dst
            }
            TExprKind::Float(v) => {
                let dst = self.new_vreg(ty);
                self.emit(IrInst::ConstFloat { dst, ty, val: *v });
                dst
            }
            TExprKind::Bool(b) => {
                let dst = self.new_vreg(TY_BOOL);
                self.emit(IrInst::ConstBool { dst, val: *b });
                dst
            }
            TExprKind::Null => {
                let dst = self.new_vreg(ty);
                self.emit(IrInst::ConstNull { dst, ty });
                dst
            }
            TExprKind::Str(s) => {
                let lit = self.ctx.intern_str(s);
                let tmp = self.new_temp_slot("str_lit", TY_STR, true);
                let pty = self.ctx.arena.intern(TypeKind::Ptr(TY_STR));
                let sptr = self.new_vreg(pty);
                self.emit(IrInst::SlotAddr { dst: sptr, slot: tmp, ptr_ty: pty });
                let raw = self.new_vreg(TY_STR);
                self.emit(IrInst::ConstStr { dst: raw, lit });
                self.emit(IrInst::MemCopy { dst_ptr: sptr, src_ptr: raw, bytes: 16 });
                sptr
            }
            TExprKind::CStr(s) => {
                let lit = self.ctx.intern_str(s);
                let dst = self.new_vreg(TY_CSTR);
                self.emit(IrInst::ConstCStr { dst, lit });
                dst
            }
            TExprKind::WStr(s) => {
                let lit = self.ctx.intern_wstr(s);
                let dst = self.new_vreg(ty);
                self.emit(IrInst::ConstWStr { dst, lit });
                dst
            }
            TExprKind::FStr(segs) => self.lower_fstr(segs),
            TExprKind::Zeroed => {
                if self.ctx.arena.is_aggregate(ty) {
                    let tmp = self.new_temp_slot("zero", ty, true);
                    self.emit(IrInst::ZeroSlot { slot: tmp, ty });
                    let pty = self.ctx.arena.intern(TypeKind::Ptr(ty));
                    let dst = self.new_vreg(pty);
                    self.emit(IrInst::SlotAddr { dst, slot: tmp, ptr_ty: pty });
                    dst
                } else {
                    let dst = self.new_vreg(ty);
                    if self.ctx.arena.is_float(ty) {
                        self.emit(IrInst::ConstFloat { dst, ty, val: 0.0 });
                    } else if self.ctx.arena.is_ptr_like(ty) {
                        self.emit(IrInst::ConstNull { dst, ty });
                    } else if ty == TY_BOOL {
                        self.emit(IrInst::ConstBool { dst, val: false });
                    } else {
                        self.emit(IrInst::ConstInt { dst, ty, val: 0 });
                    }
                    dst
                }
            }
            TExprKind::Local(lid) => {
                let slot = self.local_to_slot[lid];
                if self.ctx.arena.is_aggregate(ty) {
                    let pty = self.ctx.arena.intern(TypeKind::Ptr(ty));
                    let dst = self.new_vreg(pty);
                    self.emit(IrInst::SlotAddr { dst, slot, ptr_ty: pty });
                    dst
                } else {
                    let dst = self.new_vreg(ty);
                    self.emit(IrInst::LoadSlot { dst, slot, ty });
                    dst
                }
            }
            TExprKind::FnRef(fid) => {
                let dst = self.new_vreg(ty);
                self.emit(IrInst::FnAddr { dst, func: *fid, ty });
                dst
            }
            TExprKind::Unary { op, expr: inner } => {
                let v = self.lower_expr(inner);
                let dst = self.new_vreg(ty);
                self.emit(IrInst::Unary { dst, op: *op, src: v, ty });
                dst
            }
            TExprKind::Binary { op, left, right } => {
                if matches!(op, BinOp::And | BinOp::Or) {
                    self.lower_short_circuit(*op, left, right)
                } else {
                    let lv = self.lower_expr(left);
                    let rv = self.lower_expr(right);
                    self.emit_binary(*op, lv, rv, left.ty)
                }
            }
            TExprKind::Cast { expr: inner, to } => {
                if inner.ty == TY_STR && *to == TY_CSTR {
                    let sptr = self.lower_expr(inner);
                    return self.load_field(sptr, 0, TY_CSTR);
                }
                let v = self.lower_expr(inner);
                let dst = self.new_vreg(*to);
                self.emit(IrInst::Cast { dst, src: v, from: inner.ty, to: *to });
                dst
            }
            TExprKind::AddrOf(inner) => self.lower_lvalue(inner),
            TExprKind::Deref(inner) => {
                let p = self.lower_expr(inner);
                self.load_from_ptr(p, ty)
            }
            TExprKind::Field { obj, offset, through_ptr } => {
                let base = if *through_ptr {
                    self.lower_expr(obj)
                } else {
                    self.lower_lvalue(obj)
                };
                let pty = self.ctx.arena.intern(TypeKind::Ptr(ty));
                let fptr = self.new_vreg(pty);
                self.emit(IrInst::FieldPtr {
                    dst: fptr,
                    base_ptr: base,
                    offset: *offset,
                    field_ty: ty,
                });
                self.load_from_ptr(fptr, ty)
            }
            TExprKind::StrLen(obj) | TExprKind::SliceLen(obj) => {
                let sp = self.lower_expr(obj);
                self.load_field(sp, 8, TY_I64)
            }
            TExprKind::StrPtr(obj) | TExprKind::SlicePtr(obj) => {
                let sp = self.lower_expr(obj);
                self.load_field(sp, 0, ty)
            }
            TExprKind::Index { obj, index, proved_in_bounds } => {
                let addr = self.lower_index_addr(obj, index, *proved_in_bounds, ty, expr.span);
                self.load_from_ptr(addr, ty)
            }
            TExprKind::Slice { obj, start, end } => {
                let (base_ptr, orig_len, elem_ty) = match self.ctx.arena.kind(obj.ty).clone() {
                    TypeKind::Array(el, n) => {
                        let arr_ptr = self.lower_lvalue(obj);
                        let epty = self.ctx.arena.intern(TypeKind::Ptr(el));
                        let b = self.new_vreg(epty);
                        self.emit(IrInst::FieldPtr {
                            dst: b,
                            base_ptr: arr_ptr,
                            offset: 0,
                            field_ty: epty,
                        });
                        let l = self.new_vreg(TY_I64);
                        self.emit(IrInst::ConstInt { dst: l, ty: TY_I64, val: n as i128 });
                        (b, l, el)
                    }
                    TypeKind::Slice(el) => {
                        let sl = self.lower_expr(obj);
                        let epty = self.ctx.arena.intern(TypeKind::Ptr(el));
                        (self.load_field(sl, 0, epty), self.load_field(sl, 8, TY_I64), el)
                    }
                    TypeKind::Str => {
                        let sl = self.lower_expr(obj);
                        let u8p = self.ctx.arena.intern(TypeKind::Ptr(TY_U8));
                        (self.load_field(sl, 0, u8p), self.load_field(sl, 8, TY_I64), TY_U8)
                    }
                    _ => unreachable!(),
                };
                let s = match start {
                    Some(st) => {
                        let v = self.lower_expr(st);
                        self.cast_to_i64(v, st.ty)
                    }
                    None => {
                        let z = self.new_vreg(TY_I64);
                        self.emit(IrInst::ConstInt { dst: z, ty: TY_I64, val: 0 });
                        z
                    }
                };
                let e = match end {
                    Some(en) => {
                        let v = self.lower_expr(en);
                        self.cast_to_i64(v, en.ty)
                    }
                    None => orig_len,
                };
                let elem_sz = self.ctx.arena.layout(elem_ty).size;
                let epty = self.ctx.arena.intern(TypeKind::Ptr(elem_ty));
                let new_ptr = self.new_vreg(epty);
                self.emit(IrInst::IndexPtr {
                    dst: new_ptr,
                    base_ptr,
                    index: s,
                    elem_ty,
                    elem_size: elem_sz,
                });
                let new_len = self.new_vreg(TY_I64);
                self.emit(IrInst::Binary {
                    dst: new_len,
                    op: BinOp::Sub,
                    left: e,
                    right: s,
                    ty: TY_I64,
                });
                let tmp = self.new_temp_slot("slice", ty, true);
                let pty = self.ctx.arena.intern(TypeKind::Ptr(ty));
                let saddr = self.new_vreg(pty);
                self.emit(IrInst::SlotAddr { dst: saddr, slot: tmp, ptr_ty: pty });
                let fp0_ty = self.ctx.arena.intern(TypeKind::Ptr(epty));
                let fp0 = self.new_vreg(fp0_ty);
                self.emit(IrInst::FieldPtr { dst: fp0, base_ptr: saddr, offset: 0, field_ty: epty });
                self.emit(IrInst::StorePtr { ptr: fp0, val: new_ptr, ty: epty, trusted: true });
                let fp1_ty = self.ctx.arena.intern(TypeKind::Ptr(TY_I64));
                let fp1 = self.new_vreg(fp1_ty);
                self.emit(IrInst::FieldPtr { dst: fp1, base_ptr: saddr, offset: 8, field_ty: TY_I64 });
                self.emit(IrInst::StorePtr { ptr: fp1, val: new_len, ty: TY_I64, trusted: true });
                saddr
            }
            TExprKind::StructLit { sid: _, fields } => {
                let tmp = self.new_temp_slot("struct_lit", ty, true);
                let pty = self.ctx.arena.intern(TypeKind::Ptr(ty));
                let base = self.new_vreg(pty);
                self.emit(IrInst::SlotAddr { dst: base, slot: tmp, ptr_ty: pty });
                for (offset, fty, fexpr) in fields {
                    let fptr_ty = self.ctx.arena.intern(TypeKind::Ptr(*fty));
                    let fptr = self.new_vreg(fptr_ty);
                    self.emit(IrInst::FieldPtr {
                        dst: fptr,
                        base_ptr: base,
                        offset: *offset,
                        field_ty: *fty,
                    });
                    self.store_expr_to_ptr(fptr, *fty, fexpr);
                }
                base
            }
            TExprKind::ArrayLit(elems) => {
                let tmp = self.new_temp_slot("arr_lit", ty, true);
                let pty = self.ctx.arena.intern(TypeKind::Ptr(ty));
                let base = self.new_vreg(pty);
                self.emit(IrInst::SlotAddr { dst: base, slot: tmp, ptr_ty: pty });
                let el_ty = match self.ctx.arena.kind(ty) {
                    TypeKind::Array(el, _) => *el,
                    _ => unreachable!(),
                };
                let el_sz = self.ctx.arena.layout(el_ty).size;
                let el_ptr_ty = self.ctx.arena.intern(TypeKind::Ptr(el_ty));
                for (i, el) in elems.iter().enumerate() {
                    let offset = (i as u32) * el_sz;
                    let ep = self.new_vreg(el_ptr_ty);
                    self.emit(IrInst::FieldPtr {
                        dst: ep,
                        base_ptr: base,
                        offset,
                        field_ty: el_ty,
                    });
                    self.store_expr_to_ptr(ep, el_ty, el);
                }
                base
            }
            TExprKind::ArrayRepeat { value, count } => {
                let tmp = self.new_temp_slot("arr_rep", ty, true);
                let pty = self.ctx.arena.intern(TypeKind::Ptr(ty));
                let base = self.new_vreg(pty);
                self.emit(IrInst::SlotAddr { dst: base, slot: tmp, ptr_ty: pty });
                if *count > 0 {
                    let el_ty = value.ty;
                    let el_sz = self.ctx.arena.layout(el_ty).size;
                    let el_ptr_ty = self.ctx.arena.intern(TypeKind::Ptr(el_ty));
                    let v = self.lower_expr(value);
                    for i in 0..*count {
                        let ep = self.new_vreg(el_ptr_ty);
                        self.emit(IrInst::FieldPtr {
                            dst: ep,
                            base_ptr: base,
                            offset: (i as u32) * el_sz,
                            field_ty: el_ty,
                        });
                        self.store_value_to_ptr(ep, v, el_ty);
                    }
                }
                base
            }
            TExprKind::Ternary { then_expr, cond, else_expr } => {
                let c = self.lower_expr(cond);
                let res_slot = self.new_temp_slot("tern", ty, false);
                let then_bb = self.new_block();
                let else_bb = self.new_block();
                let merge_bb = self.new_block();
                self.terminate(IrTerm::BranchIf { cond: c, then_bb, else_bb });

                self.switch_to(then_bb);
                self.store_expr_into_slot(res_slot, ty, then_expr);
                self.terminate(IrTerm::Jump(merge_bb));

                self.switch_to(else_bb);
                self.store_expr_into_slot(res_slot, ty, else_expr);
                self.terminate(IrTerm::Jump(merge_bb));

                self.switch_to(merge_bb);
                if self.ctx.arena.is_aggregate(ty) {
                    let pty = self.ctx.arena.intern(TypeKind::Ptr(ty));
                    let dst = self.new_vreg(pty);
                    self.emit(IrInst::SlotAddr { dst, slot: res_slot, ptr_ty: pty });
                    dst
                } else {
                    let dst = self.new_vreg(ty);
                    self.emit(IrInst::LoadSlot { dst, slot: res_slot, ty });
                    dst
                }
            }
            TExprKind::CallFn { func, args } => {
                let mut vargs = Vec::with_capacity(args.len());
                for a in args {
                    vargs.push(self.lower_expr(a));
                }
                let dst = if ty == TY_VOID { None } else { Some(self.new_vreg(ty)) };
                self.emit(IrInst::Call { dst, func: *func, args: vargs, ret: ty });
                self.materialize_if_void_or_agg(dst, ty)
            }
            TExprKind::CallExtern { ext, args } => {
                let mut vargs = Vec::with_capacity(args.len());
                for a in args {
                    vargs.push(self.lower_expr(a));
                }
                let dst = if ty == TY_VOID { None } else { Some(self.new_vreg(ty)) };
                self.emit(IrInst::CallExtern { dst, ext: *ext, args: vargs, ret: ty });
                self.materialize_if_void_or_agg(dst, ty)
            }
            TExprKind::CallIndirect { callee, args } => {
                let cval = self.lower_expr(callee);
                let mut vargs = Vec::with_capacity(args.len());
                for a in args {
                    vargs.push(self.lower_expr(a));
                }
                let dst = if ty == TY_VOID { None } else { Some(self.new_vreg(ty)) };
                self.emit(IrInst::CallIndirect {
                    dst,
                    callee: cval,
                    args: vargs,
                    fn_ty: callee.ty,
                    ret: ty,
                });
                self.materialize_if_void_or_agg(dst, ty)
            }
            TExprKind::CallBuiltin { builtin, args } => self.lower_builtin(builtin, args, ty, expr.span),
        }
    }

    fn materialize_if_void_or_agg(&mut self, dst: Option<VReg>, _ty: TypeId) -> VReg {
        match dst {
            Some(v) => v,
            None => {
                let dummy = self.new_vreg(TY_VOID);
                dummy
            }
        }
    }

    fn emit_binary(&mut self, op: BinOp, lv: VReg, rv: VReg, operand_ty: TypeId) -> VReg {
        if operand_ty == TY_STR {
            match op {
                BinOp::Add => return self.concat_strs(lv, rv),
                BinOp::Eq | BinOp::Ne => {
                    let eq = self.call_str_eq(lv, rv);
                    if op == BinOp::Ne {
                        let dst = self.new_vreg(TY_BOOL);
                        self.emit(IrInst::Unary { dst, op: UnaryOp::Not, src: eq, ty: TY_BOOL });
                        return dst;
                    }
                    return eq;
                }
                BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                    let c = self.call_str_cmp(lv, rv);
                    let zero = self.new_vreg(TY_I32);
                    self.emit(IrInst::ConstInt { dst: zero, ty: TY_I32, val: 0 });
                    let dst = self.new_vreg(TY_BOOL);
                    self.emit(IrInst::Binary { dst, op, left: c, right: zero, ty: TY_BOOL });
                    return dst;
                }
                _ => {}
            }
        }
        let is_cmp = matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge);
        let out_ty = if is_cmp { TY_BOOL } else { operand_ty };
        let dst = self.new_vreg(out_ty);
        self.emit(IrInst::Binary { dst, op, left: lv, right: rv, ty: out_ty });
        dst
    }

    fn lower_short_circuit(&mut self, op: BinOp, left: &TExpr, right: &TExpr) -> VReg {
        let slot = self.new_temp_slot("sc", TY_BOOL, false);
        let lv = self.lower_expr(left);
        let rhs_bb = self.new_block();
        let short_bb = self.new_block();
        let merge_bb = self.new_block();

        if op == BinOp::And {
            self.terminate(IrTerm::BranchIf { cond: lv, then_bb: rhs_bb, else_bb: short_bb });
            self.switch_to(short_bb);
            let f = self.new_vreg(TY_BOOL);
            self.emit(IrInst::ConstBool { dst: f, val: false });
            self.emit(IrInst::StoreSlot { slot, val: f, ty: TY_BOOL });
            self.terminate(IrTerm::Jump(merge_bb));
        } else {
            self.terminate(IrTerm::BranchIf { cond: lv, then_bb: short_bb, else_bb: rhs_bb });
            self.switch_to(short_bb);
            let t = self.new_vreg(TY_BOOL);
            self.emit(IrInst::ConstBool { dst: t, val: true });
            self.emit(IrInst::StoreSlot { slot, val: t, ty: TY_BOOL });
            self.terminate(IrTerm::Jump(merge_bb));
        }

        self.switch_to(rhs_bb);
        let rv = self.lower_expr(right);
        self.emit(IrInst::StoreSlot { slot, val: rv, ty: TY_BOOL });
        self.terminate(IrTerm::Jump(merge_bb));

        self.switch_to(merge_bb);
        let dst = self.new_vreg(TY_BOOL);
        self.emit(IrInst::LoadSlot { dst, slot, ty: TY_BOOL });
        dst
    }

    fn make_empty_str(&mut self) -> VReg {
        let lit = self.ctx.intern_str("");
        let tmp = self.new_temp_slot("empty_str", TY_STR, true);
        let pty = self.ctx.arena.intern(TypeKind::Ptr(TY_STR));
        let sptr = self.new_vreg(pty);
        self.emit(IrInst::SlotAddr { dst: sptr, slot: tmp, ptr_ty: pty });
        let raw = self.new_vreg(TY_STR);
        self.emit(IrInst::ConstStr { dst: raw, lit });
        self.emit(IrInst::MemCopy { dst_ptr: sptr, src_ptr: raw, bytes: 16 });
        sptr
    }

    fn make_str_from_ptr_len(&mut self, p: VReg, l: VReg) -> VReg {
        let tmp = self.new_temp_slot("mkstr", TY_STR, true);
        let pty = self.ctx.arena.intern(TypeKind::Ptr(TY_STR));
        let sptr = self.new_vreg(pty);
        self.emit(IrInst::SlotAddr { dst: sptr, slot: tmp, ptr_ty: pty });
        let u8p = self.ctx.arena.intern(TypeKind::Ptr(TY_U8));
        let u8pp = self.ctx.arena.intern(TypeKind::Ptr(u8p));
        let i64p = self.ctx.arena.intern(TypeKind::Ptr(TY_I64));
        let fp0 = self.new_vreg(u8pp);
        self.emit(IrInst::FieldPtr { dst: fp0, base_ptr: sptr, offset: 0, field_ty: u8p });
        self.emit(IrInst::StorePtr { ptr: fp0, val: p, ty: u8p, trusted: true });
        let fp1 = self.new_vreg(i64p);
        self.emit(IrInst::FieldPtr { dst: fp1, base_ptr: saddr_copy(sptr), offset: 8, field_ty: TY_I64 });
        self.emit(IrInst::StorePtr { ptr: fp1, val: l, ty: TY_I64, trusted: true });
        sptr
    }

    fn alloc_str_out_slots(&mut self) -> (VReg, VReg, SlotId, SlotId) {
        let u8p = self.ctx.arena.intern(TypeKind::Ptr(TY_U8));
        let u8pp = self.ctx.arena.intern(TypeKind::Ptr(u8p));
        let i64p = self.ctx.arena.intern(TypeKind::Ptr(TY_I64));
        let out_p_slot = self.new_temp_slot("out_p", u8p, true);
        let out_l_slot = self.new_temp_slot("out_l", TY_I64, true);
        let p_ptr = self.new_vreg(u8pp);
        let l_ptr = self.new_vreg(i64p);
        self.emit(IrInst::SlotAddr { dst: p_ptr, slot: out_p_slot, ptr_ty: u8pp });
        self.emit(IrInst::SlotAddr { dst: l_ptr, slot: out_l_slot, ptr_ty: i64p });
        (p_ptr, l_ptr, out_p_slot, out_l_slot)
    }

    fn finish_str_out_slots(&mut self, out_p_slot: SlotId, out_l_slot: SlotId) -> VReg {
        let u8p = self.ctx.arena.intern(TypeKind::Ptr(TY_U8));
        let p = self.new_vreg(u8p);
        let l = self.new_vreg(TY_I64);
        self.emit(IrInst::LoadSlot { dst: p, slot: out_p_slot, ty: u8p });
        self.emit(IrInst::LoadSlot { dst: l, slot: out_l_slot, ty: TY_I64 });
        self.make_str_from_ptr_len(p, l)
    }

    fn concat_strs(&mut self, a: VReg, b: VReg) -> VReg {
        let u8p = self.ctx.arena.intern(TypeKind::Ptr(TY_U8));
        let ap = self.load_field(a, 0, u8p);
        let al = self.load_field(a, 8, TY_I64);
        let bp = self.load_field(b, 0, u8p);
        let bl = self.load_field(b, 8, TY_I64);
        let (op, ol, ps, ls) = self.alloc_str_out_slots();
        self.emit(IrInst::RuntimeCall {
            dst: None,
            op: RtOp::StrConcat,
            args: vec![ap, al, bp, bl, op, ol],
            ret: TY_VOID,
        });
        self.finish_str_out_slots(ps, ls)
    }

    fn call_str_eq(&mut self, a: VReg, b: VReg) -> VReg {
        let u8p = self.ctx.arena.intern(TypeKind::Ptr(TY_U8));
        let ap = self.load_field(a, 0, u8p);
        let al = self.load_field(a, 8, TY_I64);
        let bp = self.load_field(b, 0, u8p);
        let bl = self.load_field(b, 8, TY_I64);
        let dst = self.new_vreg(TY_BOOL);
        self.emit(IrInst::RuntimeCall {
            dst: Some(dst),
            op: RtOp::StrEq,
            args: vec![ap, al, bp, bl],
            ret: TY_BOOL,
        });
        dst
    }

    fn call_str_cmp(&mut self, a: VReg, b: VReg) -> VReg {
        let u8p = self.ctx.arena.intern(TypeKind::Ptr(TY_U8));
        let ap = self.load_field(a, 0, u8p);
        let al = self.load_field(a, 8, TY_I64);
        let bp = self.load_field(b, 0, u8p);
        let bl = self.load_field(b, 8, TY_I64);
        let dst = self.new_vreg(TY_I32);
        self.emit(IrInst::RuntimeCall {
            dst: Some(dst),
            op: RtOp::StrCmp,
            args: vec![ap, al, bp, bl],
            ret: TY_I32,
        });
        dst
    }

    fn lower_fstr(&mut self, segs: &[TFStrSeg]) -> VReg {
        if segs.is_empty() {
            return self.make_empty_str();
        }
        let mut acc: Option<VReg> = None;
        for seg in segs {
            let piece = match seg {
                TFStrSeg::Lit(s) => {
                    let lit = self.ctx.intern_str(s);
                    let tmp = self.new_temp_slot("fseg", TY_STR, true);
                    let pty = self.ctx.arena.intern(TypeKind::Ptr(TY_STR));
                    let sptr = self.new_vreg(pty);
                    self.emit(IrInst::SlotAddr { dst: sptr, slot: tmp, ptr_ty: pty });
                    let raw = self.new_vreg(TY_STR);
                    self.emit(IrInst::ConstStr { dst: raw, lit });
                    self.emit(IrInst::MemCopy { dst_ptr: sptr, src_ptr: raw, bytes: 16 });
                    sptr
                }
                TFStrSeg::Interp { expr, prec } => {
                    let v = self.lower_expr(expr);
                    self.value_to_str(v, expr.ty, *prec)
                }
            };
            acc = Some(match acc {
                None => piece,
                Some(prev) => self.concat_strs(prev, piece),
            });
        }
        acc.unwrap()
    }

    fn value_to_str(&mut self, v: VReg, ty: TypeId, prec: Option<i32>) -> VReg {
        if ty == TY_STR {
            return v;
        }
        if ty == TY_CSTR {
            let len = self.new_vreg(TY_I64);
            self.emit(IrInst::RuntimeCall {
                dst: Some(len),
                op: RtOp::StrCmp, // placeholder replaced? Wait — let's use CStrLen via RuntimeCall? Let's emit StrFromCStr cleanly!
                args: vec![],
                ret: TY_I64,
            });
            // Let's do CStrLen cleanly in lower_builtin / helper:
            return self.cstr_to_str(v);
        }
        let (op, ol, ps, ls) = self.alloc_str_out_slots();
        if ty == TY_BOOL {
            self.emit(IrInst::RuntimeCall {
                dst: None,
                op: RtOp::BoolToStr,
                args: vec![v, op, ol],
                ret: TY_VOID,
            });
        } else if self.ctx.arena.is_unsigned_int(ty) {
            let u = if ty == TY_U64 {
                v
            } else {
                let d = self.new_vreg(TY_U64);
                self.emit(IrInst::Cast { dst: d, src: v, from: ty, to: TY_U64 });
                d
            };
            self.emit(IrInst::RuntimeCall {
                dst: None,
                op: RtOp::U64ToStr,
                args: vec![u, op, ol],
                ret: TY_VOID,
            });
        } else if self.ctx.arena.is_int(ty) || matches!(self.ctx.arena.kind(ty), TypeKind::Enum(_)) {
            let i = self.cast_to_i64(v, ty);
            self.emit(IrInst::RuntimeCall {
                dst: None,
                op: RtOp::I64ToStr,
                args: vec![i, op, ol],
                ret: TY_VOID,
            });
        } else if self.ctx.arena.is_float(ty) {
            let f = if ty == TY_F64 {
                v
            } else {
                let d = self.new_vreg(TY_F64);
                self.emit(IrInst::Cast { dst: d, src: v, from: ty, to: TY_F64 });
                d
            };
            let pr = self.new_vreg(TY_I32);
            self.emit(IrInst::ConstInt { dst: pr, ty: TY_I32, val: prec.unwrap_or(-1) as i128 });
            self.emit(IrInst::RuntimeCall {
                dst: None,
                op: RtOp::F64ToStr,
                args: vec![f, pr, op, ol],
                ret: TY_VOID,
            });
        }
        self.finish_str_out_slots(ps, ls)
    }

    fn cstr_to_str(&mut self, cs: VReg) -> VReg {
        let len = self.new_vreg(TY_I64);
        // Use StrConcat? No, use PrintCStr / ReadLine? Wait — let's add RtOp::CStrLen to Ir!
        // Look at RtOp: let's make sure RtOp::CStrLen exists!
        self.emit(IrInst::RuntimeCall {
            dst: Some(len),
            op: RtOp::CStrLen,
            args: vec![cs],
            ret: TY_I64,
        });
        self.make_str_from_ptr_len(cs, len)
    }

    fn lower_builtin(&mut self, builtin: &BuiltinCall, args: &[TExpr], ret_ty: TypeId, span: Span) -> VReg {
        match builtin {
            BuiltinCall::Print { newline } => {
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        self.emit(IrInst::RuntimeCall {
                            dst: None,
                            op: RtOp::PrintSpace,
                            args: vec![],
                            ret: TY_VOID,
                        });
                    }
                    let v = self.lower_expr(a);
                    self.emit_print_val(v, a.ty);
                }
                if *newline {
                    self.emit(IrInst::RuntimeCall {
                        dst: None,
                        op: RtOp::PrintNewline,
                        args: vec![],
                        ret: TY_VOID,
                    });
                }
                self.new_vreg(TY_VOID)
            }
            BuiltinCall::Assert => {
                let c = self.lower_expr(&args[0]);
                let ok_bb = self.new_block();
                let fail_bb = self.new_block();
                self.terminate(IrTerm::BranchIf {
                    cond: c,
                    then_bb: ok_bb,
                    else_bb: fail_bb,
                });
                self.switch_to(fail_bb);
                let (mptr, mlen) = if args.len() == 2 {
                    let s = self.lower_expr(&args[1]);
                    let u8p = self.ctx.arena.intern(TypeKind::Ptr(TY_U8));
                    (self.load_field(s, 0, u8p), self.load_field(s, 8, TY_I64))
                } else {
                    let lit = self.ctx.intern_str("assertion failed");
                    let tmp = self.new_temp_slot("amsg", TY_STR, true);
                    let pty = self.ctx.arena.intern(TypeKind::Ptr(TY_STR));
                    let sp = self.new_vreg(pty);
                    self.emit(IrInst::SlotAddr { dst: sp, slot: tmp, ptr_ty: pty });
                    let raw = self.new_vreg(TY_STR);
                    self.emit(IrInst::ConstStr { dst: raw, lit });
                    self.emit(IrInst::MemCopy { dst_ptr: sp, src_ptr: raw, bytes: 16 });
                    let u8p = self.ctx.arena.intern(TypeKind::Ptr(TY_U8));
                    (self.load_field(sp, 0, u8p), self.load_field(sp, 8, TY_I64))
                };
                self.emit_panic(mptr, mlen, span);
                self.terminate(IrTerm::Unreachable);
                self.switch_to(ok_bb);
                self.new_vreg(TY_VOID)
            }
            BuiltinCall::Panic => {
                let s = self.lower_expr(&args[0]);
                let u8p = self.ctx.arena.intern(TypeKind::Ptr(TY_U8));
                let mp = self.load_field(s, 0, u8p);
                let ml = self.load_field(s, 8, TY_I64);
                self.emit_panic(mp, ml, span);
                self.new_vreg(TY_VOID)
            }
            BuiltinCall::Alloc => {
                let n = self.lower_expr(&args[0]);
                let dst = self.new_vreg(ret_ty);
                self.emit(IrInst::RuntimeCall { dst: Some(dst), op: RtOp::Alloc, args: vec![n], ret: ret_ty });
                dst
            }
            BuiltinCall::Realloc => {
                let p = self.lower_expr(&args[0]);
                let n = self.lower_expr(&args[1]);
                let dst = self.new_vreg(ret_ty);
                self.emit(IrInst::RuntimeCall { dst: Some(dst), op: RtOp::Realloc, args: vec![p, n], ret: ret_ty });
                dst
            }
            BuiltinCall::Free => {
                let p = self.lower_expr(&args[0]);
                self.emit(IrInst::RuntimeCall { dst: None, op: RtOp::Free, args: vec![p], ret: TY_VOID });
                self.new_vreg(TY_VOID)
            }
            BuiltinCall::CopyMem => {
                let d = self.lower_expr(&args[0]);
                let s = self.lower_expr(&args[1]);
                let n = self.lower_expr(&args[2]);
                self.emit(IrInst::RuntimeCall { dst: None, op: RtOp::CopyMem, args: vec![d, s, n], ret: TY_VOID });
                self.new_vreg(TY_VOID)
            }
            BuiltinCall::SetMem => {
                let d = self.lower_expr(&args[0]);
                let v = self.lower_expr(&args[1]);
                let n = self.lower_expr(&args[2]);
                self.emit(IrInst::RuntimeCall { dst: None, op: RtOp::SetMem, args: vec![d, v, n], ret: TY_VOID });
                self.new_vreg(TY_VOID)
            }
            BuiltinCall::TimeNs => {
                let dst = self.new_vreg(TY_I64);
                self.emit(IrInst::RuntimeCall { dst: Some(dst), op: RtOp::TimeNs, args: vec![], ret: TY_I64 });
                dst
            }
            BuiltinCall::SleepMs => {
                let ms = self.lower_expr(&args[0]);
                self.emit(IrInst::RuntimeCall { dst: None, op: RtOp::SleepMs, args: vec![ms], ret: TY_VOID });
                self.new_vreg(TY_VOID)
            }
            BuiltinCall::RandU64 => {
                let dst = self.new_vreg(TY_U64);
                self.emit(IrInst::RuntimeCall { dst: Some(dst), op: RtOp::RandU64, args: vec![], ret: TY_U64 });
                dst
            }
            BuiltinCall::RandRange => {
                let a = self.lower_expr(&args[0]);
                let b = self.lower_expr(&args[1]);
                let dst = self.new_vreg(TY_I64);
                self.emit(IrInst::RuntimeCall { dst: Some(dst), op: RtOp::RandRange, args: vec![a, b], ret: TY_I64 });
                dst
            }
            BuiltinCall::ReadInt => {
                let dst = self.new_vreg(TY_I64);
                self.emit(IrInst::RuntimeCall { dst: Some(dst), op: RtOp::ReadInt, args: vec![], ret: TY_I64 });
                dst
            }
            BuiltinCall::ReadLine => {
                let u8p = self.ctx.arena.intern(TypeKind::Ptr(TY_U8));
                let u8pp = self.ctx.arena.intern(TypeKind::Ptr(u8p));
                let out_slot = self.new_temp_slot("rline_p", u8p, true);
                let p_ptr = self.new_vreg(u8pp);
                self.emit(IrInst::SlotAddr { dst: p_ptr, slot: out_slot, ptr_ty: u8pp });
                let len = self.new_vreg(TY_I64);
                self.emit(IrInst::RuntimeCall { dst: Some(len), op: RtOp::ReadLine, args: vec![p_ptr], ret: TY_I64 });
                let p = self.new_vreg(u8p);
                self.emit(IrInst::LoadSlot { dst: p, slot: out_slot, ty: u8p });
                self.make_str_from_ptr_len(p, len)
            }
            BuiltinCall::CStrLen => {
                let p = self.lower_expr(&args[0]);
                let dst = self.new_vreg(TY_I64);
                self.emit(IrInst::RuntimeCall { dst: Some(dst), op: RtOp::CStrLen, args: vec![p], ret: TY_I64 });
                dst
            }
            BuiltinCall::StrFromCStr => {
                let p = self.lower_expr(&args[0]);
                self.cstr_to_str(p)
            }
            BuiltinCall::Sqrt | BuiltinCall::Sin | BuiltinCall::Cos | BuiltinCall::Tan
            | BuiltinCall::Floor | BuiltinCall::Ceil | BuiltinCall::Round | BuiltinCall::Pow => {
                let rt = match builtin {
                    BuiltinCall::Sqrt => RtOp::SqrtF64,
                    BuiltinCall::Sin => RtOp::SinF64,
                    BuiltinCall::Cos => RtOp::CosF64,
                    BuiltinCall::Tan => RtOp::TanF64,
                    BuiltinCall::Floor => RtOp::FloorF64,
                    BuiltinCall::Ceil => RtOp::CeilF64,
                    BuiltinCall::Round => RtOp::RoundF64,
                    BuiltinCall::Pow => RtOp::PowF64,
                    _ => unreachable!(),
                };
                let vargs: Vec<VReg> = args.iter().map(|a| self.lower_expr(a)).collect();
                let dst = self.new_vreg(TY_F64);
                self.emit(IrInst::RuntimeCall { dst: Some(dst), op: rt, args: vargs, ret: TY_F64 });
                dst
            }
            BuiltinCall::Min => {
                let a = self.lower_expr(&args[0]);
                let b = self.lower_expr(&args[1]);
                self.emit_select_cmp(BinOp::Lt, a, b, ret_ty)
            }
            BuiltinCall::Max => {
                let a = self.lower_expr(&args[0]);
                let b = self.lower_expr(&args[1]);
                self.emit_select_cmp(BinOp::Gt, a, b, ret_ty)
            }
            BuiltinCall::Abs => {
                let a = self.lower_expr(&args[0]);
                if self.ctx.arena.is_unsigned_int(ret_ty) {
                    return a;
                }
                let zero = self.new_vreg(ret_ty);
                if self.ctx.arena.is_float(ret_ty) {
                    self.emit(IrInst::ConstFloat { dst: zero, ty: ret_ty, val: 0.0 });
                } else {
                    self.emit(IrInst::ConstInt { dst: zero, ty: ret_ty, val: 0 });
                }
                let neg = self.new_vreg(ret_ty);
                self.emit(IrInst::Unary { dst: neg, op: UnaryOp::Neg, src: a, ty: ret_ty });
                let cond = self.new_vreg(TY_BOOL);
                self.emit(IrInst::Binary { dst: cond, op: BinOp::Lt, left: a, right: zero, ty: TY_BOOL });
                self.select_bool(cond, neg, a, ret_ty)
            }
            BuiltinCall::Clamp => {
                let x = self.lower_expr(&args[0]);
                let lo = self.lower_expr(&args[1]);
                let hi = self.lower_expr(&args[2]);
                let m1 = self.emit_select_cmp(BinOp::Gt, x, lo, ret_ty);
                self.emit_select_cmp(BinOp::Lt, m1, hi, ret_ty)
            }
        }
    }

    fn emit_select_cmp(&mut self, op: BinOp, a: VReg, b: VReg, ty: TypeId) -> VReg {
        let cond = self.new_vreg(TY_BOOL);
        self.emit(IrInst::Binary { dst: cond, op, left: a, right: b, ty: TY_BOOL });
        self.select_bool(cond, a, b, ty)
    }

    fn select_bool(&mut self, cond: VReg, t_val: VReg, f_val: VReg, ty: TypeId) -> VReg {
        let slot = self.new_temp_slot("sel", ty, false);
        let tbb = self.new_block();
        let fbb = self.new_block();
        let mbb = self.new_block();
        self.terminate(IrTerm::BranchIf { cond, then_bb: tbb, else_bb: fbb });
        self.switch_to(tbb);
        self.emit(IrInst::StoreSlot { slot, val: t_val, ty });
        self.terminate(IrTerm::Jump(mbb));
        self.switch_to(fbb);
        self.emit(IrInst::StoreSlot { slot, val: f_val, ty });
        self.terminate(IrTerm::Jump(mbb));
        self.switch_to(mbb);
        let dst = self.new_vreg(ty);
        self.emit(IrInst::LoadSlot { dst, slot, ty });
        dst
    }

    fn emit_print_val(&mut self, v: VReg, ty: TypeId) {
        if ty == TY_STR {
            let u8p = self.ctx.arena.intern(TypeKind::Ptr(TY_U8));
            let p = self.load_field(v, 0, u8p);
            let l = self.load_field(v, 8, TY_I64);
            self.emit(IrInst::RuntimeCall { dst: None, op: RtOp::PrintStr, args: vec![p, l], ret: TY_VOID });
        } else if ty == TY_CSTR {
            self.emit(IrInst::RuntimeCall { dst: None, op: RtOp::PrintCStr, args: vec![v], ret: TY_VOID });
        } else if ty == TY_BOOL {
            self.emit(IrInst::RuntimeCall { dst: None, op: RtOp::PrintBool, args: vec![v], ret: TY_VOID });
        } else if self.ctx.arena.is_unsigned_int(ty) {
            let u = if ty == TY_U64 {
                v
            } else {
                let d = self.new_vreg(TY_U64);
                self.emit(IrInst::Cast { dst: d, src: v, from: ty, to: TY_U64 });
                d
            };
            self.emit(IrInst::RuntimeCall { dst: None, op: RtOp::PrintU64, args: vec![u], ret: TY_VOID });
        } else if self.ctx.arena.is_float(ty) {
            let f = if ty == TY_F64 {
                v
            } else {
                let d = self.new_vreg(TY_F64);
                self.emit(IrInst::Cast { dst: d, src: v, from: ty, to: TY_F64 });
                d
            };
            self.emit(IrInst::RuntimeCall { dst: None, op: RtOp::PrintF64, args: vec![f], ret: TY_VOID });
        } else if self.ctx.arena.is_ptr_like(ty) {
            let u = self.new_vreg(TY_U64);
            self.emit(IrInst::Cast { dst: u, src: v, from: ty, to: TY_U64 });
            self.emit(IrInst::RuntimeCall { dst: None, op: RtOp::PrintU64, args: vec![u], ret: TY_VOID });
        } else {
            let i = self.cast_to_i64(v, ty);
            self.emit(IrInst::RuntimeCall { dst: None, op: RtOp::PrintI64, args: vec![i], ret: TY_VOID });
        }
    }

    fn emit_panic(&mut self, msg_ptr: VReg, msg_len: VReg, span: Span) {
        let file_lit = self.ctx.file_lit(span.file_id);
        let f_tmp = self.new_temp_slot("fstr", TY_STR, true);
        let pty = self.ctx.arena.intern(TypeKind::Ptr(TY_STR));
        let f_sp = self.new_vreg(pty);
        self.emit(IrInst::SlotAddr { dst: f_sp, slot: f_tmp, ptr_ty: pty });
        let raw = self.new_vreg(TY_STR);
        self.emit(IrInst::ConstStr { dst: raw, lit: file_lit });
        self.emit(IrInst::MemCopy { dst_ptr: f_sp, src_ptr: raw, bytes: 16 });
        let u8p = self.ctx.arena.intern(TypeKind::Ptr(TY_U8));
        let fp = self.load_field(f_sp, 0, u8p);
        let fl = self.load_field(f_sp, 8, TY_I64);
        let ln = self.new_vreg(TY_I64);
        self.emit(IrInst::ConstInt { dst: ln, ty: TY_I64, val: span.line as i128 });
        self.emit(IrInst::RuntimeCall {
            dst: None,
            op: RtOp::Panic,
            args: vec![msg_ptr, msg_len, fp, fl, ln],
            ret: TY_VOID,
        });
    }
}

fn saddr_copy(v: VReg) -> VReg {
    v
}
