# P7.S10 -- Bound the splice, diagnose the recursive impl (spec)

Authoritative input: [slice10-brief](./slice10-brief.md). Its numbers and site claims are
probe-backed (2026-08-27) and are **not** re-litigated here. Roadmap entry:
[P7-language-prereqs](../P7-language-prereqs.md), P7.S10.

## Problem

A recursive `impl: Ord` -- one whose `cmp` compares values of its *own* type with a surface
comparison instead of delegating to its fields -- makes the compiler recurse without bound
at lowering and abort. Re-confirmed on `main` at spec time with this witness (a real
package tree, `lib/` as `core`):

```sooth
import: intrinsics * ;
import: core::prelude | if Bool Ord lt gt | ;
import: core::cmp | Ordering Less Equal Greater | ;
type: Wrap v i64 ;
impl: Ord for Wrap
  : cmp
    | a b |
    a b lt ~[ Less ] ~[ Equal ] if ;
;
: main ( -- )
  1 Wrap 2 Wrap lt ~[ 1 ] ~[ 0 ] if . ;
```

```text
thread 'main' has overflowed its stack
fatal runtime error: stack overflow, aborting
EXIT=134
```

No diagnostic is printed. The cycle exists only after dispatch resolves a bare `cmp` to a
concrete type's member, so it is not visible to `check_combinator_cycles`; widening that
pass was built and measured to reject the ordinary field-delegating impl (brief, "What was
refuted"). Dead design; not revisited.

## Shape of the fix

Two parts, the second much larger:

1. A splice-depth budget in `lower_resolved_word_call`
   (`src/ir/func_builder/calls.rs:214`), reusing the existing `member_splice_depth`.
2. An error path for lowering. `FuncBuilder` has none: every lowering function returns
   `()`. `Result` is threaded from the guard site out to the existing fallible boundary
   `pub fn lower(module: &Module) -> Result<IrModule, String>` (`src/ir/driver.rs:10`),
   which already `?`-propagates to `main.rs`.

`catch_unwind` is rejected by maintainer decision (brief). Do not propose it.

### The threading closure is smaller than `func_builder` as a whole

`func_builder` is ~7200 lines / 205 functions, but the set that must become fallible is
the transitive *caller* closure of `lower_terms` / `lower_term` / `lower_call` /
`lower_resolved_word_call`. Measured at spec time: **18 non-test functions**, plus test
functions and out-of-`ir` call sites.

| File | Functions |
| --- | --- |
| `src/ir/func_builder/calls.rs` | `lower_terms`, `lower_term`, `lower_call`, `lower_resolved_word_call`, `lower_self_tail_combinator`, `lower_enum_call`, `lower_eliminator` |
| `src/ir/func_builder/control_flow.rs` | `lower_if`, `lower_clauses` |
| `src/ir/func_builder/mod.rs` | `lower_word_parts`, `lower_materialized` |
| `src/ir/func_builder/word_families.rs` | `lower_array_word` |
| `src/ir/driver.rs` | `lower`, `lower_word`, `lower_instantiation`, `lower_line` |
| `src/ir/destructors.rs` | `synthesize_aggregate_destructors`, `synthesize_struct_destructor_override` |

Out-of-`ir` call sites of the now-fallible public functions: `src/repl.rs` has **six**
non-test sites (`#[cfg(test)]` begins at `src/repl.rs:3846`) --
`synthesize_aggregate_destructors` at 2849, 3245, 3432; `lower_line` at 3414;
`lower_word` at 3221; `lower_instantiation` at 1590. The last two are themselves in the
`driver.rs` row above. `src/repl.rs:4527` is a test site. Plus unit tests in
`src/backend/qbe.rs` (2 `lower_line` sites) and `src/ir/driver.rs` (~15 test sites).

This table is the *measured* closure, not a budget. If threading pulls in a function not
listed (e.g. `lower_poly_call`, which measured as a callee rather than a caller), thread it
too and note the correction in the phase report.

## Requirements

Numbering: `R1..` guard, `R2..` error path, `R3..` diagnostic, `R4..` tests.

### R1 -- the budget guard

- **R1.1** `lower_resolved_word_call` (`calls.rs:214`) is the sole guard site. The ordinary
  `inline` combinator splice (`calls.rs:665`) is **not** guarded: it is separate code and
  already proven finite pre-lowering by `check_combinator_cycles`.
- **R1.2** The counter is the existing `member_splice_depth` (`func_builder/mod.rs:399`).
  No new counter. Its `+= 1` / `-= 1` bracket has one statement between the two and is
  balanced on every `Ok` path (after the phase-1 threading an `Err` unwinds through the
  bracket via `?`, leaving the counter incremented; harmless, since the builder is
  discarded and the error goes straight to `main.rs`).
- **R1.3** The budget is a named `const` in `calls.rs`, value **64**. Measured legitimate
  maximum across the corpus is 2; the pathological case overflows at 148 (2MB test-thread
  stack). The const carries a one-line comment naming both measurements.
- **R1.4** The check is `if self.member_splice_depth >= SPLICE_BUDGET { return Err(..) }`,
  evaluated **before** the `+= 1`, inside the `self.combinators.get(sym_name)` arm (the
  splicing arm). The non-splicing arm below it is untouched.
- **R1.5** The guard bounds recursion; it does not change acceptance. No program that
  compiles today may stop compiling (see R4.3).

### R2 -- the error path

- **R2.1** The error type is `String`, matching `lower`'s existing
  `Result<IrModule, String>`. No new error enum, no new module: this is a craft project and
  one fallible boundary already exists.
- **R2.2** Every function in the closure table above returns `Result<T, String>` (`T = ()`
  for the void ones, the existing return type otherwise) and propagates with `?`. No
  `.unwrap()`, `.expect()`, `let _ =`, or `if let Ok(..)` dam anywhere in the closure:
  **the threading leaves no lowering error silently discarded**.
- **R2.3** Non-test callers outside `ir` (`src/repl.rs`) propagate or surface the error in
  whatever way their own enclosing signature already supports. Unit tests may `.unwrap()`
  -- a test asserting success is the one place where an unwrap is the assertion.
- **R2.4** **Ruling on `.expect()` conversion (do not leave this open).** The existing
  `.expect("checked …")` calls in `func_builder` are *not* bulk-converted. They encode
  checker guarantees and converting them is a different slice. The only obligation is the
  narrow one the brief states: **no `.expect()` or panic remains on the path the budget
  guards**, i.e. between the guard's `return Err(..)` and `lower()`'s return, every frame
  must be a plain `?`. The guard fires before any `.expect()` on that path is reached, so
  this is satisfied by R2.2 and must be *verified* by R4.2, not assumed.
- **R2.5** No pre-staged plumbing: nothing in this slice introduces an import, helper, or
  field one phase before its call site. See "Phasing" for why the threading is one phase.

### R3 -- the diagnostic

- **R3.1** The message names the impl member via `crate::resolve::render_word`
  (`src/resolve.rs:158`, `pub(crate)`, already called cross-module; `ir/driver.rs` already
  calls into `crate::resolve`, so there is no visibility barrier). Its output is
  **pre-delimited** -- `` `cmp` (member of trait `Ord` for `Wrap`) `` -- and is printed bare.
  Wrapping it in further backticks is a defect.
- **R3.2** The member named is the **outermost** one: the symbol whose splice took
  `member_splice_depth` from 0 to 1. `FuncBuilder` gains one field recording it, set on the
  0→1 transition and cleared on the 1→0 transition. That recorded symbol -- **not** the
  guard's current `sym_name` -- is both what the message names and what R3.3's span lookup
  is keyed by. The two coincide for self-recursion but diverge under mutual recursion
  between two impl members, so the spec fixes the outermost for both.
- **R3.3** The location is the offending impl member's **own declaration**, not a call
  site. The two differ and the spec does not blur them: the message points at the `impl:`
  block's `: cmp`, which is the defect itself and is stable however the recursion was
  reached.

  No call-site span is usable. `rename_terms` copies `term.span` verbatim and a surface
  comparison such as `lt` is itself an `inline` combinator spliced at `calls.rs:665`
  *before* any member splice, so on entry to `lower_resolved_word_call` every frame,
  outermost included, already points into `lib/cmp.sth` (measured:
  `Span { line: 146, col: 3, module: 2 }` at every depth). Capturing instead at
  `splice_uid_stack.is_empty()` was prototyped and **rejected**: it survives a direct call
  but returns a library span the moment a non-`inline` wrapper sits between the user and
  the comparison, which nothing prevents.

  The declaration span is `WordDef.span`, taken from the member name token after `:` at
  `expect_word_any_spanned()` (`src/parser.rs:2855`) -- no synthesis, and the same field
  `word_span()` already feeds to `combinator_cycle_error`. It is **not** reachable from
  `FuncBuilder` today, so this slice adds a name-keyed
  `member_spans: HashMap<String, Span>`, built in `src/ir/driver.rs` as
  `module.words.iter().map(|w| (w.name.clone(), w.span))` and looked up by R3.2's recorded
  outermost symbol. The key matches exactly: the mangled `cmp;Ord;2;Wrap__m0` *is* the
  `module.words` name, and `calls.rs:216` already performs the identical lookup against
  `member_uid_seeds`.

  **Ruling on a lookup miss (do not leave this open).** A miss is reachable: every
  `empty_member_uid_seeds()` site passes an empty companion map, so the REPL and destructor
  paths can reach the guard with no span available. On a miss the guard still fires and
  still reports, emitting the same message with the location clause omitted entirely rather
  than substituting a zero span, a library span, or silently skipping the guard. Bounding
  the recursion is the safety property and it must not depend on a span being present. It mirrors the existing `member_uid_seeds` exactly, in shape and in plumbing:
  the two `FuncBuilder` constructor call sites plus the five companion
  `empty_member_uid_seeds()` sites, all non-test: `src/ir/driver.rs:735`, `:925`, `:993`,
  `src/ir/destructors.rs:405`, and the struct default at `src/ir/func_builder/mod.rs:468`.
  All mechanical. It is a purely additive immutable borrow, independent of the
  phase-1 `Result` threading; it lands in **phase 2**, with its uses, so no field is
  introduced ahead of its call site.
- **R3.4** House style is `combinator_cycle_error` (`src/check/combinators.rs:291-303`); the
  new message reads as its sibling, including the trailing `(line {}, col {})`.
- **R3.5** **Ruling on the doubled `"error: "` prefix (do not leave this open).** The new
  message **omits** the `"error: "` literal. `main.rs` does `eprintln!("error: {e}")`, so a
  message that opens with its own prefix renders `error: error: …`. `combinator_cycle_error`
  does exactly that today; that pre-existing doubling is **out of scope** and is not fixed
  here -- this slice only declines to add a second instance of it. The golden pins the
  choice by asserting the rendered stderr line contains the literal `"error: "` (with its
  trailing space) exactly once.
- **R3.6** Exact message text. Builder-side (no prefix):

  ```text
  a trait member cannot dispatch back to itself (lowering would splice it forever): {rendered} exceeded the splice budget of {SPLICE_BUDGET} (line {line}, col {col})
  ```

  Rendered on stderr for the witness above, `main.rs` supplying the one prefix:

  ```text
  error: a trait member cannot dispatch back to itself (lowering would splice it forever): `cmp` (member of trait `Ord` for `Wrap`) exceeded the splice budget of 64 (line 6, col 5)
  ```

  The line/col are the **member declaration's** -- the `: cmp` inside `impl: Ord for Wrap`
  in the user's own source -- not a call site. The literal `6`/`5` above are illustrative of
  the witness as written; the golden asserts them against the fixture it itself writes
  (R4.1), not against this document.
- **R3.7** Exit is non-zero and *not* 134. `main.rs`'s existing error path supplies the exit
  code; nothing new is added for it.

### R4 -- tests

- **R4.1 (golden, the exit criterion).** A new integration test file
  `tests/phase7_slice10.rs`, modelled on `tests/phase7_slice3s_flip.rs` (its `Tree` /
  `sooth_build` / `build_error` helpers and `tests/common/mod.rs::fixture_package` are the
  scaffolding source of truth -- **do not invent Sooth syntax, and never copy from
  `docs/book/`, which teaches rejected syntax**). It builds the `Wrap` witness above and
  asserts, on the captured stderr:
  1. it contains the exact R3.6 message text, with the fixture's own line/col substituted;
  2. the count of `"error: "` occurrences in stderr is exactly 1 (R3.5);
  3. the rendered `(line N, col M)` equals the fixture's own `: cmp` declaration position
     (R3.3). A "does not contain `cmp.sth`" assertion is **forbidden here**: the message
     format carries no file path at all, so it passes under every implementation including
     one reporting the library span, and it is exactly the assertion that would otherwise
     have to catch a regression.
  3b. a **second fixture variant**, identical but with two blank lines inserted before the
     `impl:` block, reports a line greater by exactly that offset. Assertion 3 alone can be
     satisfied by a pinned constant; the pair cannot, and this two-way check is how the
     mechanism was verified in the first place.
  4. the build exit status is unsuccessful and `status.code() == Some(1)`, i.e. not a
     signal death (R3.7).
- **R4.2 (`.expect()`-free path).** The golden proves it for the **one** path the `Wrap`
  witness traverses (`lower` → `lower_word` → `lower_word_parts` → `lower_terms` →
  `lower_call` → `lower_resolved_word_call`): a surviving `.expect()` or panic there
  produces Rust panic output and a different exit, failing assertions 1 and 4. The
  implementer states in the phase report that the observed exit was 1, not 101 or 134.

  That is narrower than the exit criterion, which covers all 18 closure functions. A
  swallowing dam in any untraversed member (`lower_eliminator`, `lower_clauses`,
  `lower_if`, `lower_array_word`, `lower_self_tail_combinator`, `lower_instantiation`,
  `lower_line`, the two `destructors.rs` functions) is invisible to the golden. The phase
  report must therefore also record a grep inventory over the 18 functions for `.expect(`,
  `.unwrap(`, `let _ =` and `if let Ok(` applied to a now-fallible call, with each hit
  either removed or justified.
- **R4.3 (no false rejection -- mandatory).** P7.S8's
  `a_concrete_impl_ord_delegating_to_lt_builds_and_runs`
  (`tests/phase7_slice3s_flip.rs:397`) must still pass unmodified, and the
  field-delegating `impl: Ord for Point` (`POINT_IMPL`, same file, line 93) must still
  build and run. The whole suite green is the broader form of this.
- **R4.4 (unit tests beside the stage functions).** Per CLAUDE.md, the changed stage code
  gets unit tests beside it. Two things are separable and both get one:
  - in `src/ir/func_builder/calls.rs`: the depth arithmetic -- a lowering that legitimately
    re-splices returns `Ok` and leaves `member_splice_depth` back at 0.
  - in `src/ir/driver.rs`: error propagation -- an error raised inside the lowering tree
    arrives at `lower()`'s `Err` unchanged, rather than being swallowed.

    `check()` does not run the trait/impl pre-passes, so a plain `lower_src`-style unit test
    over an `impl:` fixture dies with "binds no word". The required route is to call them
    directly: `check_trait_decls` and `check_impl_decls` are both `pub(crate)`
    (`src/check.rs:75-79`), so a unit test in `src/ir/driver.rs` can run them before
    `check` and drive the **real** recursive fixture. Attempt that first and name it in the
    phase report. A *synthetic* forced error is a documented fallback only, not a coequal
    branch: it proves the propagation mechanism against itself and nothing about the guard.
    If the fallback is used, the phase report must say why the real route failed.
- **R4.5 (mutation test the guard -- state the recipe, do not ship a placebo).** This project
  has shipped placebo tests repeatedly, so the reviewer must be handed a concrete recipe.
  Both mutations are run against a **committed** tree (uncommitted work has been destroyed
  by mutation-restore steps before):
  1. **Raise the budget**: `SPLICE_BUDGET` 64 → 100_000. The named test
     `a_recursive_impl_ord_is_a_located_diagnostic_not_a_stack_overflow` (R4.1) must FAIL --
     the subprocess dies by signal again, so assertion 4 (`code() == Some(1)`) trips. The
     abort is contained in the child process, so the test harness reports a failure rather
     than dying.
  2. **Delete the guard** (remove the `if … >= SPLICE_BUDGET` block): the same named test
     must FAIL, same way.
  3. **Corrupt the span**: make the `member_spans` lookup yield another word's span (or the
     library span). Assertion 3/3b must FAIL. Without this, the span half of the diagnostic
     is unguarded -- which is how the original version of this spec shipped an assertion
     that could not fail.
  Classify on `test result: FAILED`, not on exit status alone. A mutation that leaves the
  suite green means the test is a placebo and the phase is not done.
