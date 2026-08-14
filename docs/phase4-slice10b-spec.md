# Phase 4 Slice 10b: `times` moves to the library (delivered)

`times` is no longer a compiler intrinsic. The `check_term` interception, `check_abstract_quotation_times`, the four bespoke `times_*` diagnostics and the `calls.rs` lowering arm (with its two unit tests) are deleted; `times` is ordinary Sooth source in `lib/combinators.sth`, a thin wrapper over a self-tail-recursive `times-helper`. After 10b the compiler knows no intrinsic words at all.

The migration was not a pure delete-and-import: the intrinsic was the only loop that could hold an enclosing linear local parked across itself, so the slice also carries a checker relaxation (P0) and a splice-visibility fix (R10), both landed first, against the unmodified intrinsic.

## What shipped

**The library word** (`lib/combinators.sth`, both names on the `export:` line, `times-helper` exported because the REPL's dlopen import path retains only exported words and cannot resolve a transitively spliced private helper):

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

`~[ ... ]` is 10a's inline-only quotation type, so every call site splices; `times`/`times-helper` mint no function symbols, and a leaf combinator's call site is now three splices deep (leaf, `times`, `times-helper`).

**P0: `check_linear_across_back_edge` takes `frame_floor: Option<usize>`** (`src/check/terms.rs:1049-1075`). Its second clause (any unconsumed linear local in scope) skips a local bound below the floor. `Some(base_depth)` is passed only at the spliced self-tail *combinator* site; the whole-word TCO site passes `None`. The floor is the entry depth of the `if` arm the back-edge sits in, not a frame lifetime, despite the parameter's name.

Soundness: the clause relocates a diagnostic, it does not grant a permission. Deleting the combinator-site call outright flips exactly one test, `while_body_linear_local_across_back_edge_is_error` (per the R9 mutation table below): the diagnostic re-points from the back-edge to end-of-scope, a wording change, not a leak. Disposal of an enclosing linear is independently enforced by end-of-scope disposal and the branch-join `MaybeMoved` guard, and a self-tail call has no position after it. The clause's only job is to *locate* that rejection at the back-edge instead of at scope end. Passing a floor at the whole-word TCO site would open a real hole (a self-call must supply the word's full declared inputs), which is why only one site passes it.

The floor over-exempts one shape: a linear bound in the combinator's own frame *before* the tail-`if`, which is re-created every iteration. Harmless, because the two co-guards above still catch it, and pinned by a permanent reject golden asserting the scope-end wording, the tripwire if either co-guard is ever widened.

**R10: disposal visibility under splicing.** `check_drop_import_visibility` gated on `ctx.module()`, which for a spliced body is the splice *destination* (`lib/combinators.sth`), so a quotation disposing a linear through a `drop` overload declared in the caller's own file was rejected. Both the gate (`src/check/word_families.rs:762-771`) and `drop_import_visibility_error`'s `caller` derivation (`:792`) now use `span.module`; the two `engine.rs` tests whose synthetic span hardcoded `module: 0` set it to `caller`. Out of scope, recorded: the gate never fires at all for a disposal inside a *generic* word body, so this rule is import hygiene, not a soundness guarantee. `drop_import_visibility_error`'s `caller` derivation is unpinned by construction: a caller-supplied literal's full body is always validated once, up front, under the caller's own (unspliced) `ctx` -- which by then always equals `span.module` -- before any nested splice could reach this line under a different `ctx`, so no reject golden distinguishes the two derivations here (unlike the gate itself, R9 mutation (d)).

**Corpus.** Seven examples take `import: c | times | "../lib/combinators.sth" ;` (`array_ctor`, `array_totals_hand`, `combinator_in_times_hand`, `filter_while_hand`, `inplace_fold`, `times`, and `combinator_in_times`, which already imported `c` qualified and qualifies its one bare call to `c::times`). `lib/arrays.sth` takes the first library-to-library selective import in the tree. Stdout and exit codes are byte-identical throughout, including `inplace_fold`'s `1 3 6 10 1 3 6 10`, which only compiles because of P0.

**Diagnostics.** The three stale `` "only `call` and `times`" `` strings in `src/check.rs` (`:841`, `:1674`, `:1689`) dropped the `` and `times` ``. The retired bespoke `times` diagnostics re-point onto the general capture-admission (D3) rejections, reworded but still caught. README, ROADMAP and DESIGN no longer describe `times` as an intrinsic.

