# P7.S6c -- Runtime bounds-checked indexing of a generic-length array in a non-inline poly body (spec)

Status: **spec, 2 phases planned**. Input: [slice6c-brief](./slice6c-brief.md).
Roadmap: [P7-language-prereqs](../P7-language-prereqs.md), P7.S6c. Sequence: after
S6b (explicit length arguments at call sites, `6c4dcbb`).

## Problem

`&>`/`&!>` on an `array['T 'N]` inside a **non-inline** poly body is
unconditionally rejected. `poly_reference_word`'s `">"` arm
(`src/check/poly.rs`, the `count` match at `:6299`) has a `Len::Var(v)` case
that returns `poly_generic_length_index_error` (`:6305`, defined at `:11388`)
before it ever reaches the array-index check. The array's length is the type
variable `'N`, not concrete at the single generic diagnostic walk over the
poly body's *declaration*, so that walk refuses to try.

The documented workaround is `inline`: an inline word's body re-splices and
re-checks per call site, where `'N` is already a concrete `u32`, which is why
every combinator in `lib/core/combinators.sth` indexes with `&>` and this slice
does not. A non-inline poly word has no per-call-site re-check, so indexing one
is dead today, and S6a's impl-dispatch golden had to distinguish monomorphs by
a hardcoded per-impl constant rather than an indexed element.

## What already works (verified by reading, not by trusting roadmap prose)

The roadmap framed this as a three-layer change (checker + lowering + QBE
backend). Two of the three layers are already built and already load-bearing
for the accepted `Len::Concrete` case; only the checker layer is closed.

