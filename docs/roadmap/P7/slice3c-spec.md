# Phase 7 Slice 3c: slicing a buffer into a view (spec)

## Goal

Add `Type::Slice(SliceId, bool)`: a borrowed, length-carrying view over a buffer. A
non-`inline` word can take `Slice[T]` over a concrete element type as a parameter and index
it without naming a length variable, so the checker never has to prove an index against an
abstract `'N`. This is the signature shape the trait-bounds consumers (P7.S3e) want, and it
removes the
two warts of today's only working spelling: a fixed length in the type (`&[i64 5]`) and a
threaded-length second parameter (`( &[i64 5] usize -- i64 )`).

A slice is **second-class, input-only, non-owning**: constructing one consumes nothing,
no non-`inline` user word may return one, and the declaration-level output ban
(`stored_reference_output_error`) covers it exactly as it covers `&T` (an `inline` word
is exempt from that ban for both, the same pre-existing carve-out either way). Element access
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
- **Key issues:** The representation is `Type::Slice(SliceId, bool)`, its own variant. That
  choice trades silent unsoundness (reusing a fat `Type::Ref` would leave every existing
  `IrType::Ptr` site compiling and wrong, a diffuse failure set) for an *enumerable*
  failure set: a fixed list of forced match arms plus nine wildcard sites that must be
  ported deliberately. Every one of those ports needs a test that fails when its arm is
  missing.

## Requirements

Each requirement is an independently verifiable claim. Anchors pair `path:line` with a
symbol name; line numbers may drift, so re-locate by symbol.

### R1 `Type::Slice(SliceId, bool)` exists as its own variant with an interned registry

- **R1.1** A new `Type::Slice(SliceId, bool, &'static str)` variant in `src/ast.rs:1437`
  (`enum Type`), modelled on `Type::Ref(RefId, bool, &'static str)` at `ast.rs:1462` —
  **not** `Type::Array`, which has no mutability. The `bool` carries the view's mutability
  (shared vs mutable) **inline**, because it is the *classification* bit (`is_copy`,
  linearity), asked at sites that hold no registry, exactly as `Type::Ref`'s doc comment
  (`ast.rs:1454-1461`) explains. The `SliceId` is a small `Copy` index into a per-module
  interned registry keyed by `(element, mutable)`; the leaked display spelling is
  `Slice[T]` (shared) / `!Slice[T]` (mutable), mirroring the `!` mutability marker in
  `&!T`, so a shared and a mutable slice of the same element are distinct, each
  byte-identical to its own kind. The `SliceId` newtype mirrors `ArrayId` (`ast.rs:991`)
  with a crate-internal `from_index`.
