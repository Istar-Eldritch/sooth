# P7.S10 -- Bound the splice, diagnose the recursive impl (spec, as shipped)

Status: **done**. Input: [slice10-brief](./slice10-brief.md). Roadmap: [P7-language-prereqs](../P7-language-prereqs.md), P7.S10.

## Problem

A recursive `impl: Ord` -- one whose `cmp` compares values of its *own* type with a surface
comparison instead of delegating to its fields -- recursed without bound at lowering and
died with `stack overflow, aborting` / `EXIT=134`, no diagnostic. The cycle only exists
after bound dispatch resolves a bare `cmp` to a concrete type's member, so
`check_combinator_cycles` cannot see it; widening that pass was built and measured to
false-reject the ordinary field-delegating impl (brief, "What was refuted"). `catch_unwind`
was rejected by maintainer decision. Neither is revisited.

## Shape of the fix

1. A splice-depth budget in `lower_resolved_word_call` (`src/ir/func_builder/calls.rs`),
   reusing the existing `member_splice_depth`.
2. An error path: `FuncBuilder` had none (every lowering function returned `()`), so
   `Result<_, String>` is threaded from the guard out to the existing fallible boundary
   `lower(&Module) -> Result<IrModule, String>` (`src/ir/driver.rs`), which already
   `?`-propagates to `main.rs`.

The threading closure is the transitive *caller* closure of
`lower_terms`/`lower_term`/`lower_call`/`lower_resolved_word_call`: **18 non-test
functions** across `func_builder/{calls,control_flow,mod,word_families}.rs`, `ir/driver.rs`
(`lower`, `lower_word`, `lower_instantiation`, `lower_line`) and `ir/destructors.rs` (the
two `synthesize_*` functions), plus 6 non-test `src/repl.rs` call sites and test sites in
`src/backend/qbe.rs` and `src/ir/driver.rs`. Shipped as measured; no correction needed.

## Requirements

### R1 -- the budget guard

- **R1.1** `lower_resolved_word_call` is the sole guard site. The ordinary `inline`
  combinator splice in `lower_call` is not guarded: separate code, already proven finite
  pre-lowering by `check_combinator_cycles`.
- **R1.2** Counter is the existing `member_splice_depth`; no new counter. The `+= 1`/`-= 1`
  bracket is balanced on every `Ok` path. An `Err` unwinds through it leaving the counter
  raised, which is harmless: the builder is discarded and the error goes to `main.rs`.
- **R1.3** `const SPLICE_BUDGET: u32 = 64` in `calls.rs`, with a comment naming both
  measurements: legitimate corpus maximum 2; unguarded overflow at depth 148 on a 2MB
  test-thread stack.
- **R1.4** `if self.member_splice_depth >= SPLICE_BUDGET { return Err(..) }`, evaluated
  **before** the `+= 1`, inside the `self.combinators.get(sym_name)` (splicing) arm. The
  non-splicing arm is untouched.
- **R1.5** The guard bounds recursion; it does not change acceptance (see R4.3).

### R2 -- the error path

- **R2.1** Error type is `String`, matching `lower`'s existing signature. No new error enum
  or module.
- **R2.2** Every closure function returns `Result<T, String>` and propagates with `?`. No
  `.unwrap()`, `.expect()`, `let _ =` or `if let Ok(..)` dam anywhere in the closure: the
  threading leaves no lowering error silently discarded.
- **R2.3** Non-`ir` callers (`src/repl.rs`) propagate through their own signatures. Unit
  tests may `.unwrap()` -- there the unwrap is the assertion.
- **R2.4 (ruling)** Existing `.expect("checked …")` calls in `func_builder` are **not**
  bulk-converted; they encode checker guarantees and are a different slice. The only
  obligation: no `.expect()` or panic on the path the budget guards, i.e. every frame
  between the guard's `return Err(..)` and `lower()`'s return is a plain `?`. Verified by
  R4.2, not assumed.
- **R2.5** No pre-staged plumbing: nothing introduced a phase ahead of its call site.

