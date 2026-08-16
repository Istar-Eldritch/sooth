# Phase 2 Slice 3 brief: structs / records (aggregate value types)

Input for the spec-writer. Slice 3 adds the first aggregate value type to Sooth's
typed core: user-declared structs. It introduces a user-extensible type namespace, an
inline-aggregate memory model (the layout machinery the rest of Phase 2 builds on), a
new `type:` declaration form, and generated construction / access / update words. Still
heap-free. Enums, `match`, and the `|` sum syntax are Slice 4; generics are Phase 4.

Built on the completed scalar core (integers `i8`..`u64`, `f32`/`f64`, `bool`; arithmetic,
bitwise, comparison, conversions; type-directed `.` print), the lexer/parser/checker/IR/
QBE-emit/driver/REPL pipeline, and the `| ... |` locals and `if/else/then` from Phases 0/1.

## Where this sits

- Slice 1 (typed spine) and Slice 2 (integer tower) are done; floats/bitwise/bool axes
  are done. Scalar core is complete.
- Slice 3 is the Phase-2-opening aggregate slice: the inline-aggregate layout model it
  introduces is reused by Slice 4 (enums/`match`), Slice 5 (arrays), and Slice 6 (the VM
  dogfood). Getting the representation right here is the point.
- Phase 3 (heap, affine resources, references) builds on the Copy-vs-affine distinction
  that Slice 3 leaves implicit (all structs Copy this slice) and Slice 7 makes explicit.

## Decisions locked

- **D1. Declaration form `type:`.** A colon-suffixed defining word, sibling to `:` (word
  definition). Slice 3 implements only the struct (product) form. The sum/enum form (`|`
  variants) is declared in the grammar's intent but is Slice 4 territory and out of scope
  here.
- **D2. Struct syntax: bare `name type` field pairs, no parens, no field colon.** Fields
  are whitespace-delimited `name type` pairs; newlines are formatting only (Sooth stays
  whitespace-insensitive, newline == space). `;` terminates. Example:

  ```
  type: Vec2
    x i64
    y i64 ;
  ```

  is identical to `type: Vec2 x i64 y i64 ;`. Field names are lowercase by convention;
  type names are capitalized by convention. Convention is not lexically enforced.

- **D3. Field pairing is positional.** Within a struct body the token stream is a flat
  sequence of `name type name type ...`; each field is one name token followed by one
  type token. An odd token count, or an unknown type name, is a sharp located error. This
  works because every type is a single token today (`i64`, `f64`, `Vec2`); when generics
  arrive in Phase 4 with bracketed `Optional<i32>` the type stays a single parser unit, so
  the pairing rule survives.

- **D4. Inline value aggregate (model B).** A struct value occupies one typed virtual-
  stack slot whose representation is an aggregate with a computed layout (field offsets,
  size, alignment). Backed by QBE aggregate types and frame-local `alloc` (heap-free). This
  is deliberately not an "exploded onto the stack" model: enums (Slice 4) need a single
  tagged value and arrays (Slice 5) need indexable memory, so the aggregate/layout
  mechanism is built once here and reused.

- **D5. Carried-stack marshalling generalizes to per-slot sizes.** The word-boundary /
  REPL carried-stack spill currently uses fixed 8-byte slots. A struct slot needs its full
  aggregate size. The marshalling (prologue/epilogue in `lower_line`) becomes size-aware
  per slot. This is the central implementation change and the main risk.

- **D6. All Slice 3 structs are Copy.** Fields are scalars (or other Copy structs) only, so
  every struct is trivially Copy: `dup` copies the aggregate bytes, drop is a no-op. No
  `Copy` marker syntax this slice (that is Slice 7). The affine / non-Copy case does not
  exist yet because no resource-owning field type exists.

- **D7. User-extensible type namespace.** `Type` gains a `Struct` variant keyed into a
  per-program struct registry; `Type::from_name` consults the registry for capitalized
  names. `IrType` mirrors with a struct/layout entry. Frontend `Type` and backend `IrType`
  stay distinct (the existing invariant).

