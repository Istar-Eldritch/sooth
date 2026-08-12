//! `FuncBuilder` quotation method group: quotation materialization,
//! env/bundle packing, indirect/poly calls, struct/enum word lowering,
//! tag dispatch, and the universal drop primitive.

use super::*;

impl<'a> FuncBuilder<'a> {
    /// Slice 7a (R9/D5): at a materialization boundary, turn a phantom
    /// quotation `Value` (a `call`/`times`-splice marker that defines no
    /// runtime bytes) into a real `(code, env)` aggregate of signature `sig`.
    /// A `Value` already carrying an aggregate (a getter/`@`/word-return
    /// result, or another boundary's output) is returned untouched -- only a
    /// phantom recorded in `quot_bodies` is built here, so a boundary can call
    /// this unconditionally on whatever it is about to store/return.
    pub(in crate::ir) fn materialize_if_phantom(&mut self, val: Value, ty: IrType) -> Value {
        match ty {
            IrType::Quotation(sig) if self.quot_bodies.contains_key(&val) => {
                self.materialize_quot_value(val, sig)
            }
            _ => val,
        }
    }

    /// Slice 7a/7b (R9/R16): build the runtime `(code, env)` aggregate for
    /// phantom quotation `phantom`, recording the callee `IrFunc` to mint
    /// (deduped by symbol). `code` gets the callee's address (`FuncAddr`). The
    /// `env` slot is 7a's null pointer for a non-capturing literal (byte for
    /// byte), else the one captured local's live value snapshotted from the
    /// enclosing frame -- a scalar's value, or a reference's pointer. The
    /// symbol is `{enclosing}__quot{id}`, unique because word names are unique
    /// and `QuotId` is per-function.
    fn materialize_quot_value(&mut self, phantom: Value, sig: QuotSigId) -> Value {
        let id = self.quot_bodies[&phantom];
        let symbol = format!("{}__quot{}", self.cur_word_name, id.0);
        // The captured locals (sorted by name), resolved against the live frame
        // at this boundary.
        let captures = self.quotation_captures(id);
        if !self.materialized.iter().any(|m| m.symbol == symbol) {
            let env_caps = captures
                .iter()
                .map(|(name, value)| EnvCapture {
                    name: name.clone(),
                    ty: self.value_type(*value),
                    referent: self.ref_inner.get(value).copied(),
                })
                .collect();
            self.materialized.push(MaterializedQuot {
                symbol: symbol.clone(),
                effect: sig.0,
                body: self.quot_defs[id.0].clone(),
                captures: env_caps,
            });
        }
        let layout = quotation_layout(WORD_WIDTH);
        let dst = self.fresh_value(IrType::Quotation(sig));
        self.push_alloc(Instr::Alloc(dst, layout.size, layout.align));
        let code = self.fresh_value(IrType::Code);
        self.push_instr(Instr::FuncAddr(code, symbol));
        let code_ptr = self.field_ptr(dst, layout.code_offset);
        self.push_instr(Instr::FieldStore(code_ptr, code));
        let env = self.build_env(&captures);
        let env_ptr = self.field_ptr(dst, layout.env_offset);
        self.push_instr(Instr::FieldStore(env_ptr, env));
        dst
    }

    /// 7b/R16: the `env` word for a materialized closure. Q2a's ladder: zero
    /// captures -> a null pointer (byte for byte the 7a shape); exactly one ->
    /// the capture's live value inline (a scalar's value or a reference's
    /// pointer, one word); two or more -> a stack-allocated bundle of one word
    /// per capture, each `FieldStore`d in the sorted order the body reads, with
    /// the `env` word pointing at it.
    fn build_env(&mut self, captures: &[(String, Value)]) -> Value {
        match captures {
            [] => {
                let z = self.fresh_value(IrType::Ptr);
                self.push_instr(Instr::Const(z, 0));
                z
            }
            [(_, value)] => *value,
            many => {
                let size = many.len() as u32 * WORD_WIDTH;
                let bundle = self.fresh_value(IrType::Ptr);
                self.push_alloc(Instr::Alloc(bundle, size, WORD_WIDTH));
                for (i, (_, value)) in many.iter().enumerate() {
                    let slot = self.field_ptr(bundle, i as u32 * WORD_WIDTH);
                    self.push_instr(Instr::FieldStore(slot, *value));
                }
                bundle
            }
        }
    }

