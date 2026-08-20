# Phase 7 Slice 3c: slicing a buffer into a view (spec)

## Goal

Add `Type::Slice(SliceId)`: a borrowed, length-carrying view over a buffer. A
non-`inline` word can take `Slice['T]` as a parameter and index it without naming a
length variable, so the checker never has to prove an index against an abstract `'N`.
This is the signature shape the trait-bounds consumers (P7.S3e) want, and it removes the
two warts of today's only working spelling: a fixed length in the type (`&[i64 5]`) and a
threaded-length second parameter (`( &[i64 5] usize -- i64 )`).

A slice is **second-class, input-only, non-owning**: constructing one consumes nothing,
no user word may return one, and the declaration-level output ban
(`stored_reference_output_error`) covers it exactly as it covers `&T`. Element access
keeps the existing runtime out-of-bounds trap; there is **no** fallible `Option`/`Result`
accessor in this slice (see R9 and the roadmap correction in R10).

The brief (`docs/roadmap/P7/slice3c-brief.md`) is probe-grounded against the built
compiler; its "Locked decisions" are binding and are not re-opened here.

## Problem Statement

- **Business context:** A word that works over a borrowed buffer of *any* length cannot
  be written today. A generic-length array (`&[i64 'N]`) cannot be indexed
  (`cannot index a generic-length array`), and `len` refuses a reference outright
  (`len is not permitted on a reference`). The only escape is `inline`, which is why
  every generic word in `lib/arrays.sth` is `inline`; a slice-shaped signature is the
  prerequisite for the P7.S3e trait-bounds consumers.
- **Current state:** `str` (`Type::Str` / `IrType::Str`) is the one existing
  pointer-plus-length type, but it is a *single* 8-byte opaque address of a
  statically-built `{ptr, len}` descriptor (`emit_str_literal`, `backend/qbe.rs:733`,
  `STR_LEN_OFFSET = 8` at `:724`). A slice's pointer and length are computed at runtime,
  so that static-descriptor trick does not carry: a slice needs a genuinely two-word
  value at the IR and ABI level.
- **Key issues:** The representation is `Type::Slice(SliceId)`, its own variant. That
  choice trades silent unsoundness (reusing a fat `Type::Ref` would leave every existing
  `IrType::Ptr` site compiling and wrong, a diffuse failure set) for an *enumerable*
  failure set: a fixed list of forced match arms plus nine wildcard sites that must be
  ported deliberately. Every one of those ports needs a test that fails when its arm is
  missing.

## Requirements

Each requirement is an independently verifiable claim. Anchors pair `path:line` with a
symbol name; line numbers may drift, so re-locate by symbol.

### R1 `Type::Slice(SliceId)` exists as its own variant with an interned registry

- **R1.1** A new `Type::Slice(SliceId, &'static str)` variant in `src/ast.rs:1437`
  (`enum Type`), modelled on `Type::Array(ArrayId, &'static str)` at `ast.rs:1451`:
  a small `Copy` `SliceId` index into a per-module interned registry keyed by element
  type, plus a leaked `Slice[T]` display spelling so two slices of the same element
  compare byte-identical. The `SliceId` newtype mirrors `ArrayId` (`ast.rs:991`) with a
  crate-internal `from_index`.