### R3 -- the diagnostic

- **R3.1** The member is named via `crate::resolve::render_word`. Its output is
  pre-delimited -- `` `cmp` (member of trait `Ord` for `Wrap`) `` -- and printed bare;
  wrapping it in further backticks is a defect.
- **R3.2** The member named is the **outermost**: the symbol whose splice took
  `member_splice_depth` 0→1, recorded in the new `member_splice_outermost: Option<String>`
  field (set on the 0→1 transition, cleared on 1→0). Not the guard's current `sym_name` --
  the two coincide under self-recursion but diverge under mutual recursion. The `None` arm
  falls back to `sym_name` rather than `.expect()`: a panic on this path would reinstate
  the abort the guard replaces.
- **R3.3** The location is the offending member's **own declaration** (`WordDef.span`, the
  member name token after `:`), never a call site. No call-site span is usable:
  `rename_terms` copies spans verbatim and a surface `lt` is itself spliced before any
  member splice, so every frame already points into `lib/cmp.sth`. Capturing at
  `splice_uid_stack.is_empty()` was prototyped and rejected (returns a library span behind
  any non-`inline` wrapper). Shipped as a name-keyed `member_spans: HashMap<String, Span>`
  built in `driver.rs` from `module.words`, mirroring `member_uid_seeds` in shape and
  plumbing, with a companion `empty_member_spans()` in `src/ir.rs` for the five
  non-`Module` construction sites.
  **Ruling on a lookup miss:** reachable (REPL and destructor paths hand out the empty
  map). The guard still fires and still reports, **omitting the location clause entirely**
  rather than substituting a zero or library span. Bounding the recursion must not depend
  on a span being present.
- **R3.4** House style is `combinator_cycle_error`, including the trailing
  `(line {}, col {})`.
- **R3.5 (ruling)** The message **omits** the `"error: "` literal, since `main.rs` already
  does `eprintln!("error: {e}")`. `combinator_cycle_error`'s pre-existing doubling is out
  of scope. The golden pins this by asserting exactly one `"error: "` in stderr.
- **R3.6** Shipped text (builder-side, no prefix):

  ```text
  a trait member cannot dispatch back to itself (lowering would splice it forever): {rendered} exceeded the splice budget of {SPLICE_BUDGET} (line {line}, col {col})
  ```

  Rendered for the witness: ``error: a trait member cannot dispatch back to itself (lowering would splice it forever): `cmp` (member of trait `Ord` for `Wrap`) exceeded the splice budget of 64 (line 6, col 5)``.
  Line/col are the member declaration's, in the user's own source; the golden asserts them
  against the fixture it writes itself.
- **R3.7** Exit is non-zero and not 134; `main.rs`'s existing error path supplies it.

### R4 -- tests

- **R4.1 (golden, exit criterion)** `tests/phase7_slice10.rs`, modelled on
  `tests/phase7_slice3s_flip.rs` (`Tree`/`sooth_build`, `tests/common/mod.rs::fixture_package`;
  never copy syntax from `docs/book/`). Asserts on stderr: (1) the exact R3.6 text with the
  fixture's own line substituted; (2) exactly one `"error: "`; (3) the reported
  `(line N, col M)` is the fixture's `: cmp` position -- a "does not contain `cmp.sth`"
  assertion is **forbidden**, the format carries no path so it passes vacuously;
  (3b) a second fixture with two blank lines before `impl:` reports a line greater by
  exactly that offset (a pinned constant satisfies 3 alone, not the pair);
  (4) `status.code() == Some(1)`, not a signal death.
- **R4.2 (`.expect()`-free path)** The golden covers the one traversed path; observed exit
  was 1, not 101 or 134. The other closure members are invisible to it, so the phase report
  carries a grep inventory over all 18 for `.expect(`, `.unwrap(`, `let _ =`,
  `if let Ok(` on a now-fallible call, each hit removed or justified.
