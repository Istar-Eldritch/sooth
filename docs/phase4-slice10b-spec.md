# Phase 4 Slice 10b: `times` moves to the library (spec)

10b deletes the `times` intrinsic (the `check_term` interception, `check_abstract_quotation_times`,
the four bespoke diagnostics, and the `calls.rs` lowering arm) and re-expresses `times` as
ordinary Sooth source in `lib/combinators.sth`, a thin wrapper over a private
self-tail-recursive `times-helper`. Every recon claim below was rebuilt and rerun against a
scratch tree with the intrinsic fully deleted (`main` at `0313b74`, which is one commit past
the brief's `86aee0a` anchor; the only change in between is `calls.rs`, unrelated to the
deleted arm). Every open question the brief left is resolved here with a probe, except one,
which resolves the other way: the migration is **not** a pure lightweight delete-and-import.
It silently drops one capability that only the intrinsic had, and closing that gap needs a
checker change outside the brief's sanctioned edits. That change is **folded into this slice
as Phase 1** (Decision P0), not deferred to a prerequisite slice.

A prototype covering the intrinsic deletion, the library word, and the Phase 1 relaxation is
committed beside this spec as
[`phase4-slice10b-p0-prototype.patch`](./phase4-slice10b-p0-prototype.patch). It builds, and
the soundness evidence in the P0 section below was measured against it. Treat it as reference,
not as the delivery: it carries no tests and none of the corpus or test-suite sweep, and it
does **not** contain R10's `span.module` fix, so it is short of Phase 1 as specified here.

**Warnings, all measured against the patch as committed:**

