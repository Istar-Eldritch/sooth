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

A working prototype of the whole thing (intrinsic deletion, the library word, and the Phase 1
relaxation) is committed beside this spec as
[`phase4-slice10b-p0-prototype.patch`](./phase4-slice10b-p0-prototype.patch). It builds, and
the soundness evidence in the P0 section below was measured against it. Treat it as reference,
not as the delivery: it carries no tests and none of the corpus or test-suite sweep.

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
`check_linear_across_back_edge` (`src/check/terms.rs:477`), whose second clause flags **any**
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
fix is a relaxation of `check_linear_across_back_edge` so a spliced self-tail combinator's
back-edge check scopes to the combinator's own frame (`>= base_depth`) rather than the whole
enclosing scope. That relaxation is sound because consumption of an enclosing linear local is
separately rejected already (a `times`/`while` body that *consumes* `acc` still fails, see
Q1 below): the only thing the relaxation newly admits is an enclosing linear local the body
merely parks and never touches, which the loop provably cannot re-enter or double-dispose.

This is the linear analog of 6g's borrow-grant hole, and like 6g it also benefits `while`.
**Decision P0 (below) folds it into this slice as Phase 1.** Without it 10b cannot go green:
`inplace_fold.sth` (and its dependents in `phase4_slice6h`, `phase4_slice6h_fill_corpus`, and
the `qbe_baseline` corpus) stay red.

## Recon corrections (measured, against the deleted-intrinsic scratch tree)

The brief's own prose is wrong in several places it flagged as unverified, and in two it did
not.

1. **The intrinsic deletes cleanly and `back_edge_outs` survives.** Removing the interception
   (`terms.rs:326-458`, comment through the closing brace at 458), `check_abstract_quotation_times`
   (`terms.rs:1246-1293`), and the four diagnostics (`times_needs_quotation_error` through
   `times_body_row_effect_error`, `terms.rs:1314-1357`) leaves a warning-free `cargo build`.
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
   (before any test edits), the failing set is **twelve targets, 55 tests**, and it does not
   match the brief's list:
   - **Two brief-named files do not fail at all**: `phase0.rs` (199 pass) and
     `phase4_slice10a_inline_quotation.rs` (22 pass). Their `times` mentions are in comments or
     in `my-times`-style user words, not bare intrinsic calls.
   - **Two failing targets the brief never named**: the crate's own unit tests (`--lib`, 3
     failures) and `phase4_slice6h_fill_corpus.rs` (1). Two of the three `--lib` failures are
     in files the brief did **not** sanction (`src/check/engine.rs`, `src/check/word_families.rs`).
   - `phase4_generics.rs` (12) and `phase4_slice6g.rs` (6) were named but unmeasured; now
     measured.

4. **The corpus enumeration missed one file.** The brief named seven bare-`times` corpus
   files. `examples/combinator_in_times.sth:20` is an eighth: it imports combinators as `c`
   (qualified) but drives its outer loop with a bare `times`, which the intrinsic served.

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

**Q3 (full fallout enumeration). Resolved: twelve targets, categorized.** The complete list,
with the edit each needs (measured, not inferred):