- **R4.3 (no false rejection, mandatory)** P7.S8's
  `a_concrete_impl_ord_delegating_to_lt_builds_and_runs`
  (`tests/phase7_slice3s_flip.rs`) passes unmodified and `POINT_IMPL` still builds and runs.
- **R4.4 (unit tests beside the stage code)**
  - `calls.rs`: `splice_depth_bracket_is_balanced_on_a_legitimate_resplice` (a re-splice at
    `SPLICE_BUDGET - 1` returns `Ok` and restores the depth) and
    `splice_depth_guard_fires_at_the_budget_and_omits_a_missing_span` (R3.3's miss ruling).
  - `driver.rs`: `a_recursive_impl_ord_error_propagates_unchanged_to_lowers_result` drives
    the **real** recursive fixture (`parse_with_core` runs `check_trait_decls`/
    `check_impl_decls`, so no synthetic forced error was needed). Added beyond the spec:
    `splice_depth_guard_names_the_outermost_member_of_a_three_cycle` -- R3.2 needs a cycle
    length not dividing 64, so a 2-member ping-pong is a parity placebo; a 3-member trait
    cycle separates the outermost (`a`) from the firing frame (`b`, `64 % 3 == 1`).
    `check::CombinatorEntry` is re-exported `#[cfg(test)] pub(crate)` for the direct-index
    fixtures.
- **R4.5 (mutation recipe, against a committed tree)** (1) `SPLICE_BUDGET` 64 → 100_000;
  (2) delete the `if … >= SPLICE_BUDGET` block -- both must make
  `a_recursive_impl_ord_is_a_located_diagnostic_not_a_stack_overflow` FAIL via assertion 4
  (signal death, contained in the child process); (3) corrupt the `member_spans` lookup to
  yield another word's span -- assertions 3/3b must FAIL, or the span half is unguarded.
  Classify on `test result: FAILED`, not exit status.
- **R4.6** The budget's legitimacy margin is not re-measured; the brief measured max 2, and
  R4.3 plus a green suite is the standing evidence that 64 clears it.

## Out of scope

Any change to `check_combinator_cycles`, `is_combinator`, or the P7.S8 uid rules;
type-directed cycle detection on resolved dispatch targets (deferred, revisit only if the
budget message proves confusing); `combinator_cycle_error`'s doubled prefix; bulk
`.expect()` removal; the other two P7.S8 follow-ups.

## Phasing (as delivered)

**Phase 1 -- thread `Result` through the lowering tree (`hard`, L).** Not split, and not
splittable: a partial thread does not compile, `Result` is `#[must_use]`, and the only way
to stop the wave at a boundary is an `.expect()` dam -- exactly the panic this slice
removes, and forbidden by R2.2. All 18 functions plus the repl/test call sites, no
behaviour change, no error produced yet. Exit: green triple with no test modified except to
unwrap a newly-`Result` call. Shipped as `f92c17a8`.

**Phase 2 -- the guard and the diagnostic (M).** R1, R3, R4.1/4.2/4.4/4.5; R4.3 verified by
name. `SPLICE_BUDGET`, `member_splice_outermost` and `member_spans` all land here with
their uses. Shipped as `2f09730e` plus two review-fix commits (`c6025d1d`, `e4398a2b`).

**Phase 3 -- bookkeeping (S).** P7.S10 flipped to `[ done ]` with the shipped diagnostic
text and the split-signal re-run recorded; the P7.S8 entry reconciled against the shipped
fix. Docs only (`c3226b74`, `5d129aed`, `f6a98cfe`).

## Exit criteria (met)

- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green.
- A recursive-type `impl: Ord` produces a located diagnostic and exit 1, never SIGABRT,
  pinned on exact message text.
- The diagnostic points at the `impl:` block's own member declaration in user source; no
  call-site span is used, and none is available (R3.3).
- The field-delegating `impl: Ord for Point` still builds and runs;
  `a_concrete_impl_ord_delegating_to_lt_builds_and_runs` passes unmodified.
- No `.expect()`/panic on the guarded path; the threading discards no lowering error.