- **R1.2** The registry interns by `(element, mutable)` (one `SliceId` per concrete
  element + mutability pair), mirroring how `intern_ref_type` (`ast.rs:970`) dedups on
  `d.referent == referent && d.mutable == mutable` — not the array interner, which has no
  mutability. Element types are **concrete and `Copy` only** (locked): a generic element
  (`Slice['T]` over a word's own variable) is out of scope, blocked on
  generic-instantiation-over-own-variable, and a linear element has no buffer to view
  (`linear array elements are not supported yet`).
- **R1.3** The three compile-forced exhaustive `Type` matches each gain a deliberate
  arm, no `_ =>` catch-all: `Type::name` (`ast.rs:1764`) renders `Slice[T]`;
  `ir_type_of` (`ir/types.rs:266`, beside the `Type::Str => IrType::Str` arm) maps to
  the new `IrType::Slice` (R2); the value-containment graph
  (`check/declarations.rs:1181`) treats a slice as reference-shaped (contains a
  reference, never owns storage).
- **R1.4** `is_ref()` (`src/ast.rs:1723`, today `matches!(self, Type::Ref(..))`) gains a
  `Type::Slice(..) => true` arm. Without it, R5 makes `contains_reference` true while
  `is_ref()` stays false — exactly the condition the input-signature gate
  `check_reference_free_signature` rejects (`src/check/word_entry.rs:170`,
  `!slot.ty.is_ref() && contains_reference(...)`, "a reference … nested inside an
  aggregate") — so a `Slice` **input** parameter (the whole point, including the `sum`
  exit golden `( Slice[i64] -- i64 )`) would be rejected at declaration. The output ban is
  unaffected: the same function's output loop tests `contains_reference` **alone** with no
  `is_ref` guard (`word_entry.rs:166`), so a declared `Slice` output stays rejected
  regardless (R5). This arm also makes `is_linear` (`check/builtins.rs`,
  `!ty.is_ref() && !is_copy(...)`) correctly non-linear for **both** shared and mutable
  slices, matching the "second-class, non-owning, expires silently" model (R12) rather
  than dragging a mutable slice into move-tracking and owing it a `drop`.

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
  `check_array_word` `len` arm (`word_families.rs:775`) with the slice receiver. `len` is
  **receiver-dependent**: it *consumes* a slice (matching `str`) where it leaves an array
  in place, so `s len` ends the view `s`. A shared view is `Copy`, so a named local
  reborrows and this is invisible; a Phase 4 mutable view is not, which is where the
  asymmetry becomes visible.
- **R9.2** The index ops `&>` (shared) and `&!>` (mutable) accept a slice receiver and
  produce an element reference (`&T` / `&!T`) bounds-checked against the **runtime**
  length. The slice-receiver branch lives in `check_reference_word`
  (`src/check/word_families.rs:12`, the `>` arm at `:35`), which handles every
  `&`-prefixed name and is dispatched **before** `check_array_word` (`terms.rs:357` ahead
  of `:478`); today its `>` arm calls `ref_parts(stack[n-2].ty, refs)`, assuming a
  `Type::Ref` receiver whose referent is an array. A `Slice` receiver is **not** a
  `Type::Ref`, so it needs its own branch matching the receiver as a bare `Type::Slice`
  **before** the `ref_parts` extraction, not a fallback inside it. An out-of-range index
  traps at runtime via the existing `emit_oob_trap` (`backend/qbe.rs:796`), with the same
  located message array indexing produces. `&>` already produces a `&T` output today and
  is exempt from the signature audit, so no new grammar is required. A `&!>` element
  reference off a mutable slice is exclusivity- and extent-tracked exactly as an array
  element reference is (R12). The shared `&>` receiver lands in Phase 3; the mutable `&!>`
  receiver lands in Phase 4 with R12's exclusivity guard.

### R10 Construction and sub-ranging as compiler-known words

- **R10.1** Construction word **`slice`**: `( &[T N] -- Slice[T] )`, turning a borrowable
  buffer reference into a view carrying its length at runtime. Sub-ranging word
  **`subslice`**: `( Slice[T] usize usize -- Slice[T] )` taking a start offset and a
  length. Both are compiler-known arms in the array-word family (`check_array_word`,
  `word_families.rs:716`), dispatched by name, exempt from the signature audit exactly as
  `&>` is. **No new grammar.** The **shared** forms (`slice` from a shared `&[T N]`,
  shared `subslice`) land in Phase 3; the **mutable** forms (`slice` from a `&![T N]`,
  mutable `subslice`) land in Phase 4 alongside R12's exclusivity guard, so a mutable
  slice never exists in a build before its guard is active.
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
  valid only under this reading, and the spec states it. An out-of-range sub-range gets
  its **own** runtime trap (`sooth_subslice_trap`), not R9.2's index message: a range
  failure has no index, so it reports the requested start and length against the length of
  the view being cut. The three numbers print unsigned, so an underflowed start reads as
  itself rather than as a `-1` the source never wrote.

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

A slice is exclusivity-tracked, non-escaping, non-linear, and banned from outputs exactly
as a `&T` is. The output ban is already covered by R5 (`contains_reference` true); a slice
is non-linear via R1.4's `is_ref()` arm (`is_linear`'s `!ty.is_ref()` clause), so it
expires silently and is owed no `drop`. Exclusivity: a
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
- [ ] `( -- Slice[i64] )` as a declared user output is rejected with
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
| `src/ast.rs:1437` | `enum Type` | add `Type::Slice(SliceId, bool, &'static str)`, mutability inline like `Type::Ref` (R1.1) |
| `src/ast.rs:991` / `:970` | `ArrayId` / `intern_ref_type` | `SliceId` newtype; registry keys on `(element, mutable)` like `intern_ref_type` (R1.2) |
| `src/ast.rs:1723` | `is_ref` | **soundness predicate** `matches!(Type::Ref(..))`: add slice arm so a slice input is legal + non-linear (R1.4) |
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
| `src/check/word_entry.rs:166`/`:170` | `check_reference_free_signature` | output ban (`contains_reference` alone) vs input gate (`is_ref` guard) (R1.4, R5) |
| `src/check.rs:425` | `find_zero_unsafe_element` | **soundness wildcard**: name slice zero-unsafe (R6) |
| `src/check/captures.rs:144`/`:169` | `classify_capture` | **soundness wildcard** `_ => Scalar`: reference class (R8.1) |
| `src/repl.rs:195`/`:219` | `remap_type` | **soundness wildcard** `other => other`: rebase `SliceId` (R8.2) |
| `src/repl.rs:604` region | `format_stack`, rich-value renderers | forced/functional render arms (R2.2, R7) |
| `src/check/operators.rs:342` | `.` printable set | slice printability decision (R7) |
| `src/check/poly.rs:783` | poly `len` | poly twin arm (R8.3, R9.1) |
| `src/check/poly.rs` | `poly_is_copy`, `is_reference_slot`, `poly_type_str` | poly predicate twins (R8.3) |
| `src/check/poly.rs:96` | `PolyBorrow` | poly borrow tracking (R11, R12) |
| `src/check/engine.rs:55` | `Deriv` | per-place borrow tracking (R12) |
| `src/check/word_families.rs:716` | `check_array_word` | `slice`/`subslice`/`len` arms (R9.1, R10) |
| `src/check/word_families.rs:12`/`:35` | `check_reference_word` | `&>`/`&!>` slice-receiver branch, before `ref_parts` (R9.2) |
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
poly-path measurement (R11). Deliver R1 (including the R1.4 `is_ref` arm), R4, R5, R6,
R8.1, R8.3, and the R7 checker
guards (operators printable set, extern boundary). No construction word yet, so soundness
is unit-tested by minting a `SliceId` directly and asserting each predicate. The output
ban (R5-backed) is unit-tested the same way, driven directly rather than end-to-end:
`check_reference_free_signature` is called on a hand-built `StackEffect` declaring a
`Slice['T]` output, because there is no surface spelling to write a source golden against
until Phase 3's construction words exist (`slice_output_is_rejected_and_slice_input_is_admitted`,
`src/check/word_entry.rs`).

**Scope:**

- Modify: `src/ast.rs:1437` (`enum Type`), `:991`/`:970` (`SliceId` + interner keyed on
  `(element, mutable)`), `:1764` (`Type::name`), `:1723` (`is_ref` slice arm, R1.4).
- Modify: `src/check/builtins.rs:250` (`is_copy`), `:279` (`contains_reference`);
  `src/check.rs:425` (`find_zero_unsafe_element`); `src/check/captures.rs:144`
  (`classify_capture`); `src/check/operators.rs:342`; `src/check/declarations.rs:176`/`:187`
  (`extern`), `:1181` (containment); `src/check/poly.rs` (`poly_is_copy`,
  `is_reference_slot`, `poly_type_str`, poly `len` at `:783`).
- Out of bounds: any `IrType` change (Phase 2), construction/index words (Phase 3), borrow
  tracking (Phase 4).

**Phase 1 exit notes (R11 measurement):** the poly-path extent was walked over every
`Type`/`PolyType` match site a slice can reach in a generic body: `poly_is_copy`,
`is_reference_slot`, poly `len`, `poly_type_str`, the eliminator-arm escape check, quotation-
parameter folding, and array-ref-parts derivation. Every site either delegates to the
monomorphic predicate through `PolyType::Concrete(Type::Slice(..))` (a slice element is
concrete by construction, R1.2) or does not distinguish a slice at all. No poly-path
capability was found that requires re-opening a locked non-goal (generic elements, row
unification); the predicate twins delivered here (R8.3) are the full Phase 1 poly
surface. The one capability the measurement finds genuinely absent — tracking a *mutable*
slice as an exclusivity place inside a poly body — is out of scope by design, not by gap:
`PolyBorrow` (`check/poly.rs:96`) has no borrow arm for anything yet (its `place` is a
bare `String` with no `projected` flag), and that arm is already scheduled for Phase 4
alongside R12's exclusivity guard, so it is not owed until a mutable slice can exist.

