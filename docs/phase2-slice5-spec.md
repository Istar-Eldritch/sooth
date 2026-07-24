# Phase 2 Slice 5 — fixed-size arrays `[T N]` + `usize`

**Status: specified.** Slice 5 of Phase 2 (typed core), on the scalar core +
Slice 3 structs + Slice 4 enums. Adds fixed-capacity, heap-free **value arrays**
(`[T N]`) plus the target-width **`usize`** index/length type, the first runtime
failure path (a bounds trap), and the first load-bearing use of a target
word-width parameter inside an aggregate. Design locked in
`phase2-slice5-brief.md` (D1–D10, M1–M6); this spec turns it into numbered,
traceable requirements and a phased delivery plan. Decisions are **not** reopened
here.

## What ships

- Fixed-size arrays as an **inline value aggregate** `[T N]` (D1, D2): heap-free,
  compile-time count `N ≥ 1`, element `T` any sized value type (scalar, struct,
  enum, or another array → nesting), all-`Copy` (blit on copy, drop no-op),
  reusing the Slice 3/4 layout / field-load-store / carried-slot machinery. No
  new memory model.
- New lexer tokens `[` / `]`; a **type expression** in the type-reading path
  (types are no longer always a single word); array types **interned**
  structurally into a per-program `ArrayId` registry keyed by `(element, count)`
  shape (D3, M1), mirroring `Struct(StructId)` / `Enum(EnumId)`. `Type` stays
  `Copy`.
- The `fixed`-layer array vocabulary: `fill` (construct, D4), `get` / `set` /
  `len` (access, D5), plus constant-index and runtime-index **bounds checking**
  with a runtime **out-of-bounds trap** (D6).
- `usize`: a distinct **target-width** unsigned integer (D7), its size/align
  derived from a single **target word-width parameter** (currently 8 for the
  QBE/x86-64 target), never a hardcoded literal `8`. Integer-tower ops
  (arithmetic, comparison, type-directed `.`) extend to `usize`; integer
  literals coerce into `usize` positions, computed values need explicit `>usize`
  (D8).
- Arrays are not `.`-printable and reject `=` / arithmetic via the existing
  guards (D9); arrays and `usize` cross the REPL line boundary via carried-slot
  marshalling, a residual array renders `<[T N]>` (D10).
- Dogfood `examples/stack.sth`: a bounded `i64` stack over an embedded array +
  a runtime `usize` cursor, exercising array-as-field, the trap path,
  non-consuming `get`, functional `set`, and `len`.

**Out of scope** (see full list below): growable/heap arrays, `set!` /
references / borrows, the `{ … }` literal, generics, `isize`, loop combinators,
a large-`N` runtime fill helper, slicing/views, first-class multidimensional
arrays, the bytecode-VM dogfood.

## Why this is a real slice, not a convenience

Every bounded container has a **runtime cursor** into its array (a stack's `top`,
a ring buffer's head/tail, the VM's PC). That index is a runtime `usize`, not a
compile-time constant, so **dynamic indexing is mandatory** even for the simplest
user container, which makes the bounds trap load-bearing: static-only
constant-index arrays cannot express a `Stack`. Three genuinely new mechanisms
land here, each called out in Key risks:

1. **A multi-token type in the type-reading path.** Types were always a single
   word; `[T N]` forces `parse_slot` / `expect_field_type_token` / `resolve_type`
   to parse a type expression and **intern** the `(element, count)` shape during
   resolution (today `resolve_type` is `&self`).
2. **A backend-neutral dynamic element-addressing IR op** (`base + index*stride`,
   index a runtime `Value`), keeping `Ptr` opaque and word-width-neutral (no
   pointer-as-`u64` arithmetic).
3. **A target word-width parameter** feeding `usize`/layout sizing: the first
   target-defined size stored *inside* an aggregate (an array index/length
   field). `Ptr` retrofits to the same parameter in Slice 7.

Plus the first **runtime failure path** in Sooth (the bounds trap: a loud,
located abort, not silent corruption).