- **R1.2** The registry interns by element `Type` (one `SliceId` per concrete element),
  mirroring the array interner (`ast.rs:1006-1020`). Element types are **concrete and
  `Copy` only** (locked): a generic element (`Slice['T]` over a word's own variable) is
  out of scope, blocked on generic-instantiation-over-own-variable, and a linear element
  has no buffer to view (`linear array elements are not supported yet`).
- **R1.3** The three compile-forced exhaustive `Type` matches each gain a deliberate
  arm, no `_ =>` catch-all: `Type::name` (`ast.rs:1764`) renders `Slice[T]`;
  `ir_type_of` (`ir/types.rs:266`, beside the `Type::Str => IrType::Str` arm) maps to
  the new `IrType::Slice` (R2); the value-containment graph
  (`check/declarations.rs:1181`) treats a slice as reference-shaped (contains a
  reference, never owns storage).

### R2 `IrType::Slice(SliceId)` exists, lowered as a 16-byte `{ptr, len}` aggregate

- **R2.1** A new `IrType::Slice(SliceId)` variant in `src/ir/types.rs:96` (`enum
  IrType`), keyed by a `Copy` `SliceId` into a per-`SliceId` IR layout registry, so
  `IrType` stays `Copy` (the same discipline as `Struct`/`Enum`/`Array`). Its runtime
  representation is a **two-word aggregate**: an opaque element pointer plus a
  target-width length. `str`'s single-word static-descriptor shape does **not** carry and
  must not be imitated.
- **R2.2** Every compile-forced `IrType` arm gains a deliberate arm (the brief
  probe-recounts ~13; none may be a silent wildcard). Named sites:
  - `field_width` (`ir/layout.rs:307`, the `... | Str | Cstr | Code => 8` one-word arm) —
    a slice is 16 bytes, not 8.
  - `carried_slot_bytes` (`ir/layout.rs:323`) — a slice marshals as a 16-byte aggregate
    rounded up to an 8-multiple, like `Quotation`, not the 8-byte scalar arm.
  - register-class / ABI classification in `backend/qbe.rs` at the two `IrType` matches
    around `:311` and `:360` (the `w`/`l` spellings), `field_load_op` (`qbe.rs:365`),
    `field_store_op` (`qbe.rs:399` region), and the aggregate-blit classification
    (`qbe.rs:423` region): a slice is an aggregate (`:S`-style / blit), never a scalar
    load/store.
  - `ir/driver.rs:507` (the load-vs-blit dispatch that ends `Code | Quotation =>
    unreachable!()`): a slice is blit-copied, not scalar-`Load`ed.
  - `Instr::Print` dispatch (`backend/qbe.rs:1125` region) and both REPL rich-value
    renderers (`rich_value_size` / `render_rich_value` in `src/repl.rs`): a printed or
    REPL-rendered slice needs its own arm (or an explicit located "not printable/
    renderable" arm consistent with the checker's printable-set decision, R7).

### R3 `qbe_abi_ty` classifies a slice as a 16-byte aggregate (soundness wildcard)

`qbe_abi_ty` (`backend/qbe.rs:341`, currently ending `_ => width()`) must gain an explicit
slice arm returning the aggregate ABI spelling, **not** a scalar register width. This is
the `IrType`-side twin of the fat-`Ref` failure the separate-variant decision avoids: a
slice crossing a param/return/arg boundary with a scalar width is silently wrong with no
compile error. The arm is anchored, not left to the wildcard.

### R4 `is_copy` splits a slice by mutability (soundness wildcard, sharpest)

`is_copy` (`check/builtins.rs:233`, wildcard `_ => true` at `:251`) must answer a slice
the way it answers `Type::Ref(_, mutable, _) => !mutable` at `:250`: a **shared** slice is
`Copy`, a **mutable** slice is not. The `_ => true` wildcard would make a `&!` slice
freely duplicable and break exclusivity outright. A slice therefore carries its
mutability (shared vs mutable view), mirroring how `Type::Ref` carries it as the
classification bit. The poly twin `poly_is_copy` inherits this via `PolyType::Concrete`
delegation and gets the same treatment (R8).

### R5 `contains_reference` reports a slice as reference-bearing (soundness wildcard)

`contains_reference` (`check/builtins.rs:279`, wildcard `_ => false` at `:299`) must
return `true` for a slice. This is what keeps the output ban covering slices: a slice
reported reference-free would let `stored_reference_output_error`
(`check/builtins.rs:311`, via `check_reference_free_signature`) stop firing, and a user
word could declare `( -- Slice['T] )`. The grep hazard is explicit: neither `is_copy` nor
`contains_reference` contains the token `Str`, so a grep-driven port misses exactly the
two most load-bearing sites.

### R6 `find_zero_unsafe_element` names a slice zero-unsafe (soundness wildcard)

`find_zero_unsafe_element` (`check.rs:425`) names `Type::Str | Type::Cstr |
Type::Quotation(_)` *explicitly* as zero-unsafe and falls everything else to a wildcard
treated as zero-**safe**. A slice must be added to the explicit zero-unsafe set;
otherwise an all-zero slice is silently admitted out of the array constructor.

### R7 The remaining checker wildcard/guard sites gain slice arms

Each an anchored, deliberate arm (functional gaps, not soundness holes, but none dropped):

- `check/operators.rs:342` — the `.` printable set: decide and encode whether a slice is
  printable; the REPL/print renderers in R2.2 must match this decision.
- `check/word_families.rs:685` (`len`/`cstr` quotation guard) and `:775` (`check_array_word`
  `len` arm) — see R9 for the `len`-answers-a-slice behaviour.
- `check/declarations.rs` extern boundary (the two `extern` arms near `:176`/`:187`) — a
  slice at the FFI boundary is rejected with a located error (an aggregate view is not a
  C-ABI scalar).
- `repl.rs:604` region `format_stack` — a slice on the REPL stack renders through R2.2.

### R8 `classify_capture`, `remap_type`, and the poly twins gain slice arms (soundness wildcards)

- **R8.1** `classify_capture` (`check/captures.rs:144`, wildcard `_ =>
  CaptureClass::Scalar`) must classify a captured slice by its reference nature (the same
  `FrameRooted`/escape reasoning the `Type::Ref(..)` arm at `:169` uses), not as a plain
  scalar. A slice analysed as a scalar at a quotation-materialization boundary is an
  escape-analysis hole of the same shape as R4–R6.
- **R8.2** `remap_type` (`repl.rs:195`, wildcard `other => other` at `:219`) must rebase a
  `SliceId` by the module base on cross-module import, the way it rebases
  `Struct`/`Enum`/`Array`/`OwnedCell`/`Ref` ids. An un-rebased `SliceId` collides in
  session space — the same bug class as `project_span_lacked_module_id`.
- **R8.3** The poly predicate twins gain deliberate arms: `poly_is_copy`
  (`check/poly.rs`, mutability split per R4), poly `len` (`check/poly.rs:783`),
  `is_reference_slot` (a mutable/shared slice is reference-shaped), and `poly_type_str`
  (renders `Slice[T]`). OQ4 (R11) measures the full poly-path extent at Phase 1 start;
  the predicate arms here are the non-optional minimum.

### R9 A `len` arm, and index ops taught a runtime bound

- **R9.1** `len` over a slice answers its **runtime** length (the carried length, never
  rediscovered by scanning), where `len` refuses a reference today. Dispatched in the
  `check_array_word` `len` arm (`word_families.rs:775`) with the slice receiver.
- **R9.2** The index ops `&>` (shared) and `&!>` (mutable) accept a slice receiver and
  produce an element reference (`&T` / `&!T`) bounds-checked against the **runtime**
  length. An out-of-range index traps at runtime via the existing `emit_oob_trap`
  (`backend/qbe.rs:796`), with the same located message array indexing produces. `&>`
  already produces a `&T` output today and is exempt from the signature audit, so no new
  grammar is required. A `&!>` element reference off a mutable slice is exclusivity- and
  extent-tracked exactly as an array element reference is (R12).

### R10 Construction and sub-ranging as compiler-known words

- **R10.1** Construction word **`slice`**: `( &[T N] -- Slice[T] )`, turning a borrowable
  buffer reference into a view carrying its length at runtime. Sub-ranging word
  **`subslice`**: `( Slice[T] usize usize -- Slice[T] )` taking a start offset and a
  length. Both are compiler-known arms in the array-word family (`check_array_word`,
  `word_families.rs:716`), dispatched by name, exempt from the signature audit exactly as
  `&>` is. **No new grammar.**
- **R10.2** (naming decision, OQ2 resolved) `slice`/`subslice` are ordinary word-shaped
  names, deliberately **not** sigil-shaped. The recorded longer-term intent for
  projections is a distinct `&`-consistent prefix-sigil form; keeping these two as plain
  words means that later change adopts the sigil namespace without either word needing to
  move or fight for it. Reasoning stated as a spec decision, not left dangling.
- **R10.3** (OQ1 resolved) `subslice` **constructs a fresh `Slice` value** from the
  receiver's pointer and length (pointer offset by `start`, new length `len`), consuming
  or reborrowing the receiver as an ordinary input — the way `&>` consumes an array
  reference to produce an element reference. It is **never** a reference-to-a-reference:
  references cannot nest (probed closed on all four forms), so `s 0 mid subslice` is a
  re-derivation, not a re-borrow. The recursive consumer's `s 0 mid subslice rec` shape is
  valid only under this reading, and the spec states it.

### R11 (OQ4) Poly-path extent measured, then scoped

Phase 1 begins with a measurement step: enumerate which `PolyBorrow`/poly-walk arms a
slice used inside a polymorphic body actually forces, given `PolyBorrow` is coarser than
`Deriv` (no `projected` flag, `check/poly.rs:96`) and poly `len` is a separate match
(`poly.rs:783`). A slice inside a poly body is the point of the slice, so the poly twins
are **not** optional. The predicate twins land in Phase 1 (R8.3); the poly borrow-tracking
arms land in Phase 4 (R12). If the measurement finds a poly capability that cannot be
delivered without re-opening a locked non-goal (generic elements, row unification), that
specific capability is declared out of scope in the phase's exit notes with a one-line
rationale, and the concrete-element poly consumer is what ships.

### R12 Borrow rules ported: exclusivity, non-escaping, output ban

A slice is exclusivity-tracked, non-escaping, and banned from outputs exactly as a `&T`
is. The output ban is already covered by R5 (`contains_reference` true). Exclusivity: a
mutable slice participates in the coarse per-place borrow table (`Deriv`,
`check/engine.rs:55`; `PolyBorrow`, `check/poly.rs:96`) so at most one `&!` view is live
and a `&` cannot coexist with a `&!`, matching array/struct-field behaviour. Range-aware
tracking is **out of scope**: two simultaneously-live disjoint mutable sub-slices stay
rejected by the coarse table. Sequential mutation (one live mutable borrow at a time, each
retired by the call that consumes it), including recursive divide-and-conquer taking one
half at a time, is what is delivered.

### R13 (OQ3) Reborrow-chain-depth test through a slice's stored reference

A named golden/unit test mutates through the **innermost** hop of a
`&!buffer -> &!Slice -> &!sub-slice -> &!element` chain while the outer hops are still
live. This is the one new shape the brief's reborrow probe did not cover: today's deepest
chains route through owned-cell deref and array indexing, never through a reference stored
inside an aggregate that outlives the expression, which is exactly what a `{ptr, len}`
slice is. The test is concrete and required, not aspirational.

### R14 Roadmap exit criterion corrected to match the locked decision

The current P7.S3c exit criterion in `docs/roadmap/P7-language-prereqs.md` (the `**Exit:**`
line at `:187`, and the body sentence at `:139`-region: "Indexing a slice is *fallible* …
it reports through an `Option`/`Result` … so the compile-time guarantee is kept without a
runtime panic") describes a change that cannot be built until traits (P7.S3e) provide a
user-declarable `Option`/`Result` carrier. This slice keeps the existing runtime OOB trap.
The roadmap text must be rewritten so its exit criterion states the runtime-trap reality,
not the stale fallible-accessor promise. This spec's own exit criteria (below) already
state the trap; the roadmap edit is a scoped Phase 5 deliverable so the stale promise is
not quietly restated anywhere.

## Success Criteria

- [ ] `: sum ( Slice[i64] -- i64 ) | s | 0 s len >i64 ~[ | i | s i >usize &> @ add ] times ;`
      compiles as a **non-`inline`** word and, called on a length-5 buffer via `slice`,
      prints `25` (golden, diffed against the length-threading twin `( &[i64 5] usize -- i64 )`
      that prints `25` today).
- [ ] A recursive `rec ( Slice[i64] -- i64 )` doing divide-and-conquer over `subslice`
      halves compiles and runs (golden).
- [ ] `( -- Slice['T] )` as a declared user output is rejected with
      `stored_reference_output_error` (`a reference cannot be stored`) — the output ban
      covers slices (asserts R5).
- [ ] `dup` on a `&!` (mutable) slice is a located error; `dup` on a shared slice is
      accepted (asserts R4).
- [ ] Two simultaneously-live mutable sub-slices are rejected with the existing coarse
      borrow-table diagnostic (asserts R12).
- [ ] An out-of-range slice index traps at runtime with the located OOB message, and no
      `Option`/`Result` accessor exists (asserts R9.2, R14).
- [ ] `docs/roadmap/P7-language-prereqs.md`'s P7.S3c exit criterion states the runtime
      trap, not a fallible `Option`/`Result` accessor (asserts R14).

## Scope & Boundaries

**In scope:** everything in R1–R14.

**Out of scope (binding, from the brief — none of these may be scheduled in any phase):**

- Any fallible / `Option`-returning accessor, and the `Option`-vs-`Fallible` choice: both
  wait on traits (P7.S3e), where P7.S3e's own index-failure-carrier note is discharged.
- Generic element types (`Slice['T]` over a word's own type variable): blocked on
  generic-instantiation-over-own-variable.
- Range-aware borrow tracking and simultaneously-live disjoint mutable sub-slices: the
  parallel map-reduce case is a scoped combinator in P10.
- Owning / allocated variable-length buffers (a `Box<[T]>` analogue): P9 layer work; this
  slice views storage it does not own.
- Length arithmetic in signatures (`['T 'N+'M]`): ruled out phase-wide.
- Slicing as a route to linear elements: arrays cannot hold them yet.
- Aggregate-static sources: `static:` grammar admits only scalars, so there is no
  aggregate static to slice from; the array case is the target.

## Codebase Map

| Location | Symbol | Role in this work |
|----------|--------|-------------------|
| `src/ast.rs:1437` | `enum Type` | add `Type::Slice(SliceId, &'static str)` (R1.1) |
| `src/ast.rs:991` / `:1006` | `ArrayId` / array interner | pattern for `SliceId` + element-keyed registry (R1.2) |
| `src/ast.rs:1764` | `Type::name` | forced arm: render `Slice[T]` (R1.3) |
| `src/ir/types.rs:96` | `enum IrType` | add `IrType::Slice(SliceId)` (R2.1) |
| `src/ir/types.rs:266` | `ir_type_of` | forced arm beside `Type::Str => IrType::Str` (R1.3) |
| `src/ir/layout.rs:307` | `field_width` | forced: slice is 16 bytes not 8 (R2.2) |
| `src/ir/layout.rs:323` | `carried_slot_bytes` | forced: 16-byte aggregate marshalling (R2.2) |
| `src/ir/driver.rs:507` | load-vs-blit dispatch | forced (ends `unreachable!()`): blit a slice (R2.2) |
| `src/backend/qbe.rs:311`/`:360`/`:365`/`:399`/`:423` | register/load/store/blit classification | forced aggregate arms (R2.2) |
| `src/backend/qbe.rs:341` | `qbe_abi_ty` | **soundness wildcard** `_ => width()`: 16-byte aggregate ABI (R3) |
| `src/backend/qbe.rs:1125` | `Instr::Print` dispatch | forced print arm (R2.2) |
| `src/backend/qbe.rs:796` | `emit_oob_trap` | runtime OOB path reused for slice indexing (R9.2) |
| `src/check/builtins.rs:250`/`:251` | `is_copy` | **soundness wildcard** `_ => true`: mutability split (R4) |
| `src/check/builtins.rs:279`/`:299` | `contains_reference` | **soundness wildcard** `_ => false`: true for slice (R5) |
| `src/check/builtins.rs:311` | `stored_reference_output_error` | output ban, kept covering slices via R5 |
| `src/check.rs:425` | `find_zero_unsafe_element` | **soundness wildcard**: name slice zero-unsafe (R6) |
| `src/check/captures.rs:144`/`:169` | `classify_capture` | **soundness wildcard** `_ => Scalar`: reference class (R8.1) |
| `src/repl.rs:195`/`:219` | `remap_type` | **soundness wildcard** `other => other`: rebase `SliceId` (R8.2) |
| `src/repl.rs:604` region | `format_stack`, rich-value renderers | forced/functional render arms (R2.2, R7) |
| `src/check/operators.rs:342` | `.` printable set | slice printability decision (R7) |
| `src/check/poly.rs:783` | poly `len` | poly twin arm (R8.3, R9.1) |
| `src/check/poly.rs` | `poly_is_copy`, `is_reference_slot`, `poly_type_str` | poly predicate twins (R8.3) |
| `src/check/poly.rs:96` | `PolyBorrow` | poly borrow tracking (R11, R12) |
| `src/check/engine.rs:55` | `Deriv` | per-place borrow tracking (R12) |
| `src/check/word_families.rs:716` | `check_array_word` | `slice`/`subslice`/`len`/index arms (R9, R10) |
| `src/check/word_families.rs:685`/`:775` | `len`/`cstr` guards, `len` arm | slice `len` behaviour (R9.1) |
| `src/check/declarations.rs:1181` | value-containment graph | forced reference-shaped arm (R1.3) |
| `src/check/declarations.rs:176`/`:187` | `extern` boundary | reject a slice at FFI (R7) |
| `docs/roadmap/P7-language-prereqs.md:139`/`:187` | P7.S3c body + exit | rewrite fallible→trap (R14) |

## Open Questions

- [x] ~~OQ1 sub-ranging without ref nesting~~ — resolved R10.3: `subslice` re-derives a
  fresh `Slice`, never nests.
- [x] ~~OQ2 naming~~ — resolved R10.2: `slice`/`subslice`, word-shaped, non-sigil.
- [x] ~~OQ3 reborrow chain depth~~ — resolved into a required test, R13.
- [x] ~~OQ4 poly-path extent~~ — resolved into a Phase-1 measurement step + scoped
  delivery, R11.
- [ ] (recorded, not closed) The `Type::Variant` predicate hole that slice3b's exit
  findings flag (`is_copy`/`contains_reference` `_ =>` treating a narrowed variant as
  `Copy`) is adjacent to R4/R5 but belongs to whichever slice owns that variant family;
  this slice adds the slice arm only, not the variant arm.

## Implementation Plan

Five phases. Phase 1 mirrors slice3a's Phase 1 (variant + arms + unit tests, no golden,
since no value can be constructed yet); the first end-to-end golden lands in Phase 3.

### Phase 1: `Type::Slice` variant, forced `Type` arms, checker soundness ports  *(hard)*

The type exists and every check-level predicate answers it correctly. Begin with the OQ4
poly-path measurement (R11). Deliver R1, R4, R5, R6, R8.1, R8.3, and the R7 checker
guards (operators printable set, extern boundary). No construction word yet, so soundness
is unit-tested by minting a `SliceId` directly and asserting each predicate. The output
ban (R5-backed) is testable end-to-end: a declared `( -- Slice['T] )` output is rejected.

**Scope:**

- Modify: `src/ast.rs:1437` (`enum Type`), `:991`/`:1006` (`SliceId` + interner), `:1764`
  (`Type::name`).
- Modify: `src/check/builtins.rs:250` (`is_copy`), `:279` (`contains_reference`);
  `src/check.rs:425` (`find_zero_unsafe_element`); `src/check/captures.rs:144`
  (`classify_capture`); `src/check/operators.rs:342`; `src/check/declarations.rs:176`/`:187`
  (`extern`), `:1181` (containment); `src/check/poly.rs` (`poly_is_copy`,
  `is_reference_slot`, `poly_type_str`, poly `len` at `:783`).
- Out of bounds: any `IrType` change (Phase 2), construction/index words (Phase 3), borrow
  tracking (Phase 4).

### Phase 2: `IrType::Slice` variant, 16-byte aggregate lowering and ABI  *(hard)*

The representation lowers as a genuine two-word aggregate. Deliver R2, R3, R8.2, and the
R2.2/R7 render arms. No new source-level behaviour; the deliverable is that every forced
IR/backend arm and the `qbe_abi_ty`/`remap_type` wildcards are ported, verified by unit
tests over layout/ABI classification (a slice is 16 bytes, aggregate ABI spelling, blit
not scalar-load).

**Scope:**

- Modify: `src/ir/types.rs:96` (`enum IrType`), `:266` (`ir_type_of`); `src/ir/layout.rs:307`
  (`field_width`), `:323` (`carried_slot_bytes`); `src/ir/driver.rs:507`.
- Modify: `src/backend/qbe.rs:311`/`:360`/`:365`/`:399`/`:423` (classification), `:341`
  (`qbe_abi_ty`), `:1125` (`Instr::Print`); `src/repl.rs:195` (`remap_type`), `:604` region
  (renderers).
- Out of bounds: construction/index semantics (Phase 3).

### Phase 3: construction, sub-ranging, `len`, and indexed access with a runtime bound  *(hard)*

The first end-to-end golden. Deliver R9 and R10: `slice`/`subslice` compiler-known words,
`len` answering a runtime length, `&>`/`&!>` over a slice bounds-checked against the
runtime length with the existing OOB trap on miss. The `sum` exit golden lands here.

**Scope:**

- Modify: `src/check/word_families.rs:716` (`check_array_word`: `slice`, `subslice`, `len`
  at `:775`, index dispatch), `:685` guard.
- Modify: lowering for the two new words and the slice-receiver index op (the `ir/driver.rs`
  / `backend/qbe.rs` index-emit path that reuses `emit_oob_trap`, `qbe.rs:796`).
- Out of bounds: exclusivity/non-escape tracking beyond what `&>` already enforces (Phase 4).

### Phase 4: borrow rules ported, poly borrow arms, reborrow-chain-depth test  *(hard)*

Deliver R12 and the poly borrow-tracking arms deferred from R11, plus the R13 reborrow
chain test. A mutable slice is exclusivity-tracked in `Deriv`/`PolyBorrow`; the recursive
`rec` divide-and-conquer golden lands here.

**Scope:**

- Modify: `src/check/engine.rs:55` (`Deriv` handling for a slice place),
  `src/check/poly.rs:96` (`PolyBorrow`).
- Out of bounds: range-aware / disjoint concurrent mutable sub-slices (locked out of scope).

### Phase 5: roadmap correction, regression sweep, growth-signal re-check  *(standard)*

Deliver R14: rewrite the P7.S3c body + exit in `docs/roadmap/P7-language-prereqs.md` from
the fallible-accessor promise to the runtime-trap reality. Run the full regression suite,
and re-run CLAUDE.md's growth-structure signals against every file the slice modified
significantly (notably `src/check/word_families.rs`, `src/backend/qbe.rs`, and
`src/check/poly.rs`, echoing slice3b's deferred `poly.rs` split decision).

**Scope:**

- Modify: `docs/roadmap/P7-language-prereqs.md:139`/`:187`.
- Verify: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.
- Out of bounds: any code change beyond fmt/clippy fixes surfaced by the sweep.

## Testing

Per CLAUDE.md: every stage function gets a unit test beside it (happy path + one
error/edge), every exit criterion is a golden, diagnostics assert the exact message, and
guards are mutation-tested (delete the arm, the test must fail).

**Goldens (`tests/phase7_slice3c.rs`):**

- `sum_over_a_slice_noninline_prints_twentyfive` (the exit criterion, diffed against the
  length-threading twin).
- `recursive_divide_and_conquer_over_subslices_runs`.
- `slice_out_of_range_index_traps_at_runtime` (asserts the located OOB message, no
  `Option`/`Result`).
- `declared_slice_output_is_stored_reference_error`.
- `two_simultaneous_mutable_subslices_is_error`.
- `dup_on_mutable_slice_is_error` / `dup_on_shared_slice_ok`.

**Unit tests beside their stage:**

- `src/ast.rs`: `slice_type_name_renders_element`, `slice_interns_by_element`.
- `src/check/builtins.rs`: `is_copy_shared_slice_is_copy_mutable_slice_is_not`,
  `contains_reference_true_for_slice`.
- `src/check.rs`: `find_zero_unsafe_element_names_slice`.
- `src/check/captures.rs`: `classify_capture_slice_is_reference_not_scalar`.
- `src/repl.rs`: `remap_type_rebases_sliceid_across_modules`.
- `src/ir/layout.rs`: `slice_field_width_is_sixteen`,
  `carried_slot_bytes_slice_is_aligned_aggregate`.
- `src/backend/qbe.rs`: `qbe_abi_ty_slice_is_aggregate_not_scalar_width`.
- `src/check/word_families.rs`: `len_over_a_slice_answers_runtime_length`,
  `slice_constructs_a_view_from_an_array_reference`,
  `subslice_rederives_a_fresh_slice_not_a_nested_borrow`.
- `src/check/poly.rs`: `poly_is_copy_mutable_slice_is_not`, `poly_len_over_a_slice_ok`.

**R13 (OQ3) reborrow chain, named explicitly:**
`mutate_innermost_hop_of_buffer_slice_subslice_element_chain_while_outer_live` — mutates
through the innermost `&!element` of a `&!buffer -> &!Slice -> &!sub-slice -> &!element`
chain with the outer hops still live, the one shape the brief's reborrow probe did not
cover.

**Mutation-tested guards (delete the arm → test fails):** R4 mutability split (both
directions), R5 `contains_reference` (the output-ban golden must flip to accept), R6
zero-unsafe arm, R8.1 `classify_capture`, R8.2 `remap_type`, R3 `qbe_abi_ty`, and the R9.2
bounds-trap.

**Regression, green and untouched:** `tests/phase7_slice3a.rs`, `tests/phase7_slice3b.rs`,
`tests/qbe_baseline.rs`, the poly-reference and array suites.

## Risks & Mitigations

| Risk | Mitigation |
|---|---|
| A wildcard soundness site missed by a grep port (esp. `is_copy`/`contains_reference`, which lack the token `Str`) | R3–R8 anchor every site by symbol; each has a mutation-tested guard, so a missing arm fails a test rather than compiling clean |
| A slice silently inheriting `str`'s single-word answers at the IR/ABI level | separate `IrType::Slice` variant forces every arm (R2); `str`'s static-descriptor shape is explicitly not imitated |
| The OQ3 stored-reference-in-aggregate reborrow shape opening a new borrow hole | R13 is a required, named test that exercises exactly that shape |
| Poly-path extent under-scoped (OQ4) | R11 front-loads a measurement step; predicate twins land Phase 1, borrow arms Phase 4, and any un-deliverable capability is declared out of scope with a rationale |

## References

- `docs/roadmap/P7/slice3c-brief.md` (probe-grounded brief; locked decisions binding)
- `docs/roadmap/P7/slice3a-spec.md`, `slice3b-spec.md` (spec conventions, forced-arm and
  poly-twin patterns, `Type::Variant` predicate-hole finding)
- `docs/roadmap/P7-language-prereqs.md` (P7.S3c exit criterion, corrected by R14)

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "Type::Slice variant, forced Type arms, checker soundness ports", "effort": "M", "difficulty": "hard" },
    { "phase": 2, "focus": "IrType::Slice variant, 16-byte aggregate lowering and ABI", "effort": "M", "difficulty": "hard" },
    { "phase": 3, "focus": "slice/subslice construction words, len, indexed access with runtime bound", "effort": "M", "difficulty": "hard" },
    { "phase": 4, "focus": "borrow rules ported, poly borrow arms, reborrow-chain-depth test", "effort": "M", "difficulty": "hard" },
    { "phase": 5, "focus": "roadmap exit-criterion correction, regression sweep, growth-signal re-check", "effort": "S", "difficulty": "standard" }
  ]
}
```
