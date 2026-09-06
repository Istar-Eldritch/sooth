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
   arm is untouched (it never read `count`), and its `i64`-literal arm defers to
   runtime when the count is unknown instead of range-checking against a count
   it does not have (`R2`).
3. `poly_generic_length_index_error` becomes dead and is deleted with its
   now-unreachable call site (`R3`).
4. Integration goldens: a non-inline word indexing a generic-length array,
   called at a known length via S6b syntax, plus a runtime out-of-bounds case
   that traps via `sooth_oob_trap` (`R4`).

## Requirements

### R1 -- the `Len::Var` arm defers instead of rejecting

- **R1.1** In `poly_reference_word`'s `">"` arm, the `count` match
  (`src/check/poly.rs:6299`) no longer distinguishes `Len::Var` as fatal. It
  yields `None` (unknown count) for `Len::Var` and `Some(count)` for
  `Len::Concrete`, feeding the widened `check_poly_array_index` (R2). The rest
  of the arm is unchanged: `poly_ref_array_parts` already returns the element
  type and `Len`, the mutability check already fired above, and the resulting
  slot is still `PolyType::Ref(elem, mutable)`.
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
- **R2.2** The `i64`-literal arm keeps its today behaviour when `count` is
  `Some`: an in-range literal passes, an out-of-range literal is
  `array_index_out_of_range_error`, a computed `i64` needs the explicit `>usize`
  conversion (`size_conversion_needed_error`). When `count` is `None`, a
  *literal* `i64` index against an as-yet-unknown `'N` cannot be range-checked
  at declaration-check time, so it falls through to the same
  `size_conversion_needed_error` "cannot verify statically, convert to `usize`"
  outcome the computed case already gets. This is not a new permissiveness: it
  is a strict subset of today's already-open literal-vs-computed split, at a
  length that happens to be a variable rather than a constant.
- **R2.3** The `poly_op_on_variable_error` catch-all (`other =>`) is unchanged.
- **R2.4** The one internal caller of `check_poly_array_index` is R1.1's call
  site; there is no other. The existing unit test
  `check_poly_array_index_bounds_checks_a_literal_and_requires_conversion_otherwise`
  (`src/check/poly.rs:18521`) updates its four calls to wrap `count` in `Some`,
  and gains an `i64`-literal-with-`None`-count case asserting the deferral (R4.3).

### R3 -- delete the dead diagnostic

- **R3.1** `poly_generic_length_index_error` (`src/check/poly.rs:11388`) has
  exactly one call site (the `:6305` arm R1.1 removes) and zero test assertions
  on its message anywhere in `src/` or `tests/` (confirmed by grep: the only
  `tests/` hit, `phase7_slice6a.rs:91`, is a comment on the pre-existing `len`
  read-back, not this message). Once R1.1 lands, it is unreachable and is
  deleted outright, not kept as an unused diagnostic. This follows the
  mutation-test-the-guards convention: a diagnostic that never fires is deleted,
  not preserved as false safety.
- **R3.2** S6a's spec and the `poly_ref_array_parts` neighbourhood carry prose
  ("workaround is `inline`", "`poly_generic_length_index_error` stands") that
  this slice makes false. Update those comments where they sit; do not leave a
  dangling reference to a deleted function.

### R4 -- tests

Integration goldens run through the real `sooth` binary
(`tests/phase7_slice6c.rs`), mirroring `tests/phase7_slice6b.rs`'s harness. All
accept goldens assert `status.success()` and stdout; the trap golden asserts a
non-zero exit and the trap's stderr.

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
  per monomorph and `bounds_check` fires with `count = 4`.
