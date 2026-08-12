//! `FuncBuilder` calls method group: the term-lowering dispatchers
//! (`lower_terms`, `lower_term`, `lower_self_tail_combinator`, `lower_call`).

use super::*;

impl<'a> FuncBuilder<'a> {
    pub(in crate::ir) fn lower_terms(&mut self, terms: &[Term], tail: bool) {
        // Only the final term of a body can be in tail position (R1); a term
        // followed by any further term is not. This positional `tail` threading
        // is the same syntactic rule as the checker's `tail_position_calls`
        // (src/check.rs); the two must stay in lockstep if the rule changes.
        let last = terms.len().wrapping_sub(1);
        for (i, term) in terms.iter().enumerate() {
            self.lower_term(term, tail && i == last);
        }
    }

    pub(in crate::ir) fn lower_term(&mut self, term: &Term, tail: bool) {
        match &term.kind {
            TermKind::IntLit(n) => {
                let v = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(v, *n));
                self.const_vals.insert(v, *n);
                self.stack.push(v);
            }
            TermKind::FloatLit(x) => {
                let v = self.fresh_value(IrType::Float { bits: 64 });
                self.push_instr(Instr::ConstF(v, *x));
                self.stack.push(v);
            }
            TermKind::StrLit(s) => {
                let v = self.fresh_value(IrType::Str);
                self.push_instr(Instr::StrLit(v, s.clone()));
                self.stack.push(v);
            }
            TermKind::Call(name) => self.lower_call(name, term.span, tail),
            TermKind::Bind(names) => {
                // R10: a binding is a compile-time rebinding of SSA values, so
                // it emits nothing. Leftmost name takes the deepest value.
                let bound = self.stack.split_off(self.stack.len() - names.len());
                for (name, value) in names.iter().zip(bound) {
                    self.locals.push((name.clone(), value));
                }
            }
            TermKind::If {
                then_branch,
                else_branch,
                ..
            } => self.lower_if(then_branch, else_branch, tail),
            // R12: a quotation literal interns its body and lowers to a phantom
            // `Value` with a placeholder `IrType` and *no* `Instr`. The checker
            // guarantees this phantom reaches only `call`/`times`/shuffle/bind
            // or a materialization boundary (a store, a word output, or a
            // branch join, R11) -- where it is turned into a real `(code, env)`
            // aggregate *before* it enters a `Phi`, operand, terminator, or
            // runtime code value; it never enters one as a bare phantom. `I64`
            // is the plainest non-aggregate placeholder (the IR side has no
            // `if`-condition concern, so the checker's `Cstr` choice does not
            // bind here).
            TermKind::Quotation(body) => {
                let id = QuotId(self.quot_defs.len());
                self.quot_defs.push(body.clone());
                let v = self.fresh_value(IrType::I64);
                self.quot_bodies.insert(v, id);
                self.stack.push(v);
            }
            // Slice 6h (D3): one `Instr::Alloc` for the array's inline
            // aggregate, then a byte-granular zero-init loop of exactly
            // `ArrayLayout::size` bytes. Byte granularity (not a word store)
            // because an array is not word-padded, so a wider store would
            // overrun the allocation; the runtime cost is a deliberate trade
            // for one obviously-correct path (code size stays O(1) in Count).
            TermKind::ArrayCtor(ty) => {
                let Type::Array(id, _) = *ty else {
                    unreachable!("the parser only ever interns a Type::Array for an ArrayCtor term")
                };
                // Save/restore the enclosing loop state so this loop composes
                // with a later `Alloc` or loop (see `save_loop_state`).
                let saved_loop_state = self.save_loop_state();
                let size = self.arrays.layouts[id.index()].size;
                // The destination reaches the loop body and the exit block by
                // dominance; it must not be carried, or the back-edge staging
                // blit would copy a stale snapshot over each iteration's store.
                let dst = self.alloc_array(id);

                let seed = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(seed, 0));
                self.const_vals.insert(seed, 0);
                // Carry only the induction index, with `stage_aggregates =
                // false`.
                let outs = self.begin_loop(&[seed], false);
                let index_phi = outs[0];

                // Header (current after `begin_loop`): loop while index < size.
                let bound = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(bound, size as i64));
                self.const_vals.insert(bound, size as i64);
                let cmp = self.fresh_value(IrType::Bool);
                self.push_instr(Instr::Cmp(cmp, CmpOp::Lt, index_phi, bound));
                let body_block = self.fresh_block();
                let exit_block = self.fresh_block();
                self.seal_block(Terminator::Jnz(cmp, body_block, exit_block));