## Locked decisions (do not reopen)

- **D1** arrays = fixed-capacity inline value aggregate; heap-free; all-`Copy`;
  reuse Slice 3/4 layout/field/carried-slot machinery; no new memory model.
- **D2** type spelling `[ elem count ]` (element then decimal count `≥ 1`,
  space-separated); element = any sized value type incl. nested array; new lexer
  `[` / `]`.
- **D3** `Type` stays `Copy`; array types **structural + interned** into an
  `ArrayId` registry, deduped by `(element, count)`; `IrType::Array(ArrayId)`;
  `ArrayLayout { elem, count, stride, size, align }` parallels
  `StructLayout`/`EnumLayout`.
- **D4** construction `fill ( T -- [T N] )` (element from the value, `N` from the
  literal), lowered by **unrolling** N stores into a fresh slot; `{ … }` literal
  deferred.
- **D5** access `get ( [T N] usize -- T )` **non-consuming** read (Copy carve-out,
  M4); `set ( [T N] usize T -- [T N] )` **functional** write (fresh blit, O(N),
  original unchanged); `len ( [T N] -- usize )` **compile-time constant**,
  non-consuming; English words, flat namespace (overloading is Phase 4).
- **D6** dynamic indexing with a runtime **bounds trap**: a constant OOB index =
  compile error naming length + index; a runtime index = `index < N` guard to a
  trap that prints a located len+index message and exits nonzero, reusing the
  hosted print/exit path; requires the new dynamic element-addressing IR op.
- **D7** `usize` = distinct target-width unsigned integer (`Type::Usize`,
  `IrType::Usize`); width from a **target word-width parameter** (8 today), never
  a hardcoded `8`; first target-defined size *inside* an aggregate; integer-tower
  ops extend to `usize`.
- **D8** integer **literals coerce** into `usize` positions; **computed** values
  need explicit `>usize`; `>usize` and `usize`→int conversions exist,
  truncate/extend is **target-defined**; no silent width-mixing of non-literals.
- **D9** arrays are not `.`-printable (sharp located error); `=` / arithmetic on
  an array = sharp error via existing operand guards; consistent with
  structs/enums.
- **D10** REPL parity: arrays marshal across the line via carried slots, residual
  renders `<[T N]>`; `usize` slots carry and print; array effects +
  `fill`/`get`/`set`/`len` parse at REPL scope.
- **M1** count = decimal literal, no const-expr eval (respects *no comptime
  interpreter*); non-literal count = located error.
- **M2** layout: `stride = round_up(elem_size, elem_align)`; `align = elem_align`;
  `size = count * stride`; `carried_slot_bytes = round_up(size, 8)`; all derived
  from element widths + the word-width parameter.
- **M3** nesting falls out of interning + the combined registry; the recursion
  detector gains an array node with a single edge to its element type; arrays
  cannot introduce a cycle a struct/enum could not.
- **M4** `get` non-consuming for arrays specifically (Copy element, indexed-in-a-
  loop pattern); struct getters unchanged; forward-compatible with Phase 3
  borrows.
- **M5** `set` is the pure sibling of a future borrow-based `set!` (Phase 3);
  nothing here forecloses in-place mutation; `set!` will share `set`'s offset
  computation + bounds trap.
- **M6** `fill` unrolls N stores (constant N, no loop keyword, no runtime
  dependency); a large-`N` runtime fill helper is deferred (Slice 6).

## Requirements (by stage)

### Frontend — lexer + AST + parser (R1–R7)

- **R1 (D2).** Lexer gains `Token::LBracket` / `Token::RBracket` and `[` / `]` in
  `is_delimiter` + their char arms, alongside the existing
  `Semicolon`/`LParen`/`RParen`/`Pipe` (`src/lexer.rs`: `is_delimiter` @19,
  delimiter tokens @10–13, char arms @78–81). No other lexer change: `usize`,
  `fill`, `get`, `set`, `len`, `>usize` all tokenize as ordinary words.
