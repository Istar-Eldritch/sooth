# Phase 2 Slice 4 — enums / ADTs + clause-style pattern matching

**Status: spec (design locked).** Slice 4 of Phase 2 (typed core), built on the scalar
core and Slice 3 structs on `main` (post the `a0c6217` type-resolver dedupe). Design is
fully locked in [phase2-slice4-brief.md](./phase2-slice4-brief.md); this spec turns the
locked decisions (D1–D10, M1–M6) into staged requirements, diagnostics, and runnable
goldens. Do not reopen the locked decisions.

## What ships

User-declared sum types (enums / ADTs) and their exhaustiveness-checked eliminator:

- the `type:` declaration form extended with `|`-separated variants (D1), each variant a
  name plus zero or more `name type` field pairs (the Slice 3 field grammar);
- per-variant constructor words that yield the *enum* (D2), the Slice 3 struct constructor
  being the degenerate one-variant case;
- a tagged inline-aggregate representation (D3): one typed virtual-stack slot backed by a
  QBE aggregate of a fixed-width discriminant tag plus a max-variant payload, laid out by
  the reused Slice 3 layout machinery; heap-free, monomorphic, all-Copy;
- clause-style word definition (D4): a word whose top input is an enum is defined by
  `|`-led clauses, one per variant, in place of a term body; no inline `match` keyword;
- exhaustive-only, exact-coverage elimination with branch-join folded into the single
  declared output effect (D5, M6);
- the `then` → `end` control-flow-closer rename (D6), behaviour-preserving;
- top-of-scope `| … |` locals extended to clause bodies (D7);
- enum values crossing word-call and REPL line boundaries via the reused size-aware
  carried-stack marshalling.

Generics, `Option<T>`/`Result<T,E>`, open multimethods, static overloading, `Bool`-as-enum,
the `_` wildcard, inline `match`, and recursive/heap data are all out of scope (Phase 4 /
Slice 7 / Phase 3, see brief).

## Why it's a real slice, not a table-fill

Slice 4 adds the language's first *sum* type and its eliminator, breaking assumptions the
product-only Slice 3 could hold:

1. **A second aggregate registry** — the type namespace now carries sums as well as
   products, resolved through a *separate* enum registry (D10) that shares the layout
   machinery but not the struct registry's code.
2. **A tagged layout** — a value is a discriminant plus a payload sized/aligned to the
   *largest* variant, not a fixed field record; per-variant payloads overlay one region.
3. **The eliminator is control flow** — clause dispatch loads a discriminant and branches,
   the closed dual of a future open multimethod; exhaustiveness is a compile error.
4. **`|` overloaded three ways** — variant separator, clause marker, locals delimiter;
   disambiguated by a variant pre-pass (D8), the load-bearing parser change of the slice.

## Locked decisions (do not reopen)

- **D1.** Enum via the `type:` form; `|`-separated variants make a body a sum, its absence a
  Slice 3 struct. Optional leading `|`. Each variant = name + zero or more `name type`
  pairs; `;` terminates; at least one variant required.
- **D2.** Variants are not standalone types. A variant constructor is a generated word named
  after the variant yielding the *enum* (`Circle ( f64 -- Shape )`, `Halt ( -- Cmd )`);
  fields consumed in declared order, first field deepest.
- **D3.** Representation = tagged inline aggregate (Slice 3 Model B, tagged): one slot, a
  QBE aggregate of a fixed-width tag + a max-variant payload; each variant's fields laid out
  in the payload like a struct via the existing layout machinery. Frame-local `alloc`,
  heap-free. All Copy: `dup` copies bytes, drop is a no-op. No `Copy` marker syntax.
- **D4.** Elimination = clause-style word definition, not inline `match`. A word whose top
  input is an enum is defined by `|`-led clauses (one per variant). A clause consumes the
  scrutinee off the top, pushes the matched variant's fields (first field deepest, atop any
  inputs below the scrutinee), then runs its body. To eliminate mid-computation, factor a
  word.