- **R4.6** The budget's *legitimacy* margin is not re-measured here; the brief measured it
  (max 2 across the corpus). R4.3 plus a green suite is the standing evidence that 64 is
  above the legitimate ceiling.

## Out of scope

- Any change to `check_combinator_cycles`, `is_combinator`, or the P7.S8 uid rules.
- Precise, type-directed cycle detection on resolved dispatch targets (a sharper message at
  the cost of a second graph at a new pipeline stage). Deferred deliberately; revisit only
  if the budget diagnostic proves confusing in practice.
- Fixing `combinator_cycle_error`'s pre-existing doubled prefix (R3.5).
- Bulk `.expect()` removal in `func_builder` (R2.4).
- The other two P7.S8 follow-ups (unsatisfied-`Ord` attribution, REPL trait/impl
  accumulation).

## Phasing

**The `Result` threading cannot be honestly split, and is one phase marked `hard`.**

The reasoning, since the instinct is to split a wide refactor: a partial thread does not
compile. Making `lower_terms` fallible forces every caller to handle the `Result`, and
`Result` is `#[must_use]`, so under `-D warnings` an ignored one is fatal. The only way to
stop the wave at a phase boundary is an `.expect()` dam at each frontier -- which is
(a) exactly the panic the slice exists to remove, and (b) forbidden by R2.2. A split would
therefore be a fake split: two phases of churn on the same 18 functions, with the
intermediate state shipping the defect in a new spelling. The measured closure is 18
non-test functions across 6 files, which is wide but mechanical and tractable as one unit.
`hard` is the honest label.