- **R2 (D3, M1).** `ast::Type` gains `Array(ArrayId)` and **stays `Copy`**; a
  `Copy` `ArrayId` newtype modelled on `StructId` (`src/ast.rs` @70) / `EnumId`
  (@109); a `Module`-level interned array-type registry alongside `structs` (@16)
  / `enums` (@20), keyed and deduped by `(element: Type, count: u32)` shape.
  `Type::name` (@286) renders an array as `[T N]`; `Type::from_name` (@248) is
  unchanged (arrays are never a single name).
- **R3 (D2, D3, M1, M3).** The type-reading path accepts a **type expression**:
  either a single word (scalar / struct / enum, via the existing
  `resolve_type_name(structs, enums, name)` @37) **or** a bracketed array
  `[ elem count ]`, where `elem` is itself a type expression (nested
  `[[i64 4] 4]` recurses) and `count` is a decimal literal `≥ 1`. Resolving a
  bracket **interns** the `(element, count)` shape and returns
  `Type::Array(id)`. This threads through `parse_slot` (`src/parser.rs` @459),
  `parse_slots` @448 / `parse_effect` @441, `expect_field_type_token` @509 /
  `parse_typedef` @480 (struct fields + enum variant fields), and `resolve_type`
  @537. Because interning mutates registry state during resolution (today
  `resolve_type` is `&self`), the resolver either takes `&mut` interning access
  or runs a post-parse interning pass over recorded shapes; pick one and keep it
  consistent with `prepass_type_decls` @55 / `build_registries` @100.
- **R4 (D2, M1).** A malformed array type is a **located** error at parse/semantic
  time: unknown element type (X1), zero/negative count `[T 0]` (X2), a
  non-literal count `[T n]` / `[T k]` (X3). Errors name the offending element or
  count text.
- **R5 (D7, D8).** `usize` is recognised as a type name (extend the scalar table
  behind `Type::from_name` @248 / `resolve_type_name` @37 to add `Type::Usize`);
  `>usize` is recognised in the conversion word family; an integer-literal path
  supports D8 coercion (a bare integer literal is admissible in a `usize`
  position without an explicit conversion).
- **R6 (D4, D5).** `fill` / `get` / `set` / `len` are ordinary positional words
  (built-in handling in the checker + IR, like the struct/enum generated words at
  `parse_worddef` @358 / the generated-word sigs), **not** new syntax. They parse
  at word scope with no grammar change beyond R1.
- **R7 (D2, D3).** Array-typed effect slots (`( [i64 4] -- i64 )`) and array
  fields in `type:` declarations parse and resolve via R3, so an array is a
  first-class field/parameter/return type wherever a scalar/struct/enum is.

### Checker — `src/check.rs` (R8–R14)

- **R8 (D3, M1).** Register array types; the interned registry participates in
  the duplicate/consistency checks alongside structs/enums (`check_duplicate_type_names`
  @121); structural dedup means two spellings of `[i64 4]` resolve to one
  `ArrayId`.
- **R9 (D7, D8).** Resolve `usize`; extend the integer-tower guards to `usize`:
  the numeric operand guard, conversion-source check, and type-directed print
  builders (@659–@769) accept `usize`. Implement D8 literal coercion: an integer
  literal unifies with a `usize`-typed position (index, count operand, `usize`
  operand); a computed (non-literal) integer in a `usize` position without
  `>usize` is a sharp error (X10). `>usize` and `usize`→int conversions
  type-check; their truncate/extend is documented target-defined.
- **R10 (D5).** Signatures for the array words, from the element type + literal
  count in scope:
  - `fill ( T -- [T N] )` — `N` from the preceding count literal, element from
    `T`;
  - `get ( [T N] usize -- T )` — **non-consuming**: the array stays on the stack
    (R12);
  - `set ( [T N] usize T -- [T N] )`;
  - `len ( [T N] -- usize )` — **non-consuming**, folds to the compile-time
    constant `N`.
  Arity/element/index-type mismatches reach the existing arity/type-mismatch
  path (X8).