    /// 7b/R16: the enclosing locals a quotation body captures, as `(name, live
    /// value)` pairs resolved against `self.locals` at the materialization
    /// boundary, sorted by name so the build order matches the body's read
    /// order. A free global word contributes nothing (it is not a local).
    fn quotation_captures(&self, id: QuotId) -> Vec<(String, Value)> {
        let body = &self.quot_defs[id.0];
        let mut names = HashSet::new();
        free_locals_into(body, &mut HashSet::new(), &mut names);
        let mut names: Vec<String> = names.into_iter().collect();
        names.sort();
        names
            .into_iter()
            .filter_map(|name| {
                self.locals
                    .iter()
                    .rev()
                    .find(|(n, _)| *n == name)
                    .map(|(n, value)| (n.clone(), *value))
            })
            .collect()
    }

    /// Slice 7a (R11): at a branch join, materialize each arm's escaping
    /// quotation phantom into a runtime `(code, env)` value so the join `Phi`s
    /// two real aggregates. A slot needs materializing when the two arms leave
    /// *different* phantoms (`t != e`, both in `quot_bodies`); a shared phantom
    /// (`t == e`, a literal bound before the `if`) is forwarded untouched and
    /// spliced past the join. The target signature is the declared output at
    /// that slot (`cur_outputs[i]`) when the join is a word-body tail, else the
    /// referent of the `&!Quotation` store target directly below it
    /// (`ref_inner`, an in-frame `&!ref if..end !`) -- the two contexts the
    /// checker erases a differing join in. Materialization is appended to each
    /// arm's already-sealed block
    /// (via `reopen_block`), since the differing-pair decision needs both
    /// arms' stacks, known only after both are lowered.
    pub(super) fn materialize_join_quotations(
        &mut self,
        then_pred: BlockId,
        mut then_stack: Vec<Value>,
        else_pred: BlockId,
        mut else_stack: Vec<Value>,
    ) -> (Vec<Value>, Vec<Value>) {
        let mut jobs: Vec<(usize, QuotSigId)> = Vec::new();
        for (i, (t, e)) in then_stack.iter().zip(&else_stack).enumerate() {
            if t != e && self.quot_bodies.contains_key(t) && self.quot_bodies.contains_key(e) {
                let target = match self.cur_outputs.get(i) {
                    Some(&IrType::Quotation(sig)) => Some(sig),
                    _ => match i
                        .checked_sub(1)
                        .and_then(|b| self.ref_inner.get(&then_stack[b]))
                    {
                        Some(&IrType::Quotation(sig)) => Some(sig),
                        _ => None,
                    },
                };
                match target {
                    Some(sig) => jobs.push((i, sig)),
                    None => unreachable!(
                        "a differing quotation join slot maps to a declared quotation output or an in-frame store target"
                    ),
                }
            }
        }
        if jobs.is_empty() {
            return (then_stack, else_stack);
        }
        let (then_pos, then_term) = self.reopen_block(then_pred);
        for &(i, sig) in &jobs {
            then_stack[i] = self.materialize_quot_value(then_stack[i], sig);
        }
        self.reseal_block_at(then_pos, then_term);
        let (else_pos, else_term) = self.reopen_block(else_pred);
        for &(i, sig) in &jobs {
            else_stack[i] = self.materialize_quot_value(else_stack[i], sig);
        }
        self.reseal_block_at(else_pos, else_term);
        (then_stack, else_stack)
    }