- **Lowering already runs per-monomorph, not per-declaration.** `src/ir/driver.rs`
  emits one monomorphized `IrFunc` per distinct recorded instantiation (R9). By
  the time this second walk runs, a call site's `'N` is a concrete `u32`.
- **`apply_subst`'s `PolyType::Array` arm already grounds `Len::Var` to a
  concrete count** for that per-monomorph walk: `subst.len_of(*ln)` reads the
  count this instantiation seeded (by inference or, since S6b, an explicit
  `at[i64 4]`). This is already exercised for every other `Len::Var` position
  in a poly body (e.g. S6b's `len` read-back). Indexing is the one position it
  is never reached for, because the checker returns `Err` first.
- **The IR-level runtime bounds guard already exists and needs no change.**
  `bounds_check(index, count, line)` (`src/ir/func_builder/word_families.rs:933`)
  emits the `Cmp`/`Jnz`/trap-block/`sooth_oob_trap` sequence for a non-literal
  index against a concrete `u32` count -- the live path for every monomorphic
  array indexed by a computed index. Once a poly-body array's `&>` reaches the
  `">"` arm (`word_families.rs:49`) with a monomorph-concrete `IrType::Array(id)`
  (which `apply_subst` + `intern_array_type` already produce), `array_parts(id)`
  (`:280`) returns a real `count` and `bounds_check` fires exactly as elsewhere.
  `emit_oob_trap`/`OOB_TRAP_SYMBOL` (`src/backend/qbe.rs:906`, `src/ir/types.rs:19`)
  are already wired and emitted unconditionally at module top (`qbe.rs:165`).

The brief's original "thread the runtime length value through the substitution
instead of a compile-time `u32`" is **not needed**: the count *is* a
compile-time `u32` by the time lowering runs, per monomorph. No `TermKind`,
`PolyType`, `Subst`, or IR change.

## Shape of the fix

Checker-only, at one call site plus its helper:

1. The `Len::Var(v)` arm of the `count` match stops returning
   `poly_generic_length_index_error` and instead defers to runtime, the same
   treatment a `Len::Concrete` computed index already gets (`R1`).
2. `check_poly_array_index` widens `count: u32` to `Option<u32>`; its `usize`
   arm is untouched (it never read `count`). Its `i64` arm's non-literal
   sub-case (a computed value, needing `>usize`) is likewise untouched by an
   unknown count -- it already ignores `count`. The literal sub-case never
   actually sees `count = None`: R1.1 intercepts a literal index against an
   unknown length before this function is ever called (`R2`).
3. `poly_generic_length_index_error`'s call site does not move: it stays in
   `poly_reference_word`'s `Len::Var(v)` arm, where `sig`/`v` are already in
   scope to build the message. That arm now calls it only for a **literal**
   index; a **non-literal** index at that same arm defers to
   `check_poly_array_index` with `count = None`, the same treatment a
   `Len::Concrete` computed index already gets. Its text is narrowed to this
   residual case -- the old blanket "cannot index a generic-length array"
   wording is no longer true once a `usize` index is admitted, so the message
   is rewritten in place, not relocated (`R3`).
4. Integration goldens: a non-inline word indexing a generic-length array,
   called at a known length via S6b syntax, plus a runtime out-of-bounds case
   that traps via `sooth_oob_trap` (`R4`).

## Requirements

### R1 -- the `Len::Var` arm defers for a non-literal index, rejects for a literal one

- **R1.1** In `poly_reference_word`'s `">"` arm, the `count` match
  (`src/check/poly.rs:6299`) no longer treats every `Len::Var` as fatal. The
  arm already has `index_lit` (`stack[n - 1].int_val`, bound at `:6251`) in
  scope at this point, so the `Len::Var(v)` case (`:6304`) decides directly:
  a **literal** index (`index_lit.is_some()`) still calls
  `poly_generic_length_index_error` immediately -- `sig`/`v` are already in
  scope here to name the length variable (R3) -- and anything else (a `usize`
  local, a `usize` produced by `>usize`, or a non-literal `i64`) yields
  `count = None`. The `Len::Concrete` case is unchanged and yields
  `Some(count)`. Both feed the same widened `check_poly_array_index` call
  (R2). `int_val` is only ever `Some` for a bare `IntLit` term
  (`TermKind::IntLit`, `:1126`, always typed `Concrete(Type::I64)`); a
  `usize`-typed slot -- whether a local or a `>usize` conversion's result --
  is built through `PolySlot::new` and so always carries `int_val: None`.
  Checking `index_lit.is_some()` is therefore exactly the literal-`i64`
  predicate, with no risk of misfiring on a `usize` value. The rest of the arm
  is unchanged: `poly_ref_array_parts` already returns the element type and
  `Len`, the mutability check already fired above, and the resulting slot is
  still `PolyType::Ref(elem, mutable)`.
- **R1.2** The `Len::Concrete` path stays byte-identical: it still passes a
  concrete count and still statically range-checks an `i64` literal. This slice
  adds a case, it does not move the existing one.

### R2 -- `check_poly_array_index` accepts an unknown count

- **R2.1** `count: u32` widens to `count: Option<u32>` (`src/check/poly.rs:6434`).
  The `usize` arm (`PolyType::Concrete(Type::Usize) => Ok(())`) is untouched: it
  never reads `count`, and a `usize`-typed index over a generic-length array is
  admitted exactly as it is over a concrete-length one, deferring entirely to
  the runtime `bounds_check`. This is the normal shape for a loop counter or a
  `usize` parameter.
- **R2.2** The `i64`-literal sub-branch keeps its today behaviour when `count`
  is `Some`: an in-range literal passes, an out-of-range literal is
  `array_index_out_of_range_error`, a computed (non-literal) `i64` needs the
  explicit `>usize` conversion (`size_conversion_needed_error`, whose actual
  text is `` `>usize` first (a bare integer literal coerces automatically, a
  computed value does not) `` -- `src/check.rs:3032`). By construction (R1.1),
  **this function is never called with a literal index and `count = None`**:
  R1.1's `Len::Var(v)` arm intercepts that exact combination and errors
  itself, in scope for `sig`/`v`, before this function is ever reached. Match
  that invariant explicitly rather than assume it silently, e.g.:

  ```rust
  PolyType::Concrete(Type::I64) => match index_lit {
      Some(idx) => match count {
          Some(c) if idx >= 0 && idx < i64::from(c) => Ok(()),
          Some(c) => Err(array_index_out_of_range_error(ctx, span, c, idx)),
          None => unreachable!(
              "a literal index against an unknown length is intercepted at the \
               Len::Var call site (R1.1) before this function is ever reached"
          ),
      },
      None => Err(size_conversion_needed_error(ctx, span, op, Type::Usize)),
  }
  ```

  (Exact match arms are an implementation choice; the invariant the
  `unreachable!` documents is the requirement.) `count = None`'s only two live
  paths are this `None => size_conversion_needed_error` arm and the untouched
  `usize` arm above -- neither reads `count` at all, so this function's only
  change for an unknown count is the parameter's type and this documented
  dead branch. `poly_generic_length_index_error` is **not** called from this
  function; R1.1 places that call at the original `Len::Var` arm instead,
  where `sig`/`v` are already in scope and no threading is needed.
- **R2.3** The `poly_op_on_variable_error` catch-all (`other =>`) is unchanged.
- **R2.4** The one internal caller of `check_poly_array_index` is R1.1's call
  site; there is no other. By construction (R1.1) that caller never passes a
  literal index alongside `count = None`. The existing unit test
  `check_poly_array_index_bounds_checks_a_literal_and_requires_conversion_otherwise`
  (`src/check/poly.rs:18521`) updates its four existing calls to wrap `count`
  in `Some(..)`, and gains two new cases for `count = None`: a `usize` index
  (admitted, mirroring the untouched arm) and a computed (non-literal) `i64`
  index (`size_conversion_needed_error`, mirroring the `Some`-count case's
  existing behaviour for the same shape). It does **not** gain a
  literal-index-with-`None`-count case -- that combination is unreachable from
  this function's one real caller, so exercising it here would test dead code
  rather than the actual diagnostic. The narrowed `poly_generic_length_index_error`
  message is asserted by the migrated R3.1 test instead (R4.3), which drives
  it through `poly_reference_word`'s own `Len::Var` arm, its real call site.

### R3 -- narrow the diagnostic to its residual case (not delete)

- **R3.1** `poly_generic_length_index_error` (`src/check/poly.rs:11388`) has
  exactly one call site today (the `:6305` arm R1.1 narrows, not removes --
  it still fires, now only for a literal index), and zero assertions on its
  message in `tests/` (confirmed by grep: the only `tests/` hit,
  `phase7_slice6a.rs:91`, is a comment on the pre-existing `len` read-back,
  not this message). **It does have a test assertion in `src/`**:
  `poly_reference_word_rejects_indexing_a_generic_length_array`
  (`src/check/poly.rs:18156`) `assert_eq!`s the exact message against a
  `` &a 0 &> @ `` fixture -- an `i64` **literal** (`0`) index into an
  `array['T 'N]`. Under R1.1 this exact program still errors (a literal index
  against an unknown length still cannot be range-checked), just with the
  narrowed message from R2.2's wording, built at the same call site (R1.1),
  so this is the only test in the suite this slice's checker change touches,
  and it is migrated, not deleted: **in Phase 1** (not Phase 2), rename it to
  reflect the narrower behaviour (e.g.
  `poly_array_index_literal_unknown_length_requires_usize_conversion`,
  following this project's `thing_condition_expected` convention) and update
  its `assert_eq!` to the new message text. Phase 1's exit criteria (below)
  name this migration explicitly so an implementer following the phase plan
  does not land red on it.
- **R3.2** Once R1.1 lands, `poly_generic_length_index_error` is not dead --
  it is still called from the same site (`poly_reference_word`'s `Len::Var(v)`
  arm, `:6304-6305`), just under a narrower condition (only when the index is
  a literal, R1.1) and with revised text (R2.2's message, built at this same
  call site since `sig`/`v` are already in scope there -- no threading into
  `check_poly_array_index` is needed). No deletion, no relocation: the
  function and its doc comment are updated in place to describe the residual
  case, not the blanket one. This is a **Phase 1** change, not Phase 2
  bookkeeping -- the doc comment goes stale the moment R1.1 lands, so leaving
  it for Phase 2 would land Phase 1 with a self-contradicting comment in the
  tree it just edited.
- **R3.3** S6a's spec and the roadmap carry prose ("workaround is `inline`",
  "`poly_generic_length_index_error` stands") that this slice makes false in
  the general case (a `usize` index no longer needs the workaround; only a
  literal one still triggers this diagnostic, and for a narrower reason). Two
  in-repo comments are stale, and **Phase 1 already rewrites both**, as a
  direct consequence of R1.1/R3.2, not as separate work: the `Len::Var(v)`
  arm's own doc comment (`src/check/poly.rs:6299-6301`, "a dependent-bounds
  problem this slice defers" -- no longer accurate once this slice resolves
  it for the non-literal case) and `poly_generic_length_index_error`'s own
  doc comment (`:11386-11387`, "the element cannot be statically
  bounds-checked without a known count" -- true only for the literal-index
  residual case now). Phase 2's check on these two comments (see the Phase 2
  exit criterion) is **confirmatory** -- verifying Phase 1's own rewrite
  landed, not new work. What Phase 2 actually does: update the **live
  roadmap doc** (`P7-language-prereqs.md`'s S6c entry, marking it `[ done ]`
  and replacing its now-stale "unscoped, needs discovery" / "cross-layer
  change" framing with the landed reality -- current design only, per this
  project's no-history convention for ROADMAP/DESIGN). **Do not** rewrite the
  historical mentions in already-landed, condensed specs/briefs
  (`docs/roadmap/P4/slice13-spec.md:195`, `docs/roadmap/P7/slice6a-spec.md:200`,
  `docs/roadmap/P7/slice6a-brief.md:104`, `docs/roadmap/P7/slice6b-brief.md:195`)
  -- those are frozen historical record of what was true when each was written,
  not live claims, and are out of this phase's scope.

### R4 -- tests

Integration goldens run through the real `sooth` binary
(`tests/phase7_slice6c.rs`). The accept goldens (R4.1) reuse
`tests/phase7_slice6b.rs`'s `build_and_run` shape (asserts `status.success()`
and stdout); the trap golden (R4.2) does **not** reuse it as-is, since that
helper asserts a zero exit -- it needs its own assertion path (or a
parameterized variant) checking a non-zero exit and the trap's stderr text
instead.

- **R4.1 (accept, exit criterion)** A non-inline word declaring `array['T 'N]`
  in its signature indexes it with `&>` at a **non-literal** (computed) index,
  called at a known length via S6b's explicit syntax. Candidate fixture (see the
  paper-check obligation below; the exact shape is locked in Phase 1, not here):

  ```sooth
  : at['T: Copy 'N: Len] ( array['T 'N] usize -- 'T )
    | arr i |
    &arr i &> @ | v |
    arr drop
    v ;
  ```

  called `at[i64 4]` over a length-4 array with an in-bounds index, asserting
  the read element on stdout. The index is a `usize` local, so R2.1's `usize`
  arm admits it and the count is `None` at check time; lowering grounds `'N = 4`
  per monomorph and `bounds_check` fires with `count = 4`. The fixture's
  `main.sth` needs `import: intrinsics * ;` at its head -- `>usize` (used in
  R4.2's out-of-range variant) is an intrinsic, not a bare word, and every
  existing integration golden that uses it prepends this import
  (`tests/phase7_slice6b.rs:79`'s `single_file` helper does this
  automatically; reuse that helper or its equivalent rather than writing a
  bare `.sth` file and hitting an avoidable unknown-word compile error).

  **Substitution note.** The live roadmap's decided exit demo
  (`P7-language-prereqs.md:1135`) is a non-inline **by-index sum**
  (`sum['T 'N: Len] ( array['T 'N] -- 'T )` "that actually sums by index",
  called `sum[i64 4]`), not a single-element `at`. This spec substitutes the
  latter because the former needs a loop over the array, and `while` on a
  quotation literal is rejected inside any poly body -- verified directly
  (`` : sum['T: Copy] ( array['T 4] -- 'T ) 0 [ dup 4 lt ] [ 1 + ] while drop
  ; `` fails with `` error: `while` is not permitted on a quotation literal in
  `sum` (line 1) ``). A by-index loop is not currently expressible in a
  non-inline poly body regardless of this slice, so `at` is the smallest
  fixture that still exercises indexing at a bound `'N`; a real `sum` demo is
  blocked on the loop-combinator gap, not on anything this slice controls.

  **`&!>` golden.** `poly_reference_word`'s `mutable` flag
  (`src/check/poly.rs:6231`) is computed once, ahead of the count match both
  sigils share, so `&!>` reaches the identical R1.1/R2 path as `&>`. Add a
  second small accept fixture using `&!>` to mutate an element in place (e.g.
  `&!arr i &!> v !`) and assert the mutation is visible on stdout, so the exit
  criterion's `&>`/`&!>` claim has a witness for both sigils, not one.
- **R4.2 (runtime out-of-bounds trap)** The same word (or a second fixture if
  R4.1's accept body cannot be driven out of range) called with an
  out-of-range index against a length-4 array. `emit_oob_trap` prints to
  stderr and `exit(1)`. The golden asserts concrete things, not the
  unfalsifiable "did not corrupt memory": (1) the built binary's exit code is
  non-zero, and (2) stderr contains the trap's message. Model this directly on
  the existing pure-array (not slice) `bounds_check` precedent,
  `runtime_out_of_range_array_index_traps_and_aborts_native`
  (`tests/phase0.rs:1355`), which is the same guard this slice's monomorph
  reaches and already establishes the right shape: a deliberately **distinct**
  length and index (so a swapped or duplicated trap arg would still be caught,
  not pass a same-valued assertion by accident), plus a `.` sentinel placed
  *before* the out-of-range access and a second one placed *after* it,
  asserting the first prints and the second does not -- proof the process
  aborted at the trap rather than merely happening to exit nonzero for an
  unrelated reason. Two instantiations are not required here (the trap is
  layout-independent), but the index must be a genuine runtime value (e.g.
  produced by `>usize` on a computed value or read from a local), not a
  literal the checker could fold -- `bounds_check` skips its guard entirely for
  a `const_vals`-known index (`word_families.rs:934`), which is pre-existing
  `Len::Concrete` behaviour this slice does not change, but it means a bare
  out-of-range literal here would silently not exercise the guard at all. Same
  `import: intrinsics * ;` requirement as R4.1 (the `>usize` conversion that
  makes the index a genuine runtime value needs it).
- **R4.3 (unit, beside the stage)** Two witnesses, matching R1.1's own new
  decision and R2.4's two new cases:
  - `check_poly_array_index` with `count = None`: a `usize` index is admitted
    (`Ok(())`), and a computed (non-literal) `i64` index still rejects with
    `size_conversion_needed_error` -- both **defer**, one to the runtime
    guard and one to the pre-existing conversion diagnostic; neither reads
    `count`. With `count = Some(k)` its four existing cases are unchanged.
  - The migrated R3.1 test
    (`poly_array_index_literal_unknown_length_requires_usize_conversion`) is
    the witness for the one case that **rejects** rather than defers: a
    literal `i64` index against an unbound length, asserted by its
    `assert_eq!` on the narrowed `poly_generic_length_index_error` text (not
    a bare `expect_err`, so the test can distinguish the correct diagnostic
    from any other error a naive fix might produce), driven through
    `poly_reference_word`'s own `Len::Var` arm (R1.1) -- not through
    `check_poly_array_index`, which this literal-plus-unknown-length
    combination never reaches (R2.4).
- **R4.4 (no false rejection)** The S6a and S6b integration files
  (`tests/phase7_slice6a.rs`, `tests/phase7_slice6b.rs`) pass unmodified: the
  `Len::Concrete` path (R1.2) and the `len` read-back are untouched.
- **R4.5 (mutation recipe, run against a committed tree)** Each mutation must
  make a **named** test fail, classified on `test result: FAILED`, with a
  confirmed rebuild:
  1. revert R1.1's `Len::Var` arm to return an error again -- R4.1's accept
     golden must fail (the index is rejected at check time).
  2. delete R1.1's `index_lit.is_some()` guard in the `Len::Var(v)` arm, so it
     always defers to `check_poly_array_index` with `count = None` instead of
     ever calling `poly_generic_length_index_error` directly -- the migrated
     R3.1 test
     (`poly_array_index_literal_unknown_length_requires_usize_conversion`, née
     `poly_reference_word_rejects_indexing_a_generic_length_array`) is already
     this witness: its fixture (`` &a 0 &> @ ``) is an `i64` **literal** index,
     so under the mutation it now reaches `check_poly_array_index` with
     `index_lit = Some(0)` and `count = None`, hitting R2.2's documented
     `unreachable!()` branch -- the test panics rather than returning the
     expected message, still a `FAILED` (or aborted) result under `cargo
     test`. No new fixture is needed; it is reachable by a test this slice
     already migrates in Phase 1.
  3. delete the runtime `bounds_check` call reached by R4.1's monomorph --
     R4.2's trap golden must fail (the OOB access no longer traps), regardless
     of whether the deleted call is also reached by the pre-existing
     concrete-length path. Sharing the guard with another path does not excuse
     this slice from proving its own golden can detect the guard's removal;
     require R4.2 in this mutation's failure list.

  **Paper pre-check obligation (Phase 1):** before locking R4.1's fixture, hand-
  check the candidate against the parser and checker, four points the brief did
  not resolve: (a) whether an `array['T 'N]` **owning** signature parameter plus
  a `&arr` borrow of the bound local is accepted in a non-inline body (the
  combinators borrow a local too, but they are `inline`); (b) whether `@`
  loading a `Copy`-bounded generic element out of `&'T` is admitted, and whether
  the `'T: Copy` bound is required; (c) the `arr drop` / borrow-release ordering;
  (d) that `import: intrinsics * ;` is present at the top of the fixture source
  before `>usize` is used (R4.1/R4.2). If any of (a)-(c) fails, adjust the
  fixture (e.g. an `&!array['T 'N]` mutate-in-place shape `&!arr i &!> v !`,
  which avoids the `Copy` load) rather than widening this slice's checker
  scope.

## Out of scope

- Any change to `bounds_check`, `bounds_check_dynamic`, `emit_oob_trap`, or
  `OOB_TRAP_SYMBOL` -- all already correct for this shape.
- **Static (compile-time-provable) bounds proofs for a generic length** -- ruled
  out by DESIGN.md's "dependent types: never". This slice is runtime-checked
  indexing only, the same as every other computed-index array access.
- Widening `Len` to a non-array const kind (boolean, string) -- unrelated, and
  already out of scope per S6a for the same DESIGN.md reason.
- Field projection into a generic aggregate (`&f`), which is a separate standing
  gap; indexing does not touch it.
- `tree-sitter-sooth/grammar.js` and `docs/book/`.

## Phasing

**Phase 1 -- R1 + R2 + R3.1 + R3.2 + R4 (S).** The checker change is one arm's
decision plus a signature widening; the diagnostic's call site never moves
(R3.2), only the condition under which it fires and its text narrow. Land it
with the goldens, the unit test, and the migrated R3.1 test in one phase,
because the accept golden is the only end-to-end witness that the deferral
reaches the runtime guard, and the migrated test is the only pre-existing
test this change touches -- leaving either for Phase 2 would land Phase 1 red
or with a self-contradicting stale doc comment. The paper pre-check (R4.5)
runs first, before the fixture is written. Exit: R4.1 builds and runs (both
`&>` and `&!>` goldens); R4.2 traps with the exact stderr text; R4.3's two
unit-level witnesses both assert their message text; R4.4's regression files
pass unmodified; the migrated R3.1 test
(`poly_array_index_literal_unknown_length_requires_usize_conversion`) asserts
the new message; mutations 1, 2, and 3 (R4.5) all confirmed to fail the named
test; `poly_generic_length_index_error`'s own doc comment and the
`Len::Var(v)` arm's doc comment (R3.2) both describe the residual
literal-only case, not the old blanket rejection.

**Phase 2 -- R3.3 + bookkeeping (S).** No further checker change: R3.1's
diagnostic narrowing and R3.2's in-place doc-comment update both already
landed in Phase 1. This phase is prose-only -- confirm the two in-repo
comments R3.2 already rewrote read correctly (R3.3, confirmatory only), mark
S6c `[ done ]` in `P7-language-prereqs.md` and replace its stale "unscoped,
needs discovery" framing with the landed design (current state only;
historical specs/briefs are explicitly not touched, R3.3), and re-run the
growth signals over `src/check/poly.rs` (already at 3/5 per
`poly_rs_split_deferred`; this slice nets a handful of lines moved, not a
large addition, so it does not tip the count). Exit: `cargo fmt --check &&
cargo clippy -- -D warnings && cargo test` green, `P7-language-prereqs.md`'s
S6c entry reads `[ done ]`, and no in-repo comment still claims indexing a
generic-length array is rejected outright (re-confirming Phase 1's own
rewrite, not new work).

## Exit criteria

- A non-inline word declaring `array['T 'N]` can index it with `&>`/`&!>`
  using a runtime (non-literal) index (R4.1); a call site binds `'N`
  explicitly (S6b) or by inference.
- An out-of-range access at runtime traps via `sooth_oob_trap`: a non-zero exit
  code, and stderr contains the trap's located `line`/`index`/`len` message
  (R4.2) -- not an unfalsifiable "did not corrupt memory" claim.
- `poly_generic_length_index_error` is narrowed to the one residual case it
  still covers (a literal index against an unknown length) with accurate text,
  called from the same site it always was; no code path claims a
  `usize`-typed generic-length index is rejected outright (R3).
- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "defer generic-length indexing to the runtime guard for a non-literal index; narrow the literal-index diagnostic in place; accept and trap goldens; migrate the affected unit test", "effort": "S", "difficulty": "standard" },
    { "phase": 2, "focus": "prose bookkeeping: stale comments, live roadmap entry, growth-signal re-check", "effort": "S", "difficulty": "standard" }
  ]
}
```