- **D8. Generated words per struct `S` with fields `f1: T1 ... fn: Tn`.**
  - **Constructor** = the capitalized type name: `S ( T1 ... Tn -- S )`. Fields consumed in
    declared order (first field deepest on the stack). This is the same rule as an enum
    variant being its own constructor (Slice 4), so "record is a one-variant enum" shows up
    in the surface for free.
  - **Field getter** `S>fi ( S -- Ti )`. Consumes the struct, projects one field. This is a
    single field load and never copies at any struct size; to read without consuming, write
    `dup S>fi` (free for Copy).
  - **Field setter (functional)** `S<fi ( S Ti -- S )`. Returns a new struct with `fi`
    replaced. No in-place mutation. When the input is dead the optimizer can reuse its
    storage in place.
  - **Destructure** `S> ( S -- T1 ... Tn )`. Explodes all fields onto the stack in declared
    order (first field deepest). Inverse of the constructor.
  - Naming rationale: `>` reads as data flowing out of the struct (getter/destructure), `<`
    as data flowing into it (setter); both stay `S`-prefixed so a type's whole accessor
    surface groups under `S`. `-`, `<`, `>` are identifier-continuation characters in the
    lexer, so these are single word tokens and do not collide with the `-` operator, the
    `< > <= >= <>` comparison words, or the `>iN` conversion words (which match a whole word
    `>` + numeric type; `S>fi` / `fi<S` are neither).

- **D9. Nesting allowed; recursion-by-value forbidden.** A struct field may be any sized
  value type: a scalar or another struct. Nested access composes by juxtaposition
  (`Segment>to Vec2>x`), which is the concatenative form of lens composition, no lens
  abstraction needed. A struct that contains itself by value (directly or transitively) has
  infinite size and is a located compile error during layout, never a hang. Recursive data
  (lists, trees) needs a pointer through the recursion, which is Slice 7 / Phase 3.

- **D10. Non-consuming access and references.** Reading/keeping a struct is `dup` +
  accessor; the copy is visible on purpose (the affine spine: `dup` is the explicit copy).
  No hidden-copy "peek" accessors are generated. True borrow-without-copy of a large struct,
  and mutation through a borrow, need second-class references, which are Phase 3. For value
  types functional update is the equivalent, and the consuming getter already copies
  nothing.

## Micro-decisions locked (were open, resolved here)

- **M1. Construct / destructure / field order** = declaration order, left to right, first
  field deepest on the stack.
- **M2. `.` on a struct is a sharp error.** This is the first reachable use of the existing
  `print_requires_printable` guard in `check.rs` (until now unreachable because every scalar
  type is printable). Slice 3 makes it reachable and testable. Likewise `=` and the other
  numeric/comparison operators stay scalar-only; applying them to a struct is a sharp located
  error naming the struct type.
- **M3. Zero-field struct `type: Unit ;` is allowed** as the unit / marker value. Its
  constructor is `Unit ( -- Unit )`, it has no getters/setters, and its destructure is a
  no-op `Unit> ( Unit -- )`.
- **M4. REPL residual display of a struct slot** shows the type name placeholder (e.g.
  `<Vec2>`), not field values, since struct printing is out of scope. Scalar slot display is
  unchanged.
- **M5. Recursion / infinite-size detection** during layout is a located compile error that
  names the cycle.

## Frontend

- **Lexer.** `:` is currently a token-splitting delimiter (`is_delimiter` in `src/lexer.rs`
  matches `: ; ( ) |`), so `type:` would split into `type` + `:`. Adjust so the defining
  words `:` and `type:` lex as whole word tokens (the cleanest fix is to drop `:` from the
  delimiter set; nothing else in the surface relies on `:` splitting now that fields carry
  no colon, and both `: name` and `type: Name` still tokenize with the trailing whitespace).
  Verify existing `:` word definitions and stack effects still lex unchanged. No new literal
  kinds.
- **AST / type namespace.** `Type` (in `src/ast.rs`, currently `Int`/`Float`/`Bool`) gains a
  `Struct` variant referencing a registered struct by id/name. `from_name`/`name`/`Display`
  consult a per-program struct registry populated from `type:` declarations. A module now
  carries struct declarations alongside word definitions. A `StructDecl` AST node captures
  the name and ordered `(field-name, Type)` list.