| Target | Fails | Nature / edit |
| --- | --- | --- |
| `--lib` `check::engine::times_typing_obligations` (`engine.rs:1572`) | 1 | intrinsic-only typing obligations; **retire the test** (engine.rs, **not sanctioned**) |
| `--lib` `check::word_families::quotation_as_operand_is_rejected_at_every_audited_site` (`word_families.rs:1154`) | 1 | remove the `times` audit row (`word_families.rs:1258`); reword two `"only call and times"` rows (**not sanctioned**) |
| `--lib` `ir::func_builder::calls::each_lowers_to_a_loop_not_a_per_element_call` (`calls.rs:1615`) | 1 | inline `each` uses bare `times`; add inline `times`/`times-helper` defs (calls.rs, sanctioned) |
| `phase3_refs.rs` | 3 | inline bare `times`; add combinators import + reword any intrinsic-wording asserts |
| `phase4_combinators.rs` | 20 | bulk bare `times` (import) + intrinsic-diagnostic asserts (reword) + dogfoods (pass once imported); the 2 REPL-import tests need the `times-helper` export (Q4) |
| `phase4_generics.rs` | 11 | inline bare `times`; import / reword |
| `phase4_quotations.rs` | 2 | inline bare `times`; import / reword |
| `phase4_slice6f.rs` | 2 | **neither an import fix nor P0-gated** (the spec originally claimed both, wrongly): both tests call `check::check` in-process on `include_str!`'d dogfood source, so an `import:` line in the `.sth` file never resolves and the failure is `` unknown word `times` in `prefix-copy` ``. Fix by routing through the driver or inlining `times`/`times-helper` into the checked source. **This is a category, not one file**: every in-process-check test over a corpus file that now needs an import behaves this way |
| `phase4_slice6g.rs` | 5 | doorway-grant tests, inline bare `times`; add combinators import (grant verified to survive) |
| `phase4_slice6h.rs` | 1 | builds the corpus (`inplace_fold` etc.); fixed by corpus imports, **linear case gated on P0** |
| `phase4_slice6h_fill_corpus.rs` | 1 | corpus stdout baseline; fixed by corpus imports, **gated on P0** |
| `phase4_slice10a_exit_witnesses.rs` | 2 | resolved per Decision 4 below |
| `qbe_baseline.rs` | 1 | regenerate `.ssa` for the times-using corpus (sanctioned baseline diff) |

The starting count was 55 tests across the untouched-test tree; exporting `times-helper` (P1)
clears 2, the arrays.sth import clears the sort dogfood, and the rest are the table's edits.

Re-measured with the Phase 1 relaxation in place: still 55 tests, across 11 targets. The
relaxation changes the failing set by exactly one test, in the direction of *adding* one
(`while_body_linear_local_across_back_edge_is_error`, the deliberate re-point, see P0) and
fixing none. That it fixes none is itself a finding: the shape it enables is pinned by no
working test today, because `phase4_slice6f.rs`'s two tests cannot even reach the checker
(this table's `phase4_slice6f.rs` row). Phase 1 must therefore add its own golden.

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
in scope) skips a local bound below the floor: such a local lives in an ancestor frame that
outlives every iteration, so the loop neither rebinds it nor carries it, and its disposal
obligation stays with the enclosing word.** The `terms.rs` sanctioned edit widens from
delete-only to include this relaxation.