Phase 2 depends on phase 1's `Result` and phase 1 alone introduces no caller for a
diagnostic -- that is not pre-staged plumbing under CLAUDE.md, because every `Result` phase 1
adds is consumed by a `?` in the same phase. Phase 1 adds no import, helper, field, or const
that phase 2 is the first to use; the guard's `const`, the outermost-frame field, and the
`Span` parameter all land in phase 2 with their call sites.

### Phase 1 -- thread `Result` through the lowering tree (`hard`)

Mechanical, wide, no behaviour change: every function in the closure table returns
`Result<_, String>` and propagates with `?`; no error is *produced* yet, so every path still
yields `Ok`. Update `src/repl.rs`'s 6 non-test call sites (enumerated above) and the
`src/backend/qbe.rs` /
`src/ir/driver.rs` test call sites. R2.1–R2.3 and R2.5.

Exit: `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green, with **no test
modified except to unwrap a newly-`Result` call**. Any test whose *assertions* had to change
is a signal that the refactor changed behaviour; stop and report rather than editing the
assertion.

### Phase 2 -- the guard and the diagnostic

R1 (budget const, guard placement, counter reuse), R3 (outermost symbol, the `member_spans`
map and its 5-site threading, `render_word`, exact text, prefix ruling), R4.1, R4.2, R4.4,
R4.5. Verify R4.3 explicitly by name, not just via a green suite.

`SPLICE_BUDGET`, the outermost-symbol field and `member_spans` all land here, together with
their uses: nothing is introduced a phase ahead of its call site, which would be
clippy-fatal under `-D warnings`.

Exit: the green triple; the R4.1 golden passes, including the 3/3b span pair; all three
R4.5 mutations reported as making the named test FAIL;
`a_concrete_impl_ord_delegating_to_lt_builds_and_runs` passes unmodified.

### Phase 3 -- roadmap and brief bookkeeping

Flip the P7.S10 entry in `docs/roadmap/P7-language-prereqs.md` from `[ planned ]` to
`[ done ]` and record the shipped diagnostic text. Per house rule, ROADMAP/DESIGN state the
current design only -- no narration of how the decision was reached. No code changes.

## Exit criteria

Carried from the brief:

- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green.
- A recursive-type `impl: Ord` whose `cmp` compares its own type produces a **located
  diagnostic** and a **non-zero exit, never SIGABRT**, pinned as a golden test on the exact
  message text.
- The diagnostic points at the offending `impl:` block's own member declaration, in the
  user's source. No call-site span is used, and none is available: see R3.3.
- The field-delegating `impl: Ord for Point` still builds and runs, and P7.S8's
  `a_concrete_impl_ord_delegating_to_lt_builds_and_runs` still passes -- the explicit
  no-false-rejection guard.
- No `.expect()`/panic remains on the path the budget guards, and the `Result` threading
  leaves no lowering error silently discarded.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Thread Result through the lowering tree: the 18-function transitive caller closure of lower_terms/lower_term/lower_call in src/ir/func_builder, src/ir/driver.rs and src/ir/destructors.rs returns Result<_, String> and propagates with ?, with no expect/unwrap dams; update the src/repl.rs and test call sites. No behaviour change, no error produced yet.",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "Add the splice-depth budget guard (SPLICE_BUDGET 64, reusing member_splice_depth) in lower_resolved_word_call, thread a member_spans map so the diagnostic locates the offending impl member's own declaration, render it via render_word with no error: prefix, and pin it as a golden test plus unit tests and the three-mutation recipe.",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 3,
      "focus": "Flip the P7.S10 roadmap entry to done and record the shipped diagnostic text; docs only.",
      "effort": "S",
      "difficulty": "standard"
    }
  ]
}
```
