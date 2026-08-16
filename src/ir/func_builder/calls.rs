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
            // R12: a quotation literal interns its body and lowers to a phantom
            // `Value` with a placeholder `IrType` and *no* `Instr`. The checker
            // guarantees this phantom reaches only `call`/shuffle/bind
            // or a materialization boundary (a store, a word output, or a
            // branch join, R11) -- where it is turned into a real `(code, env)`
            // aggregate *before* it enters a `Phi`, operand, terminator, or
            // runtime code value; it never enters one as a bare phantom. `I64`
            // is the plainest non-aggregate placeholder (the IR side has no
            // `if`-condition concern, so the checker's `Cstr` choice does not
            // bind here).
            TermKind::Quotation(body, _, _) => {
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
                // (unlike a self-tail combinator's spliced body, whose user
                // code can terminate); the reset keeps this arm identical to
                // the loop template so it stays correct if the body ever
                // gains a terminating term.
                self.start_block(exit_block);
                self.terminated = false;
                self.stack.push(dst);

                self.restore_loop_state(saved_loop_state);
            }
        }
    }

    /// R10: lower a self-tail combinator (`while`, `times-helper`) as a
    /// splice-time loop: a `begin_loop`-opened header composed with the
    /// whole-word transform's self-call-driven back-edge. The body's leading
    /// quotation binding(s) are lowered *before* `begin_loop`, so the
    /// loop-invariant `Copy` quotation phantom is bound to a local (resolved
    /// statically each iteration) and excluded from the loop-carried phis;
    /// only the state row is carried. `stage_aggregates = true` reuses the
    /// slice-3 aggregate staging verbatim for a carried-aggregate state. The
    /// enclosing loop state is saved and restored so loops compose (see
    /// `save_loop_state`). A tail-position self-call inside the body is
    /// emitted as a back-edge (`lower_call`, keyed on `cur_combinator`),
    /// never a re-splice.
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

        // A quotation phantom the body never bound to a name (`dup call ...`
        // rather than `| p | p call ...`) is loop-invariant for the same
        // reason the bound one is: the parameter threads through the back-edge
        // unchanged, so every iteration sees the same literal. It must stay out
        // of the carried row -- `is_aggregate` is true for `IrType::Quotation`,
        // so `begin_loop` would stage it as an aggregate and blit from a
        // phantom that owns no bytes, leaving a pointer where the splice
        // machinery expects a phantom (the `call` below then finds no
        // `quot_bodies` entry and reaches `lower_indirect_call` with a
        // non-quotation type). Splitting only a contiguous top run keeps the
        // invariant-args-above-the-row shape `lower_call`'s back-edge assumes.
        let mut invariant_top = 0;
        while invariant_top < self.stack.len() {
            let v = self.stack[self.stack.len() - 1 - invariant_top];
            if !self.quot_bodies.contains_key(&v) {
                break;
            }
            invariant_top += 1;
        }
        let invariant = self.stack.split_off(self.stack.len() - invariant_top);

        // The carried row is whatever remains: the caller's residual plus the
        // threaded state. `begin_loop` seals the entry block, opens the
        // header, and returns one value per carried slot (a scalar phi output
        // or an aggregate stable slot).
        let row_len = self.stack.len();
        let params = mem::take(&mut self.stack);
        let outs = self.begin_loop(&params, true);
        self.stack = outs;
        self.stack.extend(invariant);
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
            // D7/R5: two generic-type instantiations sharing a bare surface
            // name (`Box`'s constructor/accessor) resolve here too -- the
            // checker records the *mangled* per-instantiation symbol
            // (`Box[i64]>val`) once its operand-type match picks a candidate
            // (`src/check/terms.rs`'s multi-candidate arm), and that mangled
            // spelling is exactly the key `structs.words`/`enums.words` still
            // carry (`ir/layout.rs`). A generated struct/enum word is never a
            // real `Instr::Call` (it inlines to alloc/blit/field ops), so it
            // must be tried before the ordinary-user-overload path below,
            // which would otherwise `expect` a `self.env` entry no struct or
            // enum word ever registers.
            if let Some(&sw) = self.structs.words.get(&sym_name) {
                self.lower_struct_word(sw);
                return;
            }
            if let Some(&ew) = self.enums.words.get(&sym_name) {
                self.lower_enum_word(ew);
                return;
            }
            let (in_arity, out_arity, ret_ty) = {
                let a = self
                    .env
                    .get(&sym_name)
                    .expect("checked user overload exists");
                (a.in_arity, a.out_arity, a.ret_ty)
            };
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
            // call` lowers exactly as `1 +` (D5). Slice 10c (R-P1-6): the
            // caller's `tail` is threaded, not pinned `false` -- the splice runs
            // in place of the `call`, so at a tail `call` the literal's own tail
            // terms are the enclosing word's, and the whole-word back-edge below
            // fires on a self-call there. `check_term`'s `"call"` arm threads the
            // same flag, so the checker sanctions exactly the splices that
            // back-edge here.
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
                        self.lower_terms(&body, tail);
                        self.locals.truncate(locals_depth);
                    }
                    None => self.lower_indirect_call(v),
                }
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
            // Slice 10c (R-P3-3): the comparison primitives. Same
            // `Instr::Cmp`, same operands, same operand-type-driven signed /
            // unsigned / float dispatch at the backend as the `=`/`<`/... rows
            // they replace; only the result's `IrType` changed, from the
            // 32-bit `Bool` to the 32-bit `u32` flag `branch` consumes, which
            // is the same `w` register. `=`/`<`/... are `lib/` words that wrap
            // these and construct a `bool`.
            _ if crate::check::COMPARISON_PRIMITIVES
                .iter()
                .any(|(n, _)| *n == name) =>
            {
                let op = crate::check::COMPARISON_PRIMITIVES
                    .iter()
                    .find(|(n, _)| *n == name)
                    .expect("guarded by the arm's own predicate")
                    .1;
                let rhs = self.stack.pop().expect("cmp: rhs");
                let lhs = self.stack.pop().expect("cmp: lhs");
                let v = self.fresh_value(IrType::Int {
                    bits: 32,
                    signed: false,
                });
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
            // Slice 10c (R-P3-1): `branch` is the machine-level two-way
            // conditional. Its arms arrive as quotation operands rather than
            // embedded term lists, but once resolved the lowering is the
            // jump-and-join `if` always had: a conditional jump on the flag,
            // each arm in its own block, one join. Both operands are declared
            // `~` (inline-only), so the checker has already ruled out a
            // materialized value here; each is a phantom whose body an earlier
            // `TermKind::Quotation` recorded.
            "branch" => {
                let else_q = self.stack.pop().expect("branch: else quotation");
                let then_q = self.stack.pop().expect("branch: then quotation");
                let then_id = self.quot_bodies[&then_q];
                let else_id = self.quot_bodies[&else_q];
                let then_body = self.quot_defs[then_id.0].clone();
                let else_body = self.quot_defs[else_id.0].clone();
                self.lower_if(&then_body, &else_body, tail);
            }
            // Slice 10c (R-P3-2): `tag` reads a scalar enum's discriminant.
            // The checker has already restricted the operand to an enum every
            // variant of which is payload-free, whose runtime value is the
            // bare discriminant, so this reads no memory and converts no
            // width; `Instr::Tag` exists only to give that register an integer
            // `IrType` (its source carries `IrType::Enum`, which would
            // mis-dispatch a later `.`/arithmetic on it).
            "tag" => {
                let v = self.stack.pop().expect("tag: enum operand");
                let dst = self.fresh_value(IrType::Int {
                    bits: 32,
                    signed: false,
                });
                self.push_instr(Instr::Tag(dst, v));
                self.stack.push(dst);
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
                // own `call` resolves them with no extra plumbing.
                // The `tail` threading and the locals-truncate mirror the
                // `call` splice above. Checked before the `&`/conversion/struct
                // dispatch since a combinator name is an ordinary word name.
                //
                // **INV-INLINE-COMBINATOR.** A quotation-taking word is always
                // inlined (spliced) here and mints no `IrFunc`; it has no opaque
                // call form, and its declared output row is discovered by
                // forward checking of the spliced terms, never solved for by row
                // unification. Threading the caller's `tail` into the splice is
                // sound only because of that: the body really does run in place
                // of the call. Slice 7b (first-class runtime quotations) is
                // where the invariant breaks and this must be revisited,
                // together with `check::terms_tail_call_self`'s walk.
                if let Some(entry) = self.combinators.get(name) {
                    let body = &entry.terms;
                    // R10: a self-tail combinator lowers to a splice-time loop
                    // (back-edge, not re-splice); every other combinator is a
                    // straight term-splice. R-P1-5: the same predicate the
                    // checker's `splice_tail` consults, so the two cannot
                    // disagree about whether this splice is a loop.
                    let self_tail =
                        crate::check::terms_tail_call_self(body, name, self.combinators);
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
                        self.lower_terms(&body, tail);
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
                    let (in_arity, quot_inputs) = {
                        let a = self.env.get(name).expect("checked user word exists");
                        (a.in_arity, a.quot_inputs.clone())
                    };
                    let split = self.stack.len() - in_arity;
                    let mut args = self.stack.split_off(split);
                    // R-D3 applies to the back-edge the same as the ordinary
                    // dispatch below: a phantom quotation carried around the
                    // loop must be materialized before it reaches the header
                    // phi, or the blit at the loop header sees a phantom type.
                    self.materialize_quot_args(&mut args, &quot_inputs);
                    self.back_edges.push((self.cur_id, args));
                    self.seal_block(Terminator::Jmp(self.header.expect("loop header")));
                    self.terminated = true;
                    return;
                }
                let (in_arity, out_arity, ret_ty, quot_inputs) = {
                    let a = self.env.get(name).expect("checked user word exists");
                    (a.in_arity, a.out_arity, a.ret_ty, a.quot_inputs.clone())
                };
                let split = self.stack.len() - in_arity;
                let mut args = self.stack.split_off(split);
                // R-D3: an ordinary `[ ... ]` parameter is a real call, so a
                // phantom quotation argument becomes its `(code, env)`
                // aggregate here, before it can reach `Instr::Call`.
                self.materialize_quot_args(&mut args, &quot_inputs);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::BOOL_ENUM_ID;
    use crate::ir::test_helpers::*;

    #[test]
    fn quotation_literal_emits_no_instr_and_records_body() {
        // R12u: `lower_term`'s `TermKind::Quotation` arm mints a phantom
        // `Value` that defines no `Instr`, records `Value -> QuotId`, and
        // pushes it; the body is interned, not emitted.
        let env: HashMap<String, Arity> = HashMap::new();
        let structs = Structs::default();
        let enums = Enums::default();
        let arrays = Arrays::default();
        let cells = Cells::default();
        let refs = Refs::default();
        let resolve: Resolver = &|_name: &str| unreachable!("not called");
        let mut b = empty_builder(
            &env,
            resolve,
            Registries {
                structs: &structs,
                enums: &enums,
                arrays: &arrays,
                cells: &cells,
                refs: &refs,
                statics: empty_statics(),
            },
        );
        let term = &line_terms("[ + ]")[0];
        assert!(matches!(term.kind, TermKind::Quotation(_, _, _)));
        b.lower_term(term, false);
        assert!(
            b.cur_instrs.is_empty(),
            "a quotation literal emits no instruction: {:?}",
            b.cur_instrs
        );
        assert_eq!(b.stack.len(), 1);
        let v = b.stack[0];
        assert!(
            b.quot_bodies.contains_key(&v),
            "the phantom value is recorded in quot_bodies"
        );
        assert_eq!(b.quot_defs.len(), 1, "the body is interned once");
    }

    #[test]
    fn call_of_literal_emits_no_call_instr() {
        // Criterion 6b (R13): `[ + ] call` fuses in place, so lowered `main`
        // contains no `Instr::Call`; the phantom quotation never becomes a
        // runtime code value.
        let module = lower_src(": main ( -- ) 1 2 [ + ] call . ;");
        let main = func(&module, "main");
        assert_eq!(count(main, is_call_instr), 0);
        assert_eq!(
            count(main, |i| matches!(i, Instr::Bin(_, BinOp::Add, ..))),
            1
        );
    }

    #[test]
    fn self_tail_combinator_saves_and_restores_loop_state() {
        // R15u/U12/D4: after `lower_self_tail_combinator` returns, all five
        // loop-state fields (`header`/`entry_block`/`alloca_home`/
        // `carried_slots`/`back_edges`) are back to their pre-call values.
        // `finalize_loop` clears only two of them, so the explicit
        // save/restore is what lets a later `Alloc` (or a second sequential
        // loop) not hoist into the dead preheader, and lets a second
        // top-level loop reseat the alloca home to its own entry. Dropping
        // the `alloca_home` member from the shared helper leaves it stuck at
        // the first loop's entry and this fails (mutation-test the guard,
        // U12).
        let env: HashMap<String, Arity> = HashMap::new();
        let structs = Structs::default();
        let enums = Enums::default();
        let arrays = Arrays::default();
        let cells = Cells::default();
        let refs = Refs::default();
        let resolve: Resolver = &|_name: &str| unreachable!("not called");
        let mut b = empty_builder(
            &env,
            resolve,
            Registries {
                structs: &structs,
                enums: &enums,
                arrays: &arrays,
                cells: &cells,
                refs: &refs,
                statics: empty_statics(),
            },
        );
        // `lower_self_tail_combinator` is called directly (bypassing the
        // `self_tail` dispatch gate) with a body that is itself the self-call
        // (`foo`), so it back-edges to the header exactly as a real `while`
        // body would.
        let state = b.fresh_value(IrType::I64);
        b.push_instr(Instr::Const(state, 7));
        b.const_vals.insert(state, 7);
        b.stack.push(state);
        let saved_header = b.header;
        let saved_entry = b.entry_block;
        let saved_alloca_home = b.alloca_home;
        b.lower_self_tail_combinator("foo", &line_terms("foo"));

        assert_eq!(b.header, saved_header, "header restored");
        assert_eq!(b.entry_block, saved_entry, "entry_block restored");
        assert_eq!(b.alloca_home, saved_alloca_home, "alloca_home restored");
        assert!(b.carried_slots.is_empty(), "carried_slots restored");
        assert!(b.back_edges.is_empty(), "back_edges restored");
    }

    #[test]
    fn lower_max_emits_a_compare_and_select_no_call() {
        // R12: `max` lowers inline to `Cmp(Gt)` plus a `Phi`-joined select, no
        // `Instr::Call` and no monomorphization.
        let ir = lower_src(": main ( -- ) 3 5 max . ;");
        let main = ir.funcs.iter().find(|f| f.name == "main").unwrap();
        assert_eq!(
            count(main, |i| matches!(i, Instr::Cmp(_, CmpOp::Gt, ..))),
            1
        );
        assert_eq!(count(main, |i| matches!(i, Instr::Phi(..))), 1);
        assert_eq!(count(main, is_call_instr), 0);
    }

    #[test]
    fn lower_max_total_emits_no_float_compare() {
        // R13: `max-total` orders by the bit-pattern rule, so the emitted
        // `Cmp`s are all over the unsigned integer key, never `Instr::Cmp`
        // with a float operand.
        let ir = lower_src(": main ( -- ) 1.5 2.5 max-total . ;");
        let main = ir.funcs.iter().find(|f| f.name == "main").unwrap();
        let float_cmps = instrs(main)
            .iter()
            .filter(|i| match i {
                Instr::Cmp(_, _, a, _) => {
                    matches!(main.value_types[a.0 as usize], IrType::Float { .. })
                }
                _ => false,
            })
            .count();
        assert_eq!(float_cmps, 0);
        assert_eq!(count(main, is_call_instr), 0);
    }

    #[test]
    fn lower_square_has_one_mul() {
        let ir = lower_src(": sq ( i64 -- i64 ) | n | n n * ;");
        let sq = &ir.funcs[0];
        let mul_count = instrs(sq)
            .iter()
            .filter(|i| matches!(i, Instr::Bin(_, BinOp::Mul, _, _)))
            .count();
        assert_eq!(mul_count, 1);
        let last = sq.blocks.last().unwrap();
        assert!(matches!(last.term, Terminator::Ret(Some(_))));
    }

    #[test]
    fn lower_dup_reuses_value_id() {
        // `dup +` squares: both operands must be the same SSA value, dup emits nothing.
        let ir = lower_src(": w ( i64 -- i64 ) dup + ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        assert!(is.iter().all(|i| !matches!(i, Instr::Const(..))));
        let bin = is
            .iter()
            .find_map(|i| match i {
                Instr::Bin(_, BinOp::Add, a, b) => Some((*a, *b)),
                _ => None,
            })
            .unwrap();
        assert_eq!(bin.0, bin.1);
    }

    #[test]
    fn lower_binding_emits_no_new_instr() {
        // R10: a binding is a compile-time rebinding of SSA values, so binding
        // the operands and mentioning them lowers to the same instructions as
        // leaving them on the stack. No `Instr` variant was added.
        let bound = lower_src(": w ( -- i64 ) 1 2 | a b | a b - ;");
        let plain = lower_src(": w ( -- i64 ) 1 2 - ;");
        assert_eq!(
            format!("{:?}", instrs(&bound.funcs[0])),
            format!("{:?}", instrs(&plain.funcs[0]))
        );
    }

    #[test]
    fn lower_swap_reorders_without_instr() {
        // `swap -` computes b - a instead of a - b, and swap itself emits no instr.
        let swapped = lower_src(": w ( i64 i64 -- i64 ) swap - ;");
        let plain = lower_src(": w ( i64 i64 -- i64 ) - ;");
        let operands = |ir: &IrModule| {
            instrs(&ir.funcs[0])
                .iter()
                .find_map(|i| match i {
                    Instr::Bin(_, BinOp::Sub, a, b) => Some((*a, *b)),
                    _ => None,
                })
                .unwrap()
        };
        let (sa, sb) = operands(&swapped);
        let (pa, pb) = operands(&plain);
        assert_eq!((sa, sb), (pb, pa));
        assert_eq!(instrs(&swapped.funcs[0]).len(), 1);
    }

    #[test]
    fn str_literal_lowers_to_a_static_data_reference() {
        // R6: a `str` literal is exactly one `Instr::StrLit`, the backend's
        // hook to emit the static descriptor and take its address.
        let ir = lower_src(": w ( -- str ) \"hi\" ;");
        let w = &ir.funcs[0];
        assert_eq!(
            count(w, |i| matches!(i, Instr::StrLit(_, s) if s == "hi")),
            1
        );
    }

    #[test]
    fn len_of_str_lowers_to_str_len_with_no_call() {
        // R8: `len` on a `str` lowers to the dedicated `StrLen`
        // instruction, not a call and not a hand-written byte offset.
        let ir = lower_src(": w ( -- usize ) \"hi\" len ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, |i| matches!(i, Instr::StrLen(..))), 1);
        assert_eq!(count(w, is_call_instr), 0);
    }

    #[test]
    fn cstr_conversion_lowers_to_str_ptr() {
        // R7: `cstr` lowers to the dedicated `StrPtr` instruction.
        let ir = lower_src(": w ( -- cstr ) \"hi\" cstr ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, |i| matches!(i, Instr::StrPtr(..))), 1);
    }

    #[test]
    fn len_and_cstr_of_str_emit_no_byte_offset_instruction() {
        // Neither `len` nor `cstr` reads the descriptor via a hand-written
        // `field_ptr` offset (`PtrOffset` + `FieldLoad`) any more; both state
        // their intent through a dedicated instruction instead, keeping the
        // descriptor's layout a backend-only concern.
        let ir = lower_src(": w ( -- ) \"hi\" len drop \"hi\" cstr drop ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, |i| matches!(i, Instr::PtrOffset(..))), 0);
        assert_eq!(count(w, |i| matches!(i, Instr::FieldLoad(..))), 0);
        assert_eq!(count(w, |i| matches!(i, Instr::StrLen(..))), 1);
        assert_eq!(count(w, |i| matches!(i, Instr::StrPtr(..))), 1);
    }

    #[test]
    fn lower_comparison_primitive_result_is_a_32_bit_flag() {
        // Slice 10c (E-P3-4 part 2): the primitive emits the same `Instr::Cmp`
        // op over the same operands as the retired `>` builtin row did; only
        // the result type changed, from the 32-bit `Bool` to the 32-bit
        // unsigned flag `branch` consumes.
        let ir = lower_src(": w ( i64 i64 -- u32 ) u> ;");
        let w = &ir.funcs[0];
        let v = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::Cmp(v, CmpOp::Gt, _, _) => Some(*v),
                _ => None,
            })
            .expect("a Gt comparison");
        assert_eq!(
            w.value_types[v.0 as usize],
            IrType::Int {
                bits: 32,
                signed: false
            }
        );
    }

    /// Slice 10c (R-P3-2): `tag` on a scalar enum emits exactly one
    /// instruction, `Instr::Tag`, which exists only to give the discriminant
    /// an integer `IrType` -- the value already *is* the discriminant, in the
    /// same 32-bit register a `u32` occupies, so nothing is loaded and no
    /// width is converted.
    #[test]
    fn lower_tag_of_a_scalar_enum_reads_no_memory_and_converts_no_width() {
        let ir = lower_src(": w ( bool -- u32 ) tag ;");
        let w = &ir.funcs[0];
        let body = instrs(w);
        let (dst, src) = body
            .iter()
            .find_map(|i| match i {
                Instr::Tag(dst, src) => Some((*dst, *src)),
                _ => None,
            })
            .expect("the `tag` operation is lowered");
        assert_eq!(src, Value(0), "the operand is the word's own `bool` input");
        assert_eq!(
            w.value_types[dst.0 as usize],
            IrType::Int {
                bits: 32,
                signed: false
            }
        );
        assert!(
            !body.iter().any(|i| matches!(
                i,
                Instr::Conv(..) | Instr::Load(..) | Instr::FieldLoad(..) | Instr::Blit(..)
            )),
            "no conversion and no memory access"
        );
    }

    #[test]
    fn lower_print_emits_print_instr() {
        let ir = lower_src(": w ( i64 -- ) . ;");
        let w = &ir.funcs[0];
        assert!(instrs(w).iter().any(|i| matches!(i, Instr::Print(_))));
        let last = w.blocks.last().unwrap();
        assert!(matches!(last.term, Terminator::Ret(None)));
    }

    #[test]
    fn lower_print_on_bool_and_float_emits_same_print_instr() {
        // `.` lowers to one `Print` regardless of operand type: the IR stays
        // neutral and the backend dispatches on the value's own `IrType`.
        let bool_ir = lower_src(": w ( bool -- ) . ;");
        assert!(instrs(&bool_ir.funcs[0])
            .iter()
            .any(|i| matches!(i, Instr::Print(_))));
        let float_ir = lower_src(": w ( f64 -- ) . ;");
        assert!(instrs(&float_ir.funcs[0])
            .iter()
            .any(|i| matches!(i, Instr::Print(_))));
    }

    #[test]
    fn lower_float_literal_is_constf_f64_typed() {
        let ir = lower_src(": w ( -- f64 ) 2.5 ;");
        let w = &ir.funcs[0];
        let v = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::ConstF(v, x) if *x == 2.5 => Some(*v),
                _ => None,
            })
            .expect("a ConstF for the float literal");
        assert_eq!(w.value_types[v.0 as usize], IrType::Float { bits: 64 });
    }

    #[test]
    fn lower_float_div_routes_to_div_op() {
        // `/` lowers to `BinOp::Div` whose result carries the float operand type.
        let ir = lower_src(": w ( -- f64 ) 1.0 2.0 / ;");
        let w = &ir.funcs[0];
        let v = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::Bin(v, BinOp::Div, _, _) => Some(*v),
                _ => None,
            })
            .expect("a Div bin op");
        assert_eq!(w.value_types[v.0 as usize], IrType::Float { bits: 64 });
    }

    #[test]
    fn lower_conv_pushes_target_typed_value() {
        // `5 >u8` lowers the literal, then a `Conv` whose dst carries the u8 type.
        let ir = lower_src(": w ( -- u8 ) 5 >u8 ;");
        let w = &ir.funcs[0];
        let dst = instrs(w)
            .iter()
            .find_map(|i| match i {
                Instr::Conv(dst, _) => Some(*dst),
                _ => None,
            })
            .expect("a Conv instr");
        assert_eq!(
            w.value_types[dst.0 as usize],
            IrType::Int {
                bits: 8,
                signed: false
            }
        );
    }

    #[test]
    fn lower_bitwise_and_or_xor_route_to_matching_binop() {
        let ir = lower_src(": w ( -- i32 ) 1 >i32 2 >i32 and 3 >i32 or 4 >i32 xor ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        assert!(is
            .iter()
            .any(|i| matches!(i, Instr::Bin(_, BinOp::And, _, _))));
        assert!(is
            .iter()
            .any(|i| matches!(i, Instr::Bin(_, BinOp::Or, _, _))));
        assert!(is
            .iter()
            .any(|i| matches!(i, Instr::Bin(_, BinOp::Xor, _, _))));
    }

    #[test]
    fn lower_not_emits_xor_with_neg1_const() {
        let ir = lower_src(": w ( -- u8 ) 5 >u8 not ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        let neg1 = is
            .iter()
            .find_map(|i| match i {
                Instr::Const(v, -1) => Some(*v),
                _ => None,
            })
            .expect("a -1 const");
        let xor = is
            .iter()
            .find_map(|i| match i {
                Instr::Bin(v, BinOp::Xor, _, b) if *b == neg1 => Some(*v),
                _ => None,
            })
            .expect("a xor against the -1 const");
        assert_eq!(
            w.value_types[xor.0 as usize],
            IrType::Int {
                bits: 8,
                signed: false
            }
        );
    }

    #[test]
    fn lower_not_on_bool_emits_xor_with_1_const_not_neg1() {
        // Type-directed `not`: on a `bool` it must flip the low bit
        // (`xor operand, 1`), not the integer-complement `xor operand, -1`,
        // since `-1`/`-2` are not valid canonical `bool` values.
        let ir = lower_src(": w ( -- bool ) true not ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        assert!(
            !is.iter().any(|i| matches!(i, Instr::Const(_, -1))),
            "bool `not` must not use a -1 mask"
        );
        let (xor_v, mask_operand) = is
            .iter()
            .find_map(|i| match i {
                Instr::Bin(v, BinOp::Xor, _, b) => Some((*v, *b)),
                _ => None,
            })
            .expect("a xor bin op");
        assert_eq!(w.value_types[xor_v.0 as usize], IrType::Enum(BOOL_ENUM_ID));
        let mask_const = is.iter().find_map(|i| match i {
            Instr::Const(v, n) if *v == mask_operand => Some(*n),
            _ => None,
        });
        assert_eq!(mask_const, Some(1));
    }

    #[test]
    fn lower_bitwise_and_or_xor_accept_bool_operands() {
        let ir =
            lower_src(": w ( -- bool ) true false and true false or drop true false xor drop ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        for op in [BinOp::And, BinOp::Or, BinOp::Xor] {
            let v = is
                .iter()
                .find_map(|i| match i {
                    Instr::Bin(v, o, ..) if *o == op => Some(*v),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("expected a {op:?} bin op"));
            assert_eq!(w.value_types[v.0 as usize], IrType::Enum(BOOL_ENUM_ID));
        }
    }

    #[test]
    fn lower_le_ge_ne_route_to_matching_cmpop() {
        let ir = lower_src(": w ( -- bool bool bool ) 1 2 <= 1 2 >= 1 2 <> ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        for op in [CmpOp::Le, CmpOp::Ge, CmpOp::Ne] {
            assert!(
                is.iter()
                    .any(|i| matches!(i, Instr::Cmp(_, o, _, _) if *o == op)),
                "expected a {op:?} comparison"
            );
        }
    }

    #[test]
    fn lower_shl_shr_route_to_matching_binop_with_lhs_type() {
        let ir = lower_src(": w ( -- u8 ) 200 >u8 3 shl 3 shr ;");
        let w = &ir.funcs[0];
        let is = instrs(w);
        let shl_ty = is
            .iter()
            .find_map(|i| match i {
                Instr::Bin(v, BinOp::Shl, _, _) => Some(*v),
                _ => None,
            })
            .expect("a Shl bin op");
        assert_eq!(
            w.value_types[shl_ty.0 as usize],
            IrType::Int {
                bits: 8,
                signed: false
            }
        );
        assert!(is
            .iter()
            .any(|i| matches!(i, Instr::Bin(_, BinOp::Shr, _, _))));
    }

    #[test]
    fn lower_add_u8_result_is_u8_typed() {
        // Drive `lower_call`'s arithmetic arm with hand-typed u8 operands
        // directly, isolating the arm from parsing/checking, and assert the
        // result carries the operand type through to its `IrType`.
        let u8 = IrType::Int {
            bits: 8,
            signed: false,
        };
        let env = HashMap::new();
        let resolve = |name: &str| name.to_string();
        let structs = Structs::default();
        let enums = Enums::default();
        let arrays = Arrays::default();
        let cells = Cells::default();
        let refs = Refs::default();
        let mut b = FuncBuilder::new(
            &env,
            &resolve,
            Registries {
                structs: &structs,
                enums: &enums,
                arrays: &arrays,
                cells: &cells,
                refs: &refs,
                statics: empty_statics(),
            },
            "w".to_string(),
        );
        let x = b.fresh_value(u8);
        let y = b.fresh_value(u8);
        b.stack = vec![x, y];
        b.lower_call("+", Span::default(), false);
        let top = *b.stack.last().unwrap();
        assert_eq!(b.value_type(top), u8);
    }

    #[test]
    fn lower_dup_of_struct_allocs_and_blits() {
        // R14: `dup` of a struct copies the aggregate bytes (fresh alloc +
        // blit), unlike a scalar `dup` which reuses the value id. Single
        // output plus a `drop` of the extra copy, so this measures only
        // `dup`'s own copy, not the multi-output bundle-pack path.
        let ir = lower_src("type: Vec2 x i64 y i64 ; : d ( Vec2 -- Vec2 ) dup drop ;");
        let d = ir.funcs.iter().find(|f| f.name == "d").unwrap();
        assert_eq!(count(d, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(d, |i| matches!(i, Instr::Blit(..))), 1);
    }

    #[test]
    fn lower_dup_of_enum_allocs_and_blits() {
        // R15: `dup` of an enum copies the aggregate bytes (fresh alloc +
        // blit), like a struct and unlike a scalar. Single output plus a
        // `drop` of the extra copy, so this measures only `dup`'s own copy,
        // not the multi-output bundle-pack path.
        let ir = lower_src(
            "type: MaybeInt | None | Some v i64 ; : d ( MaybeInt -- MaybeInt ) dup drop ;",
        );
        let d = ir.funcs.iter().find(|f| f.name == "d").unwrap();
        assert_eq!(count(d, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(d, |i| matches!(i, Instr::Blit(..))), 1);
    }

    #[test]
    fn tail_self_call_lowers_to_back_edge_not_call() {
        // Criterion 2 (R6/R7/R8): a self-tail-recursive word lowers to a header
        // carrying one phi per loop-carried (input-arity) slot, and the tail
        // self-call is a `Jmp` back to that header with no `Instr::Call` to
        // self. `go` has input arity 2, so the header has two phis.
        let ir = lower_src(": go ( i64 i64 -- i64 ) dup 0 > ~[ 1 - go ] ~[ drop ] if ;");
        let f = &ir.funcs[0];
        let header = loop_header(f);
        let phis = header_phis(header_block(f, header));
        assert_eq!(phis.len(), 2, "one header phi per loop-carried slot");
        // Each phi has the entry arm plus the single back-edge arm.
        assert!(phis.iter().all(|arms| arms.len() == 2));
        // Entry + one back-edge both target the header.
        assert_eq!(jmps_to(f, header), 2);
        assert_eq!(
            count(f, is_call_instr),
            0,
            "tail self-call is a back-edge, not a Call"
        );
    }

    #[test]
    fn lower_mid_body_binding_adds_no_header_phi() {
        // Criterion 22 (R11): a mid-body binding inside a self-tail-recursive
        // arm has its extent end at the arm's terminator, where the back-edge
        // sits, so no name is live across it and the header still carries
        // exactly one phi per loop-carried (input-arity) slot, unaffected by
        // the binding. Proved by comparing against a binding-free equivalent:
        // if a bound name ever leaked a phi onto the header, this source's
        // shape would diverge from the one below instead of both trivially
        // satisfying the same hard-coded numbers.
        let with_binding =
            lower_src(": go ( i64 i64 -- i64 ) dup 0 > ~[ | x | 1 - x go ] ~[ drop ] if ;");
        let without_binding =
            lower_src(": go ( i64 i64 -- i64 ) dup 0 > ~[ 1 - go ] ~[ drop ] if ;");
        let f1 = &with_binding.funcs[0];
        let f2 = &without_binding.funcs[0];
        let header1 = loop_header(f1);
        let header2 = loop_header(f2);
        let shape1 = header_phi_shape(f1, header1);
        let shape2 = header_phi_shape(f2, header2);
        assert_eq!(
            shape1, shape2,
            "a mid-body binding must not change the header's phi structure"
        );
        assert_eq!(shape1.0, 2, "one header phi per loop-carried slot");
    }

    #[test]
    fn non_tail_self_call_stays_a_call() {
        // R10: a self-call followed by more work (`fact *`) is not in tail
        // position, so it stays a real `Instr::Call` and no loop is built.
        let ir = lower_src(": fact ( i64 -- i64 ) dup 0 = ~[ drop 1 ] ~[ dup 1 - fact * ] if ;");
        let f = &ir.funcs[0];
        assert_eq!(
            count(f, is_call_instr),
            1,
            "non-tail self-call stays a real Call"
        );
        assert!(
            !matches!(f.blocks[0].term, Terminator::Jmp(_)),
            "a non-tail-recursive word builds no loop header"
        );
    }

    #[test]
    fn self_call_in_non_terminal_if_stays_a_call() {
        // R10 over-eager boundary: the `if` is followed by more terms
        // (`drop 5`), so it is non-terminal and its arms are not in tail
        // position; the self-call stays a real `Instr::Call`.
        let ir = lower_src(": w ( i64 -- i64 ) dup 0 > ~[ w ] ~[ drop 0 ] if drop 5 ;");
        let f = &ir.funcs[0];
        assert_eq!(count(f, is_call_instr), 1);
        assert!(!matches!(f.blocks[0].term, Terminator::Jmp(_)));
    }

    #[test]
    fn both_if_arms_tail_produce_two_back_edges() {
        // R8 multi-arm back-patch through `lower_if`: a self-tail-call in each
        // arm of a terminal `if` back-edges, so the single header phi gains two
        // back-edge arms on top of the entry arm (three total).
        let ir = lower_src(": go ( i64 -- i64 ) dup 0 > ~[ 1 - go ] ~[ 1 + go ] if ;");
        let f = &ir.funcs[0];
        let header = loop_header(f);
        let phis = header_phis(header_block(f, header));
        assert_eq!(phis.len(), 1);
        assert_eq!(phis[0].len(), 3, "entry arm + two back-edge arms");
        assert_eq!(jmps_to(f, header), 3);
        assert_eq!(count(f, is_call_instr), 0);
    }

    #[test]
    fn clause_tails_share_one_header() {
        // R9: a `|`-clause self-tail-recursive word gets a single header; each
        // clause's terminal self-call is one back-edge into it. Both clauses
        // here tail-recurse, so each header phi has three arms (entry + two
        // back-edges) and no `Instr::Call` to self remains.
        let ir = lower_src(
            "type: Flag | Go | Stop ; \
             : loop2 ( i64 Flag -- i64 ) | Go 1 - Go loop2 | Stop 1 + Stop loop2 ;",
        );
        let f = ir.funcs.iter().find(|f| f.name == "loop2").unwrap();
        let header = loop_header(f);
        let phis = header_phis(header_block(f, header));
        // Slice 9 (R1): `Flag` is zero-payload, so the general scalar-enum
        // rule makes it register-resident -- it never enters the aggregate-
        // staging path, so it keeps a header phi just like the `i64` slot
        // (both scalar): 2 phis, not 1.
        assert_eq!(phis.len(), 2, "both the i64 and the scalar Flag slot phi");
        assert!(phis.iter().all(|arms| arms.len() == 3));
        assert_eq!(jmps_to(f, header), 3, "entry + two clause back-edges");
        assert_eq!(count(f, is_call_instr), 0);
    }

    #[test]
    fn mixed_clause_header_and_join_predecessors_stay_disjoint() {
        // R9 / risk 5: some clauses back-edge and one is a base case that
        // `Ret`s. The loop header phi (preds = entry + tail clause ends) and
        // the Slice-4 dispatch-join phi (preds = non-tail clause ends) must
        // keep disjoint predecessor sets.
        let ir = lower_src(
            "type: Flag | Go | Stop ; \
             : run ( i64 Flag -- i64 ) | Go 1 - Stop run | Stop ;",
        );
        let f = ir.funcs.iter().find(|f| f.name == "run").unwrap();
        let header = loop_header(f);
        let hb = header_block(f, header);
        let hphis = header_phis(hb);
        // Slice 9 (R1): `Flag` is zero-payload, hence scalar, hence it also
        // keeps a header phi alongside the `i64` one (2, not 1).
        assert_eq!(hphis.len(), 2);
        // header preds: entry arm + the one Go back-edge.
        assert!(hphis.iter().all(|arms| arms.len() == 2));
        assert!(
            f.blocks
                .iter()
                .any(|b| matches!(b.term, Terminator::Ret(_))),
            "the Stop base case still Rets"
        );
        // Every phi that is not a header phi is a dispatch/join phi; its
        // predecessors must not overlap the header phi's predecessors.
        let header_preds: std::collections::HashSet<u32> = hphis
            .iter()
            .flat_map(|arms| arms.iter().map(|(p, _)| p.0))
            .collect();
        for block in &f.blocks {
            if block.id == header {
                continue;
            }
            for instr in &block.instrs {
                if let Instr::Phi(_, arms) = instr {
                    for (p, _) in arms {
                        assert!(
                            !header_preds.contains(&p.0),
                            "join phi pred {p:?} collides with a header phi pred"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn clause_tail_call_alloc_is_hoisted_to_entry_not_loop_body() {
        // A clause self-tail-call rebuilds its enum scrutinee on every
        // back-edge. `Stop` carries a payload here (Slice 9, R1: a
        // zero-payload variant's construct no longer allocs at all -- it is a
        // bare scalar `Const` -- so this test needs a payload-bearing variant
        // to keep exercising the alloc-hoisting invariant it is named for).
        // If that `Alloc` stayed in the loop body, QBE's `alloc*` would bump
        // the frame pointer every iteration and blow the stack well before
        // Phase 4's N >= 1_000_000 golden. It must land in the entry block
        // instead, so the loop body has none.
        let ir = lower_src(
            "type: Flag | Go | Stop n i64 ; \
             : run ( i64 Flag -- i64 ) | Go 1 - dup Stop run | Stop drop ;",
        );
        let f = ir.funcs.iter().find(|f| f.name == "run").unwrap();
        let header = loop_header(f);
        let entry = &f.blocks[0];
        assert!(
            entry.instrs.iter().any(|i| matches!(i, Instr::Alloc(..))),
            "the Stop scrutinee's alloc should be hoisted into the entry block"
        );
        let entry_id = entry.id;
        for block in &f.blocks {
            if block.id == entry_id || block.id == header {
                continue;
            }
            assert!(
                !block.instrs.iter().any(|i| matches!(i, Instr::Alloc(..))),
                "block {:?} in the loop body must not alloc",
                block.id
            );
        }
    }

    #[test]
    fn quotation_taking_word_emits_no_call_and_no_irfunc() {
        // Criterion 3b/R20: a monomorphic quotation-taking word is inlined, so
        // it mints no `IrFunc` and its caller emits no `Instr::Call`. The
        // lowered `main` is just `1 +` (the spliced literal over `3`), a pure
        // arithmetic body. Deleting the `combinator_indices` filter would put
        // an `apply` func back, and deleting the `lower_call` inline branch
        // would leave an `Instr::Call apply` in `main`.
        let ir = lower_src(
            ": apply inline ( i64 [ i64 -- i64 ] -- i64 ) call ;\n\
             : main ( -- ) 3 [ 1 + ] apply . ;\n",
        );
        assert!(
            ir.funcs.iter().all(|f| f.name != "apply"),
            "a combinator mints no `IrFunc`, but one named `apply` was emitted"
        );
        let main = ir
            .funcs
            .iter()
            .find(|f| f.name == "main")
            .expect("`main` is emitted");
        assert!(
            call_symbols(main).is_empty(),
            "the inlined caller emits no `Instr::Call`, got: {:?}",
            call_symbols(main)
        );
    }

    #[test]
    fn abstract_forward_inlines_transitively_with_no_call() {
        // Criterion 10b (R21): transitive inlining. `outer` forwards its own
        // abstract quotation parameter to `inner`, so splicing `outer` into
        // `main` must in turn splice `inner` -- two levels, outermost-first.
        // The spec names this `map`-over-`each`. The shipped library keeps
        // `map`/`fold` as leaf combinators on cost grounds rather than scope
        // ones (building them on `each` is expressible, but inlining is total,
        // so composition depth is code size at every call site), so this
        // two-combinator chain stands in for that shape. It exercises the same
        // load-bearing property the criterion guards:
        // both combinators mint no `IrFunc` and `main` emits no `Instr::Call`.
        // Breaking the transitive splice (the `lower_call` combinator branch,
        // or the checker's abstract-forward accept) leaves an `Instr::Call`
        // for `inner` behind.
        let ir = lower_src(
            ": inner inline ( i64 [ i64 -- ] -- ) call ;\n\
             : outer inline ( i64 [ i64 -- ] -- ) inner ;\n\
             : main ( -- ) 7 [ 1 + . ] outer ;\n",
        );
        assert!(
            ir.funcs
                .iter()
                .all(|f| f.name != "inner" && f.name != "outer"),
            "both combinators are inlined and mint no `IrFunc`, got: {:?}",
            ir.funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let main = ir
            .funcs
            .iter()
            .find(|f| f.name == "main")
            .expect("`main` is emitted");
        assert!(
            call_symbols(main).is_empty(),
            "transitive inlining leaves no `Instr::Call` in `main`, got: {:?}",
            call_symbols(main)
        );
    }

    // Shared with `each_lowering_test_times_def_is_pinned_to_the_library` so
    // the two tests cannot drift apart: one exercises this exact source, the
    // other pins it against `lib/combinators.sth`.
    const TIMES_DEF: &str = ": times-helper inline ( ..s i64 i64 ~[ ..s i64 -- ..s ] -- ..s )\n\
         | f | | to | | from |\n\
         from to < ~[ from f call from 1 + to f times-helper ] ~[ ] if ;\n\
         : times inline ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s )\n\
         | f | | n | 0 n f times-helper ;\n";

    #[test]
    fn each_lowers_to_a_loop_not_a_per_element_call() {
        // Criterion 14b (R19, load-bearing): the inlined `each` lowers to a
        // real loop -- an entry `Jmp` to a header carrying the index `Phi`,
        // sealed with a `Jnz`, reached by a back-edge `Jmp` -- with no
        // per-element `Instr::Call` (the element quotation is spliced, not
        // called). This is the *structural* constant-stack guarantee behind
        // criterion 14's equivalence witness: deleting the `lower_call` inline
        // branch would leave an `Instr::Call` for `each` and no loop, and
        // unrolling per element would drop the back-edge. `each` is defined
        // inline here, over an inline `times`/`times-helper` mirroring
        // `lib/combinators.sth`, so the unit needs no import closure.
        let ir = lower_src(&format!(
            "{TIMES_DEF}\
             : each inline ( ['T 'N] [ 'T -- ] -- )\n\
             | f | len >i64 | count | | arr |\n\
             count ~[ | i | &arr i >usize &> @ f call ] times\n\
             arr drop ;\n\
             : main ( -- ) 0 4 fill [ . ] each ;\n"
        ));
        assert!(
            ir.funcs
                .iter()
                .all(|f| !matches!(f.name.as_str(), "each" | "times" | "times-helper")),
            "the inlined `each` and its `times` splices mint no IrFunc, got: {:?}",
            ir.funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let main = func(&ir, "main");
        let header = loop_header(main);
        let hblock = header_block(main, header);
        assert!(
            !header_phis(hblock).is_empty(),
            "the header carries the index phi"
        );
        assert!(
            matches!(hblock.term, Terminator::Jnz(..)),
            "the header is sealed with a Jnz (index < count), got {:?}",
            hblock.term
        );
        let entry_id = main.blocks[0].id;
        assert!(
            main.blocks
                .iter()
                .any(|b| b.id != entry_id && matches!(b.term, Terminator::Jmp(h) if h == header)),
            "a non-entry body block back-edges to the header"
        );
        // The array read `&arr i &>` emits the mandatory `sooth_oob_trap`
        // bounds-check call (the hand-threaded twin emits it too); it is not a
        // per-element call to the combinator or its element quotation, so it is
        // excluded. What must be absent is any call to `each` or a spliced
        // element op: the loop body is the spliced literal, not a call.
        let user_calls: Vec<&str> = call_symbols(main)
            .into_iter()
            .filter(|s| *s != "sooth_oob_trap")
            .collect();
        assert!(
            user_calls.is_empty(),
            "the inlined `each` body is spliced, not called; unexpected calls: {user_calls:?}"
        );
    }

    #[test]
    fn each_lowering_test_times_def_is_pinned_to_the_library() {
        // Pins the same `TIMES_DEF` that `each_lowers_to_a_loop_not_a_per_element_call`
        // actually compiles (not a second, independently-typed copy) against the
        // real library, so a future body change cannot leave that test silently
        // exercising a stale shape.
        let lib =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/lib/combinators.sth"))
                .expect("the combinator library should be readable");
        let normalize = |s: &str| -> String {
            s.lines()
                .map(|line| line.split('\\').next().unwrap_or(""))
                .collect::<Vec<_>>()
                .join(" ")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert!(
            normalize(&lib).contains(&normalize(TIMES_DEF)),
            "each_lowers_to_a_loop_not_a_per_element_call's inline times/times-helper has drifted from lib/combinators.sth"
        );
    }

    #[test]
    fn while_lowers_to_a_back_edge_not_an_infinite_splice() {
        // U12 (R10, load-bearing): a self-tail combinator `while` lowers to a
        // real mid-body loop -- an entry `Jmp` to a header carrying the state
        // `Phi`, reached by a back-edge `Jmp` -- with no `Instr::Call` to
        // `while` and no re-splice. Deleting the back-edge branch in
        // `lower_call` would leave an `Instr::Call` to `while` (or splice the
        // body forever), not silently pass. `while` is defined inline so the
        // unit needs no import closure.
        let ir = lower_src(
            ": while inline ( 'a [ 'a -- 'a bool ] -- 'a ) | p | p call ~[ p while ] ~[ ] if ;\n\
             : main ( -- ) 0 [ dup 5 < ~[ 1 + true ] ~[ false ] if ] while . ;\n",
        );
        assert!(
            ir.funcs.iter().all(|f| f.name != "while"),
            "the inlined `while` mints no IrFunc, got: {:?}",
            ir.funcs.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        let main = func(&ir, "main");
        let header = loop_header(main);
        let hblock = header_block(main, header);
        assert!(
            !header_phis(hblock).is_empty(),
            "the header carries the state phi"
        );
        let entry_id = main.blocks[0].id;
        assert!(
            main.blocks
                .iter()
                .any(|b| b.id != entry_id && matches!(b.term, Terminator::Jmp(h) if h == header)),
            "a non-entry body block back-edges to the header"
        );
        assert!(
            call_symbols(main).is_empty(),
            "the `while` body is spliced with a back-edge, not called; unexpected calls: {:?}",
            call_symbols(main)
        );
    }
}