- **D5.** Exhaustive-only, exact coverage, no `_` wildcard. Exactly one clause per variant;
  missing / duplicate / unknown-variant are distinct sharp located errors. Branch-join needs
  no separate pass: every clause is checked against the word's single declared effect.
- **D6.** Rename the control-flow closer `then` → `end` (`if … else … end`). `end` becomes a
  reserved word. Behaviour-preserving: migrate the four examples with an `if…then` closer
  (`gcd`, `factorial`, `sign`, `bool_abi`), live tests, and living docs (README, ROADMAP,
  DESIGN); leave archived per-slice specs on their original `then`. `if`
  stays an inline keyword.
- **D7.** `| … |` locals extended to the *top* of a clause body (name the pushed payload and
  the stack below it), extent = that clause. Word-entry locals unchanged. No mid-body
  binding, no closing token. A Copy local mention pushes a copy; an unmoved local drops at
  scope end (a no-op for Copy).
- **D8.** Clause-vs-locals disambiguation via the variant pre-pass. A `|` immediately
  followed by a *known variant name* is a clause; a `|` followed by non-variant words closed
  by `|` is a locals block. A local (or parameter) name equal to any registered variant name
  is a sharp error (the backstop; the capitalized-variant / lowercase-local convention makes
  real collisions near-impossible). A clause-style word has no word-entry locals.
- **D9.** Aggregates nest freely: a variant field may be a scalar, struct, or enum, **and a
  struct field may be an enum** — any sized value type is a field of any aggregate, and both
  struct and enum layout size such fields via the combined registry (consistent with R10's
  combined struct+enum recursion graph, which only makes sense if struct fields can be enums).
  Value-recursion forbidden (a transitively self-containing struct or enum has infinite size →
  located compile error during layout, must terminate, never hang).
- **D10.** Separate enum registry, shared layout machinery. `Type` gains `Enum` (parallel to
  `Struct`) keyed into a per-program enum registry, logically distinct from the struct
  registry. Layout / field load-store / carried-slot / recursion-DFS machinery in `ir.rs` and
  `check.rs` is *reused* for variant payloads, which means extracting the field-placement
  core, generalizing the recursion DFS to a combined struct+enum graph, and threading the enum
  registry through `carried_slot_bytes` / layout sizing. "Untouched" here means
  **behaviour-preserving**: struct behaviour and every struct golden stay unchanged; it does
  NOT mean zero diffs to struct-side code (the shared core moves under both).
- **M1.** Tag = a fixed-width discriminant holding the variant's declaration index, placed
  first, payload following at the largest-variant alignment. Target-independent width (not
  the machine word); **`i32` default**. Not user-visible.
- **M2.** `.`, `=`, and arithmetic on an enum are sharp located errors naming the enum type,
  via the same `print_requires_printable` / operator-guard path Slice 3 made reachable.
- **M3.** A single-variant enum is allowed (newtype). A zero-variant declaration
  (`type: X | ;` or `type:` with `|` but no variant) is a malformed-declaration error.
- **M4.** REPL residual display of an enum slot = the `<TypeName>` placeholder, reusing the
  Slice 3 struct-placeholder path; no variant/field values shown.
- **M5.** Recursion / infinite-size detection during layout is a located compile error
  naming the cycle; it terminates.
- **M6.** A clause word's declared output effect is the single join target for every clause;
  no separate arm-agreement pass. A clause body is an ordinary term sequence and may itself
  use `if/else/end`.

## Requirements (by stage)

### Frontend (AST + lexer + parser)

- **R1. `then` → `end` rename (D6).** In `src/parser.rs` the `if` production stops its
  `then`-arm on `else`/`end` (was `else`/`then`), closes with `expect_word("end")`, and the
  "closer without a matching `if`" guard rejects bare `end`/`else`. `end` is reserved (no
  word may be named `end`; falls out of the guard). The lexer is unchanged (`end` is an
  ordinary word matched positionally). `TermKind::If { then_branch, else_branch }` field
  names are internal and may stay (they name the arm, not the keyword). Migrate every *live*
  `then` occurrence: the four examples with an `if…then` closer (`gcd`, `factorial`, `sign`,
  `bool_abi`; `rgb`/`rgb_bits` use `then` only as English prose in comments), all unit- and
  integration-test source strings, and README/ROADMAP/DESIGN; leave `docs/phase*-spec.md`
  untouched.
