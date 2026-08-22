//! `FuncBuilder` control-flow method group: `emit_select`, `total_order_key`,
//! `lower_if`, `seal_arm`, and `lower_clauses` — the if/select/dispatch
//! lowering primitives that build and merge SSA blocks.

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
                            // `record_reference` fills both maps together, so
                            // a value with a referent always has the bit.
                            let mutable = self.ref_mutable[&t];
                            self.record_reference(v, referent, mutable);
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

    /// Lower a tag dispatch (R16): load the scrutinee's discriminant into a
    /// temp, dispatch N-way (a `Cmp(Eq)`-tag compare-chain to each variant's
    /// arm block, the last variant the terminal fall-through since coverage
    /// is exhaustive), and merge every arm's outputs at a single join block
    /// with one `Phi` per declared output over all N arm predecessors.
    ///
    /// This is deliberately *not* the 2-predecessor `lower_if` shape: the join
    /// has N predecessors and M outputs.
    ///
    /// `scrutinee_parts` is the enum being dispatched on and, for a reference
    /// scrutinee, that reference's mutability (`None` for an owning one). An
    /// eliminator call has both in hand already (slice 3b, R7), the enum from
    /// the call it resolved to and the mode from its arms' tags.
    pub(in crate::ir) fn lower_clauses(
        &mut self,
        clauses: &[Clause],
        params: &[Value],
        scrutinee_parts: (EnumId, Option<bool>),
        tail: bool,
    ) {
        // A dispatch back-edges only where a header was opened (R6). An
        // eliminator call is a *term*, so its arms inherit tail position only
        // when the call itself sits in it: an arm of a mid-body call that ends
        // in a self-call must emit a real call, or it would back-edge past the
        // rest of the enclosing word (Phase 6 slice 3, R5).
        let tail = tail && self.header.is_some();
        let scrutinee = *params.last().expect("a dispatch has a scrutinee input");
        let stack_below: Vec<Value> = params[..params.len() - 1].to_vec();
        let (scrut_id, ref_mutable) = scrutinee_parts;
        let n = self.enums.layouts[scrut_id.index()].variants.len();

        // Slice 9 (R1/R5): a non-reference scrutinee of a zero-payload enum
        // (`Bool`, generally any all-unit-variant enum) is already the bare
        // discriminant, not a pointer to tagged storage; a reference
        // scrutinee is always a pointer regardless of the referent's own
        // representation, so it still needs the `FieldLoad`.
        let scrutinee_is_value =
            ref_mutable.is_none() && self.enums.layouts[scrut_id.index()].is_scalar;

        // Map each variant index to the arm handling it (checker-guaranteed
        // exact coverage), so dispatch on tag == variant_index lands correctly
        // regardless of the order the arms were written in.
        let clause_ids = self.dispatch_on_tag(scrutinee, scrut_id, scrutinee_is_value);
        let join_id = self.fresh_block();
        let mut clause_for_variant: Vec<Option<&Clause>> = vec![None; n];
        for clause in clauses {
            // Phase 6 slice 3 (R6): `EnumWord` gained a second variant
            // (`Destructure`) once this word's `let` stopped being the only
            // reader of the registry, so the irrefutable `let` no longer
            // compiles. A synthetic clause's `.variant` always keys a
            // constructor entry (checker-guaranteed), so the non-`Construct`
            // path is unreachable, not a real case.
            let EnumWord::Construct(_, vi) = self.enums.words[&clause.variant] else {
                unreachable!("an arm always dispatches to a variant constructor")
            };
            clause_for_variant[vi] = Some(clause);
        }

        // An eliminator call is a mid-body term, not a whole word body, so
        // its arms must not wipe the caller's locals (Phase 6 slice 3, R5):
        // restoring to the depth captured here is a no-op-preserving truncate
        // of the enclosing scope.
        let locals_depth = self.locals.len();
        let mut clause_ends: Vec<(BlockId, Vec<Value>)> = Vec::with_capacity(n);
        for vi in 0..n {
            let clause = clause_for_variant[vi].expect("checked: exhaustive coverage");
            self.start_block(clause_ids[vi]);
            self.locals.truncate(locals_depth);
            self.stack = stack_below.clone();
            // Phase 6 slice 3 (R5): an arm receives the whole narrowed value —
            // the aggregate in owning mode, the reference established at entry
            // in reference mode — because its body reads fields through
            // `&field` projections, which address the aggregate rather than
            // consuming pre-decomposed values.
            self.stack.push(scrutinee);
            // R7/R9: an arm whose body ends in a tail self-call back-edges to
            // the shared loop header and contributes no join predecessor;
            // `tail` is true iff this word is self-tail-recursive. The header
            // phi preds (entry + tail arm ends) and the dispatch-join phi
            // preds (non-tail arm ends) therefore stay disjoint.
            self.terminated = false;
            self.lower_terms(&clause.body, tail);
            if !self.terminated {
                let result = self.stack.clone();
                let pred = self.cur_id;
                self.seal_block(Terminator::Jmp(join_id));
                clause_ends.push((pred, result));
            }
        }

        // Restore the caller's locals: the last arm's bindings are still on
        // `self.locals` (each arm only cleans up the *previous* one, at loop
        // entry above).
        self.locals.truncate(locals_depth);

        // Every arm back-edged: the join is unreachable and the word is
        // terminated (no fall-through Ret).
        if clause_ends.is_empty() {
            self.terminated = true;
            return;
        }

        // Single join block: one phi per declared output, merging the
        // fall-through arm predecessors.
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
        let ir = lower_src(": w ( Bool -- i64 ) ~[ 1 ] ~[ 2 ] if ;");
        let w = &ir.funcs[0];
        let has_phi = instrs(w).iter().any(|i| matches!(i, Instr::Phi(..)));
        assert!(has_phi);
        assert!(w
            .blocks
            .iter()
            .any(|b| matches!(b.term, Terminator::Jnz(..))));
    }

    /// Slice 10c (R-P3-1): `branch` reaches this same lowering directly, over
    /// a raw 32-bit flag rather than a `bool`, with its arms arriving as
    /// quotation operands instead of embedded term lists. Driving it without
    /// the library `if` in between is what shows the primitive itself is
    /// whole.
    #[test]
    fn lower_branch_on_a_raw_flag_emits_jnz_and_join() {
        let ir = lower_src(": w ( i64 i64 -- i64 ) ueq [ 1 ] [ 2 ] branch ;");
        let w = &ir.funcs[0];
        assert!(
            instrs(w).iter().any(|i| matches!(i, Instr::Phi(..))),
            "the two arms join"
        );
        let jnz = w
            .blocks
            .iter()
            .find_map(|b| match b.term {
                Terminator::Jnz(cond, ..) => Some(cond),
                _ => None,
            })
            .expect("a conditional jump on the flag");
        assert_eq!(
            w.value_types[jnz.0 as usize],
            IrType::Int {
                bits: 32,
                signed: false
            },
            "`branch` knows a 32-bit flag, never `Bool`"
        );
    }

    /// The edge case: both arms back-edge, so the join is unreachable and is
    /// elided entirely (R8, both-arms-tail). Reached through `branch` rather
    /// than the retired grammar.
    #[test]
    fn lower_branch_with_both_arms_back_edging_elides_the_join() {
        let ir = lower_src(
            ": spin ( i64 -- i64 )\n  \
             dup 0 ueq [ 1 sub spin ] [ 1 sub spin ] branch ;",
        );
        let w = &ir.funcs[0];
        assert!(
            !instrs(w)
                .iter()
                .any(|i| matches!(i, Instr::Phi(_, arms) if arms.len() == 2)),
            "neither arm reaches a join, so no two-predecessor phi is built"
        );
    }

    #[test]
    fn lower_nway_dispatch_builds_the_compare_chain_and_join_phi() {
        // R16: the dispatch loads the discriminant (one FieldLoad on the
        // scrutinee tag), builds an N-way `Cmp(Eq)` compare-chain (N-1
        // compares for N variants, the last variant a fall-through), and
        // merges the arms at a single join with one Phi per declared
        // output. A 4-variant enum: 3 Cmp(Eq), one Phi.
        let ir = lower_src(
            "type: Cmd | Halt | Push v i64 | Add | Dbl ;
             : run ( i64 Cmd -- i64 )
               ~[ ( Halt ) drop drop 0 ] ~[ ( Push ) Push> swap drop ]
               ~[ ( Add ) drop 1 add ] ~[ ( Dbl ) drop 2 mul ] Cmd? ;",
        );
        let run = ir.funcs.iter().find(|f| f.name == "run").unwrap();
        // Three `Cmp(Eq)` compares for four variants (the last falls through).
        assert_eq!(
            count(run, |i| matches!(i, Instr::Cmp(_, CmpOp::Eq, _, _))),
            3
        );
        // Exactly one Phi (single declared output) merging all four arms.
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
    fn lower_single_variant_dispatch_jumps_without_compare() {
        // R16: a single-variant (newtype) enum needs no compare — the sole
        // arm is the terminal fall-through, reached by a direct jump.
        let ir =
            lower_src("type: Id | Wrap v i64 ; : unwrap ( Id -- i64 ) ~[ ( Wrap ) Wrap> ] Id? ;");
        let unwrap = ir.funcs.iter().find(|f| f.name == "unwrap").unwrap();
        assert_eq!(count(unwrap, |i| matches!(i, Instr::Cmp(..))), 0);
    }

    /// The two-variant boundary of the compare-chain above: with N = 2 there
    /// is exactly one compare and the second variant is the fall-through, so
    /// an off-by-one in the chain shows up here as a compare the shorter chain
    /// should not have emitted.
    #[test]
    fn lower_two_variant_eliminator_dispatches_through_one_compare() {
        let ir = lower_src(
            "type: Shape | Circle r i64 | Rect w i64 h i64 ;
             : area ( Shape -- i64 )
               ~[ ( Circle ) Circle> ] ~[ ( Rect ) Rect> mul ] Shape? ;",
        );
        let area = ir.funcs.iter().find(|f| f.name == "area").unwrap();
        assert_eq!(
            count(area, |i| matches!(i, Instr::Cmp(_, CmpOp::Eq, _, _))),
            1,
            "two variants dispatch through one compare, the second falling through"
        );
        let phi_arms: Vec<usize> = area
            .blocks
            .iter()
            .flat_map(|b| b.instrs.iter())
            .filter_map(|i| match i {
                Instr::Phi(_, arms) => Some(arms.len()),
                _ => None,
            })
            .collect();
        assert_eq!(phi_arms, vec![2], "one join, one predecessor per arm");
    }

    /// An arm's block starts with the *whole* narrowed value, never the
    /// variant's decomposed fields. A one-variant, one-field enum makes the
    /// difference countable: the arm's own `Wrap>` is the single field read,
    /// and a dispatch that decomposed first would double it.
    #[test]
    fn lower_eliminator_arm_receives_the_whole_value_not_its_fields() {
        let ir = lower_src(
            "type: Id | Wrap v i64 ;
             : unwrap ( Id -- i64 ) ~[ ( Wrap ) Wrap> ] Id? ;",
        );
        let unwrap = ir.funcs.iter().find(|f| f.name == "unwrap").unwrap();
        assert_eq!(
            count(unwrap, |i| matches!(i, Instr::FieldLoad(..))),
            1,
            "only the arm body's own destructure reads the payload"
        );
    }

    /// R5: an eliminator call is a *term*, so its arms inherit tail position
    /// only when the call itself sits in it. The word below is self-tail
    /// recursive (its closing `if` arm back-edges), so a `tail` derived from
    /// `self.header` alone would back-edge from the eliminator's arm too,
    /// skipping every term after the call. It printed 10 instead of 11 before
    /// the call's own tail flag was threaded in.
    ///
    /// The other direction — an arm of a call that *is* in tail position,
    /// which does back-edge — is `eliminator_multi_tail_runs_in_constant_stack_native`
    /// in `tests/phase0.rs`.
    #[test]
    fn lower_eliminator_arm_of_a_non_tail_call_does_not_back_edge() {
        let ir = lower_src(
            "type: N | Zero | Succ v i64 ;
             : g ( i64 N -- i64 )
               ~[ ( Zero ) Zero> ] ~[ ( Succ ) Succ> drop 1 add Zero g ]
               N? 1 add dup 10 lt ~[ Zero g ] ~[ ] if ;",
        );
        let g = ir.funcs.iter().find(|f| f.name == "g").unwrap();
        assert!(
            call_symbols(g).contains(&"g"),
            "the arm returns into the rest of the word: {:?}",
            call_symbols(g)
        );
    }

    /// Decision 6: a reference-mode call resolves its enum through the arm's
    /// declared `&Shape.Circle` referent (which erases to the enum's own
    /// `IrType::Enum`), dispatches the same way, and reads fields through the
    /// scrutinee pointer rather than loading them into the arm.
    #[test]
    fn lower_eliminator_call_over_a_reference_dispatches_without_loading_the_payload() {
        let ir = lower_src(
            "type: Shape | Circle r i64 | Rect w i64 h i64 ;
             : area ( &Shape -- i64 )
               ~[ ( &Circle ) &r @ ] ~[ ( &Rect ) dup &w @ swap &h @ mul ] Shape? ;",
        );
        let area = ir.funcs.iter().find(|f| f.name == "area").unwrap();
        assert_eq!(
            count(area, |i| matches!(i, Instr::Cmp(_, CmpOp::Eq, _, _))),
            1
        );
        let field_loads = count(area, |i| matches!(i, Instr::FieldLoad(..)));
        assert_eq!(
            field_loads, 4,
            "the discriminant, plus one `@` per field the arms read — nothing loaded on their behalf"
        );
    }

    /// Decision 6's mode selection matters even when every arm's own body
    /// reads no fields: an all-unit-variant enum (`is_scalar`) is a bare
    /// discriminant by value, but a *reference* to one is still a pointer to
    /// tagged storage. The mode `lower_eliminator` reads off the arms' tags
    /// (slice 3b, R7) is what makes `dispatch_on_tag` load the tag through the
    /// pointer instead of treating the pointer itself as the discriminant.
    #[test]
    fn lower_eliminator_call_over_a_reference_to_a_scalar_enum_loads_the_tag() {
        let ir = lower_src(
            "type: Toggle | On | Off ;
             : pick ( &Toggle -- i64 )
               ~[ ( &On ) drop 1 ] ~[ ( &Off ) drop 0 ] Toggle? ;",
        );
        let pick = ir.funcs.iter().find(|f| f.name == "pick").unwrap();
        assert_eq!(
            count(pick, |i| matches!(i, Instr::FieldLoad(..))),
            1,
            "a reference scrutinee always loads its tag through the pointer, even for an all-unit-variant enum"
        );
    }

    // The generic analogue of the test above, which R7's mode mapping would
    // otherwise want, cannot be written: `is_scalar` requires every variant to
    // declare no fields, while a generic `type:` header requires every type
    // variable it binds to appear in some field (no phantom parameters), so a
    // generic all-unit-variant enum does not exist. The concrete test above is
    // the mapping's guard -- it runs the same `lower_eliminator` derivation,
    // generic or not -- and it fails if the mode is forced to owning.
}