**Only the combinator-splice call site passes a floor.** `Some(base_depth)` at the self-tail
*combinator* site (`terms.rs:615`, the marker path `while`/`times-helper` take); `None`, i.e.
today's behaviour, at the whole-word TCO site (`terms.rs:787`). This is load-bearing, not
tidiness: a plain recursive word has no parked-ancestor shape to admit, because its self-call
must supply its full declared inputs, so a linear is either forwarded as an argument (already
legal by design, per the function's own doc comment) or genuinely stranded beneath them
(clause 1). Passing a floor at both sites is the change that would actually open a hole.

**Why this is sound, and not a new permission.** `check_reference_across_back_edge`
(`src/check.rs:1264`) already draws exactly this distinction for references, and says so: a
reference with no owned root "may cross freely: its referent lives in an ancestor frame that
outlives every iteration", which is what keeps `walk ( &!List -- ) ... walk ;` legal. The
linear check simply never made the distinction. P0 brings it to parity. References carry no
disposal obligation and linear values do, so parity is not self-evidently transferable, and the
two failure modes were probed rather than argued (all against the prototype patch):

| Hazard | Probe | Still rejected by |
| --- | --- | --- |
| body consumes the enclosing linear inside the loop (double-consume per iteration) | `[ ... acc drop ... ] c::while` | capture-admission D3: `` the quotation passed to `while` consumes the enclosing local `acc`, which is linear `` |
| consumed on one branch only (`MaybeMoved` across the edge) | `[ ... n 2 > if acc drop else end ... ]` | capture-admission D3, same message |
| own-frame linear bound inside the body, unconsumed at the edge (the leak clause 2 nominally guarded) | `[ ... 5 AccL \| tmp \| ... ]` | end-of-scope disposal: `` linear value `tmp` is never consumed ``, located at the quotation's own scope end, which is *tighter* than the back-edge check |
| enclosing linear never disposed at all | `work` binds `acc`, loops, returns nothing | end-of-scope disposal: `` drop it or return it `` |

None of the four leans on the relaxed clause. Clause 2 was over-approximating what three other
guards already cover precisely.

**Blast radius, measured.** Full `cargo test --no-fail-fast` on the prototype, then the same
run with only the relaxation reverted, diffing the failure sets: exactly one test is red only
with the relaxation, and none is fixed by it.

**That one test is a deliberate re-point, and it must be reviewed as one, not silently edited.**
`while_body_linear_local_across_back_edge_is_error` (`tests/phase4_combinators.rs:962`) binds
an outer `Spy`, never touches it inside `while`'s predicate, and disposes it on the very next
line (`sp drop`). That is the parked-with-a-disposer shape, the same one `inplace_fold.sth`
needs and the intrinsic allowed. Its own comment justifies itself with "it would ride into the
next iteration with nobody to dispose it", which is factually false about its own program. It
pins the over-approximation, so Phase 1 re-points it: keep the name and the located-error
assertion, change the program to a shape that is genuinely unguarded elsewhere (an own-frame
linear bound inside the body, i.e. the third hazard row above), so the criterion keeps a real
witness instead of losing one.

**Phase 1 owns its own golden.** The relaxation fixes zero existing tests, and the shape it
enables is pinned by nothing today (`phase4_slice6f.rs`'s two dogfood tests cannot reach the
checker at all, see Q3). Without a new golden the capability ships unguarded, which is this
project's documented placebo pattern. Phase 1 adds: an accept golden for the parked-linear
shape over a library-style self-tail combinator *and* over `while` (both, since P0 changes
`while` too), each asserting the value, plus the four hazard rejections above as reject
goldens naming the guard that catches each. All of it mutation-audited per R9.

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
   selective import.** The seven brief-named files and the eighth (`combinator_in_times.sth`)
   each gain a `times` import. Examples use `import: c | times | "../lib/combinators.sth" ;`
   (selective, so the existing bare `times` calls resolve unqualified);
   `combinator_in_times.sth`, which already imports `c` qualified, qualifies its one bare call
   to `c::times` instead. `lib/arrays.sth` gets `import: c | times | "combinators.sth" ;`.

4. **`tests/phase4_slice10a_exit_witnesses.rs` is resolved test by test.** Exactly two of its
   tests fail: `my_times_compiles_beside_the_untouched_intrinsic_and_sums` (its "untouched
   intrinsic" premise is retired, and its `my-times` half is subsumed by 10b's own goldens on
   the real `times`) is **deleted**; `combinators_library_contains_no_tilde` (false by design
   now that `times`/`times-helper` carry `~[ ... ]`) is **inverted** into a positive assertion
   that `combinators.sth` declares exactly the two expected `~` signatures. The other five
   tests (`my_times_runs_one_million_iterations_in_constant_stack`,
   `row_grounding_accepts_a_borrow_of_an_unrelated_place_of_the_same_type`,
   `my_times_carries_an_aggregate_without_aliasing`,
   `my_times_nested_in_itself_produces_correct_output`,
   `while_is_unaffected_by_the_row_and_back_edge_rewrite`) use the self-contained `my-times`
   user word, are already independent of the intrinsic, and stay unchanged: they remain
   coverage of the `~`/row mechanism in its own right.

5. **The four dead diagnostic functions are deleted outright**, and the general capture-admission
   path (Q1) already produces a rejection for each of the three bug classes they messaged.

6. **Corpus stdout and exit codes stay byte-identical** everywhere `times` still compiles.
   Verified for all eight non-linear files (`array_totals`, `array_totals_hand`,
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

## Requirements

**R1**. Delete the intrinsic: the `check_term` `if name == "times"` interception
(`terms.rs:326-458`), `check_abstract_quotation_times` (`terms.rs:1246-1293`), the four
diagnostics (`terms.rs:1314-1357`, working around `back_edge_outs` at `:1294`, which stays),
and the `calls.rs` `"times" =>` arm (`:326-418`) with its two unit tests (`:782`, `:840`).
`cargo build` warning-free afterward proves the four functions were the only dead code.

**R2**: Add `times`/`times-helper` to `lib/combinators.sth` per Decision 1, both on the
`export:` line per Decision 2.

**R3**: Add the corpus imports per Decision 3 (eight example files plus `lib/arrays.sth`),
with stdout and exit code held byte-identical (Decision 6).

**R4**: Resolve `phase4_slice10a_exit_witnesses.rs` per Decision 4.

**R5**: Sweep the remaining test targets per the Q3 table: import for bare-`times` sources,
reword for intrinsic-diagnostic asserts, retire `engine.rs`'s `times_typing_obligations`,
remove `word_families.rs`'s `times` audit row, restore `calls.rs`'s `each_lowers` test with
inline `times`/`times-helper` defs, and correct the stale `check.rs` diagnostics per Decision 7.
For any test that checks a corpus file **in process** (`lexer::lex` / `parser::parse` /
`check::check` over `include_str!`'d source, as `phase4_slice6f.rs` does), an `import:` line in
the `.sth` file does not resolve: route it through the driver or inline `times`/`times-helper`
into the checked source. Audit for this pattern rather than assuming `phase4_slice6f.rs` is the
only instance.

**R6**: Regenerate the `qbe_baseline` `.ssa` for the times-using corpus (a sanctioned
baseline diff). Machine-code size is unchanged (Q2); the `.ssa` differs only in value
numbering.

**R7 (exit witnesses on the real `times`, not `my-times`)**: new goldens:

- `0 5 [ + ] times` sums to `10`; `0 1000000 [ drop 1 + ] times` exits 0 at `ulimit -s 1024`.
- an aggregate carried through the row (a `map`-shaped body) prints arithmetically correct
  fields, no aliasing.
- a `times` call nested inside `each`/`map`/`fold`/`filter` (the newly doubled splice depth)
  and a `times` nested inside an outer loop both produce correct values under 6g's fix. This
  is the most novel risk beyond 10a and needs its own golden, not reliance on the library
  suite passing.
- the binary-size delta across the corpus is measured and recorded per Q2.

**R8 (Phase 1 / P0 witnesses, in this slice)**: `check_linear_across_back_edge` takes
`frame_floor`, passed `Some(base_depth)` at the combinator site and `None` at the whole-word
TCO site (P0). `inplace_fold.sth`'s `prefix-linear` compiles again and prints identically.
Phase 1's own goldens: an accept for the parked-linear shape over both a library-style
self-tail combinator and `while`, asserting the value; reject goldens for all four hazards in
P0's table, each asserting the message of the guard that actually catches it (D3
capture-admission, or end-of-scope disposal), so a future regression that silently moves the
rejection to a different guard is visible. `while_body_linear_local_across_back_edge_is_error`
is re-pointed per P0, not deleted.

**R9**: Mutation-test each new guard and each reworded rejection: prove the test can fail by
deleting or reverting what it guards, in a throwaway copy of the tree (never the shared
worktree; keep `target/` copies off the `/tmp` tmpfs).

## Sanctioned files, and the scope this spec adds

Brief-sanctioned: `src/check/terms.rs` (deletions **and P0's relaxation**, per Decision P0),
`src/ir/func_builder/calls.rs` (arm deletion, and its two unit tests, R1),
`lib/combinators.sth` (R2, plus `times-helper` on the export line), the recon-4 corpus files
(R3), `tests/phase4_slice10a_exit_witnesses.rs` (R4).

Investigation forces these additions the brief's file list omits, each stated so the
maintainer can veto:

- `src/check/engine.rs` and `src/check/word_families.rs`: unit tests coupled to the intrinsic
  (retire / trim). Unavoidable: the suite cannot go green while they assert intrinsic-only
  behaviour.
- `src/check.rs`: the three stale `` "only `call` and `times`" `` diagnostics (Decision 7).
  Correctness, not just green.
- `lib/arrays.sth`: the library-to-library import (Q5). The brief mentions arrays.sth in recon
  4 but as an example-style import; it is a library and needs the lib-to-lib form.
- `tests/phase4_combinators.rs`: the `while_body_linear_local_across_back_edge_is_error`
  re-point (P0). Called out separately from R5's bulk sweep because it is a criterion change,
  not an import fix.

## Sequencing

After 6g (merged `86aee0a`). Independent of 10c. `ROADMAP.md:608`'s "Next action" already names
10b/10c and needs no correction: P0 is Phase 1 of this slice, not a separate entry. Phase 1
must land and be green before Phase 3's corpus imports, since `inplace_fold.sth` cannot compile
without it. Phase 1 is independently verifiable: it is green against the *unmodified* intrinsic,
so the relaxation is reviewed on its own evidence before anything is deleted.

## Exit criteria

10b exits when: the intrinsic (interception, `check_abstract_quotation_times`, four
diagnostics, lowering arm and its two unit tests) is gone and `cargo build` is warning-free
(R1); `times`/`times-helper` are ordinary exported Sooth source (R2); every bare-`times`
corpus file compiles with an added import and prints byte-identical stdout and exit code,
`arrays.sth` via a library-to-library import (R3, Decision 6); `phase4_slice10a_exit_witnesses.rs`
is resolved test by test (R4); every failing target listed in Q3 is green, including the `engine.rs`
/ `word_families.rs` / `calls.rs` unit tests and the corrected `check.rs` diagnostics (R5);
the times-using `qbe_baseline` `.ssa` is regenerated (R6); the real-`times` goldens pass
(1M constant stack, aggregate-through-row, times nested in a combinator and in an outer loop),
and the binary-size delta is recorded (R7); `check_linear_across_back_edge` takes a frame floor
passed only at the combinator site, `inplace_fold.sth`'s linear half compiles and prints
`1 3 6 10 1 3 6 10`, Phase 1's parked-linear accept goldens pass over both a library self-tail
combinator and `while`, all four hazard rejections hold naming their actual guard, and
`while_body_linear_local_across_back_edge_is_error` is re-pointed rather than dropped (R8); and
every new guard has been shown capable of failing by mutation in a throwaway tree (R9).

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "P0: check_linear_across_back_edge gains frame_floor, Some(base_depth) at the combinator site and None at the whole-word TCO site; parked-linear accept goldens over a library self-tail combinator and over while; the four hazard reject goldens naming their actual guard (D3 capture-admission / end-of-scope disposal); re-point while_body_linear_local_across_back_edge_is_error. No intrinsic deletion yet: this phase is green against the unmodified intrinsic. Reference: docs/phase4-slice10b-p0-prototype.patch", "difficulty": "hard" },
    { "phase": 2, "focus": "delete the intrinsic (interception, check_abstract_quotation_times, four dead diagnostics, calls.rs lowering arm and its two unit tests) and add exported times/times-helper to lib/combinators.sth", "difficulty": "hard" },
    { "phase": 3, "focus": "corpus imports across the eight bare-times example files and arrays.sth library-to-library import, with byte-identical stdout including inplace_fold", "difficulty": "standard" },
    { "phase": 4, "focus": "test-suite sweep across the twelve targets including the in-process-check category, slice10a witness resolution, stale check.rs diagnostic correction, qbe baseline regeneration, real-times exit witnesses, and the mutation audit", "difficulty": "hard" }
  ]
}
```