- **R2. AST enum nodes (D1, D10).** In `src/ast.rs`: `Type` gains `Enum(EnumId, &'static
  str)`, mirroring `Struct(StructId, &'static str)` (stays `Copy`, self-renders without a
  registry). Add `EnumDecl { name, name_static, variants: Vec<VariantDecl>, span }` and
  `VariantDecl { name, name_static, fields: Vec<(String, Type)>, span }`, plus an `EnumId`
  newtype parallel to `StructId`. `Module` gains `enums: Vec<EnumDecl>`. `Type::from_name`
  is unchanged (scalars only); `Type::name`/`Display` render `Enum` via its `&'static str`.
- **R3. Shared resolver consults both registries (D10).** Extend the free
  `ast::resolve_type_name` to take the enum registry alongside the struct registry (e.g.
  `resolve_type_name(structs, enums, name)`): scalar table first, then structs, then enums.
  `Module::resolve_type_name` and the parser's `resolve_type` thread the enum slice through.
  A type name registered as neither is unknown (unchanged located error).
- **R4. Clause-carrying word body (D4, D7).** `WordDef`'s body becomes either a term
  sequence with optional entry locals (as today) or a clause list. Introduce a `WordBody`
  enum: `Terms { locals: Vec<String>, terms: Vec<Term> }` and `Clauses(Vec<Clause>)`, with
  `Clause { variant: String, locals: Vec<String>, body: Vec<Term>, span }`. `WordDef` holds
  `body: WordBody` (the current `locals` + `body` fields fold into `WordBody::Terms`).
  Downstream readers in `check.rs`, `ir.rs`, `repl.rs` match on `WordBody`.
- **R5. Variant pre-pass (D8).** Extend the parser pre-pass that scans `type:` names to also
  register every variant name of every enum `type:`, before any word body is parsed, so the
  clause-vs-locals decision can consult the variant set regardless of declaration order.
- **R6. Enum `type:` production (D1, M3).** After the type name, if the body contains a
  `Pipe` before `;` it is an enum: parse `|`-separated variants (optional leading `|`), each
  a variant name followed by `name type` field pairs until the next `|` or `;`; otherwise it
  is the Slice 3 struct (unchanged). Field-type resolution reuses `expect_field_type_token` +
  `resolve_type`. Located parse errors: an odd field-token count within a variant, a
  delimiter/defining-word where a field type belongs, a missing `;`, and **zero variants**
  (`type: X | ;` / a `|`-bearing body with no variant name) → malformed-declaration error.
- **R7. Clause-style word body + D8 disambiguation (D4, D7, D8).** In the word-definition
  production, after `( effect )`, decide: if the next token is `Pipe` **and** the token
  after it is a registered variant name → parse clauses; otherwise parse entry-locals +
  terms as today. Each clause: `|`, a variant name, an optional clause-body `| names |`
  block (present iff the token after the variant name is `Pipe` *not* immediately followed by
  a known variant name — an empty clause body is `| Variant` directly followed by the next
  `| KnownVariant` or `;`), then body terms up to the next clause-starting `|` or `;`. A
  `|`-group that is neither a valid clause nor a valid locals block is a located parse error.
  A clause-style word parses no word-entry locals.

### Checker (`src/check.rs`)

- **R8. Registration + duplicate names across registries (D10).** Register each enum so its
  name resolves as a `Type`. The duplicate-type-name check spans *both* registries: a name
  used by two structs, two enums, or one of each is a sharp located error naming the type
  (generalize `check_duplicate_struct_names` to walk struct + enum names).
