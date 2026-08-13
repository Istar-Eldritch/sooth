//! `FuncBuilder` control-flow method group: `emit_select`, `total_order_key`,
//! `lower_if`, `seal_arm`, and `lower_clauses` — the if/select/clause lowering
//! primitives that build and merge SSA blocks.

use super::*;

impl<'a> FuncBuilder<'a> {
    /// A two-block compare-and-select (`max`/`max-total`'s shared shape,
    /// R12/R13): branch on `cond`, run each closure in its own block to
    /// produce that arm's value, and join with one `Phi`. Simpler than
    /// `lower_if`/`seal_arm` because a select's arms never back-edge (they
    /// lower no user terms, just a handful of value-producing instructions),
    /// so both predecessors always reach the join.
    pub(super) fn emit_select(
        &mut self,
        cond: Value,
        then_fn: impl FnOnce(&mut Self) -> Value,
        else_fn: impl FnOnce(&mut Self) -> Value,
    ) -> Value {
        let then_id = self.fresh_block();
        let else_id = self.fresh_block();
        let join_id = self.fresh_block();
        self.seal_block(Terminator::Jnz(cond, then_id, else_id));

        self.start_block(then_id);
        self.terminated = false;
        let then_val = then_fn(self);
        let then_pred = self.cur_id;
        self.seal_block(Terminator::Jmp(join_id));

        self.start_block(else_id);
        self.terminated = false;
        let else_val = else_fn(self);
        let else_pred = self.cur_id;
        self.seal_block(Terminator::Jmp(join_id));

        self.start_block(join_id);
        self.terminated = false;
        let ty = self.value_type(then_val);
        let v = self.fresh_value(ty);
        self.push_instr(Instr::Phi(
            v,
            vec![(then_pred, then_val), (else_pred, else_val)],
        ));
        v
    }

    /// R13: the `total_cmp` bit-pattern key for one `max-total` operand.
    /// Reinterprets `operand`'s IEEE bits as an unsigned integer (an 8-byte
    /// scratch slot, stored/reloaded at the operand's own width — `Store`/
    /// `Load` already dispatch on the value's declared `IrType`, R20), then
    /// maps the bits to a monotone key: flip every bit if the sign bit is
    /// set, else flip only the sign bit. Comparing two keys as unsigned
    /// integers then reproduces the total order without ever comparing the
    /// floats themselves.
    pub(super) fn total_order_key(&mut self, operand: Value, bits: u8) -> Value {
        let uty = IrType::Int {
            bits,
            signed: false,
        };
        let slot = self.fresh_value(IrType::Ptr);
        self.push_alloc(Instr::Alloc(slot, 8, 8));
        self.push_instr(Instr::Store(slot, operand));
        let raw = self.fresh_value(uty);
        self.push_instr(Instr::Load(raw, slot));

        let sign_mask: i64 = 1i64 << (bits - 1);
        let mask_v = self.fresh_value(uty);
        self.push_instr(Instr::Const(mask_v, sign_mask));
        let masked = self.fresh_value(uty);
        self.push_instr(Instr::Bin(masked, BinOp::And, raw, mask_v));
        let zero_u = self.fresh_value(uty);
        self.push_instr(Instr::Const(zero_u, 0));
        let is_neg = self.fresh_value(IrType::Bool);
        self.push_instr(Instr::Cmp(is_neg, CmpOp::Ne, masked, zero_u));

        self.emit_select(
            is_neg,
            |b| {
                let all_ones = b.fresh_value(uty);
                b.push_instr(Instr::Const(all_ones, -1));
                let key = b.fresh_value(uty);
                b.push_instr(Instr::Bin(key, BinOp::Xor, raw, all_ones));
                key
            },
            |b| {
                let key = b.fresh_value(uty);
                b.push_instr(Instr::Bin(key, BinOp::Xor, raw, mask_v));
                key
            },
        )
    }

