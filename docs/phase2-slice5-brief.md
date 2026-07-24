# Phase 2 Slice 5 — fixed-size arrays + `usize` (brief)

Design capture for the spec-writer. Everything below is **locked** from the
design conversation; the spec turns it into numbered, traceable requirements and
a phased delivery plan. Do not reopen decisions here.

## What ships

Fixed-size, heap-free **arrays** as an inline value aggregate, plus the
target-width **`usize`** index/length type. Arrays are the fixed-capacity
*storage substrate* for user-defined bounded containers (a `Stack`,
`RingBuffer`, the eventual VM's `Memory`): the compiler supplies the `[T N]`
primitive + `fill`/`get`/`set`/`len` words + a bounds trap; the user wraps an
array in their own `type:` struct and exposes custom words. This is the
**`fixed` layer** of the `core/fixed/alloc/hosted` stack. Growable containers
(`Vec`/`Map` that resize) are the **`alloc` layer**, built later on Slice 7
pointers, and are explicitly out of scope.

## Why this is a real slice, not a convenience

Every bounded container has a **runtime cursor** into its array (a stack's
`top`, a ring buffer's head/tail, the VM's PC). That index is a runtime `usize`,
not a compile-time constant, so **dynamic indexing is mandatory** even for the
simplest user container, which makes the bounds trap load-bearing. Static-only
(constant-index) arrays cannot express a `Stack`. That is what decides the trap
question below.

## Locked decisions

- **D1 — arrays are a fixed-capacity inline value aggregate.** Fixed size known
  at compile time, heap-free, all-`Copy` (blit on copy, lives in a frame slot),
  reusing the Slice 3/4 layout / field-load-store / carried-slot machinery. No
  new memory model. Growable/heap arrays are `alloc`-layer / Slice 7.
- **D2 — type spelling `[T N]`.** `[` element-type count `]`, element then
  count, space-separated (e.g. `[i64 4]`, `[f64 16]`). The element type may be
  any sized value type (scalar, struct, enum, or another array → nesting).
  `count` is a decimal literal `>= 1`. New lexer tokens `[` / `]`.
- **D3 — `Type` stays `Copy`; array types are interned.** Array types are
  *structural* (anonymous, not name-declared) but interned into a per-program
  **array-type registry** keyed by a `Copy` `ArrayId`, deduplicated by
  `(element, count)` shape, exactly mirroring `Struct(StructId, …)` /
  `Enum(EnumId, …)`. So `Type` gains `Array(ArrayId)` and stays `Copy`, and the
  id keys into the layout/marshalling machinery like the other aggregates.
  `IrType` gains `Array(ArrayId)`; an `ArrayLayout { elem, count, stride, size,
  align }` parallels `StructLayout`/`EnumLayout`.
- **D4 — construction is `fill`.** `<value> <count-literal> fill  ( T -- [T N] )`:
  the element type comes from the value, `N` is the literal count. Lowers by
  **unrolling** `N` element stores into a fresh frame slot (simple, no runtime
  dependency; a large-`N` runtime fill helper is deferred). The `{ … }`
  stack-snapshot literal is **deferred** (you can build any array with `fill` +
  `set`).
- **D5 — access words `get` / `set` / `len`.**
  - `get ( [T N] usize -- T )` is a **non-consuming read**: the array stays on
    the stack, the element is copied out, no blit. This is a Copy carve-out
    (see M4), so reads are cheap even in a recursive walk.
  - `set ( [T N] usize T -- [T N] )` is a **functional write**: blit a fresh
    copy of the array, store the element into it, yield the new array. O(N) per
    update. The original is unchanged (value semantics).
  - `len ( [T N] -- usize )` is a **compile-time constant** (the count lives in
    the type), non-consuming.
  - `get`/`set` are named (not Forth `@`/`!`) matching Sooth's English word
    culture (`dup`/`swap`/`drop`). The flat-namespace collision with future
    container verbs is temporary and resolved by Phase 4 overloading.
- **D6 — dynamic indexing with a runtime bounds trap.** A **compile-time
  constant** index is bounds-checked at compile time: an out-of-range literal is
  a sharp located error naming the length and the index. A **runtime** index
  emits an `index < N` check (N is the compile-time `usize` count) guarding an
  **out-of-bounds trap**: the trap reuses the hosted print/exit path, prints a
  located message naming the length and the index, and exits nonzero. This is
  the **first runtime failure path** in Sooth, and it is the sharp-failure ethos
  applied at the one boundary the compile-error lever cannot reach (a loud,
  located abort, not silent corruption). A **new backend-neutral IR op for
  dynamic element addressing** (base + index*stride) is required so `Ptr` stays
  opaque and word-width-neutral (no pointer-as-`u64` arithmetic).
- **D7 — `usize` is a distinct target-width unsigned integer.** `Type::Usize`
  and `IrType::Usize`; its width is a **target parameter** (currently 8 bytes
  for the QBE / x86-64 target), never a hardcoded literal `8`. Slice 5 is where
  target word width becomes **load-bearing and testable**: `usize` is the first
  type whose size is target-defined *and stored inside an aggregate* (an array
  index/length field), so introduce a single **target word-width parameter**
  that `usize` size/align derive from, threaded into layout sizing; `Ptr`
  retrofits to the same parameter in Slice 7. Integer-tower operations
  (arithmetic, comparison, type-directed `.` printing) extend to `usize`.
- **D8 — literal coercion, explicit conversion for computed values.** Integer
  *literals* coerce implicitly to `usize` in `usize`-typed positions (index,
  count, `usize` operands), so `arr 3 get` and `top 1 +` read naturally. A
  *computed* runtime integer value requires an explicit `>usize`. `>usize` and
  `usize`→int conversions exist; their truncate/extend behaviour is
  **target-defined** (the width is not a compile-time constant) and documented
  as such. No silent width-mixing of non-literals.
- **D9 — arrays are not `.`-printable.** `.` on an array is a sharp located
  error (index it and print elements); `=` / arithmetic on an array is a sharp
  error via the existing operand guards. Consistent with structs/enums.
- **D10 — REPL parity.** Arrays cross the REPL line boundary via carried-slot
  marshalling (mirror struct/enum); a residual array slot renders as a
  `<[T N]>` placeholder. `usize` slots carry and print. Array-typed effects and
  `fill`/`get`/`set`/`len` parse at REPL scope.

## Micro-decisions

- **M1 — count is a decimal literal**, no const-expression evaluation (respects
  the *no comptime interpreter* invariant). A non-literal count is a located
  parse/semantic error.
- **M2 — layout.** `stride = round_up(elem_size, elem_align)`;
  `align = elem_align`; `size = count * stride`; `carried_slot_bytes =
  round_up(size, 8)`. All derived from element widths + the target word-width
  parameter (word-width-neutral).
- **M3 — nesting falls out of interning + the combined registry.** Array-of-
  struct, array-of-array, and struct/enum-with-an-array-field all work by the
  element `Type` being an interned aggregate id. The value-recursion cycle
  detector (`visit_recursion` / `node_edges`) gains an array node with a single
  edge to its element type; an array cannot introduce an infinite cycle a
  struct/enum could not (a fixed-count array of a self-containing type is caught
  by the element edge). Include arrays in the type graph.
- **M4 — `get` is non-consuming for arrays specifically.** Reading a `Copy`
  element does not consume the array's resource, so leaving the array on the
  stack is consistent with the affine spine (Copy types never need explicit
  `dup`), and it avoids an O(N) blit per read. Struct getters keep their
  existing semantics; this is an array carve-out justified by Copy + the
  indexed-in-a-loop access pattern. Forward-compatible with borrow-based reads
  in Phase 3.
- **M5 — `set` is the pure sibling of a future `set!`.** The functional
  `set ( [T N] usize T -- [T N] )` stays when Phase 3 adds a borrow-based
  in-place `set!`; nothing here forecloses in-place mutation (the O(N) write
  cost is the concrete motivation for references). `set!` will share `set`'s
  offset computation and bounds trap, differing only in "store into the borrowed
  place" vs "store into a fresh copy."
- **M6 — `fill` unrolls** `N` element stores (simple, no runtime dependency,
  no loop keyword needed since `N` is constant). A large-`N` runtime fill helper
  is deferred until a consumer needs it (the VM's large memory, Slice 6).

## Work by stage

### Frontend (lexer + AST + parser)

- New lexer tokens `[` / `]` (`is_delimiter`, `Token::LBracket`/`RBracket`).
- `Type` gains `Array(ArrayId)` (stays `Copy`); an `ArrayId` newtype parallel to
  `StructId`/`EnumId`; a `Module`-level array-type registry (interned).
- The type-reading path is the novel frontend change: a type is no longer always
  a single word. `parse_slot` (effect slots), `expect_field_type_token` +
  `parse_typedef`/enum variant fields, and `resolve_type` must accept a **type
  expression**: either a single word (scalar/struct/enum) or a bracketed array
  `[ elem count ]`, interning the `(element, count)` shape into the array
  registry and returning `Type::Array(id)`. Nested `[[i64 4] 4]` recurses. Since
  interning mutates registry state during resolution (today `resolve_type` is
  `&self`), decide the interning path (a resolve that can intern, or a
  post-parse interning pass).
- `usize` recognised as a type name (extend `Type::from_name` / the scalar
  table); `>usize` recognised in the conversion family; a `usize` integer
  literal path per D8's literal coercion.
- `fill` / `get` / `set` / `len` are ordinary positional words (generated sigs
  or built-in handling in the checker + IR, like the struct/enum generated
  words), not new syntax.

### Checker (`src/check.rs`)

- Register array types; resolve `usize`; extend the integer-tower checks
  (arithmetic / comparison operand guards, conversion source check, type-directed
  print) to `usize`, with D8 literal coercion into `usize` positions.
- Signatures for the array words: `fill ( T -- [T N] )` (N from the literal,
  element from T), `get ( [T N] usize -- T )` (non-consuming; the array remains),
  `set ( [T N] usize T -- [T N] )`, `len ( [T N] -- usize )`.
- Constant-index bounds check (D6): a literal index `>= N` is a sharp located
  error naming length + index.
- `.` / `=` / arithmetic on an array reach the existing printable/operand guards
  and name the array type (D9).
- Include arrays in `visit_recursion` / `node_edges` (M3).

### IR + backend (`src/ir.rs`, `src/backend/qbe.rs`)

- `IrType::Array(ArrayId)` + `ArrayLayout`; layout via the shared `place_fields`/
  `scalar_size_align`/`round_up` machinery (homogeneous stride, M2). `usize`
  size/align from the target word-width parameter (D7), not literal 8.
- `carried_slot_bytes` / `scalar_size_align` gain array + `usize` arms.
- A **new dynamic element-addressing IR op** (base + index*stride, index a
  runtime `Value`) lowered per-backend; keeps `Ptr` opaque (no `u64` pointer
  math). `get` = elem-addr + `FieldLoad` (non-consuming: the array `Value` stays
  live). `set` = `Alloc` + `Blit` (fresh copy) + elem-addr + `FieldStore`, yield
  the new array. `fill` = `Alloc` + N unrolled `FieldStore`s.
- Bounds trap (D6): `Cmp(index < N-const)` + `Jnz` to a trap block that calls a
  new **runtime out-of-bounds helper** (prints a located len+index message,
  exits nonzero) reusing the hosted print/exit path. First runtime failure path.
- Emit a QBE aggregate type per array (opaque sized blob, alignment-annotated),
  like the enum aggregate.
- Marshalling: an array slot is blitted out of / into the carried buffer at its
  `carried_slot_bytes` offset (extend the struct/enum arms).

### Print + REPL (`src/repl.rs`)

- `Session` gains the array registry (or shares the interned registry);
  `format_stack` renders an array slot as `<[T N]>` and advances the buffer by
  its size. `usize` carried slots print via the type-directed `.`.
  Array-typed effects and the array words parse at REPL scope
  (`parse_line_with_structs` / `typed_env` thread whatever the array words need).

## Success criteria (each → a golden; native binary or REPL, not IL strings)

1. **Type spelling + registration.** `[T N]` parses in effect slots and in
   struct/variant fields; new `[`/`]` tokens; a zero-length `[T 0]`, a
   non-literal count, and an unknown element type are sharp located errors. All
   Phase 0/1 + Slice 1-4 goldens still pass.
2. **`usize` enters the tower.** Literal coercion into `usize` positions,
   `usize` arithmetic/comparison, `>usize` and `usize`→int conversions, and
   type-directed `.` on a `usize`; a structural check that `usize` size derives
   from the target word-width parameter (not a hardcoded 8).
3. **`fill` constructs a correct array** (native): the elements read back.
4. **`get` / `set` end-to-end** (native): `set` at a runtime index produces a new
   array with exactly one element changed and the original unchanged (value
   semantics); `get` reads the right element; a value flows correctly with the
   array on the stack (non-consuming `get`).
5. **Bounds behaviour.** A constant out-of-range index is a **compile error**
   (located, names length + index); a runtime out-of-range index **traps**
   (native: nonzero exit + located message, verified to abort rather than
   corrupt).
6. **Array-as-struct-field + the `Stack` dogfood** (native): `push`/`pop`/`peek`
   over an embedded `[i64 N]` + a runtime `usize` cursor.
7. **Nesting** (native): array-of-struct, array-of-array, and struct-with-an-
   array-field construct and read back through the combined registry.
8. **REPL parity.** An array crosses the line boundary (carried-slot
   marshalling), renders `<[T N]>`, `usize` prints; the `Stack` dogfood runs in
   the REPL.

## Dogfood — `examples/stack.sth`

A bounded `i64` stack built on an embedded array, exercising array-as-field, a
runtime `usize` cursor (hence the trap path), non-consuming `get`, functional
`set`, and `len`:

```
type: Stack items [i64 16] top usize ;

: empty ( -- Stack )        16 zeros  0  Stack ;   \ or `0 16 fill 0 Stack`
: push  ( Stack i64 -- Stack ) ... \ set items[top]:=x, top+1, rebuild
: pop   ( Stack -- Stack i64 ) ... \ top-1, read items[top]
```

(Exact bodies are the spec's to pin; the shape above is the design intent:
functional-update idiom fully encapsulated behind `push`/`pop`.) Optionally add
a recursive `sum` over an array to exercise `get` in a recursive walk. Runs
native and in the REPL.

## Out of scope (deferred)

Growable / heap-backed arrays and any `Vec`-that-resizes (need pointers →
Slice 7 / `alloc` layer); **in-place mutation / `set!` / references / borrows**
(Phase 3); the `{ … }` stack-snapshot literal; **generic** containers
(`Stack` over any `T`) and any type parameters (Phase 4); **`isize`** (its only
motivation is pointer differences, which arrive with pointers in Slice 7); loop
/ iteration combinators (recursion only, no loop keyword); a large-`N` runtime
fill helper; array slicing / subarrays / views; multi-dimensional arrays as a
first-class type (nested arrays only); the bytecode-VM dogfood (Slice 6).

## Key risks

- **The multi-token array type in the type-reading path.** Types were always a
  single word; `[T N]` forces `parse_slot` / `expect_field_type_token` /
  `resolve_type` to parse a type expression and **intern** structurally during
  resolution (today `resolve_type` is `&self`). This is the novel frontend bit;
  the interning path (dedup by shape, `&mut` or a post-parse pass) needs care.
- **The dynamic element-addressing IR op.** Must stay backend-neutral: no
  pointer-as-`u64` arithmetic, `Ptr` opaque. `base + index*stride` is a new
  sanctioned op, lowered in the backend.
- **The target word-width parameter (D7).** First load-bearing use of a
  target-defined size *inside an aggregate*. Risk: scattering literal `8`s.
  Introduce one parameter feeding `usize`/layout sizing; `Ptr` joins it in
  Slice 7.
- **The bounds-trap runtime helper.** First runtime failure path: exit code,
  message format, stderr vs stdout, and reuse of the hosted print/exit path
  without dragging in a new runtime dependency.
- **Non-consuming `get` vs the affine checker.** Ensure `get` leaves the array
  live without requiring a `dup`, and that this carve-out does not leak into
  struct getter semantics.

## Current-state anchors (reuse points, verified on `main`)

- **AST** (`src/ast.rs`): `Type` enum @186 (`Int`/`Float`/`Bool`/`Struct`/`Enum`);
  free `resolve_type_name(structs, enums, name)` @37 + `Module::resolve_type_name`
  @28; `StructId` @70 / `EnumId` @109 (model `ArrayId` on these); `Module`
  fields `structs` @16 / `enums` @20; `Type::from_name` @248; `Type::name` @286.
- **IR** (`src/ir.rs`): `IrType` @41; `ir_type_of` @87; `StructLayout` @108 /
  `FieldLayout` @118 / `EnumLayout` @154; `round_up` @191; `scalar_size_align`
  @199; `carried_slot_bytes(ty, structs, enums)` @216 (already takes both
  registries); `LayoutBuilder` @301 with shared `place_fields` @330 /
  `ensure_struct` @350 / `ensure_enum` @364; registries `Structs` @141 / `Enums`
  @186 with `StructWord::Construct` @130 / `EnumWord::Construct` @178; aggregate
  ops `PtrOffset` @431 / `Alloc` @439 / `Blit` @443 / `FieldLoad` @447 /
  `FieldStore` @452; `field_aggregate_value` @1036.
- **Checker** (`src/check.rs`): `check_duplicate_type_names` @121; `TypeNode`
  @154 / `node_edges` @206 / `visit_recursion` @240 (generalize for arrays);
  `is_registered_variant` @394; `check_outputs` @419; `Ctx::Word { locals }`
  @46/@58; the numeric operand-guard + conversion-source + printable-guard error
  builders @659-@769.
- **Parser** (`src/parser.rs`): `prepass_type_decls` @55 / `build_registries`
  @100; `parse_slot` @459 / `parse_slots` @448 / `parse_effect` @441;
  `parse_typedef` @480 / `expect_field_type_token` @509; `resolve_type` @537
  (`&self` today); `parse_worddef` @358.
- **Lexer** (`src/lexer.rs`): `is_delimiter` @19; existing delimiter tokens
  `Semicolon`/`LParen`/`RParen`/`Pipe` @10-@13 and their char arms @78-@81 (add
  `[`/`]`).
- **REPL** (`src/repl.rs`): `Session` @209 (`structs` @215 / `enums` @220 /
  carried buffer + slot types @221-@227); `format_stack` @163 (the `<TypeName>`
  placeholder path @156/@182); `typed_env` @251; `eval_typedef` @285;
  `parse_line_with_structs` @273.