- **R9. Variant constructor signatures (D2).** Synthesize a generated-word `Sig` per variant
  — `Variant ( T1 … Tn -- Enum )`, fields in declared order (first deepest), a zero-field
  variant being `( -- Enum )` — added to the checking env alongside `struct_generated_sigs`,
  so constructor arity/type misuse falls out of the existing call-check path.
- **R10. Layout / recursion (D9, M5).** Extend the value-recursion cycle detection to the
  combined type graph: a node is a struct or an enum; edges are struct field types and
  variant field types. A direct or transitive value-cycle is a located error naming the
  cycle; the DFS visited-state (`VisitState`) guarantees termination.
- **R11. Clause-style elimination checking (D4, D5, D7, D8, M6).** For a clause word:
  - the word's top input (`effect.inputs.last()`) must be an enum, else a sharp error
    (*clause-style body on a word whose top input is not an enum*);
  - the clauses must cover every variant of that enum *exactly once*: a missing variant
    (names it), a duplicate clause (names it), and an unknown variant name (names it) are
    three distinct sharp located errors;
  - each clause is checked with initial stack = the declared inputs below the scrutinee,
    then the variant's fields pushed first-deepest atop them; the clause body must leave the
    word's declared outputs (M6, the single join target);
  - clause-body `| names |` bind the top N of the clause-entry stack (payload then below),
    extent = the clause, reusing the word-entry local-binding path;
  - a clause-body or word-entry/parameter name equal to any registered variant name is a
    sharp located error (D8 backstop).
- **R12. Enum operator/print guards (M2).** `.` on an enum reaches
  `print_requires_printable_error`; `=` / arithmetic on an enum reach the operand-pair
  guard; each names the enum type (falls out because `Type::Enum` is neither numeric nor
  bool). Must be covered by goldens even though largely mechanical.

### IR + backend (`src/ir.rs`, `src/backend/qbe.rs`)

- **R13. `IrType::Enum` + tagged layout (D3, D10, M1).** `IrType` gains `Enum(EnumId)`
  (mirrors `Struct(StructId)`, stays `Copy`); `ir_type_of(Type::Enum(id,_)) =
  IrType::Enum(id)`. Add an `EnumLayout { name, tag_offset, tag_ty, payload_offset, size,
  align, variants: Vec<VariantLayout> }` and `VariantLayout { fields: Vec<FieldLayout> }`,
  computed once by reusing the Slice 3 field-placement core: tag first (fixed `i32`, M1),
  payload at the max variant alignment, each variant's fields laid out in the payload region
  via the same natural-alignment placement as struct fields; enum `size` = payload_offset +
  max variant payload size rounded to `align`, `align` = max(tag align, max variant field
  align). All offsets/sizes derive from field widths (word-width-neutral). `carried_slot_bytes`
  and `scalar_size_align` gain an `Enum` arm mirroring `Struct` (a carried enum occupies
  `round_up(size, 8)`). **Nested-aggregate sizing:** field-size lookup for both struct and
  enum layout consults the combined registry, so a struct field of enum type (and a variant
  field of struct/enum type) is sized via its layout, not `scalar_size_align` (D9). Every
  exhaustive `IrType` match (backend width/spelling, `scalar_size_align`) gains an `Enum` arm
  mirroring `Struct`; the member-access `unreachable!` arms stay unreachable since enum access
  is offset-driven.
- **R14. Enum registry threaded into lowering.** Extend the IR's aggregate-registry view
  (`Structs`, or a sibling `Enums`) to carry enum layouts and a generated-word map
  (variant name → `EnumWord::Construct(EnumId, variant_index)`), built once from the module
  (build path) or the accumulated REPL registry, and consulted in `lower_call` to classify a
  variant-constructor call, parallel to `StructWord`. Do not merge the struct registry (D10).