- **R11 (D6, M1).** Constant-index bounds check: a **literal** index `≥ N` (or on
  a `[T N]` where the literal is out of range) is a sharp **located** error
  naming the length `N` and the index (X4). A runtime index defers to the R17
  trap.
- **R12 (D5, M4).** `get` on an array is a **Copy carve-out**: reading the
  element does not consume the array resource, so the checker leaves the array
  `Value` live on the stack without requiring a `dup`, consistent with the
  affine spine (Copy types never need explicit `dup`). This carve-out is scoped
  to the array `get` word and **must not** change struct getter semantics.
- **R13 (D9).** `.` on an array reaches the printable guard and errors naming the
  array type `[T N]` (X6); `=` / arithmetic on an array reach the operand guards
  and error the same way (X7). Reuses the struct/enum guard machinery (@659–@769).
- **R14 (M3).** Include arrays in value-recursion detection: `node_edges` @206
  gains an array node with a single edge to its element type; `visit_recursion`
  @240 traverses it. A `[T N]` whose element (transitively) contains the type
  under construction is caught exactly as the struct/enum cycle case (X5);
  detection terminates.

### IR + backend — `src/ir.rs`, `src/backend/qbe.rs` (R15–R21)

- **R15 (D7, M2).** Introduce a single **target word-width parameter** (bytes;
  8 for the QBE/x86-64 target). `IrType::Usize` derives its size/align from this
  parameter; `scalar_size_align` @199 gains a `Usize` arm returning
  `(word_width, word_width)` — **not** a literal `8`. This is the first
  load-bearing, testable use of the parameter *inside* an aggregate; `Ptr` @199
  retrofits to the same parameter in Slice 7.
- **R16 (D3, M2).** `IrType::Array(ArrayId)` + `ArrayLayout { elem, count,
  stride, size, align }`, built via the shared `place_fields` @330 /
  `scalar_size_align` @199 / `round_up` @191 machinery: `stride =
  round_up(elem_size, elem_align)`, `align = elem_align`, `size = count *
  stride`, all resolving nested element aggregates through the combined registry
  (`LayoutBuilder` @301, `ensure_struct` @350 / `ensure_enum` @364 gain
  `ensure_array`). `carried_slot_bytes` @216 gains an `Array` arm
  (`round_up(size, 8)`, M2) and `scalar_size_align`/the array-layout builder
  cover the `Usize` element case. `ir_type_of` @87 maps `Type::Array` /
  `Type::Usize`.
- **R17 (D6).** A **new backend-neutral dynamic element-addressing IR op**:
  `base + index*stride`, `base` an aggregate `Value`, `index` a runtime `usize`
  `Value`, `stride` the compile-time constant from `ArrayLayout`. Yields an
  opaque element place (not a `u64`); `Ptr` stays opaque, no pointer-as-`u64`
  arithmetic. It sits beside the existing aggregate ops `PtrOffset` @431 /
  `Alloc` @439 / `Blit` @443 / `FieldLoad` @447 / `FieldStore` @452 and is
  lowered per-backend in `src/backend/qbe.rs`.
- **R18 (D4, D5, M4, M6).** Lower the array words:
  - `fill` = `Alloc` a fresh slot + **N unrolled `FieldStore`s** (M6) of the
    element value.
  - `get` = element-addr (R17) + `FieldLoad`, **non-consuming**: the array
    `Value` stays live (R12).
  - `set` = `Alloc` + `Blit` (fresh copy of the whole array) + element-addr +
    `FieldStore`, yielding the **new** array; the original is unchanged.
  - `len` = a constant `usize` from the layout (no memory access).
  Reuse `field_aggregate_value` @1036 for classification where applicable.
