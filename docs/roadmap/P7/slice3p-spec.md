# Spec: P7.S3p trait-member dispatch at any input position

**Status:** Implemented
**Discovery:** `docs/roadmap/P7/slice3p-brief.md`

## Problem

A trait member was dispatchable through a bound only if the bound type variable sat on the
*top of the stack* at the call: `poly_trait_member_call` found the variable via
`receiver_ty_var(stack)`, which read `stack.last()` alone. S3e therefore rejected non-trailing
receivers at *declaration* time (`member_ends_in_trait_var` / `non_trailing_receiver_error`),
shutting out both `at ( &'T i64 -- i64 )` (receiver at `inputs[0]`) and `fresh ( -- i64 )`
(no operand carries the variable). Forcing consumer: `Indexable`/`at`, and downstream the
`cmp`-shaped `Ord` P7.S3r's paper dogfood assumes S3p unblocks.

## Design rulings (as shipped)

1. **Var discovery is name-first, off the bounds, not the stack.** `poly_trait_member_call`
   (`src/check/poly.rs:882`) scans every `Bound::User` in the body's `sig.bounds`, collecting
   `(var, TraitId, &TraitMember)` for each trait declaring a member of this name (after the
   unchanged `::` qualifier split and `qualified_target` module filter), deduped on
   `(var, TraitId)` so `'T: A A` does not read as self-ambiguity. The member's full declared
   input list is then matched against the stack window at `base = stack.len() - inputs.len()`,
   which never had a positional assumption. `receiver_ty_var` is deleted.
2. **A single candidate is dispatched on name alone; operand shape never declines it.** The
   per-input check (`trait_member_operand_error`) is the sole place a ref/bare or type mismatch
   on the resolved candidate is reported. A structural *selection* gate is the concrete
   regression the probe caught: `show` passed a bare `'T` where `&'T` was declared fell through
   to ordinary dispatch and reported `unknown word`. Loose selection, precise downstream check.
3. **`matched` is the whole candidate set across every bound variable.** `[]` falls through to
   ordinary `env`/builtin dispatch (`Ok(None)`, preserving `eq`/`if`/`bool` and every non-member
   word), `[one]` dispatches on that candidate's `var`, longer goes to ruling 4.
4. **Ambiguity is by candidate count *within one variable*; candidates spanning several
   variables are separated by the operands the call consumes**
   (`candidate_fitting_the_operands`, `poly.rs:814`, returning `CandidateFit`).
   Which *trait* a call means is unanswerable when two bounds on one variable declare the
   member (same operands either way; picking one would be overload resolution the language
   lacks) — input position is not a disambiguator there either. Which *variable* it means is
   answerable, and S3e answered it positionally; deciding the multi-variable case on count
   alone would regress the legal `f ( &'T: A &'U: A -- ) ta ta`
   (`one_trait_on_two_variables_resolves_each_span_against_its_own_theta`) and make R8's
   `o.var == v` conjunct dead. So for `matched.len() >= 2`:
   - **All on one variable** — rejected, byte-for-byte the existing
     `ambiguous_trait_member_call_is_rejected` behaviour.
   - **Spanning variables** — narrowed to the *unique* candidate whose declared input list fits
     the stack window, and dispatched. The two non-decisive outcomes get different diagnostics:
     several fits (reachable via a mixed set `&'U: C &'T: A B`, where otherwise bound
     declaration order would decide) is `ambiguous_trait_member_error` (`poly.rs:5564`), which a
     module qualifier would resolve; no fit is `no_candidate_fits_operands_error`
     (`poly.rs:5597`), naming each candidate's substituted input list, since no qualifier
     changes an operand-shape mismatch.
   The ambiguity error's single-variable rendering is unchanged and load-bearing; when variables
   differ it names each trait with its variable.
5. **The declaration gate is relaxed, not removed: a member must take the trait variable
   *directly* as some input.** `member_binds_trait_var` (`src/check/declarations.rs:367`, wired
   at `:341`) is true when any input is `PolyType::Var(0)` or `PolyType::Ref(Var(0), _)`, at any
   position. The test is deliberately syntactic, so it also rejects a receiver mentioned only
   *nested* in a composite input (`sum ( ['T 4] -- i64 )`) — grounding that needs structural
   unification dispatch does not attempt. `zero_receiver_member_error` (`:375`) distinguishes
   the two: only the nullary case is the P7.S3t deferral, so the text does not falsely claim a
   nested mention never mentions `'T`.