- **R15. QBE aggregate + variant construction (D3).** Emit a QBE aggregate type per enum,
  size- and alignment-correct. Because all internal access is offset-driven (explicit
  `PtrOffset` + width-exact `FieldLoad`/`FieldStore` and `Blit`), the aggregate is emitted as
  an alignment-annotated opaque byte blob of the enum's total size (`type :E = align A { b N
  }`), not a member list — the payload has no single member layout across variants, and
  caller/callee agree because they share `:E`. A variant constructor allocs a frame slot
  (`Alloc(size, align)`), stores the discriminant (an `i32` `Const` via `FieldStore` at
  `tag_offset`), stores each field at `payload_offset + field.offset` (first deepest, reusing
  `store_field`), and yields the value. `dup` blits the aggregate (Copy); drop is a no-op —
  the existing `IrType::Struct` arms in `dup`/marshalling extend to `IrType::Enum`.
- **R16. Clause dispatch (elimination) (D4, D5, M6).** Lowering a clause word: the scrutinee
  is the last param (a pointer to its aggregate). Load the discriminant into a **temp**
  (`FieldLoad` `i32` at `tag_offset`, a value used for comparison, **not** pushed onto the
  virtual stack; add a small load-tag helper if `load_field_onto_stack` only pushes). Build an
  **N-way** dispatch, *not* the binary `lower_if` shape: for each variant emit
  `Cmp(Eq, tag, variant_index)` feeding a `Jnz` to that variant's clause block, chaining the
  false-edges; exhaustiveness (R11) lets the final variant be the terminal fall-through, so no
  `default`/trap block is needed. Each clause block: the stack below = params minus the
  scrutinee; load that variant's fields from `payload_offset + field.offset` onto the stack
  first-deepest (reuse `load_field_onto_stack` with enum payload offsets); bind clause-body
  locals; lower the clause body; jump to a single **join block**. The join block has **one
  `Phi` per declared output** (M of them), each merging **all N clause predecessors** (not
  two), then `Ret`. This is a new N-predecessor / M-output join; the 2-predecessor,
  single-value `lower_if` is only the degenerate case, do not assume its shape. Verify at
  runtime with a **3-plus-variant** enum (so a two-way miscompile is caught), not by reading
  IL.
- **R17. Marshalling (D3).** The `lower_line` prologue/epilogue and REPL buffer treat an enum
  slot exactly as a struct slot: an enum slot is blitted out of / into the carried buffer at
  its `carried_slot_bytes` offset. Extend the `IrType::Struct` arms in `lower_line` (prologue
  load / epilogue store) and `carried_slot_bytes` to `IrType::Enum`. Scalars and structs
  unchanged.

### REPL (`src/repl.rs`)

- **R18. Enum declarations + residual display (M4).** `Session` gains an `enums:
  Vec<EnumDecl>` registry parallel to `structs`. `eval_typedef` branches on whether the
  `type:` body contains a `Pipe`: an enum body parses into the enum registry (with the same
  append-then-rollback-on-error discipline as structs and a self/forward reference resolving
  against the accumulated registries), a struct body unchanged. `parse_line_with_structs`
  and the `Parser` gain the enum slice so a clause-style word (and enum effect types) parse
  at REPL scope. **The D8 clause-vs-locals variant set at REPL scope is the union of the
  session's accumulated variant names (from `Session.enums`) and the current line's**, so a
  clause word defined on a line *after* its enum declaration disambiguates correctly (the
  parser pre-pass alone scans only the current unit). `typed_env` adds the variant-constructor
  sigs. `format_stack` renders an
  enum slot as its `<TypeName>` placeholder (reuse the struct arm) and advances the buffer by
  the enum's `size` cells. `defined type {name}` message unchanged.

## Non-functional

Green (`cargo fmt --check && cargo clippy -- -D warnings && cargo test`) each phase.
Load-bearing invariants held: **QBE-only** backend; **backend-neutral IR** (`Ptr` opaque, no
hardcoded machine word — the tag is a fixed `i32`, not the machine word; register classes and
QBE spellings derived only in the backend); **frontend `Type` and backend `IrType` stay
distinct**; **no JIT** (REPL via `dlopen`); `core` `no_std`. No regressions: scalar and struct
behaviour otherwise unchanged, and the `then` → `end` rename must be *behaviour-preserving*
(only the keyword changes; no golden's semantics move). Reuse the Slice 3 aggregate / layout /
carried-slot machinery — **no new memory model**. Verify layout, construction, dispatch, and
marshalling by **running native-binary and REPL goldens, not IL-string assertions**. No
premature abstraction: no generics scaffolding, no open-multimethod hooks, no `_` wildcard, no
inline `match`, no auto-derived `=`/print for enums.

## Diagnostics (assert message text AND the enum/variant/type names)

- **X1.** Unknown variant field type (names the type).
- **X2.** Duplicate type name across the struct + enum registries (names the type).
- **X3.** Recursive enum / infinite size (names the cycle; terminates, does not hang).
- **X4.** Non-exhaustive clause word (names the missing variant).
- **X5.** Duplicate clause (names the variant).
- **X6.** Unknown variant in a clause (names it and the enum).
- **X7.** Clause-style body on a word whose top input is not an enum (names the word / type).
- **X8.** A clause meeting/violating the declared output effect (the M6 join, reusing the
  existing output-effect and branch-type mismatch diagnostics).
- **X9.** Constructor arity / field-type mismatch (falls out of the call-check path).
- **X10.** `.` / `=` / arithmetic on an enum (M2; names the enum type).
- **X11.** Malformed / zero-variant declaration (M3).
- **X12.** A local (word-entry or clause-body) or parameter name equal to a registered
  variant name (D8 backstop; names the collision).

## Success criteria → golden (each exit criterion maps to a runnable golden, not IL)

1. **Declaration + registration, no regressions.** A `|`-variant `type:` parses and registers
   an enum usable in effects and bodies; a `|`-free `type:` still parses as a Slice 3 struct;
   all Phase 0/1/Slice-1/2/3/floats/bitwise/bool goldens still pass (after the mechanical
   `then` → `end` migration). *Goldens:* parser/checker unit tests + the migrated existing
   suites stay green, plus behavioural goldens for **X1** (unknown variant field type, asserts
   message text + the type name) and **X2** (duplicate type name across the struct+enum
   registries, asserts message text + the type name).
2. **Constructors build correct values (native binary).** A zero-field variant and a
   multi-field variant construct with correct discriminant and payload. *Golden:* a native
   binary that constructs then reads back a variant's payload (via a clause word) and prints
   the fields, including a variant whose payload alignment forces padding (e.g. an all-`f64`/
   `i64`, align-8 variant so the payload lands at `payload_offset` 8 with tag padding,
   exercising the round-up, not just asserting M1).
3. **Clause elimination end-to-end (native binary).** A clause word dispatches to the correct
   clause and puts the variant's payload on the stack first-deepest, for a multi-field-variant
   enum and a zero-field-variant enum, with a value flowing *underneath* the scrutinee handled
   correctly. *Golden:* `shapes.sth`'s `area` (multi-field) and `unwrap-or` (zero-field +
   value below the scrutinee) run in a native binary with the expected output. Plus a
   **nested-aggregate** native golden (D9): a variant carrying a struct payload (e.g. a `Vec2`)
   constructed, passed through a clause word, and a nested field read back; and an enum used as
   a struct field, to guard the combined-registry field sizing.
4. **Exhaustiveness / shape diagnostics (behavioural).** Assert message text and the
   variant/type names for X4 (non-exhaustive, names missing variant), X5 (duplicate clause),
   X6 (unknown variant), X7 (clause body on non-enum top input), X8 (clause violating the
   declared output effect). *Goldens:* checker negative tests.
5. **`then` gone, `end` works.** `if … else … end` compiles and runs; `then` is a parse
   error; every migrated golden passes. *Goldens:* the migrated examples/tests + a negative
   `then`-is-now-unmatched test.
6. **Clause-body locals + variant-name collision.** A clause-body `| names |` binds the
   payload by name (extent = the clause); word-entry locals unchanged; X12 is a sharp error.
   *Goldens:* `shapes.sth`'s `Rect | w h |` clause (native) + a checker negative for X12 + a
   term-word-with-entry-locals still parsing.
7. **Enum crosses word + REPL boundaries.** An enum value survives a word-call boundary and a
   REPL line boundary (size-aware marshalling), including a large-payload variant (multiple
   `i64` fields, exceeding one 8-byte carried cell, so the marshalling can't pass trivially);
   the REPL shows `<TypeName>`. *Goldens:* a native binary passing an enum through a word call
   - return, and a scripted REPL session (`tests/phase1.rs`) that declares an enum, then on a
   *later* line defines a clause word over it (exercising REPL-scope D8 variant-set seeding
   from `Session.enums`), constructs a value showing `<Shape>`, and eliminates it on a further
   line.
8. **Remaining behavioural diagnostics + dogfood.** X10 (`.`/`=`/arithmetic on an enum), X3
   (recursive enum is a compile error, not a hang: cover both a *direct* self-recursive
   `type:` and a *mutual* `A -> B -> A` cycle, parity with the Slice 3 struct recursion
   goldens), X11 (malformed/zero-variant), X9 (constructor arity/type mismatch); and
   `examples/shapes.sth` runs correctly as a native binary and in the REPL. *Goldens:* checker
   negatives + the `shapes.sth` native + REPL goldens, plus the headline negatives
   (non-exhaustive `area` missing `Rect`, `.` on a `Shape`, a recursive `type:`).

## Dogfood — `examples/shapes.sth` (S8-style)

```
type: Shape
| Circle r f64
| Rect   w f64 h f64
;

