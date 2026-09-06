# Phase 7 Slice 6c: runtime bounds-checked indexing of a generic-length array in a non-inline poly body (brief)

**Sequence.** After S6b (explicit length arguments at call sites, done, `6c4dcbb`).
Touches `src/check/poly.rs` (one call site + one error function), and possibly
nothing else. No shared files with the testing/hosted-layer work (S7) or the
higher-kinded-type work (P7b).

**Motivation.** `&>`/`&!>` on `array['T 'N]` inside a **non-inline** poly body was,
at the time of writing, unconditionally rejected (`poly_generic_length_index_error`,
then at `src/check/poly.rs:11388`, call site `:6296`) because the checker's single
generic walk over a poly body's declaration cannot statically prove `index <
'N` for every possible instantiation — `'N` is not yet concrete at that walk.
The documented workaround is `inline`: an inline word's body re-splices and
re-checks per call site, where `'N` is already concrete, which is why every
combinator in `lib/core/combinators.sth` uses this shape. A **non-inline** poly
word has no such per-call-site re-check, so indexing one is dead today.

## What already works (verified by reading, not by trusting the roadmap prose)

The roadmap frames this as a three-layer change ("the checker... lowering...
and the QBE backend"). Reading the actual monomorphization path shows two of
those three layers are **already built and already load-bearing for the
existing accepted case** (`Len::Concrete`); only the checker layer is closed.

- **Lowering already runs per-monomorph, not per-declaration.** `src/ir/driver.rs`
  emits "one monomorphized `IrFunc` per distinct recorded instantiation" (R9,
  comment at `driver.rs:295`). This is a *second* walk over the poly body,
  separate from the single generic diagnostic pass that
  `poly_generic_length_index_error` guards. By the time this second walk runs,
  a specific call site's `'N` is already resolved to a concrete `u32`.
- **`apply_subst`'s `PolyType::Array` arm already grounds `Len::Var` to a
  concrete count for this per-monomorph walk** (`src/check/poly.rs:10261-10269`):

  ```rust
  PolyType::Array(elem, len) => {
      let elem_ty = apply_subst(sig, elem, subst, name, span, ctx, arrays, cells, refs)?;
      let count = match len {
          Len::Concrete(k) => *k,
          Len::Var(ln) => subst.len_of(*ln).ok_or_else(|| {
              poly_unbound_output_error(ctx, span, name, &sig.len_var_names[*ln as usize])
          })?,
      };
      Ok(intern_array_type(arrays, elem_ty, count))
  }
  ```

  `subst.len_of(*ln)` reads the concrete count this call site's monomorphization
  seeded (inference from an operand's count, or, since S6b, an explicit
  `sum[i64 4]`). This path already exists and is already exercised for every
  other `Len::Var` position in a poly body (e.g. `len` read-back, S6b's own
  golden) — indexing is the one position it is never reached for, because
  `poly_generic_length_index_error` returns `Err` before this per-monomorph
  walk is ever entered for an indexing term.
- **The IR-level bounds guard for a runtime index against a *compile-time*
  count already exists and needs no change.** `bounds_check(index: Value, count:
  u32, line: u32)` (`src/ir/func_builder/word_families.rs:933-958`) emits the
  `Cmp`/`Jnz`/trap-block/`sooth_oob_trap` sequence for exactly this shape: a
  non-literal index against a concrete `u32` array count. It is already the
  live path for every monomorphic array indexed by a computed (non-literal)
  index (`mutation_through_reference_emits_no_rebuild`,
  `src/backend/qbe.rs:2599-2624`, pins this exact instruction shape). Once a
  poly-body array's element access reaches `lower_reference_word`'s `">"` arm
  (`src/ir/func_builder/word_families.rs:44-61`) with a monomorph-concrete
  `IrType::Array(id)` (which `apply_subst` + `intern_array_type` already
  produce), `self.array_parts(id)` returns a real `count: u32` and
  `self.bounds_check(index, count, line)` fires exactly as it does for any
  other array today. **The roadmap's "thread the runtime length value through
  the monomorphization substitution instead of a compile-time `u32`" is not
  needed** — the count *is* a compile-time `u32` by the time lowering runs,
  per-monomorph. `emit_oob_trap`/`OOB_TRAP_SYMBOL` (`src/backend/qbe.rs:906-914`,
  `src/ir/types.rs:19`) are also already wired and already emitted
  unconditionally at module top (`qbe.rs:165`) — nothing new to add there
  either.

## What's actually missing

Only the checker-side rejection, and only at one call site:

- **`src/check/poly.rs:6296-6304`** (pre-diff), the `Len::Var(v) => Err(poly_generic_length_index_error(...))`
  arm inside the `count` match that feeds `check_poly_array_index`
  (`:6306`). This is the single generic diagnostic pass over the poly body's
  *declaration* — it runs once, independent of any call site, which is why it
  cannot see a concrete `'N` and currently refuses to try.
- **`check_poly_array_index`** (`src/check/poly.rs`, called at `:6306` with a
  concrete `count: u32` today) needs to either (a) be skippable when `count` is
  not yet known (defer the static literal-index check entirely to the
  per-monomorph walk, where a *literal* index against a concrete count can
  still be checked statically and a non-literal one falls through to the
  runtime guard — this mirrors exactly how a `Len::Concrete` array already
  behaves for a computed index today), or (b) run only its non-literal-index
  branch when the length is a `Len::Var`, deferring literal-index range
  checking to the monomorphized walk (a literal index against an as-yet-unknown
  `'N` cannot be range-checked at declaration-check time in any case — this is
  not new, it is a strict subset of today's already-open literal-vs-computed
  split, just at a length that happens to be a variable rather than a
  constant).

**Resolved by reading `check_poly_array_index`'s current body**
(`src/check/poly.rs:6434-6455`): it already splits on the index's *type*, not
just literal-vs-computed:

```rust
match index_pt {
    PolyType::Concrete(Type::Usize) => Ok(()),
    PolyType::Concrete(Type::I64) => match index_lit {
        Some(idx) if idx >= 0 && idx < i64::from(count) => Ok(()),
        Some(idx) => Err(array_index_out_of_range_error(ctx, span, count, idx)),
        None => Err(size_conversion_needed_error(ctx, span, op, Type::Usize)),
    },
    other => Err(poly_op_on_variable_error(ctx, span, op, other, sig)),
}
```

A `usize`-typed index (the normal shape for a loop counter, since `i64` needs
an explicit `>usize` conversion first) is accepted unconditionally today, for
the already-supported `Len::Concrete` case too: `count` is never even read on
that arm. Zero static bounds checking happens for a computed index; it is
already fully deferred to the runtime `bounds_check` guard at lowering. The
only place `count` matters is the `i64`-literal arm, range-checking a literal
at check time.

So the fix is bounded and mechanical, not a new branch from scratch: when
`count` is unknown (`Len::Var`), the `usize` arm needs no change at all (it
never touched `count`), and the `i64`-literal arm needs to stop range-checking
against an unknown count and instead defer to runtime, the same treatment the
`None` (computed) case already gets today, just triggered by an unknown count
instead of a non-literal index. Likely shape: widen `count` to `Option<u32>`
(or thread the `Len` variant through directly), and have the literal-`i64` arm
fall through to `size_conversion_needed_error` or an equivalent "cannot verify
statically, defer" outcome when `count` is `None`, rather than indexing
`i64::from(count)`.

**Correction (post-spec-write): this brief's own grep claim below was wrong.**
`poly_generic_length_index_error` (`src/check/poly.rs:11388`, pre-diff) has
exactly one call site in the whole tree (`:6305`, pre-diff) — that part holds
— but it does **not** have zero test assertions on its message: at base commit
`a9eca84`, `src/check/poly.rs:18164` asserted its exact message verbatim via
`assert_eq!`. "Safe to delete outright... no test pinning its wording to
migrate" was false when written. The spec (and the shipped design) instead
**narrows** the diagnostic to the one residual case a `usize`-typed index
can't cover — a literal index against an unknown length — and **migrates**
the existing test (not deletes it) to pin the narrowed wording. See
`docs/roadmap/P7/slice6c-spec.md` for the design actually shipped.

## What changes

1. `src/check/poly.rs`'s `Len::Var(v)` arm at the `&>`/`&!>` array-indexing call
   site stops returning `poly_generic_length_index_error`. `check_poly_array_index`
   widens `count: u32` to `count: Option<u32>` (or an equivalent), its `usize`
   arm is untouched (never read `count`), and its `i64`-literal arm falls
   through to `size_conversion_needed_error` (or an equivalent "cannot verify
   statically, defer to runtime" outcome) when `count` is `None`, instead of
   indexing `i64::from(count)`.
2. No `TermKind`, `PolyType`, `Subst`, or IR change — the grounding and the
   runtime guard both already exist and already fire for the parallel
   `Len::Concrete` case.
3. `poly_generic_length_index_error` (`src/check/poly.rs:11388`, pre-diff) is
   **not** dead and does **not** get deleted: an existing test
   (`src/check/poly.rs:18164`, pre-diff) already asserts its exact message via
   `assert_eq!`, which this brief's own grep-based claim above missed. Its
   call site stays reachable for the one residual case a `usize`-typed index
   can't cover — a literal index against an unknown length still can't be
   range-checked — so the fix **narrows** (rewrites) the diagnostic's text to
   that one case and **migrates** the existing test to pin the new wording,
   rather than deleting either. (Shipped as
   `poly_array_index_literal_unknown_length_requires_usize_conversion` in
   `src/check/poly.rs`, at HEAD.)
4. A non-inline integration golden that actually indexes a generic-length
   array, using S6b's explicit-length syntax to call it at a known length —
   e.g. `: sum['T 'N: Len] ( array['T 'N] -- 'T ) …` written with `&>` over a
   loop, called as `sum[i64 4]` — plus a runtime out-of-bounds case exercising
   the `sooth_oob_trap` path if the body's own logic can produce one (or a
   second, deliberately-triggerable fixture if the accept-path body can't).

## Out of scope

- Any change to `bounds_check`, `bounds_check_dynamic`, `emit_oob_trap`, or
  `OOB_TRAP_SYMBOL` — all already correct for this shape.
- Static (compile-time-provable) bounds proofs for a generic length — ruled out
  by DESIGN.md's "Dependent types: never"; this slice is runtime-checked
  indexing only, same as every other computed-index array access in the
  language today.
- Widening `Len` to a non-array const kind (boolean, string) — unrelated, and
  already out of scope per S6a's brief for the same DESIGN.md reason.

## Exit criteria (draft, confirm against spec-write)

A non-inline word declaring `array['T 'N]` in its signature can index it with
`&>`/`&!>` using a runtime (non-literal, `usize`-typed) index; a call site can
bind `'N` explicitly (S6b) or by inference; an out-of-range access at runtime
traps via `sooth_oob_trap` rather than corrupting memory. A **literal** index
against a generic (unknown) length is still rejected at check time — there is
no count to range-check it against, and no runtime-defer path for a value that
is already a compile-time constant — via the narrowed
`poly_generic_length_index_error`, not by deleting the diagnostic. `cargo fmt
--check && cargo clippy -- -D warnings && cargo test` is green.

**(Resolved at spec-write / shipped, see `docs/roadmap/P7/slice6c-spec.md` for
the as-shipped exit criteria and commits.)**

## Open questions

None blocking spec-write. The sizing question this brief originally carried
(whether `check_poly_array_index` needs a real branch, or already degrades
correctly once its caller stops treating `Len::Var` as fatal) is resolved
above by reading the function body: it needs a small, mechanical signature
widening (`count: u32` → `Option<u32>`), not a new code path. The shape of
the fix (checker-only, no lowering/backend change) and its size are both now
known.
