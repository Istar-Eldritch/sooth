# P7.S8 — nested inline-combinator splice-uid collision (condensed, implemented)

One lowering-time uid rule plus the `lib/cmp.sth` flip that makes it reachable. No parser,
AST, checker or backend change. The checker's uid numbering (`INLINE_UID_STRIDE`,
`src/check.rs`; the `word_idx` seed in `check_word`) is the correct namespace and is
untouched; lowering was what disagreed with it.

Shipped in two phases: (1) test-harness soundness, provable on unflipped `main`; (2) the
fix (R1+R1b), the flip, and the measured collateral.

## 1. The defect

`lower_resolved_word_call`'s combinator branch spliced a resolved trait-member body under
the *enclosing caller's* uid, so a nested splice inside that body looked up a
`(uid, span)` key the checker never wrote (Bug A: `.expect("checked user word exists")`
panic in `calls.rs`). Separately, `FuncBuilder::trait_calls` is span-keyed with one
grounding per session, so a second grounding of the same span reached from inside a
re-spliced member body got the stale first answer and recursed without bound (Bug B).

The two bugs are independent, not sequential: R1b's gate alone fixes the generic fixture
while the concrete fixture still panics, and R1 alone fixes the concrete and self-tail
paths while the generic one overflows the stack. Both are required for every exit
criterion.

Composition **calls**; it does not splice (`src/check/poly.rs`'s `cross_calls_of`
`is_combinator` branch doc comment is correct). The stale-entry recursion happens inside
the composed callee's own `IrFunc` body.

## 2. Requirements as delivered

**R1 — a spliced member body lowers under its own check-time uid namespace.**
`lower_resolved_word_call`'s combinator branch now, for the duration of the splice:
pushes the member's own seed onto `splice_uid_stack`; resets `self.inline_uid` (the
program-wide minting counter, not just the lookup stack) to that same seed; raises
`member_splice_depth`. All three are restored on exit. The counter reset is load-bearing
on its own: with only the stack push, the concrete fixture still panics one nesting level
deeper. `alpha_rename_member_locals` keeps its disjoint `MEMBER_SPLICE_SUFFIX`, now for a
new reason: the member body and the first combinator splice nested inside it share one
uid, and a shared suffix would be a silent wrong answer rather than a panic
(witness: `ord_inline_cmp_member_local_colliding_with_a_nested_splices_local_reads_its_own`).
The bracket covers every combinator this function splices, including an `inline` builtin
overload; those are top-level words too, so the checker seeded them the same way.

**R1b — the span-keyed `trait_calls` lookup stands aside during an active member
re-splice.** Gate is `if self.member_splice_depth == 0`, a new `FuncBuilder` field
incremented only by R1's bracket, never by the ordinary combinator-splice path. Not
`splice_uid_stack.is_empty()`: that blanket version was probed and rejected as over-broad,
since it also disables `trait_calls` for a bound member call inside a combinator's
quotation argument in a generic body, a shape that works today and has no
`splice_trait_calls` fallback.

