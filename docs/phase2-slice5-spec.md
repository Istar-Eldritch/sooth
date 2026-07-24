# Phase 2 Slice 5 — fixed-size arrays `[T N]` + `usize` (delivered)

Slice 5 of Phase 2 (typed core), built on the scalar core + Slice 3 structs + Slice 4 enums. Adds heap-free fixed-size value arrays, the target-width `usize` index/length type, Sooth's first runtime failure path (a bounds trap), and the first target word-width parameter feeding aggregate sizing. Design locked in `phase2-slice5-brief.md` (D1–D10, M1–M6).

## What shipped

- **Arrays** `[T N]` as an inline value aggregate: heap-free, compile-time count `N ≥ 1` (bounded by `u32::MAX`), element `T` any sized value type (scalar/struct/enum/nested array), all-`Copy` (blit on copy, drop no-op), reusing Slice 3/4 layout / field-load-store / carried-slot machinery. No new memory model.
- **Lexer** `[` / `]` tokens; a multi-token **type expression** in the type-reading path; array types **structurally interned** into a per-`Module` `ArrayId` registry keyed by `(element, count)`. `Type` stays `Copy`.
- **Array vocabulary** (`fixed`): `fill` (construct via unrolled stores), `get` (non-consuming read), `set` (functional write, O(N) fresh blit), `len` (compile-time constant); flat namespace, ordinary positional words.
- **Bounds checking**: constant OOB = compile error; runtime index = `index < N` guard to a **runtime out-of-bounds trap** (located len+index message, nonzero exit, reuses hosted printf/exit path, no new runtime dependency; aborts, never corrupts).
- **`usize`**: distinct target-width unsigned int; size/align from a single **target word-width parameter** (8 today), never a literal `8`; first target-defined size inside an aggregate. Integer-tower ops extend to `usize`; literals coerce into `usize` positions, computed values need explicit `>usize`.
- **Guards**: arrays reject `.` / `=` / arithmetic via existing operand guards.
- **REPL parity**: arrays and `usize` marshal across the line via carried slots; residual array renders `<[T N]>`; type interning persists line-to-line.
- **Dogfood** `examples/stack.sth`: bounded `i64` stack over an embedded array + runtime `usize` cursor; native and REPL.

## Locked decisions (not reopened)

D1 arrays = fixed inline value aggregate, heap-free, all-`Copy`, no new memory model. D2 spelling `[ elem count ]`, count decimal `≥ 1`, new `[`/`]` tokens. D3 `Type` stays `Copy`; structural interning → `ArrayId`; `IrType::Array(ArrayId)`; `ArrayLayout { elem, count, stride, size, align }`. D4 `fill ( T -- [T N] )`, unrolled N stores. D5 `get` non-consuming, `set` functional, `len` compile-time constant. D6 runtime bounds trap; constant OOB = compile error. D7 `usize` target-width unsigned, width from word-width parameter. D8 literals coerce, computed values need `>usize`. D9 arrays not `.`-printable, reject `=`/arithmetic. D10 REPL marshalling + `<[T N]>` residual. M1 count = decimal literal only. M2 `stride=round_up(elem_size,elem_align)`, `align=elem_align`, `size=count*stride`, `carried_slot_bytes=round_up(size,8)`. M3 nesting from interning + combined registry; recursion detector gains array node. M4 `get` Copy carve-out scoped to arrays. M5 `set` is pure sibling of future `set!`. M6 `fill` unrolls, large-N helper deferred.

## Requirements → phases

**Phase 1 — frontend (R1–R4, R7).** `[`/`]` lexer tokens; `Type::Array` + `Copy` `ArrayId` newtype + `Module` interned registry (deduped by `(element,count)`); type-expression parsing through `parse_slot` / `expect_field_type_token` / `parse_typedef` / `resolve_type` with structural interning; nested `[[T N] M]`; located errors X1 (unknown element), X2 (zero-length), X3 (non-literal count). Count bounded by `u32::MAX`.
- `e1bf3f4a` type representation, interning, lexer — `src/ast.rs`, `src/ir.rs`, `src/lexer.rs`, `src/parser.rs`
- `fc1fae0c` `u32::MAX` bound on counts — `src/parser.rs`

**Phase 2 — `usize` into the integer tower (R5, R9).** `Type::Usize` + `IrType::Usize` recognised as type name and in `>usize` conversion family; integer-tower guards (arithmetic, comparison, conversion-source, type-directed print) extend to `usize`; D8 literal coercion + computed-value error (X9, X10); `usize`↔int conversions with target-defined truncate/extend. Branch merges require both literals to stay coercible.
- `e5d6dc77` recognise `usize`, extend tower, D8 coercion — `src/ast.rs`, `src/check.rs`, `src/ir.rs`
- `97e99680` branch-merge literal coercibility fix — `src/check.rs`