6. **Zero-receiver/nullary members (`fresh`, `tag`) are deferred, tracked P7.S3t.** With no
   operand carrying the variable, θ is fixed only by other operands, so dispatch has no signal
   for which concrete type's `fresh` is meant, and the language has no type-argument syntax or
   context binding to supply one. The gate keeps rejecting them, and the diagnostic names
   P7.S3t so the door is a tracked deferral.
7. **The `trait:` gate on builtin-named members widens to `call`, `slice`, and `subslice`**
   (`src/parser.rs:2140`). Name-only selection means a member of one of those names captures
   *every* such call in any body bounded by its trait, making quotation application and array
   slicing unreachable there (probed: `` `call` of `C` … expects `&'T`, found `[ -- i64 ]` ``).
   `is_name_dispatched_builtin` does not cover them (each is its own arm in
   `check_term`/`poly_call_term`, none is in `BUILTIN_WORDS`), and they cannot join that set,
   which also gates the `intrinsics` import (P8 S2 R2) — hence an explicit name test beside it.
   The six surface comparisons stay legal member names: they are `lib/` words and arrive
   mangled, so the spellings never collide.

## Out of scope

- Nullary members: P7.S3t (ruling 6).
- Obligation resolution (`check_poly_call`), the impl registry, monomorphization, lowering — a
  resolved obligation still keys on `(var, TraitId, member)`.
- The three-barrier fall-through partition (S3e ruling 7) and REPL bound-directed dispatch
  (bypassed via `lower_instantiation`).

## Invariants

- Selection never reads operand shape to *decline*: a single candidate is always dispatched, so
  a mismatch is the downstream located diagnostic, never a fall-through to `unknown word`.
  Shape narrowing applies only to a multi-variable set, deciding *which* variable.
- `poly_trait_member_call` still runs first in `poly_call_term`, ahead of every name-based
  special case; ordinary `eq`/`if`/`bool` dispatch in bounded bodies is untouched.
- `matched == []` falls through to `Ok(None)`; the `'T: A A` dedupe survives; the
  single-variable ambiguity rendering stays byte-identical; `check_trait_decls` stays pre-mangle.

## Tests

Migrated in `tests/phase7_slice3e.rs`: `trait_member_with_a_non_trailing_receiver_dispatches`
(was `…_is_rejected`); `a_concrete_word_of_the_members_name_captures_the_call` — the intended
dispatch-correctness claim does *not* hold through a build, since name resolution precedes the
checker and a concrete `: at` captures the spelling (S3e R18's ruled outcome), so the golden
keeps the program and asserts an operand/effect rejection, making the old silent `900`
mis-dispatch impossible; `trait_member_with_a_zero_input_receiver_is_rejected` retained against
the new text; plus `trait_member_named_{call,slice,subslice}_is_rejected` and
`trait_member_named_after_a_surface_comparison_dispatches` (ruling 7, both directions).

`src/check/poly.rs`: `a_non_trailing_receiver_dispatches_on_the_bound_variable`,
`a_non_trailing_receiver_mismatch_is_located_not_unknown` (ruling 2's regression witness),
`a_member_call_beats_a_concrete_word_of_the_same_name` (dispatch over an unresolved same-named
`env` word), `cross_variable_candidates_are_separated_by_the_operands`,
`a_cross_variable_member_call_fitting_no_candidate_names_the_operand_mismatch`,
`a_same_variable_ambiguity_survives_a_bound_on_another_variable`,
`a_no_fit_call_on_a_trailing_receiver_member_names_every_declared_shape` (guards the diagnostic
against going vaguer than the single-shape one it replaces),
`a_multi_position_duplicate_member_is_the_single_variable_ambiguity`,
`a_repeated_bound_is_not_ambiguous_with_itself`, and the preserved
`ambiguous_trait_member_call_is_rejected`.
`src/check/declarations.rs`: `member_binds_trait_var_accepts_any_receiver_position`.

End-to-end `tests/phase7_slice3p.rs`:
`indexable_at_on_pair_dispatches_a_non_trailing_receiver` compiles and runs `Indexable`/`at`
over `Pair` *and* a `Flip` impl that inverts the index, so `7 9 9 7` distinguishes *which* impl
the bounded call reached; the bounded `uses` body runs ordinary `eq`/`if` before the member
call, pinning that the widened search leaves that dispatch alone.

## Phases (delivered)

1. The whole checker change — bounds scan, name-only selection, cross-variable operand
   narrowing with its two diagnostics, the relaxed declaration gate with the P7.S3t deferral,
   the `call`/`slice`/`subslice` gate, and the S3e test migration.
2. The compiled `Indexable`/`at` dogfood golden.
