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

    /// R-D3: at an ordinary (non-spliced) call, turn each phantom quotation
    /// argument into the `(code, env)` aggregate the callee's declared
    /// parameter names. `quot_inputs` comes from the callee's `Arity`, so the
    /// slots line up with `args` positionally. This is the argument-position
    /// materialization boundary, the peer of a store, a word output, and a
    /// branch join.
    pub(in crate::ir) fn materialize_quot_args(
        &mut self,
        args: &mut [Value],
        quot_inputs: &[(usize, IrType)],
    ) {
        for &(slot, ty) in quot_inputs {
            args[slot] = self.materialize_if_phantom(args[slot], ty);
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
    /// value is discarded with no runtime effect. Shared by `drop`,
    /// `drop_level_fields`' drop-the-rest, and the synthesized struct/enum
    /// destructors themselves, so "how a value is disposed" lives in one
    /// place.
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
            StructWord::Destructure(id) => {
                let s = self.stack.pop().expect("destructure: struct operand");
                let n = self.structs.layouts[id.index()].fields.len();
                for fi in 0..n {
                    let field = self.structs.layouts[id.index()].fields[fi];
                    self.load_field_onto_stack(s, field);
                }
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
            // Phase 6 slice 3 (R6): the whole-variant destructure, unaffected
            // by P7 slice 1's retirement of the fused struct accessors (it is
            // a globally unique name, `variant_generated_sigs`). Reads every
            // field at `payload_offset + field.offset`, first field deepest,
            // mirroring `StructWord::Destructure`.
            EnumWord::Destructure(id, variant_idx) => {
                let s = self.stack.pop().expect("destructure: variant operand");
                let (payload_offset, fields) = {
                    let layout = &self.enums.layouts[id.index()];
                    (
                        layout.payload_offset,
                        layout.variants[variant_idx].fields.clone(),
                    )
                };
                for field in fields {
                    let adjusted = FieldLayout {
                        offset: payload_offset + field.offset,
                        ..field
                    };
                    self.load_field_onto_stack(s, adjusted);
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

    /// `bool` is injected as enum 0 ahead of any user enum (`BOOL_ENUM_ID`),
    /// so `Shape` is enum 1. `Circle` (vi 0) carries two fields, `Rect` (vi
    /// 1) one, `Dot` (vi 2) none.
    fn shape_enums() -> Enums {
        enums_of(
            "type: P a i64 b i64 ;\n\
             type: Shape | Circle r i64 p P | Rect q P | Dot ;\n\
             : main ( -- ) ;\n",
        )
    }

    #[test]
    fn destructure_multi_field_variant_pushes_every_field_in_order() {
        // Phase 6 slice 3 (R6): the whole-variant destructure, exercised
        // directly against hand-built state (no surface syntax reaches
        // `EnumWord::Destructure` until Phase 4's eliminator calls it).
        let enums = shape_enums();
        let id = EnumId::from_index(1);
        let structs = Structs::default();
        let arrays = Arrays::default();
        let cells = Cells::default();
        let refs = Refs::default();
        let env: HashMap<String, Arity> = HashMap::new();
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
        let receiver = b.fresh_value(IrType::Enum(id));
        b.stack.push(receiver);
        b.lower_enum_word(EnumWord::Destructure(id, 0));
        // `Circle` has two fields (`r i64`, `p P`): both land on the stack,
        // first field deepest.
        let fields = enums.layouts[id.index()].variants[0].fields.clone();
        assert_eq!(fields.len(), 2);
        assert_eq!(b.stack.len(), fields.len());
        for (v, field) in b.stack.iter().zip(&fields) {
            assert_eq!(b.value_type(*v), field.ty);
        }
    }

    #[test]
    fn destructure_zero_field_variant_pushes_nothing_and_does_not_panic() {
        // The companion to the multi-field case: a zero-field variant alone
        // would not catch a mutation that always pushes nothing regardless
        // of field count, so both are asserted.
        let enums = shape_enums();
        let id = EnumId::from_index(1);
        let structs = Structs::default();
        let arrays = Arrays::default();
        let cells = Cells::default();
        let refs = Refs::default();
        let env: HashMap<String, Arity> = HashMap::new();
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
        let receiver = b.fresh_value(IrType::Enum(id));
        b.stack.push(receiver);
        // `Dot` is variant 2 (the third), declared with no fields.
        b.lower_enum_word(EnumWord::Destructure(id, 2));
        assert!(
            b.stack.is_empty(),
            "a zero-field variant's destructure pushes nothing: {:?}",
            b.stack
        );
    }

    #[test]
    fn lower_two_output_word_returns_one_bundle_holding_both() {
        // Criterion 9 (R10): a two-output word's body ends in one `Ret` of the
        // synthesized bundle, with both outputs stored into it -- not a single
        // value returned and the other silently dropped.
        let ir = lower_src(": pair ( i64 -- i64 i64 ) dup ; : main ( -- ) 5 pair . . ;");
        let pair = ir.funcs.iter().find(|f| f.name == "pair").unwrap();
        let IrType::Struct(bundle) = pair.ret.expect("a two-output word returns its bundle") else {
            panic!("expected a struct return, got {:?}", pair.ret);
        };
        assert!(ir.structs[bundle.index()].bundle);
        assert_eq!(ir.structs[bundle.index()].fields.len(), 2);

        let last = pair.blocks.last().unwrap();
        let Terminator::Ret(Some(returned)) = last.term else {
            panic!("expected a value return, got {:?}", last.term);
        };
        assert_eq!(
            pair.value_types[returned.0 as usize],
            IrType::Struct(bundle)
        );
        assert_eq!(count(pair, |i| matches!(i, Instr::FieldStore(..))), 2);
    }

    #[test]
    fn lower_call_of_two_output_word_unpacks_the_bundle_onto_the_stack() {
        // R11: the caller reads both outputs back out of the returned bundle
        // (two field loads), so its lowering stack matches the stack the
        // checker verified -- the recon-3 desync that used to panic.
        let ir = lower_src(": pair ( i64 -- i64 i64 ) dup ; : main ( -- ) 5 pair . . ;");
        let main = ir.funcs.iter().find(|f| f.name == "main").unwrap();
        assert_eq!(count(main, |i| matches!(i, Instr::Call(Some(_), ..))), 1);
        assert_eq!(count(main, |i| matches!(i, Instr::FieldLoad(..))), 2);
        assert_eq!(count(main, |i| matches!(i, Instr::Print(_))), 2);
    }

    #[test]
    fn monomorphization_emits_one_mangled_func_per_instantiation() {
        // R9/R14: a polymorphic word is never emitted under its plain name;
        // instead one mangled `IrFunc` is emitted per distinct ground θ, and
        // each call site targets its own instantiation's symbol through the
        // R14 table, not `dupit`.
        let ir = lower_src(
            ": dupit ( 'T: Copy -- 'T 'T ) dup ;\n\
             : main ( -- ) 5 dupit . . true dupit . . ;",
        );
        assert!(
            ir.funcs.iter().all(|f| f.name != "dupit"),
            "the polymorphic word must not lower under its plain name"
        );
        let mono: Vec<&str> = ir
            .funcs
            .iter()
            .map(|f| f.name.as_str())
            .filter(|n| n.starts_with("sooth_mono_dupit"))
            .collect();
        assert_eq!(mono.len(), 2, "one IrFunc per θ (i64 and bool)");
        let main = ir.funcs.iter().find(|f| f.name == "main").unwrap();
        let calls = call_symbols(main);
        for sym in &mono {
            assert!(calls.contains(sym), "main should call `{sym}` directly");
        }
    }

    #[test]
    fn lower_single_output_word_keeps_its_scalar_return() {
        // R2/R15: nothing about the bundle path reaches a word with one
        // output; it returns its scalar directly, as before the slice.
        let ir = lower_src(": inc ( i64 -- i64 ) 1 + ;");
        let inc = ir.funcs.iter().find(|f| f.name == "inc").unwrap();
        assert_eq!(inc.ret, Some(IrType::I64));
        assert!(ir.structs.is_empty());
    }

    #[test]
    fn lower_bundle_with_a_linear_field_gets_no_destructor() {
        // Criterion 10 (R10/R11, key risk 1): the bundle for `( -- ^i64 i64 )`
        // folds linear (its first field is an owning cell), yet no drop glue is
        // synthesized for it -- the glue would free the cell the caller's
        // unpack has already moved out.
        let ir =
            lower_src(": cell-and-tag ( -- ^i64 i64 ) 7 ^ 3 ; : main ( -- ) cell-and-tag . ^> . ;");
        let (idx, layout) = ir
            .structs
            .iter()
            .enumerate()
            .find(|(_, l)| l.bundle)
            .expect("the two-output word interned a bundle");
        assert!(
            layout.is_linear,
            "an owning-cell field folds the bundle linear"
        );
        let glue = format!("sooth_struct_drop_{idx}");
        assert!(
            !ir.funcs.iter().any(|f| f.name == glue),
            "a bundle must carry no destructor, found `{glue}`"
        );
    }

    #[test]
    fn lower_two_words_with_one_output_shape_share_one_bundle() {
        // R8: bundles are interned by output tuple, deduped structurally like
        // an array shape, so two words of the same shape share one struct and
        // a third shape gets its own.
        let ir = lower_src(
            ": pair ( i64 -- i64 i64 ) dup ;\n\
             : twice ( i64 -- i64 i64 ) dup ;\n\
             : flags ( -- bool bool ) true false ;\n\
             : main ( -- ) ;",
        );
        assert_eq!(ir.structs.iter().filter(|l| l.bundle).count(), 2);
    }

    #[test]
    fn lower_drop_pops_without_instr() {
        let ir = lower_src(": w ( i64 i64 -- i64 ) drop ;");
        let w = &ir.funcs[0];
        assert!(instrs(w).is_empty());
        let last = w.blocks.last().unwrap();
        assert!(matches!(last.term, Terminator::Ret(Some(_))));
    }

    #[test]
    fn bool_enum_true_false_construct_0_and_1() {
        // Slice 9 (R2): `True`/`False` replace `TermKind::BoolLit`, lowering
        // to the same `0`/`1` scalar discriminant a bare `Const` produced
        // before this migration -- no memory aggregate, `IrType::Enum`
        // carrying `BOOL_ENUM_ID` (R1's general zero-payload-enum scalar
        // rule, not `IrType::Bool` directly).
        // Single-output words each, so neither triggers R10's bundle-return
        // packing (which would add its own, unrelated `Instr::Alloc` for the
        // bundle struct and muddy the "no aggregate" assertion below).
        let ir = lower_src(": t ( -- bool ) true ; : f ( -- bool ) false ;");
        let t = ir.funcs.iter().find(|f| f.name == "t").unwrap();
        let f = ir.funcs.iter().find(|f| f.name == "f").unwrap();
        assert_eq!(
            instrs(t).iter().find_map(|i| match i {
                Instr::Const(_, n) => Some(*n),
                _ => None,
            }),
            Some(1),
            "true -> 1"
        );
        assert_eq!(
            instrs(f).iter().find_map(|i| match i {
                Instr::Const(_, n) => Some(*n),
                _ => None,
            }),
            Some(0),
            "false -> 0"
        );
        assert!(
            !instrs(t).iter().any(|i| matches!(i, Instr::Alloc(..))),
            "a zero-payload enum construct must not allocate a memory aggregate"
        );
        let v = instrs(t)
            .iter()
            .find_map(|i| match i {
                Instr::Const(v, 1) => Some(*v),
                _ => None,
            })
            .expect("a const 1 for `true`");
        assert_eq!(t.value_types[v.0 as usize], IrType::Enum(BOOL_ENUM_ID));
    }

    #[test]
    fn lower_constructor_allocs_and_stores_each_field() {
        // The constructor allocs one aggregate slot and width-exact-stores both
        // fields; no aggregate copy for a flat struct.
        let ir = lower_src("type: Vec2 x i64 y i64 ; : mk ( i64 i64 -- Vec2 ) Vec2 ;");
        let mk = ir.funcs.iter().find(|f| f.name == "mk").unwrap();
        assert_eq!(count(mk, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(mk, |i| matches!(i, Instr::FieldStore(..))), 2);
    }

    #[test]
    fn lower_projection_read_is_single_field_load_no_copy() {
        // `&x @`: a field projection lowers to a pointer offset, and reading
        // it through `@` is a single load -- no aggregate copy, no alloc.
        let ir = lower_src("type: Vec2 x i64 y i64 ; : gx ( Vec2 -- i64 ) &x @ swap drop ;");
        let gx = ir.funcs.iter().find(|f| f.name == "gx").unwrap();
        assert_eq!(count(gx, |i| matches!(i, Instr::FieldLoad(..))), 1);
        assert_eq!(count(gx, |i| matches!(i, Instr::Blit(..))), 0);
        assert_eq!(count(gx, |i| matches!(i, Instr::Alloc(..))), 0);
    }

    #[test]
    fn lower_projection_write_mutates_in_place_no_alloc_no_blit() {
        // `&!x v !`: D3's replacement for the functional setter mutates the
        // receiver's own storage through the projected pointer -- no fresh
        // aggregate, no blit of the untouched fields.
        let ir =
            lower_src("type: Vec2 x i64 y i64 ; : sx ( Vec2 i64 -- Vec2 ) | v n | &!v &!x n ! v ;");
        let sx = ir.funcs.iter().find(|f| f.name == "sx").unwrap();
        assert_eq!(count(sx, |i| matches!(i, Instr::Alloc(..))), 0);
        assert_eq!(count(sx, |i| matches!(i, Instr::Blit(..))), 0);
        assert_eq!(count(sx, |i| matches!(i, Instr::FieldStore(..))), 1);
    }

    #[test]
    fn lower_destructure_loads_every_field() {
        let ir = lower_src("type: Vec2 x i64 y i64 ; : ex ( Vec2 -- i64 i64 ) Vec2> ;");
        let ex = ir.funcs.iter().find(|f| f.name == "ex").unwrap();
        assert_eq!(count(ex, |i| matches!(i, Instr::FieldLoad(..))), 2);
    }

    #[test]
    fn lower_zero_field_constructor_allocs_destructure_emits_nothing() {
        let ir = lower_src("type: Unit ; : u ( -- ) Unit Unit> ;");
        let u = ir.funcs.iter().find(|f| f.name == "u").unwrap();
        assert_eq!(count(u, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(u, |i| matches!(i, Instr::FieldLoad(..))), 0);
        assert_eq!(count(u, |i| matches!(i, Instr::Blit(..))), 0);
    }

    #[test]
    fn lower_constructor_allocs_stores_tag_and_each_field() {
        // R15: a variant constructor allocs the tagged aggregate, stores the
        // discriminant as a `Const`, then width-exact-stores each field. Rect
        // has two fields, so: one Alloc, one tag Const, three FieldStores
        // (tag + two fields).
        let ir = lower_src(
            "type: Shape | Circle r f64 | Rect w f64 h f64 ; : mk ( f64 f64 -- Shape ) Rect ;",
        );
        let mk = ir.funcs.iter().find(|f| f.name == "mk").unwrap();
        assert_eq!(count(mk, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(mk, |i| matches!(i, Instr::FieldStore(..))), 3);
        // The tag store writes the variant index (Rect = 1).
        assert!(instrs(mk).iter().any(|i| matches!(i, Instr::Const(_, 1))));
    }

    #[test]
    fn lower_zero_field_constructor_stores_only_the_tag() {
        // A zero-field variant constructs with just the tag store: one Alloc,
        // one FieldStore (the tag), no payload store.
        let ir = lower_src("type: MaybeInt | None | Some v i64 ; : n ( -- MaybeInt ) None ;");
        let n = ir.funcs.iter().find(|f| f.name == "n").unwrap();
        assert_eq!(count(n, |i| matches!(i, Instr::Alloc(..))), 1);
        assert_eq!(count(n, |i| matches!(i, Instr::FieldStore(..))), 1);
        // None is variant index 0.
        assert!(instrs(n).iter().any(|i| matches!(i, Instr::Const(_, 0))));
    }

    // Phase 3 Slice 1: the drop-spy's lowering (R5/R6/R16).

    #[test]
    fn lower_struct_constructor_emits_no_call_only_alloc_and_store() {
        // Constructing a linear struct value is inlined alloc + field
        // stores, not a runtime call: only `drop`'s own destructor call is
        // emitted.
        let ir = lower_src(&format!("{SPY_DEF}: w ( -- ) 7 Spy drop ;"));
        let w = ir.funcs.iter().find(|f| f.name == "w").unwrap();
        let is = instrs(w);
        let spy_drop = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(
            count(
                w,
                |i| matches!(i, Instr::Call(_, sym, _) if sym != &spy_drop)
            ),
            0,
            "the constructor emits no call: {is:?}"
        );
        assert_eq!(count(w, |i| matches!(i, Instr::Alloc(..))), 1, "{is:?}");
        assert_eq!(
            count(w, |i| matches!(i, Instr::FieldStore(..))),
            1,
            "{is:?}"
        );
    }

    #[test]
    fn lower_drop_of_linear_value_calls_the_destructor() {
        let ir = lower_src(&format!("{SPY_DEF}: w ( -- ) 7 Spy drop ;"));
        let w = ir.funcs.iter().find(|f| f.name == "w").unwrap();
        let calls: Vec<&String> = instrs(w)
            .iter()
            .filter_map(|i| match i {
                Instr::Call(None, sym, args) if args.len() == 1 => Some(sym),
                _ => None,
            })
            .collect();
        let spy_drop = struct_drop_symbol(StructId::from_index(0), None);
        assert_eq!(
            calls,
            vec![spy_drop.as_str()],
            "expected one destructor call"
        );
    }

    #[test]
    fn lower_drop_of_copy_value_emits_no_destructor_call() {
        // R2: `drop` on a Copy value keeps its no-runtime-effect discard.
        let ir = lower_src(": w ( -- ) 7 drop ;");
        let w = &ir.funcs[0];
        assert_eq!(count(w, is_call_instr), 0);
    }
}
