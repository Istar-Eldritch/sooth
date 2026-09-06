# P7.S6c -- Runtime bounds-checked indexing of a generic-length array in a non-inline poly body

**Status:** Implemented (commits below). Follows S6b (explicit length arguments
at call sites, `6c4dcbb`).

## What & why

`&>`/`&!>` on an `array['T 'N]` inside a **non-inline** poly body was
unconditionally rejected: `poly_reference_word`'s `">"` arm treated any
`Len::Var` length as fatal (`poly_generic_length_index_error`) before reaching
the array-index check, because the length is the type variable `'N`, not
concrete at the single generic diagnostic walk over the poly body's
*declaration*. The documented workaround was `inline` (an inline word re-splices
per call site, where `'N` is a concrete `u32`), which is why every combinator in
`lib/core/combinators.sth` indexes with `&>` and S6a's impl-dispatch golden had
to distinguish monomorphs by a hardcoded per-impl constant rather than an
indexed element.

Key facts and decisions:

- **The fix is checker-only.** The roadmap framed this as a three-layer change
  (checker + lowering + QBE backend); two layers were already built and
  load-bearing for the accepted `Len::Concrete` case. Lowering already runs
  per-monomorph (`src/ir/driver.rs`, one `IrFunc` per instantiation), and
  `apply_subst`'s `PolyType::Array` arm already grounds `Len::Var` to a concrete
  count via `subst.len_of`. The IR-level `bounds_check` guard
  (`src/ir/func_builder/word_families.rs`) and `emit_oob_trap`/`OOB_TRAP_SYMBOL`
  already fire for any monomorphic array indexed by a computed index. So the
  count *is* a compile-time `u32` by the time lowering runs, per monomorph: no
  `TermKind`, `PolyType`, `Subst`, or IR change was needed (the brief's
  "thread the runtime length through the substitution" was unnecessary).

- **The diagnostic is narrowed, not deleted.** A `usize` index (a bound local
  or a `>usize` conversion result) over a generic-length array now defers to the
  runtime `bounds_check`, exactly like a `Len::Concrete` computed index. A
  **literal** `i64` index against an unknown length still cannot be
  range-checked, so `poly_generic_length_index_error` still fires from its
  original call site (`poly_reference_word`'s `Len::Var` arm) for that residual
  case, with rewritten text. The literal-vs-non-literal split keys on
  `index_lit.is_some()`, which is exactly the literal-`i64` predicate: a
  `usize`-typed slot is built through `PolySlot::new` and always carries
  `int_val: None`.

- **`check_poly_array_index`'s `count` widened to `Option<u32>`.** Its `usize`
  arm and its computed-`i64` arm never read `count`; the literal-`i64` arm keeps
  its `Some`-count behaviour. `count = None` with a literal index is
  unreachable (intercepted at the `Len::Var` call site), documented with an
  `unreachable!()`.

- **Static bounds proofs stay out of scope** (DESIGN.md "dependent types:
  never"): this is runtime-checked indexing only. The by-index `sum` demo the
  roadmap once named is blocked on a separate loop-combinator gap (`while` on a
  quotation literal is rejected in any poly body), so the goldens use a
  single-element `at` shape instead.

## Implementation

Both phases on branch `p7b-s6b`, base `a9eca84`.

- **Phase 1 -- checker deferral + goldens** (`f6c09bc`): in `src/check/poly.rs`,
  the `Len::Var` arm of `poly_reference_word`'s `">"` count match now calls
  `poly_generic_length_index_error` only for a literal index and otherwise
  yields `count = None` into the widened `check_poly_array_index`
  (`count: Option<u32>`, with the documented `unreachable!()` dead branch);
  `poly_generic_length_index_error`'s text and both stale doc comments (its own
  and the `Len::Var` arm's) rewritten in place to the residual literal-only
  case. The affected unit test was migrated (not deleted) to
  `poly_array_index_literal_unknown_length_requires_usize_conversion`, asserting
  the new message. New integration goldens in `tests/phase7_slice6c.rs`:
  `generic_length_array_indexed_at_a_computed_index_builds_and_runs` (`&>`),
  `..._mutated_..._builds_and_runs` (`&!>`), and
  `..._out_of_range_computed_index_traps_at_runtime` (non-zero exit + trap
  stderr, with sentinels proving the abort point).
- **Phase 2 -- prose bookkeeping** (`e38dd51`): marked S6c done in
  `docs/roadmap/P7-language-prereqs.md`, replacing the stale "unscoped, needs
  discovery / cross-layer" framing with the landed design; re-ran the growth
  signals over `src/check/poly.rs` (still 3/5 per `poly_rs_split_deferred`, not
  tipped). Historical mentions in already-condensed specs/briefs left untouched.

## Exit criteria (met)

- A non-inline word declaring `array['T 'N]` indexes it with `&>`/`&!>` at a
  runtime (non-literal) index, with `'N` bound explicitly (S6b) or by inference.
- An out-of-range access traps via `sooth_oob_trap`: non-zero exit plus the
  located `line`/`index`/`len` message on stderr.
- `poly_generic_length_index_error` narrowed to the residual literal-index case,
  called from the same site; no path claims a `usize`-typed generic-length index
  is rejected outright.
- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green.