- **R4.2 (runtime out-of-bounds trap)** The same word (or a second fixture if
  R4.1's accept body cannot be driven out of range) called with an
  out-of-range index, e.g. `9` against a length-4 array. `emit_oob_trap` prints
  to stderr and `exit(1)`, so the golden asserts the built binary exits
  non-zero and did **not** corrupt memory / return a garbage value. Two
  instantiations are not required here (the trap is layout-independent), but the
  index must be a genuine runtime value, not a literal the checker could fold.
- **R4.3 (unit, beside the stage)** `check_poly_array_index` with an `i64`
  literal and `count = None` defers (returns the `size_conversion_needed_error`
  outcome) rather than panicking on `i64::from(count)`; with `count = Some(k)` it
  is unchanged. Both directions in one test (R2.4).
- **R4.4 (no false rejection)** The S6a and S6b integration files
  (`tests/phase7_slice6a.rs`, `tests/phase7_slice6b.rs`) pass unmodified: the
  `Len::Concrete` path (R1.2) and the `len` read-back are untouched.
- **R4.5 (mutation recipe, run against a committed tree)** Each mutation must
  make a **named** test fail, classified on `test result: FAILED`, with a
  confirmed rebuild:
  1. revert R1.1's `Len::Var` arm to return an error again -- R4.1's accept
     golden must fail (the index is rejected at check time).
  2. make R2.2's `None`-count `i64`-literal arm index `i64::from(count.unwrap())`
     -- if any test fails, R2.2 has a witness; if all stay green, no fixture
     reaches an `i64` *literal* index over a generic-length array (the R4.1
     candidate uses a `usize` index), so R2.2's literal deferral is untested and
     the phase report says so rather than claiming coverage. Add an `i64`-literal
     fixture only if it is reachable without the `>usize` conversion.
  3. delete the runtime `bounds_check` call reached by R4.1's monomorph -- R4.2's
     trap golden must fail (the OOB access no longer traps). If this guard is
     shared with the concrete path, this mutation is not this slice's to own;
     note it and rely on the pre-existing concrete-path coverage instead.

  **Paper pre-check obligation (Phase 1):** before locking R4.1's fixture, hand-
  check the candidate against the parser and checker, three points the brief did
  not resolve: (a) whether an `array['T 'N]` **owning** signature parameter plus
  a `&arr` borrow of the bound local is accepted in a non-inline body (the
  combinators borrow a local too, but they are `inline`); (b) whether `@`
  loading a `Copy`-bounded generic element out of `&'T` is admitted, and whether
  the `'T: Copy` bound is required; (c) the `arr drop` / borrow-release ordering.
  If any point fails, adjust the fixture (e.g. an `&!array['T 'N]` mutate-in-
  place shape `&!arr i &!> v !`, which avoids the `Copy` load) rather than
  widening this slice's checker scope.

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

**Phase 1 -- R1 + R2 + R4 (S).** The checker change is one arm plus a signature
widening; land it with the goldens and the unit test in one phase, because the
accept golden is the only end-to-end witness that the deferral reaches the
runtime guard, and neither half is meaningful alone. The paper pre-check (R4.5)
runs first, before the fixture is written. Exit: R4.1 builds and runs; R4.2
traps; R4.3's unit test; R4.4's regression files pass; mutations 1 and 3 (and 2
if reachable).

**Phase 2 -- R3 + bookkeeping (S).** Delete `poly_generic_length_index_error`
and its now-dead call site, fix the stale S6a/`poly_ref_array_parts` prose (R3.2),
mark S6c `[ done ]` in `P7-language-prereqs.md`, and re-run the growth signals
over `src/check/poly.rs` (already at 3/5 per `poly_rs_split_deferred`; this slice
removes lines rather than adding, so it does not tip the count). Exit: `cargo
fmt --check && cargo clippy -- -D warnings && cargo test` green with no
reference to the deleted function anywhere.

## Exit criteria

- A non-inline word declaring `array['T 'N]` can index it with `&>`/`&!>` using a
  runtime (non-literal) index; a call site binds `'N` explicitly (S6b) or by
  inference.
- An out-of-range access at runtime traps via `sooth_oob_trap` (`exit(1)`,
  stderr message) rather than corrupting memory or returning a garbage value.
- `poly_generic_length_index_error` is deleted; nothing references it.
- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "defer generic-length indexing to the runtime guard; accept and trap goldens", "effort": "S", "difficulty": "standard" },
    { "phase": 2, "focus": "delete the dead diagnostic and bookkeeping", "effort": "S", "difficulty": "standard" }
  ]
}
```