- **Parser.** Add the `type:` declaration production (struct/product form only): name, then a
  flat `name type` field sequence, then `;`. Reject odd token counts and malformed fields
  with located errors. The parser must resolve field type names (including forward and
  self references, so layout/recursion checks happen in the checker, not the parser).

## Checker

- **Registration + resolution.** Register each `type:` struct so its name resolves as a
  `Type`; a duplicate type name, or a field of an unknown type, is a sharp located error.
- **Layout / recursion.** Detect direct and transitive value-recursion (infinite size) as a
  located error (M5).
- **Generated word signatures.** Constructor, per-field getter, per-field setter, and
  destructure get the effects in D8; they participate in the virtual-stack type+arity check
  exactly like any other word. Struct values unify through `if/else/then` joins as any Type
  does.
- **Diagnostics (behaviour, name the types).** Model on the existing structural operator/
  conversion diagnostics. At least: unknown field type; duplicate type name; recursive
  struct; wrong arity/type to a constructor; getter/setter/destructure applied to the wrong
  type; `.`/`=`/arithmetic applied to a struct (M2). Each names the struct and/or field and
  the rule.

## IR + backend

- **IrType + layout.** `IrType` (in `src/ir.rs`) gains a struct/aggregate entry with the
  computed layout (offsets/size/align), derived once. `ir_type_of` maps a frontend
  `Type::Struct` to it. `Ptr` stays opaque; the IR never assumes a 64-bit machine word
  (word-width-neutrality invariant): field offsets and struct size are computed from field
  sizes/alignments, not hardcoded.
- **QBE aggregate types.** Emit a QBE `type :S = { ... }` aggregate per struct. Construction
  allocates a frame slot (`alloc`), stores each field at its offset, yields the aggregate
  value. A getter loads one field at its offset. A setter copies the aggregate and overwrites
  one field (functional update). Destructure loads every field. Use QBE's aggregate/C-ABI
  classification for passing structs to/from words rather than hand-rolling calling
  conventions.
- **Marshalling (D5, the risk).** Generalize the carried-stack prologue/epilogue in
  `lower_line` from fixed 8-byte slots to per-slot sizes so a struct slot carries its whole
  aggregate across a word or REPL line boundary. Scalars keep their current behaviour.
- **`dup`/drop for structs.** `dup` copies the aggregate (Copy); drop is a no-op. Small
  structs should stay register-resident where QBE allows so `dup` is free; the consuming
  getter emits a single field load with no copy.

## Print + REPL

- `.` on a struct routes to the `print_requires_printable` error path (M2), now reachable.
- REPL residual display renders a struct slot as its type-name placeholder (M4); scalar
  display unchanged.

## Out of scope (deferred)

Enums / ADTs / `match` / the `|` sum syntax (Slice 4); generalized mid-body `| ... |` locals
(Slice 4); fixed-size arrays and `usize`/`isize` (Slice 5); the `Copy` marker and optional /
non-null pointers (Slice 7); references / borrows and in-place mutation (Phase 3); generics /
polymorphic signatures (Phase 4); struct-wide auto-derived `=` or printing; positional /
tuple products and index access; lenses. Recursive data types (need pointers).

## Goal and exit criteria

Deliver user-declared struct value types: declaration, construction, field read, functional
field update, destructure, and nesting, width- and offset-correct in emitted QBE, heap-free,
with sharp located diagnostics. All prior goldens stay green; scalar behaviour is unchanged.

**Exit:**

1. A `type:` struct declaration parses and registers a new named type usable in stack
   effects and word bodies; integer/float/bool behaviour and all Phase 0/1/Slice-1/Slice-2/
   floats/bitwise/bool goldens still pass.
2. The generated words work in a native binary: constructor, per-field getter, per-field
   functional setter, destructure, for at least one flat struct and one nested struct.
3. Field read/update are offset-correct at runtime for mixed field types (e.g. an `i64` and
   an `f64` field), including a nested struct field accessed by juxtaposition.
4. `dup` + getter reads without consuming; the consuming getter copies nothing; a functional
   setter returns a correctly-updated new value while leaving a duped original intact.
5. A struct value survives a word-call boundary and a REPL line boundary (size-aware
   carried-stack marshalling).