type: MaybeInt | None | Some v i64 ;

: area ( Shape -- f64 )
| Circle   dup * 3.14159 *
| Rect     | w h |  w h *
;

: unwrap-or ( i64 MaybeInt -- i64 )
| None
| Some   swap drop
;

: main ( -- )
  2.0 Circle area .          \ ~12.56636
  3.0 4.0 Rect area .        \ 12
  5 None unwrap-or .         \ 5   (empty None clause yields the default underneath)
  5 7 Some unwrap-or . ;     \ 7
```

Exercises a multi-field-variant enum with float payloads and clause-body locals; a zero-field
variant (`None`) with an empty clause body (the D8 empty-clause disambiguation, since the
following `| Some` is a known variant); and a value flowing under the scrutinee
(`unwrap-or`'s default). Runs native and in the REPL. Plus the headline negatives: a
non-exhaustive `area` (missing `Rect`), `.` on a `Shape`, and a recursive `type:`.

## Key risks

- **Clause-vs-locals `|` disambiguation (D8)** — the load-bearing parser change: the first
  `|` of a word body, empty clause bodies, and clause-body locals must all parse
  unambiguously via the variant pre-pass, with a variant-named local rejected. Cover with
  goldens: an empty clause followed by another clause; a clause-body local; a term word with
  entry locals still parsing unchanged; a variant-named local rejected.
- **Tagged layout (D3, R13)** — tag + payload offsets/size/alignment across variants of
  differing size and alignment; the largest variant sizes the payload; mixed int/float
  variant fields read back correctly. Per-variant runtime goldens.
- **Clause dispatch codegen (R16)** — discriminant load into a temp, an N-way `Cmp(Eq)`-tag
  compare-chain to the right clause, and per-variant payload load first-deepest; verify at
  runtime with a 3-plus-variant enum, not by reading IL. Note: this is an N-predecessor /
  M-output join, *not* the 2-predecessor `lower_if`; build the N-way structure, do not assume
  the binary shape.
- **QBE aggregate for the enum (R15)** — the opaque-blob emission must have the size and
  alignment the by-value ABI classification and `alloc` expect; runtime-verified through the
  word-call-boundary golden (criterion 7).
- **`then` → `end` rename must be behaviour-preserving (D6)** — only the keyword changes; no
  golden's semantics move; the migrated suites stay green.
- **Recursion detection (D9, M5)** must terminate and name the cycle for a self- or
  mutually-recursive enum.
- **Carried-stack marshalling** was generalized in Slice 3; confirm an enum (especially a
  large-payload variant) crosses word and REPL boundaries intact (criterion 7).

## Out of scope (deferred)

The `_` wildcard / catch-all; any inline `match … end`; open multimethods and static
overloading (`generic:`/`method:`) (Phase 4); generics / type parameters, so no generic
`Option<T>`/`Result<T,E>` (monomorphic hand-written enums only) (Phase 4); `Bool` as a library
enum (Phase 4); the `Copy` marker and optional / non-null pointers (Slice 7); recursive / heap
data (need pointers, Slice 7 / Phase 3); references / borrows / in-place mutation (Phase 3);
the `?` error short-circuit sugar (Phase 5); auto-derived `=`/printing for structs/enums;
mid-body (non-top-of-scope) locals; the bytecode-VM dogfood (Slice 6).

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Behaviour-preserving `then` -> `end` control-flow-closer rename (D6, R1): the `if` production and reserved-word guard, migrate the four examples with an `if…then` closer (`gcd`, `factorial`, `sign`, `bool_abi`), all live test source strings, and README/ROADMAP/DESIGN; leave archived per-slice specs on `then`. Full suite stays green.",
      "effort": "S",
      "difficulty": "medium"
    },
    {
      "phase": 2,
      "focus": "Enum declaration frontend (R2, R3, R5, R6) + checker registration (R8, R9): `Type::Enum`, `EnumDecl`/`VariantDecl`/`EnumId`, `Module.enums`, the shared resolver extended to both registries, the variant pre-pass, the enum `type:` production (variants + fields, optional leading `|`, malformed/zero-variant errors, M3), duplicate-name-across-registries (X2), variant-constructor sigs (X9), unknown variant field type (X1). Enums resolve in effects and constructors type-check; no elimination yet.",
      "effort": "M",
      "difficulty": "medium"
    },
    {
      "phase": 3,
      "focus": "Tagged layout + variant construction codegen + recursion + marshalling (R10, R13, R14, R15, R17): `IrType::Enum`, `EnumLayout`/`VariantLayout` via the reused field-placement core (fixed i32 tag, max-variant payload, word-width-neutral), combined-graph recursion detection (X3/M5), combined-registry field sizing so a struct field may be an enum and a variant field a struct/enum (D9), the opaque-blob QBE aggregate, variant constructor lowering (alloc + tag store + field stores, first deepest), `dup`/drop, and enum carried-slot marshalling across word/REPL boundaries. Exit: constructors build correct values natively (criterion 2) and an enum crosses word + REPL boundaries incl. a large payload (criterion 7); REPL shows `<TypeName>` (M4, R18 display).",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "Clause-style elimination end-to-end (R4, R7, R11, R12, R16): the `WordBody`/`Clause` AST, the parser clause body + D8 clause-vs-locals disambiguation + clause-body locals, checker exhaustiveness/exact-coverage/branch-join (X4/X5/X6/X7/X8), clause-body local binding + variant-name-collision (X12), enum operator/print guards (X10), and IR clause dispatch (discriminant-into-temp + N-way `Cmp(Eq)`-tag chain + N-predecessor/M-output join phi, NOT the 2-pred `lower_if`, + per-variant first-deepest payload load). Exit: criteria 3, 4, 6 as runtime/behavioural goldens.",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 5,
      "focus": "Dogfood + full golden suite + REPL enum plumbing (R18): `examples/shapes.sth` native + REPL, the headline negatives (non-exhaustive `area`, `.` on a `Shape`, recursive `type:`), remaining behavioural diagnostics (X3, X10, X11, X9), the scripted REPL enum session, and wiring `parse_line_with_structs`/`eval_typedef`/`typed_env` for enum declarations and clause words at REPL scope. Exit: criteria 1, 5, 7, 8 fully green.",
      "effort": "M",
      "difficulty": "medium"
    }
  ]
}
```