**Phase 3 — layout + codegen (R6, R8, R10–R16, R18, R20, R21; plus R11/R13/R14).** Single target word-width parameter feeding `usize`/layout sizing (no hardcoded 8); `ArrayLayout` via shared `place_fields`/`scalar_size_align`/`round_up`; `carried_slot_bytes` + `scalar_size_align` array/`usize` arms; new backend-neutral dynamic element-addressing IR op (`base+index*stride`, `Ptr` opaque); `fill`/`get`/`set`/`len` codegen; opaque-blob QBE array aggregate; carried-slot marshalling; constant-index bounds check (X4); array recursion node (X5); array print/operand guards (X6/X7).
- `affc2006` emit array aggregates in QBE — `src/backend/qbe.rs`, `src/check.rs`, `src/driver.rs`, `src/ir.rs`, `src/repl.rs`, `tests/phase0.rs`

**Phase 4 — runtime bounds trap (R17 runtime arm, R19).** `index<N` guard + `Jnz` to a trap block calling a new out-of-bounds helper; located len+index message via hosted printf/exit, nonzero exit, no new runtime dependency; aborts not corrupts. First runtime failure path.
- `8ec63894` runtime bounds trap — `src/backend/qbe.rs`, `src/ir.rs`

**Phase 5 — dogfood + REPL parity + goldens (R22, R23, criteria 1–8).** `examples/stack.sth` native + REPL; array carried-slot marshalling, `<[T N]>` residual, `usize` print, array words + array-field `type:` at REPL scope; type interning persists across REPL lines; word-width structural unit test + native trap golden.
- `d26798a4` persist type interning across REPL lines — `examples/stack.sth`, `src/parser.rs`, `src/repl.rs`, `tests/phase0.rs`, `tests/phase1.rs`
- `3b38c537` golden for `usize` arithmetic/comparison/conversion (C2) — `tests/phase0.rs`

## Diagnostics (each a golden asserting message + named type)

X1 unknown element in `[T N]`. X2 zero/`<1` length. X3 non-literal count. X4 constant out-of-range index (compile error, names length + index). X5 value-recursion through array element. X6 `.` on array. X7 `=`/arithmetic on array. X8 array-word arity/element/index-type mismatch. X9 `usize` mixed with non-coercible operand. X10 computed int in `usize` position without `>usize`.

## Success criteria (runnable goldens: native binary or REPL)

1. Type spelling + registration; two spellings intern to one `ArrayId`; X1/X2/X3 negatives; no regressions across Phase 0/1 + Slice 1–4.
2. `usize` in the tower (arithmetic/comparison/`>usize`/`usize`→int/`.`/literal coercion; X10); structural unit test that `usize` size derives from the word-width parameter (flipping it changes derived size).
3. `fill` constructs correct array, read back via `get` (R18 unrolling + R17 addressing).
4. `get`/`set` value semantics at a runtime index: `set` yields a new array with one element changed, original untouched; `get` non-consuming.
5. Bounds behaviour: (a) constant OOB = compile error (X4); (b) runtime OOB traps (nonzero exit + located message + sentinel `.` after access produces no output, proving abort not fall-through).
6. Array-as-struct-field + `Stack` dogfood: `push`/`pop`/`peek` native.
7. Nesting both directions: array-of-struct, array-of-array, struct-with-array-field.
8. REPL parity: array crosses line boundary, `<[T N]>` residual, `usize` prints, `Stack` runs in REPL.

## Invariants held

QBE-only backend, no LLVM, no native backend started; backend-neutral IR (dynamic element-addressing op keeps `Ptr` opaque, no pointer-as-`u64` arithmetic; `usize`/array sizing from the word-width parameter, never a hardcoded machine word); frontend `Type` and backend `IrType` stay distinct; affine spine holds (`get` non-consuming carve-out is Copy-justified, scoped to arrays); no comptime interpreter (counts are decimal literals); no JIT (REPL via `dlopen`); `core` `no_std`. Reuses Slice 3/4 layout / field-load-store / carried-slot / recursion-DFS machinery; verified by native + REPL goldens, not IL-string assertions (the one sanctioned structural unit test is criterion 2's word-width check).

## Out of scope (deferred)

Growable/heap arrays (→ Slice 7 / `alloc`); in-place mutation / `set!` / references / borrows (Phase 3); the `{ … }` literal; generics / type parameters (Phase 4); `isize` (→ Slice 7); loop/iteration combinators; large-N runtime `fill` helper (Slice 6); slicing / views; first-class multidimensional arrays; the bytecode-VM dogfood (Slice 6).