    /// Slice 7a (R10): lower `call` on a materialized quotation value whose
    /// identity the checker could not resolve to a literal. Load the value's
    /// `code` slot and call it indirectly -- its inputs the stack below the
    /// value, its outputs pushed back (a multi-output quotation returns a
    /// bundle, unpacked, exactly as an ordinary word call). The runtime-
    /// dispatch counterpart to `call`-of-literal fusion (D5); no splice.
    pub(super) fn lower_indirect_call(&mut self, v: Value) {
        let IrType::Quotation(sig) = self.value_type(v) else {
            unreachable!("a non-literal `call` operand is a materialized quotation value")
        };
        let eff = sig.0;
        let split = self.stack.len() - eff.inputs.len();
        let mut args = self.stack.split_off(split);
        let layout = quotation_layout(WORD_WIDTH);
        let code_ptr = self.field_ptr(v, layout.code_offset);
        let code = self.fresh_value(IrType::Code);
        self.push_instr(Instr::FieldLoad(code, code_ptr));
        // 7b/R17: pass the env word as the trailing argument. Every materialized
        // body takes an env param (null for a non-capturing one), so this is
        // uniform without knowing which callee a merged/stored value holds.
        let env_ptr = self.field_ptr(v, layout.env_offset);
        let env = self.fresh_value(IrType::Ptr);
        self.push_instr(Instr::FieldLoad(env, env_ptr));
        args.push(env);
        let outs: Vec<TypedSlot> = eff
            .outputs
            .iter()
            .map(|&ty| TypedSlot { name: None, ty })
            .collect();
        let bundle = bundle_of(&outs, self.structs);
        let ret_ty = word_ret_ty(&outs, self.structs);
        let ret = if eff.outputs.len() == 1 || bundle.is_some() {
            Some(self.fresh_value(ret_ty.unwrap_or(IrType::I64)))
        } else {
            None
        };
        self.push_instr(Instr::CallIndirect(ret, code, args));
        if let Some(r) = ret {
            self.stack.push(r);
        }
        if let Some(id) = bundle {
            self.unpack_bundle(id);
        }
    }

    /// Dispatch on `scrutinee`'s runtime tag (enum `id`): seal a compare chain
    /// (`n == 1` short-circuits to a bare `Jmp`; otherwise load the tag once
    /// and `Cmp`/`Jnz` variant-by-variant, the last compare's false edge
    /// falling straight through to the final variant with no default/trap
    /// block) and return one freshly allocated, not-yet-started block per
    /// variant in declaration order. Shared by `lower_clauses` (a clause
    /// word's scrutinee) and `synthesize_enum_destructor` (the same shape,
    /// only what each variant block does next differs).
    ///
    /// `scrutinee_is_value` (Slice 9): whether `scrutinee` already *is* the
    /// discriminant (a zero-payload enum's bare scalar value) rather than a
    /// pointer to tagged storage needing a `FieldLoad`. A linear enum is
    /// never scalar (zero fields implies `is_linear == false`), so
    /// `synthesize_enum_destructor`'s call is always the pointer case; a
    /// non-reference clause scrutinee of a scalar enum is the value case.
    pub(super) fn dispatch_on_tag(
        &mut self,
        scrutinee: Value,
        id: EnumId,
        scrutinee_is_value: bool,
    ) -> Vec<BlockId> {
        let (tag_ty, tag_offset, n) = {
            let l = &self.enums.layouts[id.index()];
            (l.tag_ty, l.tag_offset, l.variants.len())
        };
        let variant_ids: Vec<BlockId> = (0..n).map(|_| self.fresh_block()).collect();
        if n == 1 {
            self.seal_block(Terminator::Jmp(variant_ids[0]));
        } else {
            let tag = if scrutinee_is_value {
                scrutinee
            } else {
                let tag = self.fresh_value(tag_ty);
                let tag_ptr = self.field_ptr(scrutinee, tag_offset);
                self.push_instr(Instr::FieldLoad(tag, tag_ptr));
                tag
            };
            for vi in 0..n - 1 {
                let idx_val = self.fresh_value(tag_ty);
                self.push_instr(Instr::Const(idx_val, vi as i64));
                let c = self.fresh_value(IrType::Bool);
                self.push_instr(Instr::Cmp(c, CmpOp::Eq, tag, idx_val));
                let false_target = if vi == n - 2 {
                    variant_ids[n - 1]
                } else {
                    self.fresh_block()
                };
                self.seal_block(Terminator::Jnz(c, variant_ids[vi], false_target));
                if vi < n - 2 {
                    self.start_block(false_target);
                }
            }
        }
        variant_ids
    }

