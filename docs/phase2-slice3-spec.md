# Phase 2 Slice 3 — structs / records (aggregate value types)

**Status: complete** (commit `6e547be`). Slice 3 of Phase 2 (typed core), built on the scalar core on `main`.

## What shipped

User-declared struct value types: a `type:` declaration form, a user-extensible type namespace, an inline-aggregate memory model (offsets/size/alignment computed from field widths, word-width-neutral), and generated construct/get/set/destructure words. Heap-free, offset-correct in emitted QBE, with sharp located diagnostics. This is the first aggregate slice; the layout mechanism is built once here for reuse by Slices 4–6.

## Why it's a real slice, not a table-fill

A struct breaks four scalar-core assumptions at once:
1. **Extensible type namespace** — types no longer a closed built-in set; resolution consults a per-program registry.
2. **Aggregate memory model** — a struct isn't register-shaped; it has fields at byte offsets, a size, an alignment.
3. **Size-aware carried-stack marshalling (central risk, D5)** — fixed 8-byte spill slots must become per-slot-sized, scalars byte-for-byte unchanged.
4. **Generated words** — constructor/getter/setter/destructure that lower to alloc/store/load, not normal calls.

## Locked decisions (do not reopen)

- **D1.** `type:` defining word, sibling to `:`. Struct (product) form only; sum/enum is Slice 4.
- **D2.** Bare `name type` field pairs, no parens/colon, `;`-terminated, whitespace-insensitive.
- **D3.** Positional field pairing (flat `name type …` stream). Odd token count / unknown type = sharp error.
- **D4.** Inline value aggregate (model B): one typed virtual-stack slot, QBE aggregate + frame `alloc`. Not exploded onto stack.
- **D5.** Carried-stack marshalling generalizes to per-slot sizes; struct carries full aggregate size, scalars unchanged. Main risk.
- **D6.** All Slice 3 structs are Copy: `dup` byte-copies, drop is a no-op. No `Copy` marker syntax.
- **D7.** `Type` gains `Struct` variant (small `Copy` `StructId` key, not owned `String`); `IrType` mirrors. Frontend `Type` and backend `IrType` stay distinct.
- **D8.** Generated words per struct `S` (fields `f1:T1 … fn:Tn`): constructor `S ( T1…Tn -- S )`; getter `S>fi ( S -- Ti )` (single load, no copy); setter `S<fi ( S Ti -- S )` (functional); destructure `S> ( S -- T1…Tn )`. `-`, `<`, `>` are identifier-continuation chars so `S>fi`/`S<fi`/`S>` are single tokens.
- **D9.** Nesting allowed (juxtaposition access `Segment>to Vec2>x`); recursion-by-value forbidden (located compile error, must terminate).
- **D10.** Non-consuming access = `dup` + accessor; copy is visible on purpose. No hidden-copy peek; true borrow is Phase 3.
- **M1** order = declaration, first field deepest. **M2** `.`/`=`/arithmetic on struct = sharp error naming type. **M3** zero-field `type: Unit ;` allowed (constructor + no-op destructure only). **M4** REPL shows `<TypeName>` placeholder. **M5** recursion detection terminates.

## Requirements (by stage)

**Frontend (R1–R4):** lexer stops splitting on `:` (dropped from `is_delimiter`; parser keys on `Word(":")`/`Word("type:")`; `Token::Colon` retired). `ast::Type` gains `Struct(StructId)`; per-program registry (name↔id, ordered `(name, Type)` fields) on `Module`; shared resolver (scalar `from_name` first, then registry). Parser: pre-pass registers all `type:` names before resolving any field/effect type (forward/self refs work), then the `type:` production. Malformed decls located errors (X8).

**Checker (R5–R10):** register structs; duplicate name (X2), unknown field type (X1). Value-recursion via cycle detection over field-type graph, names cycle, terminates (X3). Synthesize generated-word `Sig`s (M1 order; zero-field = constructor + destructure only). Constructor/accessor misuse (X4/X5) falls out of existing arity/type-mismatch path. `.`-on-struct reaches `print_requires_printable` (X6); `=`/arithmetic scalar-only (X7); both render struct name via registry. Structs flow through joins/shuffles with no special case.