    /// `tail` (R1) is true when this `if` is itself in tail position; it then
    /// hands tail position to the last term of both arms, so a self-call at the
    /// end of either arm back-edges (R7). An arm that back-edges leaves the
    /// builder `terminated` and contributes no predecessor to the join; the
    /// join is elided entirely when both arms back-edge (R8, both-arms-tail).
    pub(super) fn lower_if(&mut self, then_branch: &[Term], else_branch: &[Term], tail: bool) {
        let test = self.stack.pop().expect("if: test value");
        let then_id = self.fresh_block();
        let else_id = self.fresh_block();
        let join_id = self.fresh_block();

        let post_pop = self.stack.clone();
        // R2: each arm is a block, so a name it binds is out of scope at its
        // terminator; the checker has already rejected any use past there.
        let locals_depth = self.locals.len();
        self.seal_block(Terminator::Jnz(test, then_id, else_id));

        self.start_block(then_id);
        self.terminated = false;
        self.stack = post_pop.clone();
        self.lower_terms(then_branch, tail);
        let then_arm = self.seal_arm(join_id);
        self.locals.truncate(locals_depth);

        self.start_block(else_id);
        self.terminated = false;
        self.stack = post_pop;
        self.lower_terms(else_branch, tail);
        let else_arm = self.seal_arm(join_id);
        self.locals.truncate(locals_depth);

        match (then_arm, else_arm) {
            (None, None) => {
                // Both arms back-edged to the loop header; the join is
                // unreachable and the enclosing body is terminated.
                self.terminated = true;
            }
            (Some((_, s)), None) | (None, Some((_, s))) => {
                // A single fall-through predecessor: values flow directly, no
                // phi needed.
                self.start_block(join_id);
                self.terminated = false;
                self.stack = s;
            }
            (Some((then_pred, then_stack)), Some((else_pred, else_stack))) => {
                // R11: two arms leaving *different* quotation phantoms are a
                // materialization boundary the checker already accepted; each
                // arm mints its `(code, env)` value in its own (sealed) block
                // so the join `Phi`s two real aggregates, not two phantoms
                // (which define no bytes). The same phantom in both arms
                // (`t == e`) stays a forwarded marker and is spliced past the
                // join, so it is never materialized here.
                let (then_stack, else_stack) =
                    self.materialize_join_quotations(then_pred, then_stack, else_pred, else_stack);
                self.start_block(join_id);
                self.terminated = false;
                let mut join_stack = Vec::with_capacity(then_stack.len());
                for (t, e) in then_stack.into_iter().zip(else_stack) {
                    if t == e {
                        join_stack.push(t);
                    } else {
                        let ty = self.value_type(t);
                        let v = self.fresh_value(ty);
                        self.push_instr(Instr::Phi(v, vec![(then_pred, t), (else_pred, e)]));
                        // A merged reference is still `Ptr`, which says
                        // nothing about its referent; carry the shape across
                        // the join so a projection past it still resolves.
                        if let Some(&referent) = self.ref_inner.get(&t) {
                            self.ref_inner.insert(v, referent);
                        }
                        join_stack.push(v);
                    }
                }
                self.stack = join_stack;
            }
        }
    }

    /// Seal a just-lowered `if` arm: if it back-edged (terminated) it jumps
    /// nowhere here and yields no join predecessor; otherwise it jumps to the
    /// join, yielding `(pred, stack)`.
    fn seal_arm(&mut self, join_id: BlockId) -> Option<(BlockId, Vec<Value>)> {
        if self.terminated {
            None
        } else {
            let s = self.stack.clone();
            let pred = self.cur_id;
            self.seal_block(Terminator::Jmp(join_id));
            Some((pred, s))
        }
    }