**R2 — the seed reaches lowering without polluting `CombinatorEntry`.** No uid field on
`CombinatorEntry` (a word index is meaningless for four of `combinator_index`'s callers).
Instead `ir::lower` builds `member_uid_seeds: HashMap<String, u32>` (word name →
`word_idx * INLINE_UID_STRIDE`) from the same `module.words.iter().enumerate()` both
`src/ir/driver.rs`'s per-word pass and `src/check.rs` walk, so the two sides agree by
construction. Threaded into `FuncBuilder` beside `splice_records`/`splice_trait_calls`.
The real map goes to the per-word native path **and** the composed/transitive
instantiation path (an empty map there would leave R1 inert on exactly the path the
generic exit criterion exercises); `empty_member_uid_seeds()` goes to both REPL paths and
the destructor path, matching the existing `empty_splice_trait_calls()` pattern. A name
absent from the map falls back to `splice_uid_stack.last()`; the push/pop of
`splice_uid_stack` and `member_splice_depth` still happen, so R1b's gate still stands
aside. That fallback is the REPL's state, documented as a no-op, not a native-path hedge.

**R3 — composed-instantiation seed ruled on by measurement.** No third stride added:
`src/ir/driver.rs`'s composed and REPL sites stay at `0`, and the rationale comment now
names R1/R1b as the operative reason (a spliced member body no longer inherits any caller
counter). Proven by `an_ord_bounded_generic_word_instantiates_over_a_user_struct` and
`inline_mymax_mymax3_matches_noninline_baseline`.

**R4 — the flip.** `lib/cmp.sth`'s six surface comparisons (`eq`/`lt`/`gt`/`lte`/`gte`/`ne`)
are `inline`. Only the false clause of the header comment was deleted; the part
documenting `cmp`'s own pre-existing inlining stays, and the replacement states the
current design without narrating the reversal.

**R4b — REPL regression scoped, not fixed.** The flip turns a clean `unknown word` REPL
diagnostic into an ICE at `src/check/poly.rs:976` once a session reaches a `Bound::User`
call, because `src/check.rs`'s two REPL check sites hardcode `TraitResolveCtx::scratch()`.
Checker-side, outside R1/R1b's scope, invisible to `cargo test` (all affected tests are
ignored). Ten `#[ignore]` reason strings (re-derived against the landed code, not a
hardcoded line list) now state the true cause and name the follow-up slice. A real fix
needs a `Session`-level traits/impls accumulation table.

**R5 — collateral migrated against the live mechanism, never weakened.** Two helper-level
unsoundnesses, repaired in phase 1 before the flip:

- `back_edges` (`tests/phase4_slice10c_tail_splice.rs`, and its copy in
  `phase4_slice10c_row_gate.rs`) counted any block whose `Jmp` target id was `<=` its own,
  so a spliced eliminator's join block was miscounted. It now destructures `blocks[0]`'s
  `Jmp(target)` to get the loop header id and counts only blocks jumping to it, returning
  `0` when `blocks[0]` is not a `Jmp`. All five expected counts (`1, 1, 0, 0, 1`) unchanged.
  The witness landed pre-flip: `cmp` is already `inline`, so a self-tail loop splicing an
  eliminator-carrying inline word already produces the false-positive shape.
- the oracle's `sooth_mono_gt` reachability grep mints nothing once `gt` is `inline`.
  Replaced with a `gt`→`lt` swap control on both the baseline and candidate sides. Do not
  re-introduce a symbol-name discriminator over an inlinable word.

Three assertions in `tests/phase4_slice10c_primitives.rs` were **inverted to the new fact**
with rewritten doc comments, keeping the cost-comparison subject alive:
`the_six_comparisons_are_library_words` (the six *are* `inline`, still polymorphic under
`'T: Ord`), `the_canonical_comparison_and_branch_costs_no_call` (no comparison monomorph
minted, `w` call-free), and `the_library_eq_and_the_branch_primitive_are_both_call_free`
(measured: both call-free; renamed from `..._costs_a_call_the_branch_primitive_does_not`).

**R6/R7/R1c — new regression tests** in `tests/phase7_slice3s_flip.rs`, all through the
real binary against this repo's `lib/`, all asserting printed output:
`a_concrete_impl_ord_delegating_to_lt_builds_and_runs` (no generic word in the call chain;
panicked at `calls.rs` today), `a_self_tail_word_comparing_a_user_struct_in_its_loop_builds_and_runs`
(member splice inside a loop body; also panicked), and
`a_bound_member_call_inside_a_quotation_argument_instantiates_at_two_types` (R1b's
counter-example, passes on pristine HEAD and must keep passing; committed so a future
change cannot silently re-introduce the rejected blanket gate). Plus
`a_word_declared_before_the_impl_block_shifts_the_members_uid_seed`, the mutation-measured
guard on R2's seed formula.