**IR + backend (R11–R15):** `IrType::Struct(StructId)` (stays `Copy`); per-module layout registry computed once from field widths (natural alignment; struct align = max field align min 1; size = final offset rounded to align; scalar widths i8/u8/bool=1, i16/u16=2, i32/u32/f32=4, i64/u64/f64=8). Emit `type :S = { … }` per struct; hand layout must agree with QBE's C-ABI (load-bearing, verified at runtime). Lowering: constructor = alloc + width-exact stores; getter = single width-exact load, no copy; setter = alloc new + byte copy + overwrite one field; destructure = load all fields. Registry threaded into `FuncBuilder`/`lower_call` to classify names. Width-exact field load/store **distinct** from the 8-byte-slot marshalling path (RISK 3). `dup` byte-copies, `drop` no-op.

**Marshalling + REPL (R16–R18, D5):** generalize `lower_line` prologue/epilogue to per-slot byte offsets (scalar = byte-identical 8-byte cell; struct = aggregate copy); new top = `top + (out_bytes - in_bytes)`. REPL `Session` drops `types.len() == top/8` assumption: byte-addressable buffer sized from emitted output bytes, offsets computed from `Session.types`. `format_stack` renders struct slot as `<TypeName>`.

## Non-functional

Green (`fmt`/`clippy -D warnings`/`test`) each phase. Invariants held: QBE-only, backend-neutral IR (`Ptr` opaque, no hardcoded machine word, register classes derived in backend), distinct `Type`/`IrType`, no JIT (`dlopen`), `core` `no_std`. No regressions (scalar IL byte-for-byte unchanged). **Marshalling + ABI verified by running binary/REPL goldens, not IL strings** (NF5). No premature abstraction (no lens/optic machinery, no tuples, no auto-derived `=`/print).

## Diagnostics (assert message text AND type/field names)

X1 unknown field type · X2 duplicate type · X3 recursive struct (terminates) · X4 constructor arity/type mismatch · X5 accessor on wrong type · X6 `.` on struct · X7 `=`/arithmetic on struct · X8 malformed declaration.

## Success criteria

S1 declaration parses/registers, all prior goldens green · S2 generated words in native binary (flat + nested) · S3 offset-correct for mixed i64/f64 + nested juxtaposition · S4 dup+getter non-consuming, setter leaves original intact · S5 struct survives word-call and REPL line boundary · S6 sharp behavioural diagnostics · S7 zero-field struct end-to-end, recursion is error not hang · S8 dogfood `examples/vectors.sth` (`Vec2`, nested `Segment`, `span len2 .`→25, `shift-x Vec2>x .`→6) native + REPL.

## Resolved risks / questions

- **RISK 1** carried-stack per-slot sizing — mitigated by running-binary + REPL goldens.
- **RISK 2** hand layout vs QBE ABI — standard natural-alignment C layout, runtime-verified.
- **RISK 3** width-exact field store vs 8-byte slot store — separate code paths, sub-word-adjacency read-back golden.
- **RISK 4** recursion termination — visited-set cycle detection.
- **Q1** `Type::Struct(StructId)` small `Copy` index; diagnostics thread registry for name rendering.
- **Q2** zero-field: size 0 align 1 (1-byte member fallback if QBE rejects empty aggregate).
- **Q3** `bool` field = 1-byte member, load zero-extended.
- **Q5** dogfood uses locked D2 syntax (`type: Vec2 x i64 y i64 ;`); brief's `{ … }` was shorthand.

## Out of scope (deferred)

Enums/ADTs/`match`/`|` sum (Slice 4); mid-body locals (Slice 4); arrays/`usize` (Slice 5); `Copy` marker/optional pointers (Slice 7); references/borrows/in-place mutation (Phase 3); generics (Phase 4); auto-derived `=`/print; tuples/index access; lenses; recursive data types; heap/move.

## Implementation

| Phase | Focus | Key commits | Files |
|---|---|---|---|
| 1 | Frontend: lexer de-delimiting, type namespace, `type:` production (R1–R4) | `3125e9ba` | ast, ir, lexer, parser |
| 2 | Checker: registration, recursion, generated-word sigs, diagnostics (R5–R10) | `75bb40a8` | check |
| 3 | IR layout + aggregate codegen, width-exact field ops (R11–R15) | `f9cf3fe4`, `1e8e33f4` | ast, backend/qbe, ir, repl, lexer, parser, tests/phase0 |
| 4 | Size-aware carried-stack marshalling + REPL (R16–R18) | `2e446f01` | backend/qbe, check, ir, parser, repl, tests/phase1 |
| 5 | Dogfood + full golden suite (S1–S8) | `6e547be7` | ROADMAP, examples/vectors.sth, backend/qbe, tests/phase0, tests/phase1 |

Note: Phase 4 introduced a slice-based check/layout API and `carried_slot_bytes`; Phase 3 review feedback (cycle 2) touched lexer/parser alongside the codegen work.
