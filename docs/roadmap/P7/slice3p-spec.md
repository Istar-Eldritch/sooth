# Spec: P7.S3p trait-member dispatch at any input position

**Status:** Ready to implement
**Discovery:** `docs/roadmap/P7/slice3p-brief.md` (design probe-validated 2026-08-23)

## Problem

A trait member is dispatchable through a bound only if the bound type variable sits on the
*top of the stack* at the call. `poly_trait_member_call` (`src/check/poly.rs:835`) discovers
which variable to dispatch on by reading `receiver_ty_var(stack)` (`poly.rs:798`, called at
`:843`), which inspects `stack.last()` alone. A member whose bound variable is not its last
declared input therefore has no dispatch path, so S3e added a *declaration-time* rejection
(`member_ends_in_trait_var`, `src/check/declarations.rs:359`, wired in `check_trait_decls` at
`:341-342`, diagnostic `non_trailing_receiver_error` at `:367`). Two shapes are shut out:

- **A non-trailing (sandwiched) receiver** — `at ( &'T i64 -- i64 )`, an index/lookup shape
  whose receiver is `inputs[0]`.
- **A zero-receiver member** — `fresh ( -- i64 )`, with no operand carrying the variable at all.

Forcing consumer: `Indexable`/`at` (the probe's `Pair` fixture) and, downstream, the
`cmp`-shaped `Ord` that P7.S3r's paper dogfood assumes S3p unblocks.

## Design rulings

1. **Var discovery becomes name-first, off the bounds, not the stack.** The whole gap is
   *upstream* of the existing operand check: `poly_trait_member_call` already walks the
   member's full declared input list (`poly.rs:882-887`, via `substitute_member_var`) against
   the stack window at `base = stack.len() - inputs.len()` (`poly.rs:921-940`) and has no
   positional assumption. Replace the `receiver_ty_var(stack)` gate with a scan over this
   body's `sig.bounds` `Bound::User` entries: collect every `(var, TraitId, &TraitMember)`
   where the trait declares a member named `member` (after the existing `::` qualifier split
   and `qualified_target` module filter, `poly.rs:857-880`, unchanged). The variable comes
   from the *bound*, never from `stack.last()`. `receiver_ty_var` is deleted; it has one
   caller (`poly.rs:843`) and no other use.
2. **Selection is by member name alone; operand shape is never a selection gate.** A candidate
   is chosen because one of the body's bound traits declares the member — full stop. Do **not**
   require the stack slot at the receiver position to structurally match the member's declared
   receiver before selecting. The existing per-input structural check (`poly.rs:932-940`,
   `trait_member_operand_error`) stays the sole place a ref/bare or type mismatch is reported.
   Getting this backwards is the concrete regression the probe caught: a structural
   *selection* gate made `show` passed a bare `'T` where `&'T` was declared fall through to
   ordinary dispatch and report `unknown word` instead of the located mismatch, killing
   `trait_member_operand_mismatch_is_located`. Loose selection, precise downstream check.
3. **`matched` becomes the whole candidate set across every bound variable, and its count is
   the only ambiguity signal.** Today `matched` is built from `tids` collected for a single
   `var` (`poly.rs:874-905`), so it only ever saw same-variable collisions. Name-first search
   spans all of the body's `Bound::User` variables, so `matched` is `Vec<(u32, TraitId,
   &TraitMember)>`. `[]` falls through to ordinary `env`/builtin dispatch
   (preserving `eq`/`if`/`bool` and any non-member word), `[one]` dispatches on that
   candidate's `var`, and a longer slice goes to ruling 4. This subsumes and generalizes the S3e single-position ambiguity arm.
4. **Ambiguity is decided by candidate count *within one variable*; candidates spanning
   several variables are separated by the operands the call consumes.** Which trait a call
   means is unanswerable when two of one variable's bounds declare the member: the operands are
   the same either way, so picking one would be an overload resolution the language does not
   have. Which *variable* a call means is answerable, and S3e already answered it positionally
   — that is what keeps `f ( &'T: A &'U: A -- ) ta ta` (one trait on two variables, two
   obligations resolved against their own thetas,
   `one_trait_on_two_variables_resolves_each_span_against_its_own_theta`) callable. Deciding
   the multi-variable case on count alone would regress that legal program to a rejection and
   make R8's `o.var == v` conjunct dead. So, for `matched.len() >= 2`:
   - **All on one variable** (`'T: A B`, both declare `t1`): rejected, preserved
     byte-for-byte — the existing `ambiguous_trait_member_call_is_rejected` behaviour. Input
     *position* is not a disambiguator here either: `A`'s `t1 ( &'T -- )` and `B`'s
     `t1 ( &'T i64 -- )` on one variable stay ambiguous.
   - **Spanning variables** (`&'T: A &'U: A`, or `&'T: A &'U: B`): narrowed to the one
     candidate whose full declared input list fits the stack window, and dispatched. The two
     non-decisive outcomes are *different problems* and get different diagnostics:
     - **Several candidates fit** — reachable through a mixed set (`&'U: C &'T: A B`, all
       three declaring `t1`), where without a uniqueness requirement `'T`'s two
       equally-fitting candidates would be resolved by bound declaration order. This is the
       ambiguity error: a module qualifier naming which trait is meant would resolve it.
     - **No candidate fits** (the operands are wrong for all of them) — its own
       `no_candidate_fits_operands_error`, not the ambiguity error. Nothing is ambiguous
       here: every candidate's declared operands already disagree with the stack, so no
       qualifier changes the outcome. It is a plain operand-shape mismatch, just one with
       more than one declared shape to be wrong against.
   `ambiguous_trait_member_error` (`poly.rs:5478`) takes the candidate `(trait-name,
   ty-var-name)` set: when every candidate shares one variable it renders exactly as today
   (`` `t1` is required by both `A` and `B` on 'T (line …) ``, note line unchanged); when
   variables differ it names each trait with its variable (`` required by `A` on 'T and `B` on
   'U ``). The single-var rendering is load-bearing for the preserved test and must stay
   identical. `no_candidate_fits_operands_error` takes the same set and names each trait with
   its variable, under a `the operands at this call match none of their declared shapes` note.
5. **The declaration gate is relaxed, not removed: a member must bind the trait variable in
   *some* input.** `member_ends_in_trait_var` becomes `member_binds_trait_var`, true when *any*
   input is `PolyType::Var(0)` or `PolyType::Ref(_, Var(0))` (the trait's own variable is id 0
   in its own `PolySig`), not only the last. A member with no such input is still rejected at
   `trait:` time. The test stays deliberately syntactic, so it also rejects a receiver
   mentioned only *nested* inside a composite input (`sum ( ['T 4] -- i64 )`): grounding that
   would need structural unification through the array type, which dispatch does not attempt.
   `non_trailing_receiver_error` becomes `zero_receiver_member_error`, whose text must
   distinguish the two — only the nullary case is the P7.S3t deferral (below); a nested
   mention is not "mentions `'T` nowhere", and saying so would be false. The parser doc comment at `src/parser.rs:432` (`synth_member_word_name`)
   is updated: its invariant "every grounded signature mentions the `for` type" still holds —
   the receiver appears at *some* input, not necessarily last — so only its "last input" wording
   and the cited function name change; the mangling scheme is untouched.
6. **The zero-receiver member (open item 2, `fresh ( -- i64 )`/`tag`) is explicitly deferred,
   tracked as P7.S3t, not silently dropped.** With no operand carrying the variable, a
   name-first search would select the candidate and dispatch, but the variable never grounds
   from the call (θ is fixed only by other operands), so resolution either lands on an
   unrelated grounding or hits the pre-existing ungrounded-variable skip — dispatch has no
   signal for *which* concrete type's `fresh` is meant. The language has no type-argument
   syntax and no context binding to supply one. Rather than ship a half-working path, the
   declaration gate (ruling 5) keeps rejecting any member that binds the variable in no input.
   Lifting this needs a new call-site signal and is out of scope; `zero_receiver_member_error`
   names P7.S3t so the door is a tracked deferral, not an accident.
7. **The `trait:` gate on builtin-named members widens to `call`, `slice`, and `subslice`.**
   Ruling 2 makes selection name-only, so a bounded body's call of such a name can no longer
   fall through to the builtin arm when the operands do not fit: the member captures *every*
   call of that name in any body bounded by its trait. Probed on the code without the gate: a
   body applying its quotation parameter dies with `` `call` of `C` … expects `&'T`, found
   `[ -- i64 ]` ``, and `slice` on a `&[ i64 4 ]` with `` expects `&'T`, found `&[i64 4]` ``,
   so quotation application and array slicing become unreachable in such a body. S3r (R4)'s
   existing `is_name_dispatched_builtin` rejection does not cover the three: each is its own
   arm in `check_term`/`poly_call_term` and absent from `BUILTIN_WORDS`. They cannot simply
   *join* that set either, since it is also what the `intrinsics` import gates (P8 S2 R2) and
   none of the three is import-gated, so each gets an explicit name test beside it. This
   narrows what a `trait:` may declare (`trait: C 'T slice ( &'T -- i64 ) ;` parsed before this
   slice), which is why it is a ruling and not a refactor. The six surface comparisons stay
   legal member names: they are `lib/` words, and a body that imports one receives it mangled,
   so the spellings never collide.

## What changes

**`src/check/poly.rs`.** `receiver_ty_var` deleted. `poly_trait_member_call`'s head rewritten:
`(qualifier, member)` split and `qualified_target` resolution stay; the `tids`/single-`var`
collection is replaced by a scan over all `Bound::User` entries in `sig.bounds` producing the
`(var, TraitId, &TraitMember)` candidate set, deduped on `(var, TraitId)` (the existing
`'T: A A` dedupe must still collapse a repeated bound — otherwise it reads as self-ambiguity).
The `[] / [one] / _` match drives fall-through / dispatch / ambiguity. Everything from
`inputs` construction (`poly.rs:882`) through the truncate-and-push and obligation record
(`poly.rs:942-951`) is unchanged, now reached for any receiver position.
`ambiguous_trait_member_error` (`poly.rs:5478`) generalized per ruling 4.

**`src/check/declarations.rs`.** `member_ends_in_trait_var` → `member_binds_trait_var` (scan
all inputs); `non_trailing_receiver_error` → `zero_receiver_member_error` (new text, cites
P7.S3t); the `check_trait_decls` call site at `:341-342` updated to the new names. The doc
comment at `:349-358` rewritten to describe the relaxed rule and the deferral.

**`src/parser.rs`.** The `synth_member_word_name` doc comment at `:432` updated (ruling 5).
`parse_trait_decl`'s member-name gate extended with an explicit `call`/`slice`/`subslice`
test beside the existing `is_name_dispatched_builtin` rejection (ruling 7).

## Out of scope

- Zero-receiver / nullary members (`fresh`, `tag`): deferred, tracked **P7.S3t** (ruling 6).
- Any change to obligation resolution (`check_poly_call`), the impl registry, monomorphization,
  or lowering — dispatch position is a check-time concern; a resolved obligation still keys on
  `(var, TraitId, member)` exactly as before.
- The three-barrier fall-through partition (S3e ruling 7): unaffected, it keys on *whether*
  dispatch declined, never on receiver position.
- REPL bound-directed dispatch (already bypassed via `lower_instantiation`, per S3e).

## Invariants to preserve

- Selection never reads operand *shape* to *decline* a candidate: a single candidate is always
  dispatched, so a mismatch is the downstream per-input diagnostic and never a fall-through to
  `unknown word` (ruling 2). Shape narrowing applies only to a multi-variable candidate set
  (ruling 4), where it decides *which* variable, never whether to dispatch at all.
- `poly_trait_member_call` still runs first in `poly_call_term`, ahead of every name-based
  special case (S3e review finding 3). The name-first change is *internal* to it — the probe
  confirmed no ordering collision, ordinary `eq`/`if`/`bool` dispatch in the same bodies is
  untouched.
- `matched == []` must fall through to `Ok(None)`, or non-member words break.
- The `'T: A A` repeated-bound dedupe must survive the candidate-set rewrite.
- `ambiguous_trait_member_error`'s single-variable rendering stays byte-identical.
- `check_trait_decls` stays pre-mangle; the gate still rejects a variable-free member.

## Tests

Unit tests beside each touched function in `src/check/poly.rs` and
`src/check/declarations.rs`; an end-to-end golden in a new `tests/phase7_slice3p.rs`.
Migrate the three now-wrong S3e integration tests in `tests/phase7_slice3e.rs` in the same
phase that lifts the gate (they fail the instant it lifts):

- `trait_member_with_a_sandwiched_receiver_is_rejected` → rewritten to
  `trait_member_with_a_non_trailing_receiver_dispatches`: `at ( &'T i64 -- i64 )` on an
  `impl` now declares *and* dispatches instead of being rejected.
- `sandwiched_receiver_no_longer_silently_mis_dispatches` → rewritten to
  `a_concrete_word_of_the_members_name_captures_the_call`. The dispatch-correctness claim it
  was to become (the bounded call reaches the trait member ahead of `env.get`) does not hold
  through a *build*: name resolution runs before the checker, so a concrete `: at` in the same
  module captures the call site's spelling (S3e R18's ruled outcome — the trait loses a
  collision with a word of the member's name). The golden keeps the program and asserts it is
  still rejected, now as an operand/effect mismatch rather than a declaration rejection, so the
  original silent `900` mis-dispatch stays impossible. Dispatch winning over an *unresolved*
  same-named `env` word is pinned in `check::poly`'s tests instead
  (`a_member_call_beats_a_concrete_word_of_the_same_name`).
- `trait_member_with_a_zero_input_receiver_is_rejected`: kept as a rejection, assertion updated
  to the new `zero_receiver_member_error` text (must contain `P7.S3t`).

New coverage:

- `member_binds_trait_var` accepts a non-trailing and a trailing receiver, rejects a
  variable-free signature (unit, `src/check/declarations.rs`).
- A non-trailing receiver dispatches and records the obligation with the right `var`
  (`check_src`/`obligations_of`, mirroring `trait_member_call_records_an_obligation`).
- A bound-declared member with a mismatched operand is a *located* `trait_member_operand_error`,
  not `unknown word` — pins ruling 2 (the probe's regression witness).
- `ambiguous_trait_member_call_is_rejected` unchanged (same-variable text preserved), plus
  cross-variable coverage: `&'T: A &'U: B` both declaring the member *dispatches* per operand
  (each of `t1 t1` recording its own variable), while a call whose operands fit no candidate
  asserts the distinct `no_candidate_fits_operands_error` naming both variables, and a mixed
  set (`&'U: C &'T: A B`) stays rejected as the ambiguity — pins both the uniqueness
  requirement in ruling 4 and its two-diagnostic split.
- A multi-position, same-variable duplicate (`t1` at different input positions on one variable)
  is still the single-variable ambiguity — pins ruling 4.
- A no-fit call on a *trailing*-receiver member (a shape S3e already admitted, so the one
  place this slice can regress an existing diagnostic) keeps the enclosing word and has the
  note name every candidate's substituted input list. Pins ruling 4's diagnostic against
  going vaguer than the single-shape `trait_member_operand_error` it replaces there.
- A member named `call`, `slice`, or `subslice` is rejected at `trait:`, while the six surface
  comparisons stay accepted as member names (`tests/phase7_slice3e.rs`). Pins ruling 7 in
  both directions.
- End-to-end (`tests/phase7_slice3p.rs`, compiled): the probe's `Indexable`/`at` on `Pair`
  returns `p.a` / `p.b` for index `0` / `1` through a bounded generic body, with ordinary
  `eq`/`if`/`bool` dispatch in the same body unaffected.

## Phases

Phase 1 lands the entire checker change and keeps the suite green (it must migrate the S3e
tests it invalidates). Phase 2 adds the compiled dogfood proof.

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Name-first position-aware trait-member dispatch: replace receiver_ty_var stack discovery with a bounds scan, loose name-only candidate selection, widen ambiguity across variables and positions, relax the declaration gate to bind-in-some-input while deferring zero-receiver members (P7.S3t), and migrate the three now-wrong S3e tests",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "End-to-end compiled golden in tests/phase7_slice3p.rs: the Indexable/at Pair dogfood dispatching a non-trailing receiver to the right impl for index 0 and 1 with ordinary dispatch in the same body unaffected",
      "difficulty": "standard"
    }
  ]
}
```