**Phase 1 exit notes (R1.4 `is_ref()` reroute inventory):** `is_ref()` has seven
non-test consumers. Three are the widening's *intended* effects, each delivered and
tested here: `is_linear` (`builtins.rs`, a slice is non-linear), `is_reference_slot`
(`poly.rs`, R8.3), and the input gate in `check_reference_free_signature`
(`word_entry.rs`, `slice_output_is_rejected_and_slice_input_is_admitted`). The other four
are diagnostics the widening *reroutes*, and two of them are covered in Phase 1:

- `audit_poly_reference_free_signature`'s `top_level_ref` (`check/audits.rs`) — the poly
  twin of the input gate, tested by
  `poly_slice_output_is_rejected_and_poly_slice_input_is_admitted`. It earns its own test
  rather than inheriting the mono one: a slice in a generic body is the point of the type
  (R11), and the poly path admits a slice input by a different clause
  (`Concrete(t) if t.is_ref()`) than the one that rejects a slice output
  (`contains_poly_reference`'s `Concrete` delegation), so the two claims travel separately.
- `cannot_copy_error` (`check.rs`) picks the exclusivity wording (rather than the
  ownership wording) off `is_ref()`, which is correct for a mutable slice; asserted
  incidentally by `poly_is_copy_mutable_slice_is_not`'s exact-message check.

The remaining two (`borrow_of_reference_local_error`, `poly_reference_scrutinee_error`)
need a slice *value* to be reachable at all, so they are scoped to Phase 3 below.

**Phase 1 exit notes (arms that are forced but unobservable):** the containment-graph arm
`type_node`'s `Type::Slice(..) => None` (`check/declarations.rs`) is compile-forced, so it
cannot be missing, but its *value* is unobservable and it is therefore **not** a guarded
arm: `contains_reference` (R5) bans a slice from every field position, so the containment
walk never reaches a slice and returning any other `TypeNode` survives the whole suite. It
is not counted in Phase 1's mutation-tested tally.

### Phase 2: `IrType::Slice` variant, 16-byte aggregate lowering and ABI  *(hard)*

The representation lowers as a genuine two-word aggregate. Deliver R2, R3, R8.2, and the
R2.2/R7 render arms. No new source-level behaviour; the deliverable is that every forced
IR/backend arm and the `qbe_abi_ty`/`remap_type` wildcards are ported, verified by unit
tests over layout/ABI classification (a slice is 16 bytes, aggregate ABI spelling, blit
not scalar-load).

**Scope:**

- Modify: `src/ir/types.rs:96` (`enum IrType`), `:266` (`ir_type_of`) — Phase 1 left
  `ir_type_of`'s slice arm an `unreachable!()` placeholder rather than mint a premature
  `IrType`; it is genuinely unreachable there (no non-test caller of `intern_slice_type`),
  and replacing that `unreachable!()` with the real `IrType::Slice` mapping is what
  discharges R1.3's `ir_type_of` half. `src/ir/layout.rs:307`
  (`field_width`), `:323` (`carried_slot_bytes`); `src/ir/driver.rs:507`.
- Modify: `src/backend/qbe.rs:311`/`:360`/`:365`/`:399`/`:423` (classification), `:341`
  (`qbe_abi_ty`), `:1125` (`Instr::Print`); `src/repl.rs:195` (`remap_type`), `:604` region
  (renderers).
- Out of bounds: construction/index semantics (Phase 3).

**Phase 2 exit notes (three anchors that did not survive contact):**

- There is no `field_width` in `ir/layout.rs`; the anchored line is
  `scalar_size_align_ww`, whose `(bytes, bytes)` return contract cannot express a slice at
  all -- a slice is 16 bytes wide but word-*aligned*, so any answer it gave would be
  wrong in one component. Its arm therefore refuses (`unreachable!`, like `Quotation`'s),
  and R2.2's "a slice is 16 bytes, not 8" is delivered where the figures are real:
  `slice_layout` and `carried_slot_bytes`, both guarded.
- `ir/driver.rs`'s forced arm is not a general load-vs-blit dispatch but the **REPL
  carried-slot prologue**. A slice cannot reach it: the residual-stack check rejects a line
  leaving one on the stack through the very same `contains_reference` call that rejects a
  `&T` (R5). The arm joins `IrType::Ptr`'s `unreachable!` rather than blitting. The
  epilogue store beside it (a `_ =>` scalar-`Store` wildcard) is unreachable for that one
  same reason and is deliberately left unarmed, so the residual check stays the single
  place the claim is made.
- The backend spells every slice with **one shared `:slice = { l, l }`** aggregate, not the
  per-`SliceId` IR layout registry R2.1 imagined. All slices have an identical layout: the
  element type is erased at the backend exactly as it is for the `Ptr` every `&T` becomes,
  so a per-id registry would hold N byte-identical rows. The `SliceId` stays a
  frontend/lowering discriminator with no ABI content, and `IrType` stays `Copy` either
  way. The type is emitted only when a module holds a slice, so every existing program's
  QBE text is byte-identical (`tests/qbe_baseline.rs` untouched).

**Phase 2 review follow-ups (one settled, one left as found):**

- `member_ty` (the struct-member speller) **refuses** a slice rather than spelling
  `:{SLICE_TYPE_SYMBOL}`. That spelling was correct but dead and untested, and it
  disagreed with `field_load_op`/`field_store_op`/`scalar_size_align_ww`, which all refuse
  a slice on R5's ban. It also quietly underwrote `emit`'s type-ordering comment: the
  shared slice aggregate is emitted without being ordered against the structs a member
  would force it ahead of, so the first time the ban lapsed that unordered emission would
  be wrong with no diagnostic. The refusal asserts the ban where it can be observed;
  guarded by `member_ty_refuses_a_slice`.
- **Pre-existing and out of scope:** the collision Finding 1 fixed for the slice symbol is
  not unique to it. `array_type_symbol` mints `arr_{idx}` and quotations mint `:Q{idx}`,
  both of which a user struct name can forge -- `type: arr_0 a i64 b i64 ;` beside an
  `[i64 3]` emits both `type :arr_0 = align 8 { b 24 }` and `type :arr_0 = { l, l }`, and
  QBE accepts the duplicate silently and keeps the last, so the two values cross
  param/return boundaries at each other's size (probe-confirmed, latent). `sooth.slice` is
  the only immune symbol. The proper fix is one shared reserved-prefix helper covering all
  three, which is neither this phase's nor this slice's: touching array/quotation symbol
  minting would rewrite type names in `tests/qbe_baseline.rs` for a defect no slice work
  introduces.

**Phase 2 exit notes (what Phase 3 inherits):** the first slice *value* makes six
wildcards reachable that Phase 2 deliberately did not arm, because what they should answer
depends on how construction lowers:

- `func_builder/mod.rs`'s `is_aggregate` returns `false` for a slice, so a loop-carried
  slice would take the header-`Phi` path rather than the stable-slot + staged-back-edge
  blit an aggregate gets. Phase 3/4 must decide this deliberately; a two-word value on the
  wrong path is the `project_aggregate_return_aliasing` shape.
- `alloc_aggregate`'s `_ => unreachable!("not an aggregate IrType")` needs a slice arm the
  moment `slice` allocates a `{ptr, len}` frame slot.
- `value_size` (`func_builder/word_families.rs`) and `LayoutBuilder::size_align` both route
  a slice into `scalar_size_align_ww`'s refusal via their wildcards. That is the correct
  *behaviour* today (a slice is banned from every field and element position by R5), so
  neither is armed; a Phase 3 caller that legitimately needs a slice's byte size adds the
  arm at its first use.
- `Instr::Load`'s `_ => ("l", "loadl")` and `Instr::Store`'s `_ => storel`
  (`backend/qbe.rs`) would scalar-load or scalar-store **one word of the two**, which is
  the exact failure R2.2 was written to prevent. These are the buffer/pointer
  load-store instructions, distinct from `field_load_op`/`field_store_op`, which already
  refuse a slice. They are unreachable today (no slice value exists to load or store), and
  unarmed for the same reason as the rest: whether a slice ever travels through
  `Load`/`Store` at all, rather than the blit its 16-byte size implies, is Phase 3's
  construction decision. Whichever way that goes, these two arms must be settled
  explicitly rather than inherited.
- `Session::slices` exists so `remap_type`'s rebase has a real base to shift by, but it is
  not threaded into the checker: no session line can intern a slice until there is a
  surface spelling, which is Phase 3's business.

**Phase 2 mutation-tested guards (8, all killed):** the `qbe_abi_ty` arm deleted (R3), the
`remap_type` arm deleted (R8.2), `carried_slot_bytes` answering 8, `ir_type_of` mapping to
`IrType::Ptr`, the `format_stack` arm deleted, `rich_value_size` answering 8,
`render_rich_value`'s placeholder changed, and `member_ty` answering
`:{SLICE_TYPE_SYMBOL}` instead of refusing (the review follow-up above; that mutation
fails exactly one test, which is why the arm needed one).

### Phase 3: shared construction, sub-ranging, `len`, and shared indexed access  *(hard)*

The first end-to-end golden, **shared views only**. Deliver the shared halves of R9 and
R10: `slice` producing a **shared** `Slice[T]` from a shared `&[T N]`, **shared**
`subslice`, `len` answering a runtime length, and the **shared** `&>` receiver
bounds-checked against the runtime length with the existing OOB trap on miss. The `sum`
exit golden (a shared slice) lands here. Mutable construction (`slice` from a `&![T N]`),
mutable `subslice`, and `&!>` are deferred to Phase 4 so a mutable slice never exists in a
build before its exclusivity guard (R12) is active. The output-ban golden
`declared_slice_output_is_stored_reference_error` also becomes writable here, once a
signature can spell `Slice[T]` at all; Phase 1 only proves the same claim by a direct call
(see Phase 1's exit notes). Phase 1's widened `is_ref()` also makes the remaining two of
its four rerouted diagnostics (see Phase 1's reroute inventory) reachable for the first
time once a slice value exists: `&s` on a slice local reaches
`borrow_of_reference_local_error` (`word_families.rs`, defensible — a slice is a
reference-shaped local, so the message is accurate), and a slice used as an eliminator
scrutinee reaches `poly_reference_scrutinee_error` (`poly.rs`), whose "eliminates a
reference … pass the owned `Enum` instead" wording was written for a `&Enum` and reads
confusingly for a `Slice[T]`; give that second message its own wording here, when a slice
scrutinee is first actually reachable, rather than leaving stale text live.

**Scope:**

- Modify: `src/check/word_families.rs:716` (`check_array_word`: shared `slice`, shared
  `subslice`, `len` at `:775`), `:12`/`:35` (`check_reference_word`: shared `&>`
  slice-receiver branch), `:685` guard.
- Modify: lowering for the two new words and the shared slice-receiver index op (the
  `ir/driver.rs` / `backend/qbe.rs` index-emit path that reuses `emit_oob_trap`,
  `qbe.rs:796`).
- Also required, inherited from Phase 1: the monomorphic `len` arm must **consume** its
  slice receiver, matching the consumption semantics Phase 1's poly `len` arm already
  encodes for `Concrete(Type::Str | Type::Slice(..))` (unlike the array arms, which leave
  the receiver). Diverging here would make the two checker paths differ in permissiveness.
- Also required, inherited from Phase 1: `intern_slice_type` (`ast.rs`) enforces nothing
  about R1.2's **concrete and `Copy` element** rule — correct while it has no non-test
  caller, but the `slice` construction word is its first real caller and owes that gate
  with a located rejection for a generic or non-`Copy` element.
- Out of bounds: mutable construction/`subslice`/`&!>` and all exclusivity/non-escape
  tracking (Phase 4).

**Phase 3 exit notes (the surface spelling, and what it cost):** `Slice[T]` had no way to
be written at all, so Phase 3 owns the type syntax as well as the words. `Slice` is
intercepted by name in `resolve_type_or_apply` ahead of every user registry (it resolves
through the interned slice registry, not a declared header) and is therefore reserved:
`reject_reserved_name` refuses it as a `type:`/variant name, because a declaration under
that name would be silently *unreachable* rather than merely shadowed. The mutable
spelling `!Slice[T]` is deliberately absent until Phase 4 — with no way to write one and
`slice`/`subslice` both refusing a mutable receiver, a mutable view cannot exist in this
build at all, which is the property Phase 4's guard is owed before it can.

The registry had to reach both the parser and the checker: `slices` is threaded beside
`refs` through the parse entry points and the whole check walk (the two REPL `type:`-line
entry points keep a scratch registry, mirroring their `generics` scratch — a slice is
banned from every field position, so nothing there can intern one). Lowering gets a
`Slices` registry (element `IrType` + stride, keyed like the interner on
`(element, mutable)`), built beside `build_statics` rather than inside `build_registries`:
a slice is never a field or element of anything, so it takes no part in the layout DFS.

**Phase 3 exit notes (three inherited items, resolved):**

- The `Copy`-element gate the Phase 2 notes said `intern_slice_type` owed **is** needed
  after all, over the *type-spelling* route rather than the construction one. `slice`'s
  own construction route indeed needs no gate: no array can hold a non-`Copy` element
  (a linear one is refused as `linear array elements are not supported yet`, a reference
  one as `a reference cannot be stored`), so `slice`'s only source is already clean; that
  narrower claim is pinned by `slice_element_copy_rule_is_enforced_by_the_array_gate`,
  unchanged. But `Slice[T]` interns straight from the parser (`intern_slice_type`),
  independent of any array or `slice` call, so `Slice[Slice[i64]]`, `Slice[&i64]`,
  `Slice[&!i64]`, and `Slice[^i64]` all reached `module.slices` ungated (found by phase-3
  review, reproduced as a compiler panic in `slice_layout`/`scalar_size_align` in both
  `build` and the REPL). Fixed by `check_slice_element_gate` (`check/declarations.rs`),
  a sweep over `module.slices` beside the two array sweeps: `contains_reference` on the
  element catches a reference or nested-slice element (a slice is itself
  reference-shaped), `is_copy` catches a linear one (a cell or a linear struct/enum).
  Wired into `check_types` for the native build path and into the REPL's per-line
  dispatch (`eval_expr_or_def_line`, beside the quotation-type audit) for the REPL, since
  a word signature mints a `Slice[T]` type before any of `eval_def`/`eval_combinator_def`/
  `eval_poly_def` runs.
- `poly_reference_scrutinee_error`'s stale wording is fixed by *not reaching* it: a slice
  scrutinee in a generic body now gets the plain type mismatch, the same message the
  concrete path already gives it, rather than advice to "pass the owned `Enum` instead"
  that names nothing real. `borrow_of_reference_local_error` on `&s` needed no change —
  its "write `s`, not `&s`; naming a reference local reborrows it" is accurate for a view.
- Of the six wildcards Phase 2 handed forward, three are now armed and three stay unarmed
  for the reason Phase 2 gave. Armed: `is_aggregate` answers **true** (a slice value is an
  interior pointer to a frame slot whose `Alloc` is hoisted to the entry block, so the
  header-`Phi` path would let the back edge overwrite the slot the header still reads —
  `project_aggregate_return_aliasing`); `alloc_aggregate` mints the 16-byte slot;
  `value_size` answers 16. `Instr::Load`/`Instr::Store` gain explicit `unreachable!` arms
  rather than inheriting `("l", "loadl")`/`storel`: a slice travels by `Blit` like every
  other aggregate, and a scalar load or store would silently move one word of the two.
  `LayoutBuilder::size_align` and `scalar_size_align` keep refusing, unchanged.

**Phase 3 exit notes (a fifth `is_ref()` consumer, found by probe):** Phase 1's reroute
inventory listed seven `is_ref()` consumers, but `overlapping_projection`
(`word_families.rs`) matches `Type::Ref(..)` **directly** — it needs the mutability bit —
so the widening never reached it, and a live slice counted as no borrow at all.
Probe: `7 4 fill W &a slice swap W> drop len .` compiled, while its element-reference twin
(`&a 0 >usize &>`) was correctly rejected. Ported here, with both rows in
`consuming_the_buffer_under_a_live_slice_is_error`; `consumed_place_conflict`'s
"consuming a reference is not a place ending" early return got the same arm.

**Phase 3 exit notes (two spec corrections):**

- `slice_through_a_declared_quotation_parameter_row_runs` cannot be a **non-`inline`**
  word: a `~[ ... ]` parameter can only ever be spliced, so the language refuses a
  non-`inline` word declaring one before the row's contents matter. The golden is written
  `inline`, and asserts the non-`inline` refusal as its second half.
- `recursive_divide_and_conquer_over_subslices_runs` lands here in its **shared** form
  (`recursive_divide_and_conquer_over_shared_subslices_runs`), since shared `subslice` is
  Phase 3's; Phase 4 still owes the mutable-half twin the spec assigns it.

**Phase 3 exit notes (two review fixes):**

- `dup_on_shared_slice_ok` landed here, not in Phase 4: R4's shared half is first
  observable the moment a slice value exists, so leaving it unwritten would have left a
  success criterion unasserted for a whole phase. Its `dup_on_mutable_slice_is_error` twin
  stays Phase 4's, since no mutable view can be written in this build at all.
- `subslice` stopped borrowing R9.2's index trap. Reusing it printed
  `index 3 is out of bounds for length 1` for `3 >usize 3 >usize subslice` over a length-4
  view: "index 3" was the *length* argument and "length 1" the computed remainder, so
  every number named something the source had not written. R9.2 specifies message reuse
  for the *index* op only, and R10.3 had left `subslice`'s trap unruled. It now has its
  own (`sooth_subslice_trap`, `ir/types.rs`; `emit_subslice_trap`, `backend/qbe.rs`),
  gated on the module holding a slice so a slice-free program's IL stays byte-identical.

**Phase 3 exit notes (deferred to Phase 4, with rationale):** a slice inside a
*polymorphic* body can be `len`'d (Phase 1 armed the poly `len` arm) but can be neither
built nor indexed: `slice`/`subslice` are unknown words on the poly walk, and
`poly_reference_word`'s `>` arm reads `poly_ref_array_parts`, so a slice receiver falls to
`poly_op_on_variable_error`. All three are *rejections*, not silent accepts, so this is a
capability gap and not a soundness hole — but it means R11's "concrete-element poly
consumer" does not ship until Phase 4, which is where the poly work
(`PolyBorrow`, `check/poly.rs:96`) is already scheduled. Phase 4 must add the poly
`slice`/`subslice`/`&>` arms alongside it, threading `slices` into the poly walk the way
this phase threaded it into the concrete one.

**Phase 3 mutation-tested guards (16, all killed):** the `&>` runtime bounds trap deleted,
the `subslice` range trap deleted, `len`'s slice arm made non-consuming, the `&>`
slice-receiver mutability guard stubbed, `slice`'s mutable-receiver refusal deleted, the
`is_aggregate` slice arm removed, the reserved-`Slice` name check stubbed, the poly
slice-scrutinee arm removed, `slice_id_of` matching on the element alone (half the
interning key), `build_slices`'s stride skewed off the array's, the parser interning a
mutable view, `overlapping_projection` blind to a slice again, and `slice` dropping its
receiver's region. Three more with the review fixes: the range message's `%lu` reverted to
`%ld` (killed by the overflow golden, which is the test that can see the difference), the
`is_copy` shared-slice arm forced to non-`Copy` (killed by `dup_on_shared_slice_ok`, and by
nothing else), and the trap emission's slice gate forced open (killed by the byte-identity
baseline).

### Phase 4: mutable views, borrow rules ported, poly borrow arms, reborrow test  *(hard)*

Deliver mutable-slice construction (`slice` from a `&![T N]`), mutable `subslice`, and the
`&!>` mutable-element receiver, landing **together with** R12's exclusivity tracking and
the poly borrow-tracking arms deferred from R11, plus the R13 reborrow chain test — so a
mutable slice never exists without its guard. A mutable slice is exclusivity-tracked in
`Deriv`/`PolyBorrow`; the recursive `rec` divide-and-conquer golden (which takes one
mutable half at a time) lands here.

**Scope:**

- Modify: `src/check/word_families.rs:716`/`:12` (mutable `slice`/`subslice`/`&!>` arms),
  `src/check/engine.rs:55` (`Deriv` handling for a slice place), `src/check/poly.rs:96`
  (`PolyBorrow`).
- Inherited from Phase 3: `len` consumes its slice receiver (R9.1), and a mutable view
  is not `Copy`, so `s len` ends the only view of the buffer. Monomorphically a named
  local reborrows and the view survives the call; in a *polymorphic* body a `&!` local is
  move-tracked per binding, so the mutable poly consumer must take its length before
  deriving the view, or re-derive after. Rule on this here rather than discovering it in a
  golden.
- Out of bounds: range-aware / disjoint concurrent mutable sub-slices (locked out of scope).

**Phase 4 exit notes (three anchors that did not survive contact):**

- `Deriv` (`check/engine.rs`) needed no change at all. A view already carries a
  derivation and a region (Phase 3's `slice`/`subslice` forward the receiver's), so the
  borrow table sees it; the site that could *not* see one was the naming arm in
  `check/terms.rs`, which asks `ref_parts` whether a named local is a reference. A slice
  is not a `Type::Ref`, so a named mutable view fell through to the owned-value arm, where
  a non-linear value is "merely read" -- and `s ... s` handed out two live `&!` views of
  one buffer with nothing to say so. Fixed by `borrow_mutability` (`check.rs`),
  `ref_parts`' sibling for the sites that want the *borrow* nature rather than the
  referent (a view has no single referent: it points at a run of them).
- `PolyBorrow` (`check/poly.rs`) needed no arm either: Phase 1's `is_reference_slot`
  already counts a slice, so a live view keeps the borrow it was built from unpruned. What
  the poly path actually owed was the missing *walk* arms below, not a borrow record.
- The mutable surface spelling is Phase 4's, not Phase 3's oversight: `!Slice[T]` is
  intercepted beside `Slice[T]` in `resolve_type_or_apply`, and `!Slice` joins `Slice` in
  `reject_reserved_name` for the same reason (a declaration under an intercepted name is
  unreachable, not shadowed).

**Phase 4 exit notes (lowering had to learn reference mutability):** `slice` is the first
lowering site that needs to know whether a reference is mutable -- the view's *own* type
carries the bit (R1.1) and the `SliceId` registry is keyed on it (R1.2), while a reference
lowers to the opaque `IrType::Ptr` that says nothing about it. `FuncBuilder::ref_mutable`
carries it per `Value` beside `ref_inner`, both filled by one `record_reference` helper so
they cannot drift, and seeded at every route a reference reaches `slice` by: a prefix
borrow or field/variant-field projection (the name says it), an array-element projection
(`&>` on an array reference), a *slice*-element projection (`&>` on a view -- the same
word's other receiver arm, with its own seeding site; both element projections need a
*nested* array, the one element shape that is itself sliceable), an owned-cell payload
projection (`&^`), a declared parameter (`Type::Ref`'s own bool), a branch join (`Phi`),
and a materialized quotation's env capture -- seven routes, not four, spread over nine
seeding sites that can reach `slice` (the prefix-borrow route covers three: a named
local, a struct field, a variant field). The tenth site, a borrow of a module static, is
unreachable here because a static is scalar-only. All nine are golden-covered
individually; the alternative (looking the shape up by element alone) is not merely
untidy -- a program holding only a mutable view has no shared row to find, so it panics.
A site's mis-recorded mutability is only observable when the *other* mutability of the
same element type is absent from the registry, so a golden for one must not build a view
of the opposite mutability over the same element type (which is why the variant-field
golden reads its buffer back through a plain array-element projection).

The env capture is the one site that could report a mutability it never recorded: its
`ref_mutable` is read out of the map at a boundary where a *scalar* capture legitimately
has no entry. It asks through the same panicking accessor the other sites use, gated on
the capture having a referent at all, rather than defaulting a missing entry to shared.

**Phase 4 exit notes (the poly-path rule R11 asked for, and one gap left as found):**

- A mutable view is **single-use per binding inside a generic body**, where the
  monomorphic path reborrows a named local. The poly walk move-tracks every non-`Copy`
  local (Slice 13's design: `poly_call_term` consumes on read when `poly_is_copy` is
  false), and R4 makes a mutable view non-`Copy`. So a generic consumer can store through
  a view (`s 0 >usize &!> v !`, one derivation) but cannot read-modify-write through one:
  that needs two, and the second naming is use-after-move. Ruled rather than worked
  around -- loosening it is a change to the poly borrow model, which is Slice 13's, not
  this slice's. Pinned by `poly_mutable_slice_local_is_single_use`.
- `slice` in a poly body works off a **body borrow** (`&a slice` / `&!a slice`), including
  over a generic *length* (`['T 'N]`'s length erases into the runtime one, which is the
  point of a view); a generic *element* is refused by name (R1.2's locked non-goal). It
  does **not** work off a declared fully-concrete `&[i64 3]` parameter: those arrive as
  `PolyType::Concrete(Type::Ref(..))` and `poly_ref_array_parts` matches only
  `PolyType::Ref`. Pre-existing and not slice-specific -- `&>` has the identical gap
  (probe: `: f ( &[i64 3] 'T -- i64 'T ) | x | 0 >usize &> @ x ;` reports
  ``&> is not permitted on `&[i64 3]` ``) -- and closing it means threading `refs` into the
  poly walk to resolve a `RefId`, which would move an existing diagnostic. Left as found.

**Phase 4 mutation-tested guards (all killed):** the `borrow_mutability` slice arm
deleted (the two-live-mutable-subslices golden flips to accept), `slice` interning a
shared view off a `&!` receiver (mono and poly, separately), the poly `&>` slice-receiver
mutability guard stubbed, the poly `slice` generic-element gate opened, the poly
`subslice` arm deleted, `check_poly_slice_offset` made permissive at its own three call
sites (`subslice`'s two operands, `&>`'s one), the parser interning `!Slice[T]` as shared,
`!Slice` dropped from the reserved names, and the lowering mutability lookup hardcoded
shared at each of the nine seeding sites separately (the prefix borrow, the struct-field
and variant-field projections, the array-element and slice-element projections, the
owned-cell payload projection, the declared parameter, the branch join, the env
capture).

### Phase 5: roadmap correction, regression sweep, growth-signal re-check  *(standard)*

Deliver R14: rewrite the P7.S3c body + exit in `docs/roadmap/P7-language-prereqs.md` from
the fallible-accessor promise to the runtime-trap reality. Run the full regression suite,
and re-run CLAUDE.md's growth-structure signals against every file the slice modified
significantly (notably `src/check/word_families.rs`, `src/backend/qbe.rs`, and
`src/check/poly.rs`, echoing slice3b's deferred `poly.rs` split decision).

**Scope:**

- Modify: `docs/roadmap/P7-language-prereqs.md:139`/`:187`.
- Verify: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.
- Record in the sweep: capturing a slice *value* into a materialized quotation ICEs at
  `backend/qbe.rs:521` (``an aggregate field is copied by blit, not scalar-stored``).
  Pre-existing and not slice-specific -- capturing an array or a struct value panics
  identically, so Phase 2's `IrType::Slice` arm there is right and the gap is in the env
  bundle's one-word-per-capture shape. Capturing the *reference* and slicing inside the
  body works (`a_materialized_quotation_slices_a_captured_mutable_reference`).
- Out of bounds: any code change beyond fmt/clippy fixes surfaced by the sweep.

**Phase 5 exit notes (regression sweep):** `cargo fmt --check`, `cargo clippy -- -D
warnings`, and the full `cargo test` (1290+ tests, every suite including
`tests/phase7_slice3a.rs`, `tests/phase7_slice3b.rs`, `tests/qbe_baseline.rs`) are green
unmodified -- the phased work left nothing for this sweep to fix.

**Phase 5 exit notes (known limitation, recorded not fixed):** capturing a slice *value* into
a materialized quotation ICEs at `backend/qbe.rs:521` (``an aggregate field is copied by blit,
not scalar-stored``). Pre-existing and not slice-specific -- capturing an array or a struct
value panics identically -- so Phase 2's `IrType::Slice` arm there is right and the gap is in
the env bundle's one-word-per-capture shape. Capturing the *reference* and slicing inside the
body works (`a_materialized_quotation_slices_a_captured_mutable_reference`). Fixing it is out
of bounds for this sweep, which is docs-only.

**Phase 5 exit notes (test name overclaims, recorded not fixed):**
`len_over_a_slice_answers_runtime_length` (`src/check/word_families.rs`) is a bare
`check_src(...).unwrap()`, so it proves `len` *typechecks* on a slice receiver and yields
`usize`, not that it answers a runtime length. The runtime claim is genuinely carried by the
`sum_over_a_slice_noninline_prints_twentyfive` golden, so this is a name/assertion mismatch
rather than a coverage hole; renaming it to `len_over_a_slice_typechecks_to_usize` is a code
change, which this docs-only sweep puts out of bounds.

**Phase 5 exit notes (growth-signal re-check, echoing slice3b's deferred `poly.rs`
split):** re-run against the four files this slice grew the most
(`src/check/poly.rs` +471/-3 to 5456 lines, `src/backend/qbe.rs` +295/-4 to 3120,
`src/check/word_families.rs` +354/-4 to 2860, `src/ir/func_builder/word_families.rs`
+281/-13 to 1683 -- the last one is the lowering twin of the third, a same-named file in a
different module, and statistically level with the two above it; smaller adds
(`repl.rs` +184, `check/declarations.rs` +154, `ir/layout.rs` +149) don't cross the size
the other four are at and are not re-checked here):

- `poly.rs`: still the deferred call from `project_poly_rs_split_deferred` (recorded in
  agent memory, restated here so this doc stands on its own): the two available splits
  are a `poly/diagnostics.rs` layer-split with no precedent elsewhere in the checker, and
  a `poly/eliminator.rs` split that would cut the mutual recursion
  `poly_call_term` -> `poly_eliminator_call` -> `poly_walk` -> `poly_call_term` across a
  file boundary. This slice's additions are mostly not a fifth concern bolted alongside
  the existing ones -- most arms land inside a predicate or walk function the file
  already has (`poly_is_copy`, `is_reference_slot`, `poly_reference_word`'s
  receiver dispatch, `poly_walk`'s array/slice-element arms) -- but it does add two new
  top-level functions, `check_poly_slice_offset` and `poly_slice_generic_element_error`,
  each a helper beside the arm that calls it rather than a new axis of concern. No second
  quotation consumer was added (the deferred note's named trigger still hasn't fired), so
  both splits above are still wrong for the reason recorded before. Deferred, again.
- `word_families.rs` (`src/check`): the new `slice`/`subslice`/`&>`/`&!>` arms sit beside
  the array-word and reference-word families they extend (`check_array_word`,
  `check_reference_word`), each with its own adjacent error helper (`check_slice_offset`),
  matching the file's existing per-family-plus-helpers grouping. No import divergence, no
  orphaned function: every new arm is called from the same dispatch its sibling
  array/reference arms are. Growing, not diverging -- no split indicated.
- `word_families.rs` (`src/ir/func_builder`, the lowering twin of the checker file above --
  missed by this sweep's first pass, since the two files share a basename): every new
  method (`lower_borrow`, `slice_id_of`, `load_slice_parts`, `build_slice_value`,
  `bounds_check_dynamic`, `subslice_range_check`) is added to the file's one
  `impl<'a> FuncBuilder<'a>` block, in the same per-method style as the reference/array/
  owned-cell/struct-field primitives already there (per the file's own header comment).
  No import divergence, no orphaned function, no layer mixing introduced. Growing, not
  diverging -- no split indicated.
- `qbe.rs`: the new aggregate-classification and index-emit functions
  (`module_has_slice`, `emit_subslice_trap`) are new top-level functions, not arms added
  to an existing one, but they sit beside the existing per-`IrType` match arms they extend
  (register class, load/store op, `qbe_abi_ty`, `emit_oob_trap` reuse), not in a new
  section. High- and low-level code were already mixed here before this slice (ABI
  classification beside instruction emission is the file's existing shape); this slice
  does not introduce that mixing, just extends it. No split indicated.

## Testing

Per CLAUDE.md: every stage function gets a unit test beside it (happy path + one
error/edge), every exit criterion is a golden, diagnostics assert the exact message, and
guards are mutation-tested (delete the arm, the test must fail).

**Goldens (`tests/phase7_slice3c.rs`):**

- `sum_over_a_slice_noninline_prints_twentyfive` (the exit criterion, diffed against the
  length-threading twin).
- `recursive_divide_and_conquer_over_subslices_runs` (the **shared** form lands in
  Phase 3 as `recursive_divide_and_conquer_over_shared_subslices_runs`; Phase 4 owes the
  mutable-half twin).
- `slice_out_of_range_index_traps_at_runtime` (asserts the located OOB message, no
  `Option`/`Result`).
- `declared_slice_output_is_stored_reference_error` (lands in Phase 3, once a signature
  can spell `Slice[T]`; Phase 1 proves the same claim by direct call, see its exit notes).
- `two_simultaneous_mutable_subslices_is_error`.
- `dup_on_mutable_slice_is_error` / `dup_on_shared_slice_ok` (the error case asserts the
  **exact** located diagnostic string, per CLAUDE.md, not merely that an error occurs).
- `slice_through_a_declared_quotation_parameter_row_runs` — a word whose parameter row is
  `~[ Slice[i64] -- ]`, exercising the brief's locked quotation-parameter-input-row
  decision (distinct from the `sum` golden, which captures a slice into a quotation
  *literal*). The word is `inline`, not non-`inline` as first written: a `~[ ]` parameter
  can only be spliced, so the language refuses a non-`inline` word declaring one (see
  Phase 3's exit notes; the golden asserts that refusal too).

**Unit tests beside their stage:**

- `src/ast.rs`: `slice_type_name_renders_element`, `is_ref_true_for_slice` (R1.4),
  `slice_interns_by_element_and_mutability`.
- `src/check/builtins.rs`: `is_copy_shared_slice_is_copy_mutable_slice_is_not`,
  `contains_reference_true_for_slice`.
- `src/check.rs`: `find_zero_unsafe_element_names_slice` (asserts the exact located
  diagnostic string, not merely that an element is flagged).
- `src/check/captures.rs`: `classify_capture_slice_is_reference_not_scalar`.
- `src/check/operators.rs`: `dot_printable_set_slice_decision` (asserts the R7
  printability decision, matching the REPL/print renderers).
- `src/check/declarations.rs`: `extern_boundary_rejects_slice_with_located_error` (asserts
  the exact located FFI-rejection diagnostic).
- `src/check/word_entry.rs`: `slice_output_is_rejected_and_slice_input_is_admitted`
  (R1.4/R5, Phase 1) — driven directly against `check_reference_free_signature`, since no
  surface spelling exists yet to write this as source.
- `src/check/audits.rs`: `poly_slice_output_is_rejected_and_poly_slice_input_is_admitted`
  (R1.4/R5 poly twin, Phase 1) — the same two claims as the `word_entry.rs` test, on the
  poly path, driven against a hand-built `PolySig`.
- `src/repl.rs`: `remap_type_rebases_sliceid_across_modules`, `format_stack_renders_slice`.
- `src/ir/layout.rs`: `carried_slot_bytes_slice_is_aligned_aggregate` and
  `scalar_size_align_refuses_a_slice` (Phase 2; the planned
  `slice_field_width_is_sixteen` has no `field_width` to test -- see Phase 2's exit
  notes -- so the 16-byte claim is asserted through `slice_layout` in
  `ir_type_of_slice_is_a_two_word_aggregate_not_a_pointer` and the carried-slot test).
- `src/ir/types.rs`: `ir_type_of_slice_is_a_two_word_aggregate_not_a_pointer` (R1.3/R2.1).
- `src/backend/qbe.rs`: `qbe_abi_ty_slice_is_aggregate_not_scalar_width`,
  `user_struct_named_slice_does_not_collide_with_slice_aggregate` (Finding 1: the reserved
  symbol is unforgeable), `member_ty_refuses_a_slice` (R5's field-position ban, asserted
  where a struct member would be spelled).
- `src/check/word_families.rs`: `len_over_a_slice_answers_runtime_length`,
  `slice_constructs_a_view_from_an_array_reference`,
  `subslice_rederives_a_fresh_slice_not_a_nested_borrow`.
- `src/check/poly.rs`: `poly_is_copy_mutable_slice_is_not`, `poly_len_over_a_slice_ok`,
  `is_reference_slot_true_for_slice`, `poly_type_str_renders_slice`.

**R13 (OQ3) reborrow chain, named explicitly:**
`mutate_innermost_hop_of_buffer_slice_subslice_element_chain_while_outer_live` — mutates
through the innermost `&!element` of a `&!buffer -> &!Slice -> &!sub-slice -> &!element`
chain with the outer hops still live, the one shape the brief's reborrow probe did not
cover.

**Mutation-tested guards (delete the arm → test fails):** R1.4 `is_ref` slice arm (delete
→ the `sum` golden's `Slice` input is rejected at declaration), R4 mutability split (both
directions) at both `is_copy` and its poly twin `poly_is_copy`, R5 `contains_reference`
(the output-ban golden must flip to accept), R6 zero-unsafe arm, R8.1 `classify_capture`,
R8.2 `remap_type`, R3 `qbe_abi_ty`, the R9.2 bounds-trap, and both poly-signature routes
(`top_level_ref`'s concrete-reference clause and `contains_poly_reference`'s `Concrete`
delegation).

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
    { "phase": 3, "focus": "shared slice/subslice construction, len, shared indexed access", "effort": "M", "difficulty": "hard" },
    { "phase": 4, "focus": "mutable views, borrow rules ported, poly borrow arms, reborrow-chain-depth test", "effort": "M", "difficulty": "hard" },
    { "phase": 5, "focus": "roadmap exit-criterion correction, regression sweep, growth-signal re-check", "effort": "S", "difficulty": "standard" }
  ]
}
```