**Tests.** New file `tests/phase4_slice10b.rs` holds the P0, R10 and real-`times` goldens. `while_body_linear_local_across_back_edge_is_error` (`tests/phase4_combinators.rs:976`) is re-pointed: its old shape (an outer linear parked across the loop, disposed on the next line) was a false rejection whose own comment was factually wrong about its own program. It now uses a test-local self-tail combinator, named `while` so the existing three-substring assertion survives, that binds a linear inside the tail-`if` arm. It witnesses diagnostic *location*, not leak prevention. `phase4_slice6f.rs`'s `fold_body` now routes through `sooth::driver::emit_ssa` (an in-process `lex`/`parse`/`check` cannot resolve an `import:` line) with a module-suffixed header lookup; its `phi`/`jmp`/`storel`/no-`blit` assertions are unchanged. A `common::assert_pinned_to_combinators_lib` helper keeps every hand-copied `times`/`times-helper` definition in the test suite (`phase3_refs`, `phase4_combinators`, `phase4_generics`, `phase4_quotations`, `phase4_slice6g`, `phase4_slice10b`) in sync with the library. `phase4_slice10a_exit_witnesses.rs`: `my_times_compiles_beside_the_untouched_intrinsic_and_sums` deleted, `combinators_library_contains_no_tilde` inverted into `combinators_library_declares_exactly_the_two_times_tildes` (both signatures pinned literally, total `~` count pinned at 2). `qbe_baseline` `.ssa` regenerated for the times-using corpus.

## Delivery findings

**Fallout counts came in lower than the spec's snapshot: 46 failures over 9 targets at Phase 4 entry, not 55 over 11.** `phase4_slice6h` and `phase4_slice6h_fill_corpus` were already cleared by the Phase 3 corpus imports; `phase4_combinators` was 17 not 20, `phase4_generics` 11 not 12. Every listed target is green on exit. The counts were always a measurement, never a gate.

**Binary size: zero delta on eight of nine times-using files** (base `91f6193` vs Phase 4, `stat -c%s`), with `array_ctor` up 8 bytes (17608 to 17616), alignment noise. No KB-scale growth, so no per-iteration indirect call replaced the spliced loop. The `.ssa` does change: the spliced `times-helper` adds a self-phi for the loop bound and an empty join block per loop, which QBE folds away.

**Two rejections the intrinsic owned are now compiler crashes, both pre-existing.** The intrinsic's whole-row guard rejected a quotation riding *below* the consumed top of the row; no general guard replaced it. `[ + ] 3 [ drop ] times drop` dies in QBE (`invalid type for operand %v0 in phi %v4`), and calling the row quotation afterwards hits `unreachable!()` in `func_builder/quotation.rs`. Both reproduce identically at `91f6193` on a user-declared row combinator, so 10b only widens the reach to the library `times`. A third of the same family: `while` over an *erased* quotation panics in `control_flow.rs`, also at `91f6193`. `phase4_generics.rs`'s `quotation_left_as_a_declared_output_is_error` now pins only what still rejects (the outputs check on `main`) and says so. ROADMAP names all three as one slice's worth of unscheduled work: a general guard on a row-typed combinator's call.

**`times` no longer drives an erased quotation.** The `~[ ... ]` parameter rejects a materialized quotation value where the intrinsic accepted it and emitted an indirect call. The capability survives on any combinator declared over a plain `[ ... ]` parameter, so `phase4_quotations.rs`'s witness is re-pointed to a test-local self-tail driver of that shape (`loop_over_erased_quotation_emits_one_indirect_call`) and still asserts exactly one `CallIndirect`.

**R9 mutation audit** (throwaway tree copy):

| Mutation | Flipped | Vs prediction |
| --- | --- | --- |
| (a) `frame_floor` always `None` | both parked-linear accepts, the own-frame tripwire, `parked_linear_local_never_disposed_at_all_is_rejected`, plus `phase4_slice6f`'s two, `phase4_slice6h_fill_corpus`, `qbe_baseline` | wider than predicted; the last four are all `inplace_fold.sth` failing to compile, the same relaxation seen through the corpus imports |
| (b) delete the combinator-site call | `while_body_linear_local_across_back_edge_is_error`, alone | exactly |
| (c) drop `MaybeMoved(_)` from `src/check.rs:1385` | `quotation_consuming_an_enclosing_linear_on_one_branch_is_rejected`, alone | exactly, a wording witness: still rejected, different text |
| (d) revert R10's gate to `ctx.module()` | `spliced_body_disposes_a_locally_declared_linear`, alone | exactly |

## Deliberately not done

- The REPL retaining private words transitively reachable from an exported combinator (a `src/repl.rs` change). Exporting `times-helper` is the minimal move and leaves repl.rs untouched.
- A general guard on a row-typed combinator's call, covering the three crash shapes above.
- The disposal-visibility gate inside generic word bodies.
- R10's reject golden is a rule-intactness pin, caught by the home-scope pass before splicing, not a splice-site witness.
