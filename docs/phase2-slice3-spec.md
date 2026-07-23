# Phase 2 Slice 3 — technical specification: structs / records (aggregate value types)

User-declared struct value types: a `type:` declaration form, a user-extensible type
namespace, an inline-aggregate memory model (field offsets / size / alignment computed
from field sizes, word-width-neutral), and generated construction / field-read /
functional-update / destructure words, all heap-free and width- and offset-correct in
the emitted QBE, with sharp located diagnostics.

This is the spec for Slice 3 of Phase 2 (typed core). It implements
[phase2-slice3-brief.md](./phase2-slice3-brief.md) and builds on the completed scalar
core on `main` (lexer → parser → checker → IR → QBE-emit → driver/REPL: the fixed-width
integer tower, `f32`/`f64`, `bool`, homogeneous arithmetic / bitwise / comparison /
conversion, and the type-directed `.` print). Read alongside [../DESIGN.md](../DESIGN.md),
[../ROADMAP.md](../ROADMAP.md), and [../CLAUDE.md](../CLAUDE.md).

**Craft discipline.** This is the first *aggregate* slice, and the aggregate/layout
mechanism it builds is deliberately built **once** here and reused by Slice 4
(enums/`match`), Slice 5 (arrays), and Slice 6 (the VM dogfood). Getting the
representation right is the point of the slice. But do not import scope beyond the
brief: no enums, no `match`, no `|` sum syntax, no generics, no `Copy` marker, no
references, no heap. Every decision the brief locks (D1–D10, M1–M5) is locked;
everything it defers stays deferred (see [Scope](#scope-and-boundaries)). Follow the
existing table + special-case shape; do not build a general type system.

---

## Problem statement

The scalar core carries one `Type` per virtual-stack slot, and every slot is a value
that fits in one machine register and one fixed 8-byte carried-stack cell. A struct
breaks four of those assumptions at once, which is why it is a genuine slice and not a
table-fill:

1. **A user-extensible type namespace.** Until now every `Type` is one of a closed set
   of built-in scalars resolved by `Type::from_name`. A `type:` declaration adds a new
   named type at compile time, so type resolution must consult a per-program registry,
   and a struct type must stay a first-class `Type` that flows through effects, bodies,
   branch joins, and shuffles exactly like a scalar.
2. **An aggregate memory model.** A struct value is not register-shaped: it has fields
   at computed byte offsets, a size, and an alignment. The layout must be computed from
   field sizes/alignments (never a hardcoded machine word — the word-width-neutrality
   invariant), backed by QBE aggregate types and frame-local `alloc`, heap-free.
3. **Size-aware carried-stack marshalling (the central risk, D5).** The word-boundary /
   REPL carried-stack spill uses fixed 8-byte slots. A struct slot needs its full
   aggregate size. The prologue/epilogue in `lower_line` and the REPL `Session` buffer
   must become size-aware per slot, with every scalar behaviour byte-for-byte unchanged.
4. **Generated words.** Each struct `S` generates a constructor, per-field getter,
   per-field functional setter, and a destructure word, which must type-check like any
   word and lower to alloc/store/load rather than a normal call.

The deliverable: user-declared struct value types usable in effects and word bodies;
construction, field read, functional field update, destructure, and nesting;
offset-correct at runtime for mixed-width and nested fields; a struct value surviving a
word-call boundary and a REPL line boundary; heap-free; with sharp located diagnostics;
and all prior goldens green with scalar behaviour unchanged.

---

## Locked decisions (from the brief)

Restated so requirements can trace to them. Do not reopen these.

- **D1. Declaration form `type:`.** A colon-suffixed defining word, sibling to `:`.
  Slice 3 implements **only** the struct (product) form; the sum/enum `|`-variant form
  is Slice 4 and out of scope.
- **D2. Struct syntax: bare `name type` field pairs**, no parens, no field colon,
  `;`-terminated. Newlines are formatting only (whitespace-insensitive). Lowercase field
  / capitalized type is convention, not lexically enforced.
- **D3. Field pairing is positional.** The body is a flat `name type name type …`
  stream; each field is one name token then one type token. An odd token count or an
  unknown type name is a sharp located error. Every type is a single token today.
- **D4. Inline value aggregate (model B).** A struct value occupies **one** typed
  virtual-stack slot whose representation is an aggregate with a computed layout, backed
  by QBE aggregate types and frame-local `alloc`. **Not** "exploded onto the stack."
- **D5. Carried-stack marshalling generalizes to per-slot sizes.** The prologue/epilogue
  in `lower_line` (and the REPL `Session` buffer) become size-aware per slot; a struct
  slot carries its full aggregate size; scalars unchanged. **The central change and main
  risk.**
- **D6. All Slice 3 structs are Copy.** Fields are scalars or other Copy structs only, so
  every struct is trivially Copy: `dup` copies the aggregate bytes, drop is a no-op. No
  `Copy` marker syntax this slice.
- **D7. User-extensible type namespace.** Frontend `Type` gains a `Struct` variant keyed
  into a per-program struct registry; `IrType` mirrors with a struct/layout entry.
  Frontend `Type` and backend `IrType` stay **distinct** (existing invariant).
- **D8. Generated words per struct `S` with fields `f1: T1 … fn: Tn`:**
  - **Constructor** `S ( T1 … Tn -- S )`: fields consumed in declared order (first field
    deepest on the stack).
  - **Field getter** `S>fi ( S -- Ti )`: consumes the struct, projects one field, a
    single load, never copies at any size; read-without-consume is `dup S>fi`.
  - **Field setter (functional)** `S<fi ( S Ti -- S )`: returns a new struct with `fi`
    replaced; no in-place mutation.
  - **Destructure** `S> ( S -- T1 … Tn )`: explodes all fields in declared order (first
    field deepest). Inverse of the constructor.
  - `-`, `<`, `>` are identifier-continuation characters in the lexer, so `S>fi` /
    `S<fi` / `S>` are single word tokens that do not collide with the `-` operator, the
    `< > <= >= <>` comparison words, or the `>iN`/`>fN` conversion words (which match a
    whole word `>` + numeric type name).
- **D9. Nesting allowed; recursion-by-value forbidden.** A field may be a scalar or
  another struct. Nested access composes by juxtaposition (`Segment>to Vec2>x`). A struct
  that contains itself by value (directly or transitively) has infinite size and is a
  located compile error during layout, never a hang.
- **D10. Non-consuming access and references.** Reading/keeping a struct is `dup` +
  accessor; the copy is visible on purpose (the affine spine). No hidden-copy "peek"
  accessors. True borrow-without-copy is Phase 3.

### Micro-decisions (were open, resolved in the brief)

- **M1.** Construct / destructure / field order = declaration order, left to right,
  **first field deepest** on the stack.
- **M2.** `.` on a struct is a sharp error (the first reachable use of the existing
  `print_requires_printable` guard). `=` and the numeric/comparison/arithmetic operators
  stay scalar-only; applied to a struct they are a sharp located error naming the struct
  type.
- **M3.** Zero-field struct `type: Unit ;` is allowed as the unit/marker value:
  constructor `Unit ( -- Unit )`, no getters/setters, destructure is the no-op
  `Unit> ( Unit -- )`.
- **M4.** REPL residual display of a struct slot shows the **type-name placeholder**
  (e.g. `<Vec2>`), not field values. Scalar slot display is unchanged.
- **M5.** Recursion / infinite-size detection during layout is a located compile error
  that names the cycle. It must terminate (never loop) on a self-referential `type:`.

---

## Requirements

Numbered, independently verifiable, traceable to D1–D10 / M1–M5 and the diagnostics
(X1–X8). Each is testable by a unit test beside the stage and/or a golden.

### Frontend: lexer, type namespace, parser

- **R1 (D1).** The lexer stops splitting on `:`. Today `is_delimiter`
  (`src/lexer.rs:17`) matches `: ; ( ) |`, so `type:` would split into `type` + `:`.
  Drop `:` from the delimiter set (and from the delimiter match arm in `lex`) so both
  `: name` and `type: Name` tokenize as whole word tokens on the trailing whitespace.
  `Token::Colon` is removed (or retained only if still referenced); the parser now keys
  on `Token::Word(":")` and `Token::Word("type:")`. Existing `:` word definitions and
  stack effects must lex unchanged. No new literal kinds. **Verify** with lexer unit
  tests that `: sq ( … ) … ;` and `type: Vec2 x i64 ;` both tokenize as expected and
  that `5 .` still lexes as `Int(5), Word(".")`.
- **R2 (D7).** `ast::Type` (`src/ast.rs:52`, currently `Int`/`Float`/`Bool`) gains a
  `Struct` case that stays `Copy` (it identifies a registered struct by a small `Copy`
  key — a `StructId`/index into the per-program registry — **not** by an owned
  `String`, so `Type` keeps `Clone, Copy, PartialEq, Eq`). Two `Type::Struct` values are
  equal iff they name the same registered struct. Scalar `from_name`/`name`/`Display`
  are unchanged; a struct name resolves and renders through the registry (see R3, R4).
- **R3 (D2, D7).** A per-program **struct registry** maps a capitalized type name to its
  `StructId` and its ordered `(field-name, field-Type)` list (a `StructDecl` AST node),
  and back. `Module` (`src/ast.rs:11`) carries the registry / struct declarations
  alongside `words`. Type-name resolution during parsing consults the scalar table first
  (`Type::from_name`) then the registry; an unknown capitalized name is still the
  existing `error: unknown type` located diagnostic. Because a struct field or a word
  effect may reference a struct declared later or itself (forward / self reference), the
  parser registers **all** `type:` names in a pre-pass before resolving any field or
  effect type, so name resolution never depends on declaration order. Layout and
  recursion checks are deferred to the checker (they are not the parser's job).
- **R4 (D1, D2, D3).** The parser gains the `type:` production (struct/product form
  only): `type: Name (field-name field-type)* ;`. It reads the name, then a flat
  `name type` field sequence, resolving each field type against scalar table + registry,
  until `;`. A `StructDecl` records the name and the ordered `(field-name, Type)` list.
  Malformed declarations are located errors: an **odd token count** in the field body
  (a name with no following type), a field type that is itself a `type:`/`:`/`(`/`|`
  token, or a missing `;` (**X8**). The registry-name pre-pass (R3) and this production
  are the two parser changes; word-definition parsing (`parse_worddef`, `src/parser.rs:113`)
  is otherwise unchanged, and struct type names in a word's `( … )` effect resolve for
  free through the shared resolution path.

### Checker: registration, layout, recursion, generated words, diagnostics

- **R5 (D3, D7).** The checker registers each `type:` struct so its name resolves as a
  `Type`. A **duplicate type name** is a sharp located error naming the type (**X2**). A
  field of an **unknown type** is a sharp located error naming the field and the missing
  type (**X1**) (if not already caught during parsing, it is caught here; the diagnostic
  names the struct, the field, and the unknown type).
- **R6 (D9, M5).** The checker detects **value-recursion** (a struct that contains
  itself directly or transitively) as a located compile error that **names the cycle**
  (**X3**), via cycle detection over the field-type graph. Detection must **terminate**
  on any self-referential or mutually-recursive `type:` (never loop / never hang). This
  runs before layout so an infinite-size struct never reaches the size computation.
- **R7 (D8, M1, M3).** For each registered struct `S` with fields `f1: T1 … fn: Tn`, the
  checker synthesizes and registers the generated-word signatures so they participate in
  the virtual-stack type+arity check exactly like any other word:
  - constructor `S ( T1 … Tn -- S )` (inputs in declared order, first deepest);
  - getter `S>fi ( S -- Ti )` for each field;
  - setter `S<fi ( S Ti -- S )` for each field;
  - destructure `S> ( S -- T1 … Tn )` (outputs in declared order, first deepest).
  - A **zero-field** struct (M3) registers only `S ( -- S )` and `S> ( S -- )`; no
    getters/setters.
  These signatures must not collide with user word names or builtins (a user word named
  `S` or `S>fi` is out of scope to forbid this slice; if a collision is trivially
  detectable it may be reported, but this is not required).
- **R8 (D8).** Applying a generated word to the wrong operand is a sharp located error:
  a constructor with the wrong **arity or field types** (**X4**, naming the struct and
  the offending field type), and a getter / setter / destructure applied to a value that
  is **not** the owning struct type (**X5**, naming the accessor, the expected struct
  type, and the found type). These flow through the existing arity/type-mismatch
  machinery; the diagnostics name the struct and/or field.
- **R9 (M2).** `.` on a struct routes to the existing `print_requires_printable`
  guard (`src/check.rs:301`, call site `src/check.rs:563`), now **reachable** because a
  `Type::Struct` is not printable. Its diagnostic names the struct type (**X6**).
  Likewise `= < > <= >= <>` and `+ - * / mod and or xor not shl shr` stay scalar-only:
  applied to a struct they are a sharp located error naming the struct type (**X7**).
  The existing operand guards (`is_numeric` / `is_int` / `is_bool`) already reject a
  `Type::Struct`; the requirement is that the resulting diagnostic renders the struct
  name (through the registry) and that this is covered by tests, not that new guard code
  is invented.
- **R10 (D4, D9).** A struct `Type` unifies through `if/else/then` joins and moves
  through shuffles (`dup drop swap over rot`) as any `Type` does (structural, type is
  carried, no special case). Nested field access composes by juxtaposition with no lens
  machinery — it is just two getter calls in sequence, type-checked normally.

### IR + backend: layout, aggregate types, generated-word lowering

- **R11 (D7, word-width-neutral).** `ir::IrType` (`src/ir.rs:31`) gains a struct entry
  that stays `Copy` (it carries a `StructId`/index, **not** an inlined layout, so
  `IrType` keeps `Clone, Copy`). The computed **layout** (per-field byte offset, per-field
  size/align, whole-struct size and alignment) lives in a per-module **layout registry**
  derived once, keyed by `StructId`. `ir_type_of` maps `Type::Struct` to `IrType::Struct`.
  `Ptr` stays opaque; **all offsets and the struct size are computed from field
  sizes/alignments, never a hardcoded 64-bit machine word** (a `Ptr` field, if any
  existed, would not be assumed 8 bytes — none exist this slice, but the computation must
  be table-driven off field widths so a future non-64-bit backend or WASM lowering
  concretizes it). Natural alignment: each field is placed at the next offset aligned to
  its own alignment; struct alignment = max field alignment (min 1); struct size = final
  offset rounded up to struct alignment. Nested-struct fields use the inner struct's
  size/alignment. Scalar field sizes/aligns: `i8`/`u8`/`bool` = 1, `i16`/`u16` = 2,
  `i32`/`u32`/`f32` = 4, `i64`/`u64`/`f64` = 8.
- **R12 (D4).** A QBE aggregate type `type :S = { … }` is emitted per struct, its member
  list matching the field layout (member QBE types: `b` for `i8`/`u8`/`bool`, `h` for
  `i16`/`u16`, `w` for `i32`/`u32`, `s` for `f32`, `l` for `i64`/`u64`, `d` for `f64`,
  `:Inner` for a nested struct). The hand-computed layout (R11) must **agree** with QBE's
  own aggregate layout, because passing a struct by value across a word boundary relies
  on QBE's C-ABI classification of `:S`; this agreement is a load-bearing correctness
  property (see RISK 2) and is verified by running goldens, not by IL inspection.
- **R13 (D4, D8, M1).** Generated-word **lowering**:
  - **Constructor** allocates a frame slot for the aggregate (`alloc4`/`alloc8` per the
    struct alignment), stores each field at its computed offset (a width-exact store,
    R15), and yields the aggregate value (the alloc'd slot).
  - **Getter** `S>fi` emits a single width-exact **load** of field `fi` at its offset
    from the struct's storage; it never copies the aggregate.
  - **Setter** `S<fi` allocates a new aggregate, copies all bytes from the input, then
    overwrites field `fi` at its offset (functional update; no in-place mutation).
  - **Destructure** `S>` loads every field at its offset, pushing them in declared order
    (first deepest). A zero-field `S>` emits nothing.
  The IR must be able to **classify** a call name as constructor / getter / setter /
  destructure / user-word, which requires threading the struct registry (name → kind +
  `StructId` + field index + layout) into the lowering context (`FuncBuilder` /
  `lower_word` / `lower_line`). Recognizing these names and emitting the struct ops
  inline (alloc / offset / load / store) is the intended shape; emitting per-struct
  helper QBE functions and calling them normally is an acceptable alternative provided
  the getter still compiles to a single field load with no aggregate copy.
- **R14 (D6).** `dup` of a struct copies the aggregate bytes (a fresh `alloc` + byte
  copy); `drop` of a struct is a no-op (as for scalars). Correctness (a duped original
  stays intact after the copy is mutated via a functional setter) takes priority over
  register residency; small structs staying register-resident so `dup` is cheap is a
  QBE optimization relied on but not required for correctness.
- **R15 (D4, R11).** Struct **field** load/store must be **width-exact** to the field
  (`loadsb`/`loadub`/`storeb`, `loadsh`/`loaduh`/`storeh`, `loadw`/`storew`, `loadl`/
  `storel`, `loads`/`stores`, `loadd`/`stored`, or an aggregate copy for a nested-struct
  field), selected from the field's `IrType`, so a store to one field never clobbers an
  adjacent field. This is **distinct** from the 8-byte-slot carried-stack marshalling
  store/load (R16), which writes a whole 8-byte cell. Introduce the field-width load/store
  as needed (new `Instr` variants carrying a width, or a width parameter on `Load`/`Store`);
  keep the existing scalar-slot `Load`/`Store` behaviour (`src/backend/qbe.rs:518`,
  `:529`) unchanged for the marshalling path.

### Carried-stack marshalling and REPL (the central risk, D5)

- **R16 (D5).** Generalize the carried-stack prologue/epilogue in `lower_line`
  (`src/ir.rs:206`; prologue `PtrOffset` at `:225`, epilogue at `:265`) from fixed
  8-byte slots to **per-slot sizes**. A helper maps each carried slot `Type` to its
  carried byte size and computes cumulative byte offsets: a **scalar stays an 8-byte
  cell** (byte-for-byte the current behaviour, so every scalar golden is unchanged), a
  **struct occupies its aggregate size** (aligned). The prologue loads slot `i` from its
  computed byte offset (a scalar via the existing width-aware slot load + integer
  `Conv`-relabel where it already applies; a struct via an aggregate copy from the buffer
  offset into a fresh frame `alloc`, yielding the struct value for the line body). The
  epilogue stores each output slot at its computed byte offset (a struct via an aggregate
  copy back into the buffer). The returned advanced top is `top + (out_bytes - in_bytes)`,
  where `out_bytes`/`in_bytes` are the summed slot sizes, replacing the
  `(M - entry_depth) * 8` arithmetic. `lower_line` continues to return the emitted output
  **slot count** (or is extended to also return the output **byte size**) so the REPL can
  size its buffer from the same numbers the wrapper actually writes.
- **R17 (D5, M4).** The REPL `Session` (`src/repl.rs:176`) stops assuming
  `types.len() == top / 8`. The carried buffer must hold arbitrary per-slot bytes: keep
  `top` as the live **byte** length, size the buffer to `max(top, out_bytes)` bytes
  (widen `buf` to a byte-addressable backing, e.g. `Vec<u8>`, or keep `Vec<i64>` sized in
  8-byte units to at least `ceil(out_bytes/8)`), and compute each slot's byte offset from
  `Session.types` rather than `index * 8`. `eval_expr` (`src/repl.rs:262`) derives entry
  slot count/bytes from `types`, not `top / 8`.
- **R18 (M4).** `format_stack` (`src/repl.rs:156`) renders a **struct** slot as its
  type-name placeholder `<TypeName>` (looked up through the registry), reading no field
  bytes. Scalar slot display (signed/unsigned decimal, float `from_bits`, `bool`
  `true`/`false`) is **unchanged**. When a struct slot is present, offsets for later
  scalar slots must still be computed correctly from the per-slot sizes.

---

## Non-functional requirements

- **NF1 — Green at every phase.** `cargo fmt --check && cargo clippy -- -D warnings &&
  cargo test` passes at the end of each delivery phase; each phase is independently green
  and leaves structs provable so far as it goes.
- **NF2 — Invariants held.** Backend stays **QBE** (no LLVM). IR stays
  **backend-neutral**: `Ptr` stays opaque and the IR never assumes a 64-bit machine word
  — struct size and field offsets are computed from field sizes/alignments (R11). The
  `s`/`d`/`b`/`h`/`w`/`l` register/member classes are derived in the backend, never
  pushed into `IrType`. Frontend `Type` and backend `IrType` stay **distinct**. No
  in-process JIT (the REPL keeps `dlopen`-ing freshly compiled objects). `core` stays
  `no_std`.
- **NF3 — No regressions.** All Phase 0/1/Slice-1/Slice-2/floats/bitwise/bool goldens
  still pass unchanged (`tests/phase0.rs`, `tests/phase1.rs`, in-crate unit tests).
  Integer/float/`bool` behaviour is byte-for-byte unchanged; the scalar carried-stack
  marshalling emits identical IL for scalar-only lines (R16).
- **NF4 — Test coverage per convention.** Every stage that gains code (lexer, ast,
  parser, check, ir, backend, repl) gets `#[cfg(test)] mod tests` with a happy path plus
  at least one error/edge case, named `thing_condition_expected`. Every exit criterion is
  a golden (source in → expected stdout, or source in → expected diagnostic). Diagnostics
  are behaviour: each negative asserts the salient message substrings **and** the
  type/field names, not merely `is_err`.
- **NF5 — Marshalling and codegen verified by running artifacts, not IL strings.** The
  D5 marshalling and the aggregate ABI (R12, R13, R16) are covered by **running-binary
  and REPL goldens** (a struct returned from a word and used by another; a struct carried
  across a REPL line boundary; every field of a mixed-type nested struct read back), not
  only by IL-string assertions. IL-string unit tests may supplement but never substitute.
- **NF6 — No premature abstraction.** Follow the existing table + special-case shape. No
  general lens/optic machinery, no positional/tuple products, no auto-derived `=`/print,
  no new modules unless the CLAUDE.md growth-structure signals actually fire (2+ together).

---

## Observable success criteria

Map 1:1 to the brief's seven exit criteria plus the dogfood. Each is a golden (native
binary or REPL session) unless noted.

- **S1 (Exit 1).** A `type:` struct declaration parses and registers a new named type
  usable in stack effects and word bodies; integer/float/`bool` behaviour and **all**
  Phase 0/1/Slice-1/Slice-2/floats/bitwise/bool goldens still pass (NF3).
- **S2 (Exit 2).** The generated words work in a **native binary**: constructor,
  per-field getter, per-field functional setter, and destructure, for at least one flat
  struct and one nested struct.
- **S3 (Exit 3).** Field read/update are **offset-correct at runtime** for mixed field
  types (e.g. an `i64` and an `f64` field), including a nested struct field accessed by
  juxtaposition (`Segment>to Vec2>x`), read back per-field.
- **S4 (Exit 4).** `dup` + getter reads without consuming; the consuming getter copies
  nothing; a functional setter returns a correctly-updated new value while leaving a
  duped original intact.
- **S5 (Exit 5).** A struct value survives a **word-call boundary** (a struct argument
  and a struct return) and a **REPL line boundary** (size-aware carried-stack
  marshalling), verified by running binary + REPL goldens.
- **S6 (Exit 6).** Diagnostics are sharp and behavioural (assert message text **and** the
  type/field names): unknown field type (X1), duplicate type (X2), recursive struct (X3),
  constructor arity/type mismatch (X4), accessor-on-wrong-type (X5), `.`-on-struct (X6),
  `=`/arithmetic-on-struct (X7), malformed declaration / odd token count (X8).
- **S7 (Exit 7).** A **zero-field** struct works end to end (M3: `type: Unit ;`,
  `Unit`, `Unit>`); a **recursive** struct is a compile error, **not a hang** (M5).
- **S8 (dogfood).** `examples/vectors.sth`: a `Vec2 { x y : i64 }` and a nested
  `Segment { from to : Vec2 }`, with a reusable componentwise `sub ( Vec2 Vec2 -- Vec2 )`,
  `len2 ( Vec2 -- i64 )`, a `span ( Segment -- Vec2 )` = `Segment> swap sub`, and a
  `shift-x ( Vec2 i64 -- Vec2 )` functional-setter demo. `main` builds segment
  (0,0)–(3,4), computes `span len2 .` (prints `25`) and `5 6 Vec2 1 shift-x Vec2>x .`
  (prints `6`). Runs as a native binary and in the REPL. Plus the headline negatives:
  `.` on a `Vec2`, and a recursive `type:` rejected.

---

## Diagnostics (behaviour, name the types)

Each is a sharp **located** error modelled on the existing structural operator/conversion
diagnostics, and each is tested by asserting the message text **and** the type/field
names.

- **X1 — unknown field type.** `type: Bad x Nope ;` → names the struct, the field, and
  the unknown type `Nope`.
- **X2 — duplicate type name.** Two `type: Vec2 …` declarations → names `Vec2`.
- **X3 — recursive struct (infinite size).** `type: Loop next Loop ;` (or a mutual
  cycle) → names the cycle; terminates, never hangs (M5).
- **X4 — constructor arity/type mismatch.** Wrong count or wrong field type to `Vec2`
  (e.g. a `bool` where an `i64` field is expected) → names `Vec2` and the offending
  field type.
- **X5 — accessor on the wrong type.** `Vec2>x` applied to an `i64` (or a `Segment`) →
  names the accessor `Vec2>x`, the expected type `Vec2`, and the found type.
- **X6 — `.` on a struct.** `Vec2 .` → the `print_requires_printable` path, naming
  `Vec2` (M2).
- **X7 — `=` / arithmetic on a struct.** `Vec2 Vec2 =` (or `+`) → names the struct type
  and states the operator is scalar-only (M2).
- **X8 — malformed declaration.** Odd token count (`type: Bad x i64 y ;`), a missing
  `;`, or a field type that is a delimiter/defining word → a located parse error.

---

## Scope and boundaries

**In:** the `type:` struct (product) declaration form; the user-extensible type namespace
(`Type::Struct` + per-program registry); the inline-aggregate layout model (offsets /
size / alignment from field sizes, word-width-neutral) with QBE aggregate types and
frame `alloc`; generated constructor / getter / setter (functional) / destructure words;
nesting (struct-in-struct) with juxtaposition access; all structs Copy (byte-copy `dup`,
no-op drop); size-aware carried-stack marshalling across word and REPL boundaries; REPL
struct-placeholder display; the eight diagnostics X1–X8; `examples/vectors.sth`;
zero-field struct; recursion-as-compile-error.

**Out of scope (deferred, mirrors the brief — do not build):** enums / ADTs / `match` /
the `|` sum syntax (Slice 4); generalized mid-body `| … |` locals (Slice 4); fixed-size
arrays and `usize`/`isize` (Slice 5); the `Copy` marker and optional / non-null pointers
(Slice 7); references / borrows and in-place mutation (Phase 3); generics / polymorphic
signatures (Phase 4); struct-wide auto-derived `=` or printing; positional / tuple
products and index access; lenses / optics; recursive data types (need pointers, Phase
3); heap / move semantics (Phase 3).

---

## Advisory solution approach

Not binding, but the intended shape (mirrors how the scalar slices landed).

- **Frontend (R1–R4).** In `src/lexer.rs`, remove `:` from `is_delimiter` and from the
  delimiter match arm in `lex`; retire `Token::Colon` in favour of the parser matching
  `Word(":")` / `Word("type:")`. In `src/ast.rs`, add `Type::Struct(StructId)` (a small
  `Copy` index) and a `StructDecl { name, fields: Vec<(String, Type)> }`; give `Module` a
  registry (`Vec<StructDecl>` indexed by `StructId`, plus a name→id map). Add a shared
  resolver (scalar `from_name` first, then registry) used by both effect-slot and
  field-type resolution. In `src/parser.rs`, add a pre-pass that scans for every
  `type: Name` and registers the name (id only), then parse `type:` bodies and word defs
  resolving types against the registry. Keep the located `error: unknown type` message.
- **Checker (R5–R10).** In `src/check.rs`, register structs, report duplicates (X2) and
  unknown field types (X1), run cycle detection over the field-type graph for recursion
  (X3, terminating), and synthesize the generated-word `Sig`s (constructor/getter/setter/
  destructure, M1 order) into the env alongside user words. Constructor/accessor misuse
  (X4/X5) falls out of the existing arity/type-mismatch path once the `Sig`s are present.
  Ensure `print_requires_printable_error` and the operator guards render the struct name
  (X6/X7) by threading the registry into the diagnostic-formatting path (or by making
  `Type`'s `Display` registry-aware). Struct types flow through joins/shuffles with no
  special case (R10).
- **IR + backend (R11–R15).** In `src/ir.rs`, add `IrType::Struct(StructId)` and a
  per-module layout registry computed once from field sizes/aligns (R11); map it in
  `ir_type_of`. Thread the registry into `FuncBuilder` so `lower_call` classifies a name
  as constructor/getter/setter/destructure/user-word and emits `alloc` + width-exact
  field store/load (R13, R15); `dup`/`drop` handle a struct value (R14). In
  `src/backend/qbe.rs`, emit `type :S = { … }` per struct (R12), derive member types and
  field load/store widths, and keep the scalar slot `Load`/`Store` path unchanged.
- **Marshalling + REPL (R16–R18).** Generalize `lower_line`'s prologue/epilogue to
  per-slot byte offsets (scalar = 8-byte cell unchanged, struct = aggregate copy), and
  return the new top as a byte delta. In `src/repl.rs`, make the `Session` buffer
  byte-addressable and compute slot offsets from `Session.types`; render a struct slot as
  `<TypeName>` in `format_stack`.
- **Dogfood + goldens (S1–S8).** Add `examples/vectors.sth`; add native goldens (flat +
  nested construct/get/set/destructure; mixed `i64`/`f64` field read-back; nested
  juxtaposition; struct arg + struct return across a word call; zero-field struct;
  dup-original-intact after setter) to `tests/phase0.rs` and a REPL carried-struct
  session to `tests/phase1.rs`, following the existing `run_and_capture_stdout` /
  `run_session` helpers; add the eight negative diagnostic goldens; confirm all prior
  goldens stay green.

---

## Codebase map (anchored, verified against `main`)

Paths and line numbers confirmed by reading the current source.

- **`src/lexer.rs`** — `Token` enum at `lexer.rs:6` (`Colon`, `Semicolon`, `Word`, …);
  `is_delimiter` at `lexer.rs:17` (matches `: ; ( ) |`); the delimiter match arm in `lex`
  and the word-accumulation break at `lexer.rs:91`. → **R1**: drop `:` from the delimiter
  set / arm; retire `Token::Colon`.
- **`src/ast.rs`** — `Module { words }` at `ast.rs:11`; `Type` enum at `ast.rs:52`;
  `IntType` (private `bits`/`signed`) at `ast.rs:63`; `INT_TYPES` at `ast.rs:70`;
  `FloatType`/`FLOAT_TYPES`; `Type::from_name` at `ast.rs:112`, `name` at `ast.rs:150`,
  `Display` after; `TermKind` at `ast.rs:192`. → **R2/R3**: add `Type::Struct(StructId)`,
  `StructDecl`, the registry on `Module`, and the shared resolver (lowest common ancestor
  of parser/checker/ir).
- **`src/parser.rs`** — `parse` at `parser.rs:15`; `parse_worddef` at `parser.rs:113`;
  `parse_slot` at `parser.rs:149`; `resolve_type` at `parser.rs:180` (uses
  `Type::from_name`, emits `error: unknown type`); `parse_term` at `parser.rs:235`. →
  **R3/R4**: registry pre-pass + the `type:` production; resolve field/effect types
  against scalar table + registry.
- **`src/check.rs`** — `Sig` at `check.rs:18`; `sig_of` at `check.rs:24`; `builtin_table`
  at `check.rs:37`; `check` at `check.rs:61`; `operand_pair_mismatch_error` at
  `check.rs:199`; `print_requires_printable_error` at `check.rs:301` (call site
  `check.rs:563`); `check_operator` at `check.rs:463`. → **R5–R10**: registration,
  duplicate/unknown-field (X1/X2), recursion (X3), generated-word `Sig`s, X4–X7
  diagnostics with struct names.
- **`src/ir.rs`** — `IrType` (`Int`/`Float`/`Bool`/`Ptr`) at `ir.rs:31`, `Ptr` at
  `ir.rs:51`; `ir_type_of` at `ir.rs:63`; `Instr` at `ir.rs:88` (`PtrOffset` at
  `ir.rs:104`, `Load`/`Store`); `BinOp` at `ir.rs:117`; `Arity` at `ir.rs:158`; `lower`
  at `ir.rs:165`; `lower_line` at `ir.rs:206` (prologue `PtrOffset` at `ir.rs:225`,
  epilogue at `ir.rs:265`, `(M - entry_depth) * 8` top arithmetic after); `lower_word` at
  `ir.rs:290`; `FuncBuilder`/`lower_call` below. → **R11–R16**: `IrType::Struct` + layout
  registry, generated-word lowering, width-exact field load/store, per-slot marshalling.
- **`src/backend/qbe.rs`** — `emit` (data format strings) at `qbe.rs:11`; `width` at
  `qbe.rs:56`; `emit_conv` at `qbe.rs:118`; `emit_instr` at `qbe.rs:287`; `Print` at
  `qbe.rs:464`; `PtrOffset` at `qbe.rs:515`; `Load` (`loadl`/`loads`/`loadd`) at
  `qbe.rs:518`; `Store` (`storel`/`stores`/`stored`, `w`→`l` widening) at `qbe.rs:529`. →
  **R12/R15**: emit `type :S = { … }`, `alloc`, width-exact field load/store; keep the
  scalar slot path unchanged.
- **`src/repl.rs`** — `format_stack` at `repl.rs:156`; `Session` at `repl.rs:176` (`buf:
  Vec<i64>` at `:178`, `top: usize` at `:179`, `types: Vec<Type>` at `:182`); `eval_expr`
  carried-stack marshalling at `repl.rs:262`. → **R17/R18**: per-slot-size buffer +
  offsets; struct-placeholder display.
- **`src/driver.rs`** — `build`/`compile_so` pipeline wiring; no change beyond what the
  backend needs (the struct print error is a compile error, not a runtime path).
- **`examples/`** — `gcd`, `factorial`, `lerp`, `sign`, `bool_abi`, `rgb`, `rgb_bits`,
  `mean`, `leap` (unchanged). → add `examples/vectors.sth`.
- **`tests/phase0.rs`**, **`tests/phase1.rs`** — existing goldens (unchanged, stay green,
  NF3). → add struct native goldens to `phase0.rs` and a REPL carried-struct session to
  `phase1.rs`, following the existing helpers.

---

## Open questions and risks

Led by the genuinely hard/uncertain areas.

- **RISK 1 — carried-stack marshalling generalizing to per-slot sizes (D5, R16, R17).**
  The load-bearing change and the main risk. The spill region must become size-aware:
  a scalar stays a byte-identical 8-byte cell (so every scalar golden is unchanged), a
  struct carries its full aggregate size, cumulative offsets replace `index * 8`, and the
  returned top becomes a byte delta. The REPL `Session` must drop the `types.len() ==
  top / 8` assumption and offset from `types`. **Mitigation (NF5):** a running-binary
  golden (a struct returned from one word and consumed by another) and a REPL
  carried-struct golden that uses the struct on the next line, plus an assertion that
  scalar-only lines emit unchanged IL / unchanged REPL output. Do not accept IL-string
  checks alone here.
- **RISK 2 — hand-computed layout must agree with QBE's aggregate ABI (R11, R12).**
  Field offsets/size/alignment are computed in the frontend (word-width-neutral), but
  passing a struct by value across a word boundary relies on QBE's own `:S` C-ABI
  classification. If the two layouts disagree, a struct built by manual stores is read
  wrong when passed by value. **Mitigation:** compute standard natural-alignment C
  layout matching the chosen member types; verify at **runtime** with a struct-argument
  and struct-return golden and a per-field read-back of a mixed-width nested struct (S3,
  S5), not by reading IL.
- **RISK 3 — width-exact field load/store vs 8-byte slot load/store (R15 vs R16).** A
  struct field store must write **exactly** the field width or it clobbers an adjacent
  field; the existing marshalling `Store` widens a sub-word value to `l` and writes 8
  bytes, which is correct for an 8-byte cell but catastrophic for a packed field. These
  are two distinct code paths and must not be conflated. **Mitigation:** per-field
  read-back golden over a struct with adjacent sub-word fields (e.g. two `i8`s then an
  `i64`), asserting no field corrupts its neighbour.
- **RISK 4 — recursion detection must terminate (R6, M5).** A self- or mutually-recursive
  `type:` must be reported, never loop. **Mitigation:** cycle detection with a visited
  set over the field-type graph; a unit test and a golden for a directly recursive and a
  mutually recursive `type:` asserting a located error (not a timeout).
- **Q1 — how `Type::Struct` carries identity while staying `Copy`, and how the struct
  name renders in `Display`/diagnostics.** `Type` is `Copy` and `Display` currently
  returns `&'static str`; a struct name is not static and lives in the registry.
  **Resolution (advisory):** `Type::Struct(StructId)` carries a small `Copy` index;
  diagnostics that must render a struct name thread the registry into the formatting path
  (or `Display` is made registry-aware / falls back to `<struct#id>` when no registry is
  in scope). Interning the name as `&'static str` via a one-time leak at registration is
  a simpler alternative that keeps `Display` trivial but is less clean; decide and justify
  in the phase that lands R2/R5. `IrType::Struct` similarly carries an id, with the layout
  in a side registry (R11), so `IrType` stays `Copy`.
- **Q2 — zero-field struct layout and ABI (M3).** `type: Unit ;` has size 0. A QBE empty
  aggregate `type :Unit = { }` and passing/returning a zero-size value by value is an
  edge QBE may handle awkwardly. **Resolution (advisory):** size 0, align 1; the
  constructor may `alloc` a minimal slot or yield a placeholder handle, and the
  destructure emits nothing. Confirm the S7 zero-field golden actually builds and runs
  (native + a `Unit` crossing a boundary if feasible); if QBE rejects an empty aggregate,
  represent `Unit` with a 1-byte member and document it. This is an edge, not the core.
- **Q3 — `bool` field width in an aggregate.** `bool` is a `w` in registers but a 1-byte
  `_Bool` in C layout. **Resolution (advisory):** store a `bool` field as a 1-byte member
  (`b`, align 1, value 0/1), consistent with natural C layout, and load it back
  zero-extended. Confirm a struct with a `bool` field round-trips in a golden if one is
  written; the dogfood uses only `i64`, so this is an extra edge, not an exit criterion.
- **Q4 — dispatching generated words in the IR (R13).** The IR must classify a call name
  as a generated word, which couples lowering to the struct registry. **Resolution
  (advisory):** thread the registry (name → kind + `StructId` + field index) into
  `FuncBuilder`; inline the struct ops. Emitting per-struct helper functions is an
  acceptable alternative if the getter still lowers to a single field load with no copy.
- **Q5 — dogfood field syntax (`Vec2 { x y : i64 }`).** The brief's prose uses a
  shorthand `{ x y : i64 }`, but the **locked** surface syntax (D2) is bare `name type`
  pairs with **no** shared-type grouping and **no** colon: `type: Vec2 x i64 y i64 ;`.
  **Resolution:** `examples/vectors.sth` uses the locked D2 syntax
  (`type: Vec2 x i64 y i64 ;`, `type: Segment from Vec2 to Vec2 ;`); the brief's `{ … }`
  is descriptive shorthand, not grammar.

---

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Frontend: lexer `:` de-delimiting, type namespace, and the `type:` production (R1-R4). In src/lexer.rs drop `:` from `is_delimiter` and the delimiter match arm so `:` and `type:` lex as whole word tokens; retire `Token::Colon` (parser keys on `Word(\":\")`/`Word(\"type:\")`); verify existing `: name ( ... ) ;` defs and stack effects lex unchanged and `5 .` still lexes as Int + Word(\".\"). In src/ast.rs add `Type::Struct(StructId)` as a small Copy index (Type keeps Clone/Copy/PartialEq/Eq), a `StructDecl { name, fields: Vec<(String, Type)> }`, and a per-program registry on Module (id<->name, ordered fields), plus a shared resolver (scalar `from_name` first, then registry). In src/parser.rs add a pre-pass registering every `type: Name` (id only), then the `type:` production (name, flat `name type` field sequence, `;`) resolving field and effect types against the registry; forward/self references resolve, layout/recursion deferred to the checker. Malformed decls (odd token count, missing `;`, delimiter-as-field-type) are located errors (X8); unknown capitalized type keeps `error: unknown type`. Unit tests per stage (happy + error), asserting message text and names. Green.",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "Checker: registration, recursion, generated-word signatures, and struct diagnostics (R5-R10). Register each struct so its name resolves as a Type; report a duplicate type name (X2) and an unknown field type (X1) naming the struct/field/type. Detect value-recursion (direct and transitive) via cycle detection over the field-type graph as a located error that names the cycle and TERMINATES, never hangs (X3, M5). Synthesize and register generated-word Sigs in declaration order, first field deepest (M1): constructor `S ( T1..Tn -- S )`, getter `S>fi ( S -- Ti )`, setter `S<fi ( S Ti -- S )`, destructure `S> ( S -- T1..Tn )`; a zero-field struct registers only `S ( -- S )` and `S> ( S -- )` (M3). Constructor arity/type mismatch (X4) and accessor-on-wrong-type (X5) fall out of the existing arity/type-mismatch path. Make `.`-on-struct reach `print_requires_printable` (X6) and `=`/arithmetic-on-struct a sharp error (X7), both rendering the struct name through the registry. Struct types unify through if/else/then joins and move through shuffles with no special case (R10). Unit tests asserting message text AND type/field names. Green.",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 3,
      "focus": "IR layout + aggregate codegen for generated words (R11-R15). Add `IrType::Struct(StructId)` (keeps Copy) and a per-module layout registry computed ONCE from field sizes/alignments, word-width-neutral: natural alignment, per-field offset, struct size/align from field widths (i8/u8/bool=1, i16/u16=2, i32/u32/f32=4, i64/u64/f64=8, nested = inner size/align), never a hardcoded machine word (NF2). Map it in ir_type_of. Emit `type :S = { ... }` per struct in qbe.rs with member types matching the layout; the hand-computed layout must agree with QBE's aggregate ABI (RISK 2). Thread the struct registry into FuncBuilder so lower_call classifies a name as constructor/getter/setter/destructure/user-word and emits: constructor = alloc + width-exact field stores at offsets yielding the aggregate; getter = single width-exact field load, no copy; setter = alloc new + copy all bytes + overwrite one field (functional); destructure = load every field in declared order. Add width-exact field load/store (loadsb/loadub/storeb, loadsh/loaduh/storeh, loadw/storew, etc. by field IrType) DISTINCT from the 8-byte slot load/store so a field store never clobbers a neighbour (R15, RISK 3). `dup` copies the aggregate bytes, `drop` is a no-op (R14). Handle the zero-field edge (Q2). Unit tests for layout offsets/size/align and generated-word lowering, plus running-binary goldens: flat + nested construct/get/set/destructure, mixed i64/f64 per-field read-back, nested juxtaposition, dup-original-intact after setter, a struct argument and a struct return across a word call (NF5, RISK 2/3). Green.",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "Size-aware carried-stack marshalling and REPL (R16-R18, D5, RISK 1). Generalize lower_line's prologue/epilogue from fixed 8-byte slots to per-slot byte sizes: a helper maps each carried slot Type to its carried byte size (scalar stays a byte-identical 8-byte cell so scalar goldens are unchanged; a struct occupies its aligned aggregate size) and cumulative byte offsets; the prologue loads each slot from its byte offset (struct = aggregate copy from the buffer into a fresh frame alloc), the epilogue stores each output slot at its byte offset (struct = aggregate copy back), and the returned top becomes `top + (out_bytes - in_bytes)` replacing `(M - entry_depth) * 8`. In src/repl.rs drop the `types.len() == top/8` assumption: make the Session buffer byte-addressable, size it from the wrapper's emitted output bytes, and compute slot offsets from Session.types in eval_expr. Render a struct residual slot as `<TypeName>` in format_stack (M4), scalar display unchanged, with later scalar-slot offsets still correct past a struct slot. Unit tests: per-slot offset helper; a scalar-only line emits unchanged marshalling IL/output (NF3). REPL golden: a struct carried across a line boundary and used + displayed on the next line (NF5). Green.",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 5,
      "focus": "Dogfood + full golden suite (S1-S8). Add examples/vectors.sth using the LOCKED D2 syntax (`type: Vec2 x i64 y i64 ;`, `type: Segment from Vec2 to Vec2 ;`; the brief's `{ x y : i64 }` is shorthand, Q5), with `sub ( Vec2 Vec2 -- Vec2 )`, `len2 ( Vec2 -- i64 )`, `span ( Segment -- Vec2 )` = `Segment> swap sub`, and `shift-x ( Vec2 i64 -- Vec2 )`; main builds segment (0,0)-(3,4), prints `span len2 .` = 25 and `5 6 Vec2 1 shift-x Vec2>x .` = 6; runs native and in the REPL. Add native goldens to tests/phase0.rs (flat + nested generated words, mixed i64/f64 per-field read-back, nested juxtaposition, struct arg + struct return across a word call, zero-field struct M3, dup-original-intact) and a REPL carried-struct session to tests/phase1.rs, following run_and_capture_stdout / run_session. Add the eight negative diagnostic goldens X1-X8 asserting message text AND type/field names, with `.`-on-Vec2 (X6) and a recursive `type:` rejected (X3, must not hang) as the headline negatives. Confirm ALL prior goldens stay green (NF3). Update ROADMAP.md to mark Slice 3 done. Green.",
      "effort": "M",
      "difficulty": "standard"
    }
  ]
}
```