                // Body: store one zero byte at `dst + index`. `ElemAddr` with
                // `stride = 1` gives byte addressing off the runtime index; a
                // `FieldStore` of an 8-bit-typed `Const` picks `storeb`.
                self.start_block(body_block);
                self.terminated = false;
                let fptr = self.elem_addr(dst, index_phi, 1);
                let zero = self.fresh_value(IrType::Int {
                    bits: 8,
                    signed: false,
                });
                self.push_instr(Instr::Const(zero, 0));
                self.push_instr(Instr::FieldStore(fptr, zero));

                // Back-edge: index + 1.
                let one = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(one, 1));
                self.const_vals.insert(one, 1);
                let index_next = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Bin(index_next, BinOp::Add, index_phi, one));
                self.back_edges.push((self.cur_id, vec![index_next]));
                self.seal_block(Terminator::Jmp(
                    self.header.expect("array ctor loop header"),
                ));

                self.finalize_loop();

                // Exit: the array is the constructor's result. The fixed loop
                // body never terminates, so `terminated` is already false here
                // (unlike the `times` arm, whose spliced user body can); the
                // reset keeps this arm identical to the loop template so it
                // stays correct if the body ever gains a terminating term.
                self.start_block(exit_block);
                self.terminated = false;
                self.stack.push(dst);

                self.restore_loop_state(saved_loop_state);
            }
        }
    }

    /// R10: lower a self-tail combinator (`while`) as a splice-time loop,
    /// composing the `times` arm's mid-body loop opening with the whole-word
    /// transform's self-call-driven back-edge. The body's leading quotation
    /// binding(s) are lowered *before* `begin_loop`, so the loop-invariant
    /// `Copy` quotation phantom is bound to a local (resolved statically each
    /// iteration) and excluded from the loop-carried phis; only the state row
    /// is carried. `stage_aggregates = true` reuses the slice-3 aggregate
    /// staging verbatim for a carried-aggregate state. The enclosing loop
    /// state is saved and restored so loops compose, exactly as the `times`
    /// arm does. A tail-position self-call inside the body is emitted as a
    /// back-edge (`lower_call`, keyed on `cur_combinator`), never a re-splice.
    pub(in crate::ir) fn lower_self_tail_combinator(&mut self, name: &str, body: &[Term]) {
        let saved_loop_state = self.save_loop_state();
        let saved_combinator = self.cur_combinator.take();
        let locals_depth = self.locals.len();

        // Lower the leading quotation binding(s) before opening the loop: a
        // `Bind` term that pops any quotation phantom becomes a local, so the
        // phantom is not carried in a phi (R10). Everything after is the loop
        // body.
        let mut split = 0;
        while split < body.len() {
            if let TermKind::Bind(names) = &body[split].kind {
                let base = self.stack.len() - names.len();
                let binds_quot = self.stack[base..]
                    .iter()
                    .any(|v| self.quot_bodies.contains_key(v));
                if binds_quot {
                    self.lower_term(&body[split], false);
                    split += 1;
                    continue;
                }
            }
            break;
        }

        // The carried row is whatever remains: the caller's residual plus the
        // threaded state. `begin_loop` seals the entry block, opens the
        // header, and returns one value per carried slot (a scalar phi output
        // or an aggregate stable slot).
        let row_len = self.stack.len();
        let params = mem::take(&mut self.stack);
        let outs = self.begin_loop(&params, true);
        self.stack = outs;
        self.cur_combinator = Some((name.to_string(), row_len));

        // Lower the loop body with `tail = true`, so its tail-position
        // self-call is recognized as the back-edge. The base-case arm falls
        // through, leaving the state on the stack as the loop's result.
        self.lower_terms(&body[split..], true);
        self.finalize_loop();

        self.locals.truncate(locals_depth);
        self.cur_combinator = saved_combinator;
        self.restore_loop_state(saved_loop_state);
    }

    pub(in crate::ir) fn lower_call(&mut self, name: &str, span: Span, tail: bool) {
        let line = span.line;
        if let Some(&(_, value)) = self.locals.iter().find(|(n, _)| n == name) {
            self.stack.push(value); // i64 is Copy; reuse the value id.
            return;
        }
        // Slice 8a phase 4 (R7): a call site the checker resolved to a user
        // overload of a builtin-named word must dispatch to that word, not
        // the name-directed builtin arms below (a literal name match like
        // "+" would otherwise always win) nor the self-tail/back-edge checks
        // further down (a word named `.` overloading print must not be
        // miscategorized as a self-tail call on `.`). Same env-lookup +
        // resolve + bundle-unpack shape as the ordinary user-word path in the
        // `_` arm below, since this *is* that path, reached early.
        if let Some(sym_name) = self.builtin_overloads.get(&span).cloned() {
            let (in_arity, out_arity, ret_ty) = *self
                .env
                .get(&sym_name)
                .expect("checked user overload exists");
            let split = self.stack.len() - in_arity;
            let args = self.stack.split_off(split);
            let bundle = match ret_ty {
                Some(IrType::Struct(id)) if self.structs.layouts[id.index()].bundle => Some(id),
                _ => None,
            };
            let ret = if out_arity == 1 || bundle.is_some() {
                Some(self.fresh_value(ret_ty.unwrap_or(IrType::I64)))
            } else {
                None
            };
            let sym = (self.resolve)(&sym_name);
            self.push_instr(Instr::Call(ret, sym, args));
            if let Some(v) = ret {
                self.stack.push(v);
            }
            if let Some(id) = bundle {
                self.unpack_bundle(id);
            }
            return;
        }
        // R10: a tail-position self-call inside a self-tail combinator splice
        // is the loop back-edge. The loop carries only the state row (length
        // recorded when the loop opened); the quotation argument(s) above it
        // are loop-invariant, resolved statically through the local binding,
        // so they are dropped here and not fed to a phi. Intercepted before
        // the combinator dispatch below (which would re-splice) and distinct
        // from the whole-word self-tail back-edge (keyed on `cur_word_name`).
        if tail && self.header.is_some() {
            if let Some((cname, row_len)) = self.cur_combinator.clone() {
                if cname == name {
                    let drop_n = self.stack.len() - row_len;
                    self.stack.truncate(self.stack.len() - drop_n);
                    let args = mem::take(&mut self.stack);
                    self.back_edges.push((self.cur_id, args));
                    let header = self.header.expect("combinator loop header");
                    self.seal_block(Terminator::Jmp(header));
                    self.terminated = true;
                    return;
                }
            }
        }
        // R14/R11: a call to a polymorphic word resolves entirely through the
        // instantiation table keyed by this call site's span, never the
        // name-keyed `env`/`resolve` (which cannot distinguish one θ from
        // another). This is checked before the builtin/user dispatch below
        // because a polymorphic callee is always a user word whose name is
        // none of the builtins.
        if let Some(inst) = self.instantiations.get(&span).cloned() {
            self.lower_poly_call(&inst);
            return;
        }
        match name {
            // R13: `call`-of-literal fusion. Pop the phantom quotation `Value`,
            // resolve its body, and lower the body's terms in place, emitting
            // no `Instr::Call` and creating no runtime code value: `[ 1 + ]
            // call` lowers exactly as `1 +` (D5). `tail = false` is
            // load-bearing: the checker never sanctions a spliced term as a
            // self-tail call (R6/R13), so lowering must not back-edge here.
            "call" => {
                let v = self.stack.pop().expect("call: quotation on stack");
                // R10/D1: provenance decides. A phantom the checker resolved to
                // a literal splices its body (D5, no `Instr::Call`); a
                // materialized value whose identity was erased loads its `code`
                // slot and calls indirectly.
                match self.quot_bodies.get(&v).copied() {
                    Some(id) => {
                        let body = self.quot_defs[id.0].clone();
                        // The body is a block: a name it binds is out of scope
                        // after the splice, and the front-first local resolver
                        // would else read a stale entry on a later same-named
                        // bind. Mirror the `if` arm's save-and-truncate.
                        let locals_depth = self.locals.len();
                        self.lower_terms(&body, false);
                        self.locals.truncate(locals_depth);
                    }
                    None => self.lower_indirect_call(v),
                }
            }
            // R14: `times` lowers into a constant-stack loop, reusing
            // `begin_loop`/`finalize_loop` (D6). A synthesized index drives a
            // header `Jnz(index < count)`; the body reads the index as its top
            // input and returns the row on the back-edge (R18). `tail = false`
            // for the same reason as `call`.
            "times" => {
                // 6d/R5: a nested `times` is now legal (the checker's R18
                // rejection retired), so no `debug_assert` on `header.is_none()`
                // here; the hoist-target split (R1-R3) keeps it constant-stack.
                // R15: save the loop state (see `save_loop_state`'s doc) and
                // restore it after the loop, so a nested `times` composes and a
                // later `Alloc` in the same word does not hoist into this now-
                // dead `times` preheader.
                let saved_loop_state = self.save_loop_state();

                let qv = self.stack.pop().expect("times: quotation on stack");
                // R10/D1/D6: provenance decides, exactly as `call`. A phantom
                // the checker resolved to a literal splices its body per
                // iteration; a materialized value whose identity was erased is
                // indirect-called once per iteration (still constant stack).
                let quot_id = self.quot_bodies.get(&qv).copied();
                let count = self.stack.pop().expect("times: count on stack");

                // Synthesize the induction variable seeded 0; the row is the
                // remaining stack. `stage_aggregates = true` (R17): a carried
                // aggregate rides slice 3's entry-hoisted stable slot, and the
                // index gets a scalar phi.
                let seed = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(seed, 0));
                self.const_vals.insert(seed, 0);
                let mut params = mem::take(&mut self.stack);
                params.push(seed);
                let outs = self.begin_loop(&params, true);
                let index_phi = *outs.last().expect("times: index phi");
                let row_phis: Vec<Value> = outs[..outs.len() - 1].to_vec();

                // Header (current after `begin_loop`): loop while index < count.
                let cmp = self.fresh_value(IrType::Bool);
                self.push_instr(Instr::Cmp(cmp, CmpOp::Lt, index_phi, count));
                let body_block = self.fresh_block();
                let exit_block = self.fresh_block();
                self.seal_block(Terminator::Jnz(cmp, body_block, exit_block));

                // Body: the row plus the index (top input), spliced `tail =
                // false`. `alloca_home` stays `Some` across the splice, so an
                // aggregate the body constructs hoists its `Alloc` into the
                // invariant alloca home (R17/6d), not the per-iteration body
                // block, at any nesting depth.
                self.start_block(body_block);
                self.terminated = false;
                self.stack = row_phis;
                self.stack.push(index_phi);
                let locals_depth = self.locals.len();
                match quot_id {
                    Some(id) => {
                        let body = self.quot_defs[id.0].clone();
                        self.lower_terms(&body, false);
                    }
                    None => self.lower_indirect_call(qv),
                }
                self.locals.truncate(locals_depth);

                // Back-edge: the body's result row plus index + 1.
                let one = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Const(one, 1));
                self.const_vals.insert(one, 1);
                let index_next = self.fresh_value(IrType::I64);
                self.push_instr(Instr::Bin(index_next, BinOp::Add, index_phi, one));
                // With `tail = false` and no `Return` in a body, nothing can
                // terminate the body block, so a double seal is impossible.
                debug_assert!(
                    !self.terminated,
                    "a `tail = false` `times` body cannot terminate"
                );
                let mut args = mem::take(&mut self.stack);
                args.push(index_next);
                self.back_edges.push((self.cur_id, args));
                self.seal_block(Terminator::Jmp(self.header.expect("times loop header")));

                // Back-patch the scalar phis (row scalars + index) and append
                // the aggregate staging blits on the back-edge (unchanged from
                // slice 3).
                self.finalize_loop();

                // Exit: the carried row (scalar header-phi outputs / aggregate
                // stable slots), minus the trailing index. Reset `terminated`
                // (the body seal set it) or every term after the `times` is
                // silently dropped.
                self.start_block(exit_block);
                self.terminated = false;
                let mut exit_stack = outs;
                exit_stack.pop();
                self.stack = exit_stack;

                // R15: restore the pre-`times` loop state so the `times`
                // composes with a later `Alloc` or a second sequential `times`.
                self.restore_loop_state(saved_loop_state);
            }
            "dup" => {
                let top = *self.stack.last().expect("dup: non-empty stack");
                // A scalar is `Copy`: reuse the value id (dup emits nothing). A
                // struct or enum is copied by value: alloc a fresh slot and
                // blit the bytes, so mutating the copy leaves the original
                // intact (an enum is all-Copy too, D3).
                match self.value_type(top) {
                    IrType::Struct(id) => {
                        let copy = self.alloc_struct(id);
                        let size = self.structs.layouts[id.index()].size;
                        if size > 0 {
                            self.push_instr(Instr::Blit(top, copy, size));
                        }
                        self.stack.push(copy);
                    }
                    // Slice 9 (R1): a zero-payload enum's value is a bare
                    // scalar -- `Copy` like any other, reuse the value id.
                    IrType::Enum(id) if self.enums.layouts[id.index()].is_scalar => {
                        self.stack.push(top);
                    }
                    IrType::Enum(id) => {
                        let copy = self.alloc_enum(id);
                        let size = self.enums.layouts[id.index()].size;
                        if size > 0 {
                            self.push_instr(Instr::Blit(top, copy, size));
                        }
                        self.stack.push(copy);
                    }
                    IrType::Array(id) => {
                        let copy = self.alloc_array(id);
                        let size = self.arrays.layouts[id.index()].size;
                        if size > 0 {
                            self.push_instr(Instr::Blit(top, copy, size));
                        }
                        self.stack.push(copy);
                    }
                    _ => self.stack.push(top),
                }
            }
            "drop" => {
                let v = self.stack.pop().expect("drop: non-empty stack");
                self.emit_drop(v);
            }
            "swap" => {
                let n = self.stack.len();
                self.stack.swap(n - 1, n - 2);
            }
            "over" => {
                let below = self.stack[self.stack.len() - 2];
                self.stack.push(below);
            }
            "rot" => {
                // a b c -> b c a
                let n = self.stack.len();
                let a = self.stack[n - 3];
                self.stack[n - 3] = self.stack[n - 2];
                self.stack[n - 2] = self.stack[n - 1];
                self.stack[n - 1] = a;
            }
            "+" | "-" | "*" | "/" | "mod" | "and" | "or" | "xor" | "shl" | "shr" => {
                let op = match name {
                    "+" => BinOp::Add,
                    "-" => BinOp::Sub,
                    "*" => BinOp::Mul,
                    "/" => BinOp::Div,
                    "mod" => BinOp::Rem,
                    "and" => BinOp::And,
                    "or" => BinOp::Or,
                    "xor" => BinOp::Xor,
                    "shl" => BinOp::Shl,
                    _ => BinOp::Shr,
                };
                let rhs = self.stack.pop().expect("bin: rhs");
                let lhs = self.stack.pop().expect("bin: lhs");
                // Arithmetic/bitwise ops are homogeneous in their result
                // (checker-guaranteed): the result carries the lhs's type, so
                // the backend picks its width. `shl`/`shr`'s rhs is always an
                // `i64` count, not the lhs's type.
                let ty = self.value_type(lhs);
                let v = self.fresh_value(ty);
                self.push_instr(Instr::Bin(v, op, lhs, rhs));
                self.stack.push(v);
            }
            "not" => {
                // No unary QBE op: `not` is `xor operand, mask`. On an integer,
                // complement is `xor operand, -1` at the operand's own width
                // (`-1` is all-ones at any width in two's complement, so it
                // works whether the register is `w` or `l`). On a `bool`,
                // `not` is logical negation of a canonical 0/1 value, which
                // flips only the low bit (`xor operand, 1`); `xor -1` would
                // give -1/-2, not 0/1.
                let operand = self.stack.pop().expect("not: operand");
                let ty = self.value_type(operand);
                // `not`'s builtin row is `int_types() + Bool` only (checker-
                // guaranteed, R4): the operand is never anything else, so
                // "not an int" identifies the `Bool` case exactly, whatever
                // `Bool`'s own `IrType` migration makes it (Slice 9: it is
                // `IrType::Enum(BOOL_ENUM_ID)`, not the retired `IrType::Bool`).
                let mask: i64 = if matches!(ty, IrType::Int { .. }) {
                    -1
                } else {
                    1
                };
                let mask_v = self.fresh_value(ty);
                self.push_instr(Instr::Const(mask_v, mask));
                let v = self.fresh_value(ty);
                self.push_instr(Instr::Bin(v, BinOp::Xor, operand, mask_v));
                self.stack.push(v);
            }
            "=" | "<" | ">" | "<=" | ">=" | "<>" => {
                let op = match name {
                    "=" => CmpOp::Eq,
                    "<" => CmpOp::Lt,
                    ">" => CmpOp::Gt,
                    "<=" => CmpOp::Le,
                    ">=" => CmpOp::Ge,
                    _ => CmpOp::Ne,
                };
                let rhs = self.stack.pop().expect("cmp: rhs");
                let lhs = self.stack.pop().expect("cmp: lhs");
                let v = self.fresh_value(IrType::Bool);
                self.push_instr(Instr::Cmp(v, op, lhs, rhs));
                self.stack.push(v);
            }
            // R12 (S6): `max` over the integer tower, inline compare-and-select
            // (`Cmp(Gt)` plus a two-block phi-join), no `Instr::Call`, no
            // monomorphization.
            "max" => {
                let rhs = self.stack.pop().expect("max: rhs");
                let lhs = self.stack.pop().expect("max: lhs");
                let cmp = self.fresh_value(IrType::Bool);
                self.push_instr(Instr::Cmp(cmp, CmpOp::Gt, lhs, rhs));
                let v = self.emit_select(cmp, |_| lhs, |_| rhs);
                self.stack.push(v);
            }
            // R13 (S6): `max-total` over `f32`/`f64`, ordered by the
            // `total_cmp` bit-pattern rule (map each operand's IEEE bits to a
            // monotone unsigned key — flip every bit if the sign bit is set,
            // else flip only the sign bit — then integer-compare the keys),
            // so no float `>` is ever emitted.
            "max-total" => {
                let rhs = self.stack.pop().expect("max-total: rhs");
                let lhs = self.stack.pop().expect("max-total: lhs");
                let bits: u8 = match self.value_type(lhs) {
                    IrType::Float { bits } => bits,
                    other => unreachable!("checked: max-total operand is a float, got {other:?}"),
                };
                let lhs_key = self.total_order_key(lhs, bits);
                let rhs_key = self.total_order_key(rhs, bits);
                let cmp = self.fresh_value(IrType::Bool);
                self.push_instr(Instr::Cmp(cmp, CmpOp::Gt, lhs_key, rhs_key));
                let v = self.emit_select(cmp, |_| lhs, |_| rhs);
                self.stack.push(v);
            }
            "." => {
                let v = self.stack.pop().expect("print: value");
                self.push_instr(Instr::Print(v));
            }
            "fill" => self.lower_array_word(name),
            "len" => {
                let top = *self.stack.last().expect("len: operand");
                if self.value_type(top) == IrType::Str {
                    // R8: consuming, unlike the array `len` fold: the
                    // length is carried at runtime, not derivable from the
                    // type.
                    self.stack.pop();
                    let v = self.fresh_value(IrType::Usize);
                    self.push_instr(Instr::StrLen(v, top));
                    self.stack.push(v);
                } else {
                    self.lower_array_word(name);
                }
            }
            "cstr" => {
                // R7: discard the length, keep the bytes pointer.
                let s = self.stack.pop().expect("cstr: str operand");
                let v = self.fresh_value(IrType::Cstr);
                self.push_instr(Instr::StrPtr(v, s));
                self.stack.push(v);
            }
            "^" | "^>" | "^|>" => self.lower_owned_cell_word(name),
            "@" | "!" | "+!" => self.lower_access_word(name),
            _ => {
                // R19: a call to a monomorphic combinator is inlined, not
                // lowered to an `Instr::Call` -- the callee mints no `IrFunc`
                // (R20), so its only reachable form is this splice. The
                // caller's quotation literals sit on `self.stack` as phantom
                // `Value`s already (a `TermKind::Quotation` earlier in this
                // body recorded each `Value -> QuotId`), so the spliced body's
                // own `call`/`times` resolves them with no extra plumbing.
                // `tail = false` and the locals-truncate mirror the `call`
                // splice above. Checked before the `&`/conversion/struct
                // dispatch since a combinator name is an ordinary word name.
                if let Some(body) = self.combinators.get(name) {
                    // R10: a self-tail combinator lowers to a splice-time loop
                    // (back-edge, not re-splice); every other combinator is a
                    // straight term-splice.
                    let self_tail = body_tail_calls_self(body, name);
                    // R18/R21: alpha-rename the callee body identically to the
                    // checker, so its `| ... |` locals are fresh and a
                    // passed-down literal keeps its lexical capture under
                    // transitive inlining.
                    let uid = self.inline_uid;
                    self.inline_uid += 1;
                    let body = crate::ast::alpha_rename_locals(body, uid);
                    if self_tail {
                        self.lower_self_tail_combinator(name, &body);
                    } else {
                        let locals_depth = self.locals.len();
                        self.lower_terms(&body, false);
                        self.locals.truncate(locals_depth);
                    }
                    return;
                }
                // Every `&`-led word: the two prefix borrow operators and the
                // reference-mode accessor family.
                if name.starts_with('&') {
                    self.lower_reference_word(name, line);
                    return;
                }
                // A conversion word `>iN`/`>uN`/`>f32`/`>f64`
                // (checker-guaranteed numeric source): pop one, push the
                // target-typed result. The backend reads the two `IrType`s to
                // pick the int/float conversion op (R18).
                if let Some(target) = name
                    .strip_prefix('>')
                    .filter(|r| !r.is_empty())
                    .and_then(Type::from_name)
                    .filter(Type::is_numeric)
                {
                    let src = self.stack.pop().expect("conv: source");
                    let dst = self.fresh_value(ir_type_of(target));
                    self.push_instr(Instr::Conv(dst, src));
                    self.stack.push(dst);
                    return;
                }
                // A generated struct word (`S`/`S>`/`S>fi`/`S<fi`/`S|>fi`) lowers to
                // alloc/blit/field-load-store inline, not a normal call.
                if let Some(&sw) = self.structs.words.get(name) {
                    self.lower_struct_word(sw);
                    return;
                }
                // A variant constructor lowers to alloc + tag store + field
                // stores inline, parallel to a struct constructor (R14/R15).
                if let Some(&ew) = self.enums.words.get(name) {
                    self.lower_enum_word(ew);
                    return;
                }
                // R7: a tail-position self-call is a back-edge to the loop
                // header, not a real call. `self.header` is `Some` iff the word
                // is self-tail-recursive (R6), and `tail` marks the syntactic
                // tail position (R1); a non-tail self-call (R10) falls through
                // to the ordinary `Instr::Call` below. Pop the args as the
                // back-edge phi operands (one per carried slot; a self-call's
                // input arity is the word's own signature, so the count always
                // matches the header phi count) and jump.
                //
                // R11: the back-edge is the defined destructor insertion point
                // for this iteration's non-forwarded affine values; in Phase 2
                // every type is `Copy`, so the drop set is empty and no drop
                // glue is emitted here.
                if tail && self.header.is_some() && name == self.cur_word_name {
                    let (in_arity, ..) = *self.env.get(name).expect("checked user word exists");
                    let split = self.stack.len() - in_arity;
                    let args = self.stack.split_off(split);
                    self.back_edges.push((self.cur_id, args));
                    self.seal_block(Terminator::Jmp(self.header.expect("loop header")));
                    self.terminated = true;
                    return;
                }
                let (in_arity, out_arity, ret_ty) =
                    *self.env.get(name).expect("checked user word exists");
                let split = self.stack.len() - in_arity;
                let args = self.stack.split_off(split);
                // R11: a multi-output callee returns one bundle, unpacked back
                // onto the stack below, so the lowering stack matches the
                // stack the checker verified. The discriminator is the
                // bundle's own flag, not `out_arity >= 2`: the REPL's env
                // derives a multi-output `ret_ty` from the first output alone
                // and interns no bundle, and must not enter this branch.
                let bundle = match ret_ty {
                    Some(IrType::Struct(id)) if self.structs.layouts[id.index()].bundle => Some(id),
                    _ => None,
                };
                let ret = if out_arity == 1 || bundle.is_some() {
                    Some(self.fresh_value(ret_ty.unwrap_or(IrType::I64)))
                } else {
                    None
                };
                let sym = (self.resolve)(name);
                self.push_instr(Instr::Call(ret, sym, args));
                if let Some(v) = ret {
                    self.stack.push(v);
                }
                if let Some(id) = bundle {
                    self.unpack_bundle(id);
                }
            }
        }
    }
}