- **R19 (D6).** Bounds trap: a runtime index emits `Cmp(index < N-const)` +
  `Jnz` guarding a trap block that calls a **new runtime out-of-bounds helper**.
  The helper prints a located message naming the length and index and exits
  nonzero, reusing the hosted `printf`/exit path (`src/backend/qbe.rs` `$sfmt`
  @36, `printf` call sites @637–@684) **without** adding a new runtime
  dependency (message to stderr, nonzero exit). This is Sooth's first runtime
  failure path: it must **abort**, not fall through and corrupt.
- **R20 (D3, D9).** Emit a QBE aggregate type per array — an opaque, sized,
  alignment-annotated blob (like the enum aggregate `type :E = align A { b N }`
  @66), so the backend never reasons about element structure except through R17 +
  `FieldLoad`/`FieldStore`.
- **R21 (D1, D10).** Marshalling: an array slot is blitted out of / into the
  carried buffer at its `carried_slot_bytes` offset, extending the struct/enum
  arms of the marshalling path; `dup` blits the whole array, drop is a no-op
  (all-Copy, D1).

### Print + REPL — `src/repl.rs` (R22–R23)

- **R22 (D10).** `Session` @209 gains (or shares) the interned array registry
  alongside `structs` @215 / `enums` @220 and the carried buffer + slot types
  @221–227; `format_stack` @163 renders an array slot as `<[T N]>` via the
  existing `<TypeName>` placeholder path (@156/@182) and advances the buffer by
  the array's `carried_slot_bytes`. A `usize` carried slot prints via the
  type-directed `.`.
- **R23 (D10).** Array-typed effects and the array words parse at REPL scope:
  `parse_line_with_structs` @273 / `typed_env` @251 / `eval_typedef` @285 thread
  whatever the array words + array registry need (mirroring the enum plumbing),
  so `type:` with an array field and `fill`/`get`/`set`/`len` work line-to-line.

## Diagnostics (assert message text **and** the named type)

Each `Xn` is a golden asserting the located message **and** the type/length/index
it names.

- **X1** unknown element type in `[T N]` — names the unknown element (R4).
- **X2** zero/`< 1` length `[T 0]` — names the type, states length must be `≥ 1`
  (R4, M1).
- **X3** non-literal count `[T n]` — names the offending count token; a count
  must be a decimal literal, no const-expr eval (R4, M1).
- **X4** constant out-of-range index — **compile error**, located, names the
  length `N` **and** the index (R11, D6).
- **X5** value-recursion through an array element — located, names the cycle;
  terminates (R14, M3).
- **X6** `.` on an array — sharp located error naming the array type `[T N]`
  (R13, D9).
- **X7** `=` / arithmetic on an array — sharp located error naming the array type
  (R13, D9).
- **X8** `fill`/`get`/`set`/`len` arity / element / index-type mismatch — via the
  existing arity/type-mismatch path, names expected vs found (R10).
- **X9** `.` / arithmetic / conversion source misuse mixing `usize` with a
  non-coercible operand — names `usize` and the other type (R9, D8).
- **X10** a **computed** (non-literal) integer used in a `usize` position without
  `>usize` — located, names the source int type and `usize`, points at the
  missing conversion (R9, D8).

## Success criteria (each → a runnable golden: native binary or REPL, not IL

strings)

1. **Type spelling + registration.** `[T N]` parses in effect slots **and** in
   struct/variant fields; the new `[`/`]` tokens lex; `[i64 4]` and a second
   spelling of the same shape intern to one `ArrayId`. Negatives: `[T 0]` (X2), a
   non-literal count (X3), and an unknown element type (X1) are sharp located
   errors. **All** Phase 0/1 + Slice 1–4 goldens still pass (no regressions).
   → checker/parser goldens + full existing suite green.
2. **`usize` enters the tower.** A native program does `usize`
   arithmetic/comparison, `>usize` and a `usize`→int conversion, type-directed
   `.` on a `usize`, and a literal coercing into a `usize` position; a **computed**
   int in a `usize` position without `>usize` is X10. Plus a **structural check**
   that `usize` size derives from the target word-width parameter, **not** a
   hardcoded 8: a unit test asserts `scalar_size_align(IrType::Usize)` and the
   array/aggregate sizing that embeds a `usize` field both equal the single
   word-width parameter, and flipping that parameter in the test changes the
   derived size (proving no stray literal `8`). → native golden + IR unit test.