    /// R5/R12/R16: the universal disposal primitive. On a linear value (a
    /// struct/enum whose `is_linear` is set, or an owning cell) this is a
    /// plain `Call` to the (builtin or synthesized) destructor; a `Copy`
    /// value is discarded with no runtime effect. Shared by `drop`, `S>fi`'s
    /// drop-the-rest, `S<fi`'s drop-on-overwrite, and the synthesized
    /// struct/enum destructors themselves, so "how a value is disposed" lives
    /// in one place.
    pub(in crate::ir) fn emit_drop(&mut self, v: Value) {
        match self.value_type(v) {
            // A cell always frees on drop, regardless of its payload's own
            // linearity: the synthesized destructor drops a linear payload
            // first.
            IrType::OwnedCell(id) => {
                let symbol = cell_drop_symbol(id, self.cells.drop_generations[id.index()]);
                self.push_instr(Instr::Call(None, symbol, vec![v]));
            }
            IrType::Struct(id) if self.structs.layouts[id.index()].is_linear => {
                let symbol =
                    struct_drop_symbol(id, self.structs.layouts[id.index()].drop_generation);
                self.push_instr(Instr::Call(None, symbol, vec![v]));
            }
            IrType::Enum(id) if self.enums.layouts[id.index()].is_linear => {
                let symbol = enum_drop_symbol(id, self.enums.layouts[id.index()].drop_generation);
                self.push_instr(Instr::Call(None, symbol, vec![v]));
            }
            IrType::Array(id) if self.arrays.layouts[id.index()].is_linear => unreachable!(
                "checked: a linear array element is rejected wherever an array type is named"
            ),
            _ => {}
        }
    }

    /// R10, callee side: pop the top `n` stack values into a fresh bundle of
    /// `id` (deepest output first, matching the field order the checker
    /// interned) and yield it as the word's single returned value. Literally
    /// the struct constructor, which is the point: the bundle is the struct
    /// users hand-wrote before this ABI existed.
    pub(in crate::ir) fn pack_bundle(&mut self, id: StructId) -> Value {
        self.lower_struct_word(StructWord::Construct(id));
        self.stack.pop().expect("pack: the bundle just constructed")
    }

    /// R11, caller side: replace the returned bundle on the stack with its
    /// fields, deepest first — the exact reverse of `pack_bundle`, through the
    /// same destructure a generated `S>` uses, so a linear field is moved out
    /// of the shell exactly as `S>` moves one.
    pub(super) fn unpack_bundle(&mut self, id: StructId) {
        self.lower_struct_word(StructWord::Destructure(id));
    }

    /// R14/R11: lower a call to a polymorphic word through its per-call-site
    /// `CallInst`. The mangled symbol (not `(self.resolve)(name)`), the
    /// per-θ output arity, and the bundle come straight from the table, so
    /// two instantiations of one word reach two distinct symbols and two
    /// distinct return shapes even though `env`/`resolve` are name-keyed. The
    /// input arity is name-constant across θ and read from `poly_arities`; the
    /// row prefix, if any, stays on the stack below the popped args (S2). The
    /// bundle unpack is the same pack/unpack path a monomorphic multi-output
    /// call takes (R10/R11), so a row-variable-expanded count lowers
    /// identically to a fixed multi-output word — D4's one mechanism.
    pub(super) fn lower_poly_call(&mut self, inst: &CallInst) {
        let in_arity = self.poly_arities[&inst.callee];
        let split = self.stack.len() - in_arity;
        let args = self.stack.split_off(split);
        let ret = if inst.out_arity == 1 || inst.bundle.is_some() {
            let ret_ty = match inst.bundle {
                Some(id) => IrType::Struct(id),
                None => ir_type_of(
                    *inst
                        .output_types
                        .first()
                        .expect("out_arity == 1 guarantees a single output type"),
                ),
            };
            Some(self.fresh_value(ret_ty))
        } else {
            None
        };
        self.push_instr(Instr::Call(ret, inst.symbol.clone(), args));
        if let Some(v) = ret {
            self.stack.push(v);
        }
        if let Some(id) = inst.bundle {
            self.unpack_bundle(id);
        }
    }