    /// Lower a clause-style word (R16): load the scrutinee's discriminant into
    /// a temp, dispatch N-way (a `Cmp(Eq)`-tag compare-chain to each variant's
    /// clause block, the last variant the terminal fall-through since coverage
    /// is exhaustive), and merge every clause's outputs at a single join block
    /// with one `Phi` per declared output over all N clause predecessors.
    ///
    /// This is deliberately *not* the 2-predecessor `lower_if` shape: the join
    /// has N predecessors and M outputs.
    pub(in crate::ir) fn lower_clauses(
        &mut self,
        clauses: &[Clause],
        params: &[Value],
        scrutinee_ty: Type,
    ) {
        // A clause word is self-tail-recursive iff a header was opened (R6);
        // its clause bodies then carry tail position (D7).
        let tail = self.header.is_some();
        let scrutinee = *params.last().expect("clause word has a scrutinee input");
        let stack_below: Vec<Value> = params[..params.len() - 1].to_vec();
        // Threaded from the already-checked frontend `Type` rather than
        // re-derived from the lowered scrutinee's `IrType` — because a
        // `&!Enum` scrutinee lowers to the opaque `IrType::Ptr`, not
        // `IrType::Enum(id)`, so reading `self.value_type(scrutinee)` here
        // would make the enum arm below a reachable panic in reference mode.
        let (scrut_id, ref_mutable) = match scrutinee_ty {
            Type::Enum(id, _) => (id, None),
            Type::Ref(rid, mutable, _) => match self.refs.referent[rid.index()] {
                IrType::Enum(id) => (id, Some(mutable)),
                _ => unreachable!("checked: reference-mode clause scrutinee's referent is an enum"),
            },
            _ => unreachable!("checked: a clause word's top input is an enum"),
        };
        let payload_offset = self.enums.layouts[scrut_id.index()].payload_offset;
        let n = self.enums.layouts[scrut_id.index()].variants.len();

        // Slice 9 (R1/R5): a non-reference scrutinee of a zero-payload enum
        // (`Bool`, generally any all-unit-variant enum) is already the bare
        // discriminant, not a pointer to tagged storage; a reference
        // scrutinee is always a pointer regardless of the referent's own
        // representation, so it still needs the `FieldLoad`.
        let scrutinee_is_value =
            ref_mutable.is_none() && self.enums.layouts[scrut_id.index()].is_scalar;

        // Map each variant index to the clause handling it (checker-guaranteed
        // exact coverage), so dispatch on tag == variant_index lands correctly
        // regardless of clause source order.
        let clause_ids = self.dispatch_on_tag(scrutinee, scrut_id, scrutinee_is_value);
        let join_id = self.fresh_block();
        let mut clause_for_variant: Vec<Option<&Clause>> = vec![None; n];
        for clause in clauses {
            let EnumWord::Construct(_, vi) = self.enums.words[&clause.variant];
            clause_for_variant[vi] = Some(clause);
        }

        let mut clause_ends: Vec<(BlockId, Vec<Value>)> = Vec::with_capacity(n);
        for vi in 0..n {
            let clause = clause_for_variant[vi].expect("checked: exhaustive coverage");
            self.start_block(clause_ids[vi]);
            self.locals.clear();
            self.stack = stack_below.clone();
            // Push the variant's payload first-deepest, loading each field from
            // `payload_offset + field.offset`. In reference mode every
            // field is pushed as a reference to its own storage inside the
            // scrutinee (its address, never its value), registered in
            // `ref_inner` so a later access/projection through it resolves the
            // right shape — the same `IrType::Ptr` any other reference lowers
            // to.
            let fields = self.enums.layouts[scrut_id.index()].variants[vi]
                .fields
                .clone();
            for field in &fields {
                let adjusted = FieldLayout {
                    offset: payload_offset + field.offset,
                    ..*field
                };
                match ref_mutable {
                    Some(_) => {
                        let fptr = self.field_ptr(scrutinee, adjusted.offset);
                        self.push_reference(fptr, adjusted.ty);
                    }
                    None => self.load_field_onto_stack(scrutinee, adjusted),
                }
            }
            // Bind clause-body `| names |` locals (top N, leftmost deepest).
            let take = clause.locals.len();
            let bound = self.stack.split_off(self.stack.len() - take);
            for (name, value) in clause.locals.iter().zip(bound) {
                self.locals.push((name.clone(), value));
            }
            // R7/R9: a clause whose body ends in a tail self-call back-edges to
            // the shared loop header and contributes no join predecessor;
            // `tail` is true iff this word is self-tail-recursive. The header
            // phi preds (entry + tail clause ends) and the dispatch-join phi
            // preds (non-tail clause ends) therefore stay disjoint.
            self.terminated = false;
            self.lower_terms(&clause.body, tail);
            if !self.terminated {
                let result = self.stack.clone();
                let pred = self.cur_id;
                self.seal_block(Terminator::Jmp(join_id));
                clause_ends.push((pred, result));
            }
        }

        // Every clause back-edged: the join is unreachable and the word is
        // terminated (no fall-through Ret).
        if clause_ends.is_empty() {
            self.terminated = true;
            return;
        }

        // Single join block: one phi per declared output, merging the
        // fall-through clause predecessors.
        self.start_block(join_id);
        self.terminated = false;
        let m = clause_ends[0].1.len();
        let mut join_stack = Vec::with_capacity(m);
        for out_i in 0..m {
            let arms: Vec<(BlockId, Value)> = clause_ends
                .iter()
                .map(|(pred, st)| (*pred, st[out_i]))
                .collect();
            let ty = self.value_type(arms[0].1);
            let v = self.fresh_value(ty);
            self.push_instr(Instr::Phi(v, arms));
            join_stack.push(v);
        }
        self.stack = join_stack;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::test_helpers::*;

    #[test]
    fn lower_if_emits_phi_at_join() {
        let ir = lower_src(": w ( bool -- i64 ) if 1 else 2 end ;");
        let w = &ir.funcs[0];
        let has_phi = instrs(w).iter().any(|i| matches!(i, Instr::Phi(..)));
        assert!(has_phi);
        assert!(w
            .blocks
            .iter()
            .any(|b| matches!(b.term, Terminator::Jnz(..))));
    }

    #[test]
    fn lower_clause_word_builds_nway_dispatch_and_join_phi() {
        // R16: a clause word loads the discriminant (one FieldLoad on the
        // scrutinee tag), builds an N-way `Cmp(Eq)` compare-chain (N-1
        // compares for N variants, the last variant a fall-through), and
        // merges the clauses at a single join with one Phi per declared
        // output. A 4-variant enum: 3 Cmp(Eq), one Phi.
        let ir = lower_src(
            "type: Cmd | Halt | Push v i64 | Add | Dbl ;
             : run ( i64 Cmd -- i64 ) | Halt drop 0 | Push swap drop | Add 1 + | Dbl 2 * ;",
        );
        let run = ir.funcs.iter().find(|f| f.name == "run").unwrap();
        // Three `Cmp(Eq)` compares for four variants (the last falls through).
        assert_eq!(
            count(run, |i| matches!(i, Instr::Cmp(_, CmpOp::Eq, _, _))),
            3
        );
        // Exactly one Phi (single declared output) merging all four clauses.
        let phi_arms: Vec<usize> = run
            .blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .filter_map(|i| match i {
                Instr::Phi(_, arms) => Some(arms.len()),
                _ => None,
            })
            .collect();
        assert_eq!(phi_arms, vec![4]);
    }

    #[test]
    fn lower_single_variant_clause_word_jumps_without_compare() {
        // R16: a single-variant (newtype) enum needs no compare — the sole
        // clause is the terminal fall-through, reached by a direct jump.
        let ir = lower_src("type: Id | Wrap v i64 ; : unwrap ( Id -- i64 ) | Wrap ;");
        let unwrap = ir.funcs.iter().find(|f| f.name == "unwrap").unwrap();
        assert_eq!(count(unwrap, |i| matches!(i, Instr::Cmp(..))), 0);
    }
}