3. **`fill` constructs a correct array** (native): `fill` an `[i64 N]`, then read
   every element back via `get` and print them; the values match the fill value
   (exercises R18 unrolling + R17 addressing). → native golden.
4. **`get` / `set` end-to-end, value semantics** (native): build an array, `set`
   at a **runtime** index (a value computed at runtime, not a literal) to a new
   value, then `get` that index (changed) **and** another index (unchanged) from
   the **original** array kept alongside — proving `set` yielded a **new** array
   with exactly one element changed and the original untouched (D5 value
   semantics); confirm a value flows correctly with the array left on the stack
   after `get` (non-consuming, R12/M4). → native golden.
5. **Bounds behaviour — compile error and runtime trap.**
   (a) A constant out-of-range index is a **compile error**, located, naming
   length + index (X4).
   (b) A **runtime** out-of-range index **traps**: the native binary exits
   **nonzero** and prints a **located** message naming the length and the index;
   the golden asserts the nonzero exit code **and** the message text, and asserts
   the program **aborted** rather than corrupting — verified by (i) the exit code
   being nonzero/expected-abort (not 0), and (ii) a sentinel `.` **after** the
   out-of-bounds access producing **no** output (the trap fired before it),
   distinguishing abort from silent fall-through. This is the first runtime
   failure path. → compile-diagnostic golden + native trap golden.
6. **Array-as-struct-field + the `Stack` dogfood** (native): `examples/stack.sth`
   with an embedded `[i64 N]` and a runtime `usize` cursor runs `push` / `pop` /
   `peek` and prints expected results (exercises array-as-field, the runtime
   cursor → the trap path, non-consuming `get`, functional `set`, `len`).
   → native golden.
7. **Nesting both directions** (native): array-of-struct, array-of-array, **and**
   struct-with-an-array-field each construct and read back correctly through the
   combined registry (R16, M3). → native golden covering all three shapes.
8. **REPL parity.** An array crosses the REPL line boundary via carried-slot
   marshalling and a residual renders `<[T N]>`; a `usize` slot prints; the
   `Stack` dogfood runs in the REPL (declare `type: Stack …` then use it on later
   lines, mirroring the Slice 4 REPL-scope seeding). → scripted REPL golden.

## Dogfood — `examples/stack.sth`

A bounded `i64` stack over an embedded array, exercising array-as-field, a runtime
`usize` cursor (hence the trap path), non-consuming `get`, functional `set`, and
`len`. Shape (exact bodies are the implementation's to pin, using locked D2/D5
syntax; the design intent is a functional-update idiom fully encapsulated behind
`push`/`pop`/`peek`):

```
type: Stack items [i64 16] top usize ;

: empty ( -- Stack )            0 16 fill  0  Stack ;
: push  ( Stack i64 -- Stack )  \ set items[top] := x, top := top+1, rebuild Stack
  ... ;
: pop   ( Stack -- Stack i64 )  \ top := top-1, read items[top]
  ... ;
: peek  ( Stack -- Stack i64 )  \ read items[top-1], non-consuming get
  ... ;
```

Optionally add a recursive `sum` over an `[i64 N]` to exercise `get` in a
recursive walk (M4 rationale). Runs **native and in the REPL**.

## Non-functional