6. Diagnostics are sharp and behavioural (assert message text and the type/field names):
   unknown field type, duplicate type, recursive struct, constructor arity/type mismatch,
   accessor-on-wrong-type, and `.`/`=`/arithmetic-on-struct.
7. Zero-field struct works (M3); recursive struct is a compile error not a hang (M5).

## Dogfood (S8-style)

`examples/vectors.sth`: a `Vec2 { x y : i64 }` and a nested `Segment { from to : Vec2 }`,
with a reusable componentwise `sub ( Vec2 Vec2 -- Vec2 )`, `len2 ( Vec2 -- i64 )`, a
`span ( Segment -- Vec2 )` = `Segment> swap sub`, and a `shift-x ( Vec2 i64 -- Vec2 )`
functional setter demo. `main` builds segment (0,0)-(3,4), computes `span len2 .` (prints
`25`, the 3-4-5 triangle) and `5 6 Vec2 1 shift-x Vec2>x .` (prints `6`). Native + REPL. Plus
the headline negatives: `.` on a `Vec2`, and a recursive `type:` rejected.

## Current-state codebase anchors (post scalar core, on `main`)

- `src/lexer.rs`: `is_delimiter` matches `: ; ( ) |`; float/int literal lexing; `Token::Word`
  fallthrough. The `:` delimiter change lands here.
- `src/ast.rs`: `Type` (`Int { bits, signed }` / `Float { bits }` / `Bool`) via `INT_TYPES`/
  `FLOAT_TYPES` tables, `from_name`/`name`/`Display`; `StackEffect`; `TermKind`
  (`IntLit`/`FloatLit`/`BoolLit`/`Call`/`If`); word def carries optional `locals: Vec<String>`.
  The `Type::Struct` variant, the struct registry, and a `StructDecl` node land here (lowest
  common ancestor of parser/checker/ir).
- `src/parser.rs`: grammar `worddef := ':' Word '(' effect ')' locals? term* ';'`;
  `locals := '|' Word* '|'` (once, after the effect). Add the `type:` production.
- `src/check.rs`: `check_operator` structural operator/conversion rules; the builtin table is
  empty (`.` and friends are special-cased); located diagnostics that name both types;
  `print_requires_printable` guard (currently unreachable). Struct registration, layout/
  recursion checks, generated-word signatures, and new diagnostics land here.
- `src/ir.rs`: `IrType` (`Int`/`Float`/`Bool`/`Ptr`); `ir_type_of`; `lower_line` carried-slot
  marshalling (fixed 8-byte slots, the D5 change); `BinOp`/`CmpOp`; `Instr` (incl. `Conv`,
  `PtrOffset`, width-aware `Load`/`Store`). Struct `IrType`/layout, construction/access/
  setter/destructure lowering land here.
- `src/backend/qbe.rs`: `width()` derives `w`/`l`/`s`/`d`; `emit_conv`; `emit_canonicalize`;
  print codegen (`$fmt`/`$ufmt`/`$ffmt`/`$sfmt`/`$boolstrs`). QBE aggregate type emission,
  `alloc`, field load/store land here.
- `src/repl.rs`: residual-stack display (type-aware). Struct type-name placeholder lands here.
- `examples/`: `gcd`, `factorial`, `lerp`, `sign`, `bool_abi`, `rgb`, `rgb_bits`, `mean`,
  `leap`. Add `vectors.sth`.

## Key risks

- **Carried-stack marshalling (D5)** is the load-bearing change: per-slot sizing of the
  spill region, correct for structs crossing word/REPL boundaries, with scalars unchanged.
  Cover with a running-binary golden (a struct returned from a word and used by another) and
  a REPL carried-struct golden, not just IL string checks.
- **Layout correctness**: field offsets/size/alignment for mixed-width fields and nested
  structs. Per-field runtime goldens (read back every field of a mixed-type nested struct)
  guard against offset bugs.
- **QBE aggregate ABI**: relying on QBE's struct classification for passing structs to/from
  words; verify at runtime (a struct argument and a struct return), not just by reading IL.
- **Recursion detection** must terminate and report, never loop, on a self-referential
  `type:`.