**R8 — QBE baseline regenerated deliberately.** Every changed snapshot differs only by a
comparison monomorph folding into its caller (`examples/gcd.sth` loses
`call $sooth_mono_eq__m3__t0_i64` and the whole `sooth_mono_eq` function). Regeneration is
not a rubber stamp; a snapshot changing any other way is a finding.

## 3. Rulings on tests whose subject changed

- **Unsatisfied `Ord` now names `cmp`, not `lt`.** `an_unsatisfied_ord_bound_names_the_missing_impl`'s
  assertion and doc comment updated to the measured wording (it reports the library's line;
  the useful second line is unchanged). Not softened to a both-ways substring. The
  attribution loss is recorded as a follow-up, not buried in a test edit.
- **The oracle's dispatch-target-set equality was dropped entirely.** It is permanently
  false once the library inlines: the baseline still mints two `sooth_mono_gt__*` symbols
  (its cross-call composes into a real function regardless of `gt`'s inline-ness) while
  the candidate mints zero. Kept: the stdout-identity assertion, plus swap controls on
  both sides. Subject becomes "inline and non-inline `mymax` produce identical program
  behaviour".

## 4. Measured collateral (flip applied, before the fix)

12 tests across 6 binaries, each with its treatment above: the generic exit criterion
(R1+R1b), the unsatisfied-`Ord` diagnostic (section 3), three primitives assertions
(inverted), five loop assertions (R5 helper repair), the oracle (section 3b), and the QBE
corpus (R8). Everything else in the suite is green with the flip and without the fix, so
this is the whole `cargo test` blast radius. The ignored REPL tests are separate (R4b).

## 5. Exit criteria, as met

1. `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green with the six
   comparisons `inline` and the stale rationale gone.
2. `an_ord_bounded_generic_word_instantiates_over_a_user_struct` passes with correct output.
3. R6's `mymax`-free concrete fixture builds and runs.
4. R7's self-tail fixture builds and runs.
5. `lower_resolved_word_call` no longer reads `splice_uid_stack.last()` for a member with a
   seed, resets `self.inline_uid` alongside the stack push, and its doc comment states the
   current rule (both halves) with no history.
6. `trait_calls`'s lookup is gated on `member_splice_depth == 0`, its doc comment states
   the narrower rule, and the counter-example still passes.
7. The composed-seed rationale states the operative reason; no third stride added.
8. `back_edges` identifies the loop's back-edge in both copies; the five counts unchanged;
   guarded against riding on `opens_a_loop_header`
   (`back_edges_is_zero_for_a_single_variant_eliminators_unconditional_jump`).
9. The oracle's discriminator survives inlining, proven by the swap controls.
10. Every regenerated snapshot reviewed and stated.
11. Both follow-ups recorded in `docs/roadmap/P7-language-prereqs.md` and `ROADMAP.md`'s
    P7 status; ten `#[ignore]` reasons state the true cause.
12. All section-6 mutations reproduced their predicted failure (uid-stack push,
    `inline_uid` reset alone, the `member_splice_depth` gate, its blanket-gate regression,
    both wrong seed formulas `0` and `seed + 1`, the `back_edges` repair, both swap
    controls). R3's had nothing to revert; the two proving tests are named instead.
13. CLAUDE.md's five split signals re-run against `src/ir/func_builder/calls.rs`: 0 of 5
    fire, no split.

## 6. Standing risks

- Guessing the seed instead of measuring it: an off-by-one that works one level deep is
  the same bug one level down. The uid-seed shift fixture is the guard.
- R1 without R1b looks done: neither new concrete fixture exercises a composed cross-call,
  and R1b's absence is a genuine stack overflow, not a clean panic.
- R1b's gate must never widen to "any splice is active"; the existing suite was fully
  green under that broken gate.
- The `back_edges` repair going under-sensitive silently retires five loop guards.
- The oracle losing its subject by dropping the swap control and diffing stdout against
  itself.
- REPL stays untouched: it hands out empty splice tables and an empty seed map, both
  documented no-ops.