- It was generated before commit `9ee7b6a`, so its first two hunks *delete*
  `docs/phase4-slice11-brief.md` and `docs/phase4-slice11-spec.md` (a concurrent effort's docs).
  Apply only its `src/`, `lib/`, and `examples/` hunks.
- It deliberately leaves the four dead `times_*` diagnostics in place, so it emits 4
  `dead_code` warnings and is **not** clippy-clean: R1's warning-free state is only reached
  once they are deleted.
- Its inline comment on the relaxation carries the **retired** argument verbatim ("a local
  bound below the splice's frame floor lives in an ancestor frame that outlives every
  iteration, exactly as `check_reference_across_back_edge` lets an ancestor-rooted reference
  cross"). P0 disowns that parallel. Do not copy the comment; write the relocated-diagnostic
  rationale instead.
- The parameter is named `frame_floor`, which contradicts what P0 establishes it to be (an
  if-arm entry depth, not a frame floor). **Keep the name** rather than churn the call sites:
  it is the floor below which a local is exempt, and P0's text is the authority on what the
  quantity is. Do not reintroduce a frame-lifetime reading of the name.
- Q2's binary-size figures are not reproducible from the patch as-is: its `examples/times.sth`
  carries no import and fails with `` unknown word `times` `` until one is added. With the
  import added the figures are exact (17112 bytes both ways, output `500000500000`, and `nm`
  mints no `times`/`times-helper` symbol).

## The blocker: a linear local held across a `times` loop stops compiling

`examples/inplace_fold.sth` builds today (intrinsic) and prints `1 3 6 10 1 3 6 10`. Under a
library `times` its `prefix-linear` word is **rejected**:

```
error: linear values across a loop are not supported yet in `prefix-linear` (line 30)
  a `AccL` is live across the self-tail-call back-edge to `times-helper`: consume it before the recursive call
  note: declared ( [i64 4] AccL -- AccL )
```

`prefix-linear` binds a linear accumulator `acc` (`AccL`, linear via a `drop` overload),
takes a mutable borrow of its array (`out`), drives a `times` loop that writes running sums
through `out`, then returns `acc`. `acc` is never consumed inside the loop; it sits in a
stable frame slot, reached only by the borrow. The intrinsic's bespoke check permitted this
(it checked body move-state identity and the row, never the enclosing frame). `times-helper`
is a genuine self-tail combinator, so its splice runs the general
`check_linear_across_back_edge` (`src/check/terms.rs:1168`; its diagnostic,
`linear_across_back_edge_error`, is at `:1151`), whose second clause flags **any**
unconsumed linear local in scope, including `prefix-linear`'s `acc`.

This is not a `times-helper` defect. It is a pre-existing limitation of **every** spliced
self-tail combinator, confirmed directly against `while`:

```sooth
\ Rejected identically: an enclosing linear local merely held across a spliced `c::while`.
: work ( AccL -- AccL )
  | acc |
  0 [ | n | n 1 + dup 4 < ] c::while drop
  acc ;
\ error: ... a `AccL` is live across the self-tail-call back-edge to `while`
```

So the intrinsic `times` was the *only* loop in the language that could hold an enclosing
frame-parked linear local across itself. Deleting it removes that capability. `inplace_fold`'s
linear half is the corpus witness, and `tests/phase4_slice6f.rs`'s
`inplace_fold_linear_lowers_without_per_iteration_blit` is the unit witness. The Copy half
(`AccC`, `prefix-copy`) is unaffected: a Copy local never enters `check_linear_across_back_edge`.

**No sanctioned edit fixes this.** The brief sanctions terms.rs only for *deletion*. The
fix is a relaxation of `check_linear_across_back_edge`: it gains a `frame_floor` parameter,
passed only at the self-tail combinator's splice site, that suppresses its second clause for a
local bound below the floor. The relaxation relocates a diagnostic; it does not add a
permission. See Decision P0 below for the argument: consumption of an enclosing linear local
is independently rejected by the existing capture-admission and end-of-scope guards, so the
clause was never a soundness guard, only the thing that located the rejection at the
back-edge instead of scope-end.

This is the linear analog of 6g's borrow-grant hole, and like 6g it also benefits `while`.
**Decision P0 (below) folds it into this slice as Phase 1.** Without it 10b cannot go green:
`inplace_fold.sth` (and its dependents in `phase4_slice6h`, `phase4_slice6h_fill_corpus`, and
the `qbe_baseline` corpus) stay red.

## Recon corrections (measured, against the deleted-intrinsic scratch tree)

The brief's own prose is wrong in several places it flagged as unverified, and in two it did
not.

1. **The intrinsic deletes cleanly and `back_edge_outs` survives.** Removing the interception
   (`terms.rs:326-458`, comment through the closing brace at 458), `check_abstract_quotation_times`
   (`terms.rs:1246-1278`), and the four diagnostics (`times_needs_quotation_error` through
   `times_body_row_effect_error`, `terms.rs:1312-1357`) leaves a warning-free `cargo build`.
   `back_edge_outs` (`terms.rs:1294`) sits between them and is **not** dead: it backs the
   self-tail marker (10a R11) and must stay. The four diagnostics are contiguous only if you
   skip `back_edge_outs`; delete around it.

2. **The lowering arm carries two unit tests the brief did not name.** Deleting the `calls.rs`
   `"times" =>` arm (`calls.rs:326-418`) also requires deleting
   `times_lowers_to_a_loop_header_not_a_per_iteration_call` (`calls.rs:782`) and
   `times_saves_and_restores_loop_state` (`calls.rs:840`), which call `lower_call("times", ...)`
   directly. Both are in `calls.rs`, a sanctioned file, so this is in scope, but the brief's
   "delete the arm" understates it.

3. **The test-file blast radius the brief enumerated is wrong in both directions.** The brief
   named eleven test files. Measured with `--no-fail-fast` on the deleted-intrinsic tree
   (before any test edits), the failing set is **11 targets, 55 tests**, and it does not
   match the brief's list:
   - **Two brief-named files do not fail at all**: `phase0.rs` (199 pass) and
     `phase4_slice10a_inline_quotation.rs` (22 pass). Their `times` mentions are in comments or
     in `my-times`-style user words, not bare intrinsic calls.
   - **Two failing targets the brief never named**: the crate's own unit tests (`--lib`, 3
     failures) and `phase4_slice6h_fill_corpus.rs` (1). Two of the three `--lib` failures are
     in files the brief did **not** sanction (`src/check/engine.rs`, `src/check/word_families.rs`).
   - `phase4_generics.rs` (12) and `phase4_slice6g.rs` (6) were named but unmeasured; now
     measured.

4. **The corpus enumeration is seven example files, plus `lib/arrays.sth` as an eighth
   (library, not example) file, not eight examples.** The seven: `array_ctor.sth`,
   `array_totals_hand.sth`, `combinator_in_times.sth`, `combinator_in_times_hand.sth`,
   `filter_while_hand.sth`, `inplace_fold.sth`, `times.sth`. `array_totals.sth` and
   `filter_while.sth` contain no bare `times` and need no import: their combinators wrap it
   internally. `examples/combinator_in_times.sth:20` imports combinators as `c` (qualified)
   but drives its outer loop with a bare `times`, which the intrinsic served, so it qualifies
   that one call to `c::times` rather than taking the selective import the other six examples
   take.

5. **Both unprobed bespoke diagnostics are still caught, reworded (Q1 resolved).** See below.

6. **The doorway grant survives the migration (6g relationship confirmed).** 6g's recon-4
   accept program (a `times` body mutably borrowing an array aliased by an outer name)
   compiles and prints `2` through the library `times`; the grant-withheld variant still
   rejects with the aliasing error. D1's `inline_combinator` grant makes the library-`times`
   splice grant exactly as the intrinsic doorway did.

## Resolved open questions

**Q1 (bespoke diagnostics re-point). Resolved: yes, both, reworded.** A `times` body that
consumes an enclosing linear local is rejected as
`` the quotation passed to `times` consumes the enclosing local `r`, which is linear; a quotation may only read a `Copy` enclosing local by value (D3) ``.
A body that leaves a borrow of an enclosing place live is rejected as
`` the quotation passed to `times` borrows the enclosing place `a`; a quotation may not capture a borrow of an enclosing local (D3) ``.
Both are the general capture-admission (D3) rejections, firing at the literal check before the
back-edge, not the retired `times_body_consumes_local_error` / `times_body_borrow_across_loop_error`
wording. Balanced borrow-and-release inside a body (each/map/filter's `&arr i &> @`) still
compiles: the rejection is for a captured (unbalanced) borrow, not a balanced read.

**Q2 (binary-size delta). Resolved: zero for the measured examples; method recorded.**
`examples/times.sth` builds to **17112 bytes both ways** (intrinsic and library), byte-identical,
output `500000500000`. `nm` on the library build shows only `main` / `sooth_main`: `times`
and `times-helper` mint **no** function symbols, they splice fully (combinators are inlined
per `combinators.sth`'s own design). Method for the exit criterion: build each times-using
example (recon 4's set) before and after, `stat -c%s` the emitted binary next to its source,
record the delta. Expected delta is `0`; a red flag is any KB-scale growth, which would mean a
per-iteration indirect call replaced the spliced constant-stack loop. (The `.ssa` baselines
*do* change, in internal value numbering, while machine-code size does not; see Q3's
`qbe_baseline` row.)

**Q3 (full fallout enumeration). Resolved: 11 targets, categorized.** The complete list,
with the edit each needs (measured, not inferred):

| Target | Fails | Nature / edit |
| --- | --- | --- |
| `--lib` `check::engine::times_typing_obligations` (`engine.rs:1572`) | 1 | intrinsic-only typing obligations; **retire the test** (engine.rs, **not sanctioned**) |
| `--lib` `check::word_families::quotation_as_operand_is_rejected_at_every_audited_site` (`word_families.rs:1154`) | 1 | remove the `times` audit row (`word_families.rs:1258`); reword two `"only call and times"` rows (**not sanctioned**) |
| `--lib` `ir::func_builder::calls::each_lowers_to_a_loop_not_a_per_element_call` (`calls.rs:1615`), plus its two siblings `calls::times_lowers_to_a_loop_header_not_a_per_iteration_call` and `calls::times_saves_and_restores_loop_state` | 3 | `each_lowers` needs inline `times`/`times-helper` defs (calls.rs, sanctioned); the other two call `lower_call("times", ...)` directly on the now-deleted arm and are deleted outright by R1 (recon item 2), not fixed by the sweep |
| `phase3_refs.rs` | 3 | inline bare `times`; add combinators import + reword any intrinsic-wording asserts |
| `phase4_combinators.rs` | 20 | bulk bare `times` (import) + intrinsic-diagnostic asserts (reword) + dogfoods (pass once imported); the 2 REPL-import tests need the `times-helper` export (Q4) |
| `phase4_generics.rs` | 12 | inline bare `times`; import / reword |
| `phase4_quotations.rs` | 2 | inline bare `times`; import / reword |
| `phase4_slice6f.rs` | 2 | both tests (`inplace_fold_copy_lowers_without_per_iteration_blit`, `inplace_fold_linear_lowers_without_per_iteration_blit`) call `run_dogfood` first, which goes through `common::build_example` and the driver and already asserts exact stdout; each then calls `fold_body`, which does `check::check` in-process on `include_str!`'d dogfood source, so the added `import:` line never resolves there, and the failure is `` unknown word `times` in `prefix-copy` ``. Fix `fold_body` by routing it through `sooth::driver::emit_ssa(path)` (what `tests/qbe_baseline.rs:66` uses); under `emit_ssa` the emitted names gain a module suffix (`export function :AccC $prefix.2d.copy__m0(...)`), so its `` format!("${word}(") `` lookup must change to match. The substantive assertions (`phi`, two `jmp @blk1`, `storel`, no `blit`) survive unchanged, so the no-blit claim stays non-vacuous; do not "fix" this by loosening the blit assertion into a placebo. Once fixed, these two tests are an exact-stdout-plus-structure witness for the P0 shape (the linear one directly). **This is a category, not one file**: every test whose in-process half checks a corpus file via `lex`/`parse`/`check` directly, as `fold_body` does, behaves this way |
| `phase4_slice6g.rs` | 6 | doorway-grant tests, inline bare `times`; add combinators import (grant verified to survive) |
| `phase4_slice6h.rs` | 1 | builds the corpus (`inplace_fold` etc.); fixed by corpus imports, **linear case gated on P0** |
| `phase4_slice6h_fill_corpus.rs` | 1 | corpus stdout baseline; fixed by corpus imports, **gated on P0** |
| `phase4_slice10a_exit_witnesses.rs` | 2 | resolved per Decision 4 below |
| `qbe_baseline.rs` | 1 | regenerate `.ssa` for the times-using corpus (sanctioned baseline diff) |

**These counts are a measurement snapshot, not a gate.** They were taken on a tree carrying the
prototype's `examples/` hunk (so `inplace_fold.sth` already has its import). An independent
re-measurement on a *bare* deletion-only tree agreed on every row except `phase4_generics`,
which came out at 11 rather than 12, for a total of 54 (55 with the relaxation). Every other
per-target count matched exactly both times, and the qualitative claim (the relaxation adds
exactly one failure and fixes none) was confirmed target by target both times. Phase 2 must
**re-measure at entry** rather than treat any of these numbers as a target; the exit criteria
are deliberately count-independent ("every failing target listed in Q3 is green").

Row counts sum to 55 (5 + 3 + 20 + 12 + 2 + 2 + 6 + 1 + 1 + 2 + 1), across 11 unique targets
(13 rows, three of them `--lib`). The measured 55 already assumes `times-helper` is exported
(Decision 2, delivered alongside R2 in the intrinsic-deletion phase, not a separate "P1"
step): without the export, the two REPL tests (`repl_imported_filter_runs`,
`repl_combinators_dogfood_matches_native`, both in `tests/phase4_combinators.rs`, not a
REPL-specific file) add 2 more failures, for 57. `tests/phase4_repl_imports.rs` stays 23/23
green either way. The `arrays.sth` import clears the sort dogfood, and the rest are the
table's edits.

Re-measured with the Phase 1 relaxation in place: 56 tests, still across 11 targets. The
relaxation changes the failing set by exactly one test, in the direction of *adding* one
(`while_body_linear_local_across_back_edge_is_error`, the deliberate re-point, see P0) and
fixing none. That it fixes none is itself a finding: the shape it enables is pinned by no
working test today, because `phase4_slice6f.rs`'s two tests' `fold_body` calls cannot even
reach the checker (this table's `phase4_slice6f.rs` row). Phase 1 must therefore add its own
golden.

That 56 and Sequencing's 55 are the same measurement seen from two points in the delivery
order, not a contradiction: 56 is the deletion tree with the relaxation added on top, while
Phase 1 lands *first* and re-points the one extra test, so the number actually observed on
entering Phase 2 is 55 again. Do not read the difference as a regression.

**Q4 (REPL). Resolved: `times-helper` must be exported.** Two REPL tests
(`repl_imported_filter_runs`, `repl_combinators_dogfood_matches_native`) fail with
`` unknown word `c::times-helper__import0` ``. The REPL's dlopen import path retains only a
module's *exported* words; a private helper reached transitively through an imported
combinator's spliced body is unresolvable. Exporting `times-helper` turns both green
(verified). This **overrides the brief's decision 2** ("`times-helper` is not exported"),
which was written without the REPL constraint. The cleaner long-term fix (the REPL retains
private words transitively reachable from an exported combinator) is a `src/repl.rs` change,
kept out of scope exactly as 6g's D5 kept repl.rs out; exporting the helper is the minimal,
explicit, green-making move and leaves repl.rs untouched. The third REPL failure
(`repl_two_output_combinator_define_and_call`) is unrelated: it *defines its own* `filter`
inline with bare `times`, so it needs a test edit (import or `c::times`), not a mechanism
change.

**Q5 (arrays.sth). Resolved: a library-to-library selective import, and it works.** No lib
file imports another today, so it is unprecedented, but adding
`import: c | times | "combinators.sth" ;` (relative to `arrays.sth`'s own directory) to the
top of `lib/arrays.sth` compiles, and 6g's `sort_called_with_bound_array_locals_runs` dogfood
passes and sorts (`1 2 3 4`). The test harness imports `arrays.sth` by its real absolute path
(`CARGO_MANIFEST_DIR`), so the nested relative sub-import resolves against the real
`lib/combinators.sth`. Prefer the selective import over duplicating `times` into `arrays.sth`:
one definition, and it matches decision 3's "add an import, not a rewrite".

**Q6 (times-helper exercises 10a decisions 3/5). Resolved, and it is the blocker.** Yes:
`times-helper` is an ordinary self-tail combinator, so its splice runs the full self-tail
machinery, including `check_linear_across_back_edge` and `check_reference_across_back_edge`
(10a R13) and the ground-declared-outputs back-edge arm (10a R11/R14). Exercising them is
precisely what surfaces the linear-local regression above. The aggregate-carrying and
constant-stack guarantees (10a decisions 3/5) hold for the real `times`: `array_totals.sth`
(a `map` carrying the array through the row) prints identically, `combinator_in_times.sth` (a
combinator nested in an outer `times`) prints identically, and `0 1000000 [ drop 1 + ] times`
completes at `ulimit -s 1024`, exit 0.

## Decisions

**P0 (new, forced by the blocker, and this slice's Phase 1). `check_linear_across_back_edge`
gains a `frame_floor: Option<usize>` parameter. Its second clause (any unconsumed linear local
in scope) skips a local bound below the floor.** `base_depth` is captured per
`check_terms_relaxed` invocation (`src/check/terms.rs:73`), and `TermKind::If` re-enters
`check_terms_relaxed` per arm (`terms.rs:856`, `:884`); every real self-tail combinator's
back-edge sits inside an `if` arm. So the floor passed at the combinator site is the **if-arm
entry depth**, not a frame floor: "bound below the floor" does not mean "lives in an ancestor
frame". It also exempts a local bound in the combinator's own frame *before* the tail-`if`
(`times-helper`'s own parameters, for instance), which is re-created every iteration and has no
ancestor-frame lifetime at all. The `terms.rs` sanctioned edit widens from delete-only to
include this relaxation.

**Only the combinator-splice call site passes a floor.** `Some(base_depth)` at the self-tail
*combinator* site (`terms.rs:615`, the marker path `while`/`times-helper` take); `None`, i.e.
today's behaviour, at the whole-word TCO site (`terms.rs:787`). This is load-bearing, not
tidiness: a plain recursive word has no shape below the floor to admit, because its self-call
must supply its full declared inputs, so a linear is either forwarded as an argument (already
legal by design, per the function's own doc comment) or genuinely stranded beneath them
(clause 1). Passing a floor at both sites is the change that would actually open a hole.

**Why this is sound: it relocates a diagnostic, it does not add a permission.** Deleting the
combinator-site `check_linear_across_back_edge` call outright changes zero test outcomes
across the whole suite, and every hazard probe below still rejects (measured against the
prototype, call deleted): an own-frame linear bound before the tail-`if` is still rejected by
end-of-scope disposal, byte-identical message; a linear bound inside the `if` arm is still
rejected, but the message degrades from the back-edge wording to the scope-end wording; a
parked ancestor linear is still accepted. So clause 2 at the combinator site is **not** a
soundness guard. Disposal of an enclosing linear local is independently enforced by
end-of-scope disposal and the branch-join "not consumed on every path" guard (`MaybeMoved`),
and a self-tail call has no position after it, so an own-frame linear has nowhere to be
disposed except before the back-edge. Clause 2's only job is to *locate* that same rejection at
the back-edge ("consume it before the recursive call") instead of the vaguer scope-end
wording. `frame_floor` suppresses the clause precisely where that locating job produces a
**false rejection**: a local bound below the if-arm entry depth that the loop neither rebinds
nor carries, which covers both a genuine ancestor-frame local and, wrongly but harmlessly, the
combinator's own pre-`if` locals (see above). `check_reference_across_back_edge`
(`src/check.rs:1264`) draws an analogous ancestor-frame distinction for references, and the
function is worth knowing about, but it is not the argument for P0: references carry no
disposal obligation, and the distinction that function needs is about a reference's
*referent* frame, which is orthogonal to where a linear *value* sits relative to an `if`-arm.
P0 stands on the mutation evidence below, not on that parallel.

| Hazard | Probe | Still rejected by |
| --- | --- | --- |
| body consumes the enclosing linear inside the loop (double-consume per iteration) | `[ ... acc drop ... ] c::while` | capture-admission D3: `` the quotation passed to `while` consumes the enclosing local `acc`, which is linear `` |
| consumed on one branch only (`MaybeMoved` across the edge) | `[ ... n 2 > if acc drop else end ... ]` | capture-admission D3, same message (both hazards share one match arm at `src/check.rs:1385`) |
| linear bound inside a spliced quotation body, unconsumed at the edge | `[ ... 5 AccL \| tmp \| ... ]` | end-of-scope disposal: `` linear value `tmp` is never consumed ``, located at the quotation's own scope end, which closes before the back-edge check ever runs: this hazard gets the scope-end message whether clause 2 is present or not |
| enclosing linear never disposed at all | `work` binds `acc`, loops, returns nothing | end-of-scope disposal: `` drop it or return it `` |

All four are pre-existing guards, not P0's own code: they were true before P0 and stay true
after it. None leans on the relaxed clause; they are kept as reject goldens because they are
correct and worth pinning, not because they guard the relaxation.

Corrected in Phase 1, measured: three of the four are indifferent to P0, but the fourth is
not. "Enclosing linear never disposed at all" is the accept golden minus its disposal, so
neutering the floor makes the back-edge clause reject the same program one step earlier, with
the back-edge wording instead of `` drop it or return it ``. Its golden asserts the scope-end
wording (that being what the guard actually says), so R9's mutation (a) flips it too. Also
measured: hazard row 1's probe must put the consume *outside* the body's own `if`, or the
local reaches the literal's exit `MaybeMoved` rather than `Moved` and the row collapses into
row 2, flipping under mutation (c) as well.

**Blast radius, measured.** Full `cargo test --no-fail-fast` on the prototype, then the same
run with only the relaxation reverted, diffing the failure sets: exactly one test is red only
with the relaxation, and none is fixed by it.

**That one test is a deliberate re-point, and it must be reviewed as one, not silently edited.**
`while_body_linear_local_across_back_edge_is_error` (`tests/phase4_combinators.rs:962`) binds
an outer `Spy`, never touches it inside `while`'s predicate, and disposes it on the very next
line (`sp drop`). That is the parked-with-a-disposer shape, the same one `inplace_fold.sth`
needs and the intrinsic allowed. Its own comment justifies itself with "it would ride into the
next iteration with nobody to dispose it", which is factually false about its own program. It
pins the over-approximation, so Phase 1 re-points it to a genuinely different shape: a
test-local self-tail combinator whose own body binds a linear *inside the tail-`if` arm* and
reaches the back-edge with it unconsumed (not a linear bound inside a spliced quotation
argument, which closes its own scope first and would only ever produce the scope-end message,
never the kept assertion). The current test asserts exactly three substrings: `` `Spy` ``,
`` `while` ``, and `` live across the self-tail-call back-edge ``; it does not assert
`` linear values across a loop are not supported yet ``. The message names the *callee*, so
the assertion survives unchanged only if the re-pointed test's own self-tail combinator is
named `while`: measured, with that name and the combinator-site call present, the message
contains all three substrings unchanged. Naming the test-local combinator anything else (e.g.
`myloop`) yields `` is live across the self-tail-call back-edge to `myloop` ``, and the
existing `err.contains("`while`")` check then fails; if a different name is chosen for other
reasons, the callee substring in the assertion must be updated to match it. With the
combinator-site call deleted, the message degrades to `` linear value ... is never consumed
``, dropping the back-edge substring regardless of the combinator's name. It is therefore a
genuine mutation witness, but of **diagnostic location**, not of leak prevention: nothing here
would leak if clause 2 were absent, since the same guards in the hazard table would still
catch it, one step later and with a worse message. The spec drops any "load-bearing" framing
that implies this test prevents a leak.

**A permanent reject golden covers the shape the floor wrongly exempts.** An own-frame linear
bound in the combinator's own frame *before* the tail-`if` (the case the floor treats as
ancestor-like even though it is re-created every iteration) must stay pinned as a reject,
asserting the scope-end message. The over-exemption is harmless only because end-of-scope
disposal and the branch-join guard independently catch it; this golden is the tripwire if
either of those two co-guards is ever widened or removed.

**Phase 1 owns its own golden.** The relaxation fixes zero existing tests, and the shape it
enables is pinned by nothing today (`phase4_slice6f.rs`'s two tests' `fold_body` calls cannot
reach the checker at all, see Q3). Without a new golden the capability ships unguarded, which
is this project's documented placebo pattern. Phase 1 adds: an accept golden for the
parked-linear shape over a library-style self-tail combinator *and* over `while` (both, since
P0 changes `while` too), each asserting the value; the re-pointed if-arm reject golden above
(diagnostic location, not leak prevention); the new own-frame-before-the-tail-if reject golden
(the tripwire); and the four pre-existing hazard rejections, correctly labelled as pinning
pre-existing guards rather than P0. Every reject golden asserts on message **wording**, never
line numbers (a library-spliced rejection's span points into `lib/combinators.sth`, while the
message names the caller) and never a bare local name (a spliced local is reported mangled,
e.g. `tmp__inl0`). All of it mutation-audited per R9.

1. **`times` is a thin public wrapper over a private self-tail-recursive `times-helper`
   carrying a from/to pair.** Verified end to end:

   ```sooth
   : times-helper ( ..s i64 i64 ~[ ..s i64 -- ..s ] -- ..s )
     | f | | to | | from |
     from to < if
       from f call
       from 1 + to f times-helper
     else
     end ;

   : times ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s )
     | f | | n | 0 n f times-helper ;
   ```

   `0 5 [ + ] times` sums to `10`; `0 1000000 [ drop 1 + ] times` runs at constant stack. The
   `~[ ... ]` parameter is 10a's inline-only quotation type; a literal `[ ... ]` argument
   splices against it exactly as `my-times` proved.

2. **`times` **and** `times-helper` are exported from `lib/combinators.sth`.** This overrides
   the brief's decision 2 (Q4): the REPL cannot resolve a private transitively-spliced helper.
   Export list becomes `export: each map fold filter while times times-helper ;`.

3. **Every bare-`times` corpus file gets an import; `arrays.sth` gets a library-to-library
   selective import.** Seven example files need it: `array_ctor.sth`, `array_totals_hand.sth`,
   `combinator_in_times.sth`, `combinator_in_times_hand.sth`, `filter_while_hand.sth`,
   `inplace_fold.sth`, `times.sth` (`array_totals.sth` and `filter_while.sth` contain no bare
   `times` and need none). Examples use `import: c | times | "../lib/combinators.sth" ;`
   (selective, so the existing bare `times` calls resolve unqualified);
   `combinator_in_times.sth`, which already imports `c` qualified, qualifies its one bare call
   to `c::times` instead. `lib/arrays.sth` gets `import: c | times | "combinators.sth" ;`, an
   eighth file, but a library-to-library import (Q5), not an example.

4. **`tests/phase4_slice10a_exit_witnesses.rs` is resolved test by test.** Exactly two of its
   tests fail: `my_times_compiles_beside_the_untouched_intrinsic_and_sums` (its "untouched
   intrinsic" premise is retired, and its `my-times` half is subsumed by 10b's own goldens on
   the real `times`) is **deleted**; `combinators_library_contains_no_tilde` (false by design
   now that `times`/`times-helper` carry `~[ ... ]`) is **inverted** into a positive assertion
   that `combinators.sth` declares exactly the two expected `~` signatures, and that the file's
   total `~` count is exactly 2 (otherwise a stray `~` in a prose comment would satisfy it).
   The other five tests (`my_times_runs_one_million_iterations_in_constant_stack`,
   `row_grounding_accepts_a_borrow_of_an_unrelated_place_of_the_same_type`,
   `my_times_carries_an_aggregate_without_aliasing`,
   `my_times_nested_in_itself_produces_correct_output`,
   `while_is_unaffected_by_the_row_and_back_edge_rewrite`) use the self-contained `my-times`
   user word, are already independent of the intrinsic, and stay unchanged: they remain
   coverage of the `~`/row mechanism in its own right.

5. **The four dead diagnostic functions are deleted outright**, and the general capture-admission
   path (Q1) already produces a rejection for each of the three bug classes they messaged.

6. **Corpus stdout and exit codes stay byte-identical** everywhere `times` still compiles.
   Verified for all eight files that still compile unchanged, i.e. every times-using corpus file
   other than `inplace_fold` (`array_totals`, `array_totals_hand`,
   `combinator_in_times`, `combinator_in_times_hand`, `filter_while`, `filter_while_hand`,
   `array_ctor`, `times`). `inplace_fold` is the one exception, and only until Phase 1's
   relaxation lands within this slice; with it, `inplace_fold` builds and prints
   `1 3 6 10 1 3 6 10`, byte-identical to the intrinsic baseline (verified against the
   prototype).

7. **The three stale `` "only `call` and `times`" `` diagnostics in `src/check.rs`
   (`:841`, `:1674`, `:1689`) are corrected to drop `and`times``.** After 10b, `times` is an
   ordinary imported word, not a builtin that accepts a quotation, so the message is
   factually wrong. This touches `check.rs` (not brief-sanctioned) and the two matching
   assertion rows in `word_families.rs`. The minimal-green alternative (leave the strings,
   only delete the `times` audit row) ships a misleading diagnostic; per the project's "sharp
   errors are the point", correct them.

**R10 (new, folded into Phase 1 alongside P0; a pre-existing gap, reproducible at HEAD via
`c::while`, needing no intrinsic deletion). A quotation body that disposes a linear via a
user-declared `drop` overload compiles under the `times` intrinsic but is rejected under any
library-spliced combinator**, with `` cannot `drop` a value of type `X` `` plus the
transitive-import note, even when the type and its `drop` are declared in the caller's own
file. Cause: `check_drop_import_visibility` (`src/check/word_families.rs:756-768`) gates on
`ctx.module()`, which for a spliced body is the **splice destination** (`lib/combinators.sth`),
not the module where the quotation literal was written. `ast::Span` already carries a `module`
field (`src/ast.rs:11-15`). Fix: gate on `span.module` instead of `ctx.module()`; `span.module`
equals `ctx.module()` everywhere except under splicing, where it is strictly more correct.
**Both lines must change, not just the gate.** `drop_import_visibility_error`
(`src/check/word_families.rs:775`) also derives its `caller` from `ctx.module()`, and would
then disagree with the gate that admitted or rejected the term. Today that disagreement is
masked only by evaluation order: a home-scope pass runs first, where the two agree. If any
quotation shape is ever checked at a splice site with no prior home-scope pass, the qualifier
lookup runs against `lib/combinators.sth`'s import map and emits the fabricated-transitive note
for what is really a plain qualified-only import. One line each.
Verified: the failing case then compiles and runs; and a real two-file multi-module program
using a qualified-only import still rejects with its exact expected diagnostic
(`` cannot `drop` a value of type `lib::Res` in `main` `` plus the "has not imported by name"
note), so the rule stays intact. Two unit tests flip, and they are a harness artifact, not a
hole: `drop_res` (`src/check/engine.rs:1324-1341`) hardcodes `span.module: 0` while varying
`caller` (1 or 2), so the synthetic span contradicts what a real build produces; the fix is to
set the synthetic span's module to `caller` in `drop_of_qualified_only_imported_type_is_error`
and `drop_of_transitively_reachable_type_with_no_direct_import_is_error`. R10 needs its own
golden: a library-spliced combinator body that disposes a locally-declared linear through its
own `drop` overload compiles and runs with exact expected stdout. It also needs a **reject**
golden for the other side of the rule, verified constructible: a spliced quotation body that
disposes a *qualified-only-imported* type through a combinator declared in that type's own
module. That is the exact shape where naively trusting the splice destination would over-admit
(the destination module *can* see the destructor, the authoring module cannot), and nothing
else pins it. R9's mutation (d) is R10's discriminating mutation.

Out of scope, recorded because it frames how much the rule is worth: the visibility gate never
fires at all for a disposal inside a **generic** word body. A library exporting
`: dispose ( 'T -- ) drop ;`, called on a caller-declared linear, produces zero gate hits and
runs the destructor. That predates R10 and is not a splice artefact, so it is not fixed here,
but it means the rule is import hygiene rather than a soundness guarantee, which is the right
frame for judging R10's relaxation.

## Requirements

**R1**. Delete the intrinsic: the `check_term` `if name == "times"` interception
(`terms.rs:326-458`), `check_abstract_quotation_times` (`terms.rs:1246-1278`), the four
diagnostics (`terms.rs:1312-1357`, working around `back_edge_outs` at `:1294`, which stays),
and the `calls.rs` `"times" =>` arm (`:326-418`) with its two unit tests (`:782`, `:840`).
`cargo build` warning-free afterward proves the four functions were the only dead code.

**R2**: Add `times`/`times-helper` to `lib/combinators.sth` per Decision 1, both on the
`export:` line per Decision 2.

**R3**: Add the corpus imports per Decision 3 (seven example files plus `lib/arrays.sth`,
eight files in total: the seven examples that contain a bare `times`, plus `lib/arrays.sth`).
Note this is a different set of eight from Decision 6's, which counts the corpus files whose
output is verified unchanged. Stdout and exit code stay byte-identical (Decision 6).

**R4**: Resolve `phase4_slice10a_exit_witnesses.rs` per Decision 4.

**R5**: Sweep the remaining test targets per the Q3 table: import for bare-`times` sources,
reword for intrinsic-diagnostic asserts, retire `engine.rs`'s `times_typing_obligations`,
remove `word_families.rs`'s `times` audit row, fix `calls.rs`'s `each_lowers` test with inline
`times`/`times-helper` defs (its two siblings, `times_lowers_to_a_loop_header...` and
`times_saves_and_restores_loop_state`, are deleted outright by R1, not fixed here), and correct
the stale `check.rs` diagnostics per Decision 7. `phase4_slice6f.rs`'s `fold_body`-based halves
check a corpus file **in process** via lex/parse/check/lower/emit, so an `import:` line in the
`.sth` file never resolves there; fix by routing `fold_body` through `sooth::driver::emit_ssa`
(what `tests/qbe_baseline.rs:66` uses), adjusting its `` format!("${word}(") `` lookup for the
module-suffixed emitted name. Its `run_dogfood` half already goes through the driver and needs
only the import. Audit for the `fold_body` pattern elsewhere rather than assuming
`phase4_slice6f.rs` is the only instance.

**R6**: Regenerate the `qbe_baseline` `.ssa` for the times-using corpus (a sanctioned
baseline diff). Machine-code size is unchanged (Q2); the `.ssa` differs only in value
numbering. Mechanism: `REGEN_QBE_BASELINE=1 cargo test --test qbe_baseline` (documented at
`tests/qbe_baseline.rs:8`).

**R10**: The disposal-visibility splice fix, specified in full as Decision R10 above (both
`check_drop_import_visibility`'s gate at `src/check/word_families.rs:756-768` and
`drop_import_visibility_error`'s caller derivation at `:775` move from `ctx.module()` to
`span.module`; the two `engine.rs` synthetic spans take `module: caller`; an accept golden, a
reject golden, and R9's mutation (d)). Delivered in Phase 1 alongside P0.

**R11**: Correct the prose that still calls `times` a compiler intrinsic, since this project
keeps ROADMAP and DESIGN as current-state documents rather than history: `README.md:54` and
`:61`, `ROADMAP.md:304` ("The one compiler-known intrinsic, `times`") and `:308-312`,
`DESIGN.md:352-357` and `:513`. After 10b there is no compiler-known intrinsic at all, so the
claim is not merely stale but false.

**R7 (exit witnesses on the real `times`, not `my-times`)**: new goldens, each an
`assert_eq!` on full stdout plus exit code, never a bare "runs" or "produces correct values"
claim, living in `tests/phase4_slice10b.rs` (new file):

- `0 5 [ + ] times`: `assert_eq!` stdout `10`, exit `0`.
- `0 1000000 [ drop 1 + ] times`: `assert_eq!` stdout `1000000`, exit `0`, at `ulimit -s 1024`,
  following `tests/phase4_slice10a_exit_witnesses.rs:98-102`'s `(Some(0), "1000000")` pattern
  (checking only exit code 0 is a placebo: a loop that runs zero iterations also exits 0).
- an aggregate carried through the row (a `map`-shaped body): `assert_eq!` on its full stdout.
- a `times` call nested inside `each`/`map`/`fold`/`filter` (the newly doubled splice depth)
  and a `times` nested inside an outer loop: `assert_eq!` on full stdout for each. This is the
  most novel risk beyond 10a and needs its own golden, not reliance on the library suite
  passing.
- the binary-size delta across the corpus is measured and recorded per Q2.

**R8 (Phase 1 / P0 witnesses, in this slice)**: `check_linear_across_back_edge` takes
`frame_floor`, passed `Some(base_depth)` at the combinator site and `None` at the whole-word
TCO site (P0). `inplace_fold.sth`'s `prefix-linear` compiles again and prints exactly
`"1\n3\n6\n10\n1\n3\n6\n10\n"` with exit code 0. That assertion already exists and does not need
writing: `tests/phase4_slice6f.rs:81` and `:98` both assert it via `run_dogfood` ->
`common::build_example` -> the driver, so it is an exact-stdout enforcer, not a "it runs" check.
Phase 1's own goldens, in `tests/phase4_slice10b.rs` (new file): an accept for the
parked-linear shape over both a library-style self-tail combinator and `while`, asserting the
value; `while_body_linear_local_across_back_edge_is_error` re-pointed to the if-arm shape per
P0, with its test-local self-tail combinator **named `while`** so the existing three-substring
assertion survives unchanged (a diagnostic-location witness, not a leak-prevention one); a new reject golden for the
own-frame-before-the-tail-if shape the floor wrongly exempts, asserting the scope-end message
(the tripwire for end-of-scope disposal and the branch-join guard); and the four pre-existing
hazard rejections from P0's table, correctly labelled as pinning those guards, not P0. Every
reject golden asserts on message wording, never a line number (a library-spliced rejection's
span points into `lib/combinators.sth`, not the caller) and never a bare local name (a spliced
local is reported mangled, e.g. `tmp__inl0`).

**R9**: Mutation-test each new guard and each reworded rejection, in a throwaway copy of the
tree (never the shared worktree; keep `target/` copies off the `/tmp` tmpfs). Exactly four
mutations are required:

- (a) neuter `frame_floor` (make it always `None`): the parked-linear accept goldens go red.
  **This mutation is not exclusive to them**: it also flips the own-frame-before-the-tail-`if`
  tripwire golden and the "enclosing linear never disposed at all" hazard golden, both of
  whose messages change from the scope-end wording to the back-edge wording. Expect all four,
  or the audit will read as a failed prediction. (Measured in Phase 1: exactly those four,
  suite-wide.)
- (b) delete the combinator-site call at `terms.rs:615`: the re-pointed if-arm diagnostic
  witness goes red (degrading to the scope-end message). Nothing else moves.
- (c) remove `MaybeMoved(_)` from the match arm at `src/check.rs:1385`: the branch-only-consume
  hazard golden goes red. Note what this mutation does **not** do: it does not admit the
  hazard. Measured, the program is still rejected, with `` use after move `` instead of the D3
  capture-admission message. That golden is therefore a **wording** witness, exactly as the
  re-pointed test is a diagnostic-location witness, and it must assert the D3 wording. Written
  as `is_err()` or "still rejects" it would pass under this mutation and be a placebo.
- (d) revert R10's gate to `ctx.module()`: R10's accept golden goes red (measured: the spliced
  disposal is rejected again).

## Sanctioned files, and the scope this spec adds

Brief-sanctioned: `src/check/terms.rs` (deletions **and P0's relaxation**, per Decision P0),
`src/ir/func_builder/calls.rs` (arm deletion, and its two unit tests, R1),
`lib/combinators.sth` (R2, plus `times-helper` on the export line), the recon-4 corpus files
(R3), `tests/phase4_slice10a_exit_witnesses.rs` (R4).

Investigation forces these additions the brief's file list omits, each stated so the
maintainer can veto:

- `src/check/engine.rs` and `src/check/word_families.rs`: unit tests coupled to the intrinsic
  (retire / trim, R5), and, separately, R10's `check_drop_import_visibility` fix (gate on
  `span.module`) plus the two `engine.rs` tests whose synthetic span hardcodes `module: 0`.
  Unavoidable: the suite cannot go green while they assert intrinsic-only behaviour, and
  R10's splice-gate fix is a correctness fix, not a green-only one.
- `src/check.rs`: the three stale `` "only `call` and `times`" `` diagnostics (Decision 7).
  Correctness, not just green.
- `lib/arrays.sth`: the library-to-library import (Q5). The brief mentions arrays.sth in recon
  4 but as an example-style import; it is a library and needs the lib-to-lib form.
- `tests/phase4_combinators.rs`: the `while_body_linear_local_across_back_edge_is_error`
  re-point (P0). Called out separately from R5's bulk sweep because it is a criterion change,
  not an import fix.
- `tests/phase3_refs.rs`, `tests/phase4_generics.rs`, `tests/phase4_quotations.rs`,
  `tests/phase4_slice6f.rs`, `tests/phase4_slice6g.rs`, `tests/phase4_slice6h.rs`,
  `tests/phase4_slice6h_fill_corpus.rs`, `tests/qbe_baseline.rs` and its
  `tests/qbe_baseline/*.ssa` snapshots: R5's and R6's sweep targets, named explicitly so this
  section actually vetoes what it touches.
- `tests/phase4_slice10b.rs` (new file): the home for R7's, R8's, and R10's new goldens.
- `README.md`, `ROADMAP.md`, `DESIGN.md`: still describe `times` as a compiler intrinsic
  (`README.md:54`, `:61`; `ROADMAP.md:304`, `:308-312`; `DESIGN.md:352-357`, `:513`), which is
  now current-state-wrong; the project keeps these as current-state documents, so they are
  corrected as part of this slice, under **R11**, and verified by its exit criterion.

## Sequencing

After 6g (merged `86aee0a`). Independent of 10c. `ROADMAP.md:608`'s "Next action" already names
10b/10c and needs no correction: P0 is Phase 1 of this slice, not a separate entry. Phase 1
must land and be green before Phase 3's corpus imports, since `inplace_fold.sth` cannot compile
without it. Phase 1 is independently verifiable: it is green against the *unmodified*
intrinsic, so the relaxation is reviewed on its own evidence before anything is deleted;
applying only P0 to HEAD leaves exactly one failing test, the one Phase 1 re-points.

Phase 1's real entry state is that one P0 failure **plus two** from R10, which flips
`drop_of_qualified_only_imported_type_is_error` and
`drop_of_transitively_reachable_type_with_no_direct_import_is_error` until its harness fix
(the synthetic span's `module: caller`) lands in the same phase. Three failures on entry, zero
on exit; do not read the extra two as a regression.

The suite is **red by design** through Phases 2 and 3: the intrinsic-deletion tree alone fails
55 tests over 11 cargo targets (Q3), and that count does not clear until Phase 4's sweep
lands. Green is restored only at Phase 4, not incrementally across Phases 2 and 3.

## Exit criteria

10b exits when: the intrinsic (interception, `check_abstract_quotation_times`, four
diagnostics, lowering arm and its two unit tests) is gone and `cargo build` is warning-free
(R1); `times`/`times-helper` are ordinary exported Sooth source (R2); every bare-`times`
corpus file compiles with an added import and prints byte-identical stdout and exit code,
`arrays.sth` via a library-to-library import (R3, Decision 6); `phase4_slice10a_exit_witnesses.rs`
is resolved test by test (R4); every failing target listed in Q3 is green, including the `engine.rs`
/ `word_families.rs` / `calls.rs` unit tests and the corrected `check.rs` diagnostics (R5);
the times-using `qbe_baseline` `.ssa` is regenerated (R6); the real-`times` goldens pass
(1M constant stack with the printed value asserted, aggregate-through-row, times nested in a
combinator and in an outer loop), each an exact stdout-plus-exit-code assertion, and the
binary-size delta is recorded (R7); `check_linear_across_back_edge` takes a frame floor passed
only at the combinator site, `inplace_fold.sth`'s linear half compiles and prints
`1 3 6 10 1 3 6 10`, Phase 1's parked-linear accept goldens pass over both a library self-tail
combinator and `while`, the four pre-existing hazard rejections hold (pinning those guards, not
P0), the own-frame-before-the-tail-if reject golden holds, and
`while_body_linear_local_across_back_edge_is_error` is re-pointed to the if-arm shape rather
than dropped (R8); a library-spliced combinator body disposes a locally-declared linear
through its own `drop` overload, both `check_drop_import_visibility`'s gate and
`drop_import_visibility_error`'s caller derivation use `span.module`, the qualified-only-import
rejection stays intact end to end, and the spliced qualified-only reject golden holds (R10); no
prose in README, ROADMAP or DESIGN still calls `times` an intrinsic (R11); and all four R9
mutations have been run, each flipping the goldens R9 predicts and no others (R9).

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "P0: check_linear_across_back_edge gains frame_floor, Some(base_depth) at the combinator site and None at the whole-word TCO site; parked-linear accept goldens over a library self-tail combinator and over while; the re-pointed if-arm reject golden for while_body_linear_local_across_back_edge_is_error, whose test-local self-tail combinator must be NAMED while so the existing three-substring assertion survives unchanged (diagnostic location, not leak prevention); a new reject golden for the own-frame-before-the-tail-if shape the floor exempts; the four pre-existing hazard rejections, labelled as pinning those guards, not P0. R10: BOTH check_drop_import_visibility's gate and drop_import_visibility_error's caller derivation move from ctx.module() to span.module, fixing the drop-overload splice gap, plus an accept golden, a reject golden for a qualified-only-imported type disposed in a spliced body, and the two engine.rs tests whose synthetic span hardcodes module: 0. NO intrinsic deletion in this phase: it is green against the unmodified intrinsic, and its entry state is three failing tests (one P0 re-point, two R10 harness). Reference: docs/phase4-slice10b-p0-prototype.patch, but take ONLY its three check_linear_across_back_edge hunks; the patch's other src/ hunks are the Phase 2 intrinsic deletion and its lib/ and examples/ hunks are Phases 2 and 3, and it does not contain R10 at all", "difficulty": "hard" },
    { "phase": 2, "focus": "delete the intrinsic (interception, check_abstract_quotation_times, four dead diagnostics, calls.rs lowering arm and its two unit tests) and add exported times/times-helper to lib/combinators.sth; the suite is red by design from here through Phase 3 (55 failing tests over 11 targets measured on the deletion alone)", "difficulty": "hard" },
    { "phase": 3, "focus": "corpus imports across the seven bare-times example files and arrays.sth library-to-library import, with byte-identical stdout including inplace_fold; still red by design, green restored only at Phase 4", "difficulty": "standard" },
    { "phase": 4, "focus": "test-suite sweep across the remaining targets (re-measure the Q3 counts at entry rather than trusting them) including the in-process-check category, slice10a witness resolution, stale check.rs diagnostic correction, qbe baseline regeneration via REGEN_QBE_BASELINE=1, real-times exit witnesses each asserting exact stdout and exit code, R11's README/ROADMAP/DESIGN prose correction, and R9's four-mutation audit; restores green", "difficulty": "hard" }
  ]
}
```