Green each phase (`cargo fmt --check && cargo clippy -- -D warnings && cargo
test`). Invariants held (see CLAUDE.md / DESIGN.md): **QBE-only** backend, no
LLVM, no native backend started; **backend-neutral IR** — the dynamic
element-addressing op keeps `Ptr` opaque, no pointer-as-`u64` arithmetic, and
`usize`/array sizing flows from the target word-width parameter, never a
hardcoded machine word; frontend `Type` and backend `IrType` stay distinct; the
**affine spine** holds — the `get` non-consuming carve-out is Copy-justified and
scoped to arrays only; **no comptime interpreter** — counts are decimal literals
(M1); **no JIT** — REPL via `dlopen`; `core` `no_std`. Reuse Slice 3/4 layout /
field-load-store / carried-slot / recursion-DFS machinery — **no new memory
model**. Verify by native + REPL goldens, **not** IL-string assertions (the one
sanctioned structural unit test is criterion 2's word-width check).

## Out of scope (deferred)

Growable / heap-backed arrays and any `Vec`-that-resizes (need pointers → Slice 7
/ `alloc` layer); **in-place mutation / `set!` / references / borrows** (Phase 3);
the `{ … }` stack-snapshot literal; **generic** containers (`Stack` over any `T`)
and type parameters (Phase 4); **`isize`** (motivated only by pointer
differences → Slice 7); loop / iteration combinators (recursion only, no loop
keyword); a large-`N` runtime `fill` helper; array slicing / subarrays / views;
first-class multi-dimensional arrays (nested arrays only); the bytecode-VM
dogfood (Slice 6).

## Key risks

- **The multi-token array type in the type-reading path (R3).** Types were always
  a single word; `[T N]` forces `parse_slot` / `expect_field_type_token` /
  `resolve_type` to parse a **type expression** and **intern** structurally
  during resolution (today `resolve_type` is `&self`). Novel frontend bit; the
  interning path (dedup by shape, `&mut` resolve vs post-parse pass) needs care.
  → **Phase 1.**
- **The dynamic element-addressing IR op (R17).** Must stay backend-neutral: no
  pointer-as-`u64` arithmetic, `Ptr` opaque. `base + index*stride` is a new
  sanctioned op, lowered in the backend. → **Phase 3 (hard).**
- **The target word-width parameter (R15).** First load-bearing use of a
  target-defined size *inside* an aggregate. Risk: scattering literal `8`s.
  Introduce one parameter feeding `usize`/layout sizing; criterion 2's structural
  test guards it. `Ptr` joins it in Slice 7. → **Phase 2 → Phase 3.**
- **The bounds-trap runtime helper (R19).** First runtime failure path: exit
  code, message format, stderr vs stdout, and reuse of the hosted print/exit path
  **without** dragging in a new runtime dependency; must abort, not corrupt.
  → **Phase 4 (hard).**
- **Non-consuming `get` vs the affine checker (R12).** Ensure `get` leaves the
  array live without a `dup`, and that this carve-out does not leak into struct
  getter semantics. → **Phase 3.**

## Current-state anchors (reuse points, verified on `main`)

- **AST** (`src/ast.rs`): `Type` enum @186; `resolve_type_name(structs, enums,
  name)` @37 + `Module::resolve_type_name` @28; `StructId` @70 / `EnumId` @109
  (model `ArrayId` on these); `Module.structs` @16 / `enums` @20;
  `Type::from_name` @248; `Type::name` @286.
- **IR** (`src/ir.rs`): `IrType` @41; `ir_type_of` @87; `StructLayout` @108 /
  `FieldLayout` @118 / `EnumLayout` @154; `round_up` @191; `scalar_size_align`
  @199; `carried_slot_bytes(ty, structs, enums)` @216; `LayoutBuilder` @301 with
  `place_fields` @330 / `ensure_struct` @350 / `ensure_enum` @364; registries
  `Structs` @141 / `Enums` @186; aggregate ops `PtrOffset` @431 / `Alloc` @439 /
  `Blit` @443 / `FieldLoad` @447 / `FieldStore` @452; `field_aggregate_value`
  @1036.
- **Checker** (`src/check.rs`): `check_duplicate_type_names` @121; `TypeNode` @154
  / `node_edges` @206 / `visit_recursion` @240; `is_registered_variant` @394;
  `check_outputs` @419; `Ctx::Word { locals }` @46/@58; numeric operand-guard +
  conversion-source + printable-guard builders @659–@769.
- **Parser** (`src/parser.rs`): `prepass_type_decls` @55 / `build_registries`
  @100; `parse_slot` @459 / `parse_slots` @448 / `parse_effect` @441;
  `parse_typedef` @480 / `expect_field_type_token` @509; `resolve_type` @537
  (`&self` today); `parse_worddef` @358.
- **Lexer** (`src/lexer.rs`): `is_delimiter` @19; delimiter tokens
  `Semicolon`/`LParen`/`RParen`/`Pipe` @10–13 and char arms @78–81 (add `[`/`]`).
- **Backend** (`src/backend/qbe.rs`): opaque-blob aggregate emit @66;
  `$sfmt`/`printf` hosted print path @36 + call sites @637–@684 (reuse for the
  trap helper).
- **REPL** (`src/repl.rs`): `Session` @209 (`structs` @215 / `enums` @220 /
  carried buffer + slot types @221–227); `format_stack` @163 (`<TypeName>` path
  @156/@182); `typed_env` @251; `eval_typedef` @285; `parse_line_with_structs`
  @273.

## Phases (delivery plan)

Sequenced by dependency: frontend type-expression + interning first; then `usize`
into the integer tower; then layout + codegen (the addressing op + word-width
parameter, the hardest structural work); then bounds checking + the runtime trap
(the first failure path); then dogfood + REPL parity + the full golden suite.

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Frontend: `[`/`]` lexer tokens; `Type::Array` + `ArrayId` newtype + Module array-type registry (interned, deduped by (element,count)); type-expression parsing in the type-reading path (parse_slot / expect_field_type_token / parse_typedef / resolve_type) with structural interning; nested `[[T N] M]`; malformed-array located errors (X1 unknown element, X2 zero-length, X3 non-literal count). Covers R1-R4, R7.",
      "effort": "M",
      "difficulty": "medium"
    },
    {
      "phase": 2,
      "focus": "`usize`: Type::Usize + IrType::Usize recognised as a type name and in the conversion family (>usize); extend the integer tower (arithmetic, comparison, conversion-source, type-directed print) to usize; D8 literal coercion into usize positions and the computed-value error (X9, X10); usize<->int conversions with target-defined truncate/extend. Covers R5, R9.",
      "effort": "M",
      "difficulty": "medium"
    },
    {
      "phase": 3,
      "focus": "Layout + codegen: single target word-width parameter feeding usize/layout sizing (first load-bearing use inside an aggregate, no hardcoded 8); ArrayLayout via shared place_fields/scalar_size_align/round_up (stride/size/align, M2); carried_slot_bytes + scalar_size_align array/usize arms; the new backend-neutral dynamic element-addressing IR op (base+index*stride, Ptr opaque); fill (unrolled stores) / get (non-consuming, R12/M4) / set (functional blit) / len (constant) codegen; opaque-blob QBE array aggregate; carried-slot marshalling. Constant-index bounds check (X4) and array recursion node (X5) and array print/operand guards (X6/X7). Covers R6, R8, R10-R16, R18, R20, R21, plus R11/R13/R14.",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "Runtime bounds trap (D6): index<N guard + Jnz to a trap block calling a new runtime out-of-bounds helper that prints a located len+index message via the hosted printf/exit path and exits nonzero, without a new runtime dependency; must abort not corrupt. First runtime failure path. Covers R17 (runtime arm) + R19.",
      "effort": "M",
      "difficulty": "hard"
    },
    {
      "phase": 5,
      "focus": "Dogfood examples/stack.sth (array-as-field, runtime usize cursor, non-consuming get, functional set, len) native + REPL; REPL parity (array carried-slot marshalling, `<[T N]>` residual, usize print, array words + array-field type: at REPL scope); the full 8-criterion golden suite incl. the word-width structural unit test and the native trap golden. Covers R22, R23 + success criteria 1-8.",
      "effort": "M",
      "difficulty": "medium"
    }
  ]
}
```