    /// Lower a generated struct word inline, first field deepest.
    pub(super) fn lower_struct_word(&mut self, sw: StructWord) {
        match sw {
            StructWord::Construct(id) => {
                let n = self.structs.layouts[id.index()].fields.len();
                let split = self.stack.len() - n;
                let args = self.stack.split_off(split);
                let dst = self.alloc_struct(id);
                for (fi, arg) in args.into_iter().enumerate() {
                    let field = self.structs.layouts[id.index()].fields[fi];
                    let fptr = self.field_ptr(dst, field.offset);
                    self.store_field(fptr, arg, field);
                }
                self.stack.push(dst);
            }
            StructWord::Get(id, fi) => {
                let s = self.stack.pop().expect("getter: struct operand");
                let fields = self.structs.layouts[id.index()].fields.clone();
                self.load_field_onto_stack(s, fields[fi]);
                // R9: on a linear receiver, `S>fi` still consumes the whole
                // aggregate, so every non-extracted linear field is dropped
                // here (a no-op drop-the-rest when every other field is
                // Copy, unchanged from before this slice).
                for (j, field) in fields.iter().enumerate() {
                    if j != fi && field_is_linear(field.ty, self.structs, self.enums, self.arrays) {
                        let v = self.field_value(s, *field);
                        self.emit_drop(v);
                    }
                }
            }
            StructWord::Set(id, fi) => {
                let newval = self.stack.pop().expect("setter: new field value");
                let s = self.stack.pop().expect("setter: struct operand");
                let dst = self.alloc_struct(id);
                let size = self.structs.layouts[id.index()].size;
                if size > 0 {
                    self.push_instr(Instr::Blit(s, dst, size));
                }
                let field = self.structs.layouts[id.index()].fields[fi];
                // R11: the old shell's other fields transfer via the blit
                // above (consumed, never dropped); only the field being
                // overwritten is read back out and dropped, before the store,
                // so the order is deterministic.
                if field_is_linear(field.ty, self.structs, self.enums, self.arrays) {
                    let old = self.field_value(dst, field);
                    self.emit_drop(old);
                }
                let fptr = self.field_ptr(dst, field.offset);
                self.store_field(fptr, newval, field);
                self.stack.push(dst);
            }
            StructWord::Destructure(id) => {
                let s = self.stack.pop().expect("destructure: struct operand");
                let n = self.structs.layouts[id.index()].fields.len();
                for fi in 0..n {
                    let field = self.structs.layouts[id.index()].fields[fi];
                    self.load_field_onto_stack(s, field);
                }
            }
            StructWord::Peek(id, fi) => {
                // R10: non-consuming, so the aggregate stays on the stack;
                // only the field's value is pushed on top of it. The checker
                // already rejected a linear field, so there is no drop glue
                // to consider here (unlike `Get`).
                let s = *self.stack.last().expect("peek: struct operand");
                let field = self.structs.layouts[id.index()].fields[fi];
                self.load_field_onto_stack(s, field);
            }
        }
    }

    /// Lower a variant constructor inline (R15): alloc the enum's tagged
    /// aggregate, store the discriminant (the variant's declaration index) as
    /// an `i32` at `tag_offset`, then store each field at `payload_offset +
    /// field.offset` (first field deepest, reusing `store_field`).
    pub(super) fn lower_enum_word(&mut self, ew: EnumWord) {
        match ew {
            EnumWord::Construct(id, variant_idx) => {
                // Slice 9 (D-A/R1/R2): a zero-payload enum's constructor
                // needs no memory at all -- the discriminant is the value --
                // so `True`/`False` (and any other all-unit-variant enum's
                // constructors) lower to a bare `Const`, register-resident,
                // exactly the retired `TermKind::BoolLit`'s shape.
                if self.enums.layouts[id.index()].is_scalar {
                    let v = self.fresh_value(IrType::Enum(id));
                    self.push_instr(Instr::Const(v, variant_idx as i64));
                    self.stack.push(v);
                    return;
                }
                let (tag_ty, tag_offset, payload_offset, fields) = {
                    let layout = &self.enums.layouts[id.index()];
                    (
                        layout.tag_ty,
                        layout.tag_offset,
                        layout.payload_offset,
                        layout.variants[variant_idx].fields.clone(),
                    )
                };
                let split = self.stack.len() - fields.len();
                let args = self.stack.split_off(split);
                let dst = self.alloc_enum(id);
                let tag = self.fresh_value(tag_ty);
                self.push_instr(Instr::Const(tag, variant_idx as i64));
                let tag_ptr = self.field_ptr(dst, tag_offset);
                self.push_instr(Instr::FieldStore(tag_ptr, tag));
                for (arg, field) in args.into_iter().zip(fields) {
                    let fptr = self.field_ptr(dst, payload_offset + field.offset);
                    self.store_field(fptr, arg, field);
                }
                self.stack.push(dst);
            }
        }
    }
}
