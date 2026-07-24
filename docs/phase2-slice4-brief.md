# Phase 2 Slice 4 brief: enums / ADTs + clause-style pattern matching

Input for the spec-writer. Slice 4 adds sum types (enums / ADTs) to Sooth's typed
core and the eliminator that takes them apart. Elimination is folded into word
definition (clause-style), not a separate `match` statement. The slice reuses the
inline-aggregate layout machinery built in Slice 3 (now tagged), renames the
control-flow closer `then` to `end`, and extends the existing `| ... |` locals to
clause bodies. Still heap-free, still monomorphic. Generics (so generic
`Option<T>`/`Result<T, E>`), open multimethods, static overloading, and making `Bool`
a library enum are all Phase 4.

Built on the completed scalar core (`i8`..`u64`, `f32`/`f64`, `bool`; arithmetic,
bitwise, comparison, conversions; type-directed `.`), Slice 3 structs (the `type:`
declaration form, the `Type::Struct` registry, `compute_layout`, per-slot carried-stack
marshalling, generated words, QBE aggregate emit + field load/store), and the
`| ... |` word-entry locals and `if/else/then` from Phases 0/1.

## Where this sits

- Scalar core (Slices 1-2 + floats/bitwise/bool) and Slice 3 (structs) are done and
  merged to `main`.
- Slice 4 is the sum half of the algebraic-types story: it makes the type namespace
  carry sums as well as products, and it introduces the language's exhaustiveness-
  checked eliminator. Result/Either fall out as ordinary (monomorphic) enums.
- It reuses Slice 3's aggregate/layout machinery (a tagged aggregate is a struct with
  a discriminant prefix and a per-variant payload) and the size-aware carried-stack
  marshalling. No new memory model.
- Slice 5 (arrays), Slice 6 (the VM dogfood), and Phase 5 (errors as values) build on
  these enums. Phase 4 turns the closed clause-style eliminator into the open
  multimethod dual and lands generics; Slice 4 is designed not to foreclose that.

## Decisions locked

- **D1. Enums declared with the Slice 3 `type:` form, `|`-separated variants.** The
  presence of `|` in a `type:` body makes it a sum; its absence makes it the Slice 3
  product (struct). A leading `|` is optional. Each variant is a name followed by zero
  or more bare `name type` field pairs (the same field grammar as a struct); `;`
  terminates the declaration. At least one variant is required.

  ```
  type: Shape
  | Circle r f64
  | Rect   w f64 h f64
  ;

  type: Cmd | Halt | Push n i64 | Add ;
  ```

  Field names lowercase by convention; type and variant names capitalized by
  convention (not lexically enforced, but see D8 for why the case split is load-bearing
  for disambiguation).

- **D2. Variants are not standalone types.** There is no bare `Circle` type in stack
  effects. A variant constructor is a generated word named after the variant that
  yields the *enum*: `Circle ( f64 -- Shape )`, `Rect ( f64 f64 -- Shape )`,
  `Halt ( -- Cmd )`. Fields are consumed in declared order, first field deepest on the
  stack (same rule as a struct constructor). "A record is a one-variant enum" holds:
  the Slice 3 struct constructor is the degenerate case.

- **D3. Representation: tagged inline aggregate (Slice 3 Model B, now tagged).** An
  enum value occupies one typed virtual-stack slot backed by a QBE aggregate: a
  fixed-width discriminant tag followed by a payload region sized and aligned to the
  largest variant. Each variant's fields are laid out inside the payload region like a
  struct (offsets/size/alignment via the existing `compute_layout`). Frame-local
  `alloc`, heap-free. All Slice 4 enums are Copy (variants hold scalars or Copy
  structs/enums only): `dup` copies the aggregate bytes, drop is a no-op. No `Copy`
  marker syntax (Slice 7).

- **D4. Elimination is clause-style word definition, not an inline `match`.** A word
  whose top-of-stack input is an enum may be defined by `|`-led clauses, one per
  variant, in place of a term-sequence body. `:` ... `;` already brackets the word, so
  no `match`/`end` wrapper is needed.

  ```
  : area ( Shape -- f64 )
  | Circle   dup * 3.14159 *      \ Circle's r is on the stack
  | Rect     *                    \ Rect's w h are on the stack
  ;
  ```

  A clause consumes the scrutinee off the top of the stack (the affine destructor
  dispatch: the enum value is gone after the clause runs), pushes the matched variant's
  fields onto the stack in declared order (first field deepest, atop any inputs that
  were below the scrutinee), then runs the clause body. There is **no inline `match`
  block**; to eliminate an enum in the middle of a computation you factor the
  elimination into its own word and call it (the same "factor a word" discipline as
  the locals and the Slice 3 `span`/`sub` example).

- **D5. Exhaustive-only, exact coverage, no wildcard.** A clause word must have exactly
  one clause per variant of the scrutinee's enum. A missing variant, a duplicate
  clause, or an unknown variant name is a sharp located error. There is no `_` catch-all
  this slice (deferred). Branch-join needs no separate pass: every clause is checked
  against the word's single declared stack effect, so all clauses necessarily agree on
  output, the same guarantee `if`/`else` gives.

- **D6. Rename the control-flow closer `then` to `end`.** `if <bool> if-terms else
  else-terms then` becomes `if <bool> if-terms else else-terms end`. `end` becomes a
  reserved word. This is a behaviour-preserving keyword rename: migrate the examples
  (6 `then` tokens), the tests, and the living docs (README, ROADMAP, DESIGN); leave the
  archived per-slice specs on their original `then`. Rationale: one closer for all
  bracketed control flow, and shedding Forth's `then`-means-end-of-`if` wart. `if`
  stays an inline keyword (strict evaluation with no quotations yet means a mid-
  computation conditional must be syntax, not a word; `if`-as-combinator is Phase 4).

- **D7. Extend `| ... |` locals to clause bodies (top-of-scope only).** The existing
  word-entry locals (bind the top N inputs to names, once, after the effect, extent =
  the whole word body) are unchanged. Slice 4 allows the same `| names |` block at the
  *top of a clause body* to name the pushed payload (and stack below it). Extent = that
  clause. No mid-body binding anywhere, no closing token: to name a value computed
  partway through, factor a word. Mentioning a Copy local pushes a copy (all Slice 4 is
  Copy); a local left unmoved drops at scope end (a no-op for Copy).

  ```
  : area ( Shape -- f64 )
  | Circle   dup * 3.14159 *
  | Rect     | w h |  w h *      \ name the payload; same locals feature as a word body
  ;
  ```

- **D8. Clause-vs-locals disambiguation via the variant pre-pass.** Reusing `|` for
  three roles (variant separator in `type:`, clause marker here, locals delimiter)
  creates one real ambiguity: at a `|`, is what follows a clause (`| Circle ...`) or a
  locals block (`| a b |`)? An empty clause body makes it worse (`| Halt |` could be a
  clause for `Halt` or a locals binding named `Halt`). Resolution: the parser's
  type-name pre-pass (already scanning `type:` declarations for struct names) is
  extended to register every variant name too. Then a `|` immediately followed by a
  **known variant name** is a clause; a `|` followed by non-variant words closed by `|`
  is a locals block. To keep this unambiguous, a local (or parameter) name equal to any
  registered variant name is a sharp error. The capitalized-variant / lowercase-local
  convention makes real collisions near-impossible; the error is the backstop. A
  clause-style word has no word-entry locals (its body is purely clauses; each clause
  may carry its own clause-body locals after the variant name).

- **D9. Nesting allowed; value-recursion forbidden.** A variant field may be any sized
  value type: a scalar, a struct, or another enum. An enum that (transitively) contains
  itself by value has infinite size and is a located compile error during layout, never
  a hang, exactly as for recursive structs. Recursive data (lists, trees) needs a
  pointer through the recursion, which is Slice 7 / Phase 3. This is the constraint that
  shapes the dogfood: the classic linked-list/AST ADT is not available yet.

- **D10. Separate enum registry, shared layout machinery.** `Type` gains an `Enum`
  variant (parallel to `Struct`) keyed into a per-program enum registry; the Slice 3
  struct registry and its shipped code stay untouched (lower risk than merging). The
  layout, field load/store, and carried-slot machinery in `ir.rs` are reused for
  variant payloads (elevated once, in Slice 3, now shared). The conceptual "struct = a
  one-variant, tagless enum" unification is a possible future consolidation, not this
  slice.

## Micro-decisions locked (were open, resolved here)

- **M1. Tag = a fixed-width discriminant** holding the variant's declaration index,
  placed first in the aggregate, payload following at the largest-variant alignment.
  The width is target-independent (not the machine word, per the word-width-neutrality
  invariant); `i32` is the recommended default (spec may pick a narrower fixed width).
  Not user-visible.
- **M2. `.`, `=`, and arithmetic on an enum are sharp located errors** naming the enum
  type, via the same `print_requires_printable` / operator-guard path Slice 3 made
  reachable for structs.
- **M3. A single-variant enum is allowed** (a newtype/wrapper). A zero-variant
  declaration (`type: X | ;` or `type:` with `|` but no variant) is a malformed-
  declaration error; uninhabited types are deferred.
- **M4. REPL residual display of an enum slot** is the `<TypeName>` placeholder (reusing
  the Slice 3 struct-placeholder path); no variant/field values are shown.
- **M5. Recursion / infinite-size detection** during layout is a located compile error
  naming the cycle; it terminates.
- **M6. A clause word's declared output effect is the single join target** for every
  clause; there is no separate arm-agreement pass. A clause body is an ordinary term
  sequence and may itself use `if/else/end`.

## Frontend

- **Lexer.** `is_delimiter` already splits `; ( ) |` (and dropped `:` in Slice 3), so
  `|` is already a standalone token; nothing new there. The only lexer-adjacent change
  is treating `end` as the control-flow closer keyword in place of `then` (both are
  ordinary words matched positionally by the parser, so this is a parser-level rename,
  not a new token class).
- **AST / type namespace.** `Type` (in `src/ast.rs`, currently `Int`/`Float`/`Bool`/
  `Struct`) gains an `Enum` variant referencing a registered enum by id/name, mirroring
  `Struct`. Add an `EnumDecl` (name + ordered `Vec<VariantDecl>`, each variant a name +
  ordered `(field-name, Type)` list) and an enum registry alongside the struct registry
  on the module. `from_name`/`name`/`Display` and the shared `resolve_type_name`
  free function consult both registries. A `WordDef` body becomes either a term sequence
  (with optional entry locals, as today) or a list of clauses (each: a variant name, an
  optional clause-body locals block, a term-sequence body).
- **Parser.** Extend the `type:` production: after the name, if the body contains `|`,
  parse it as an enum (variants, each a name + `name type` field pairs, `|`-separated,
  optional leading `|`); otherwise it is the Slice 3 struct. Extend the pre-pass to
  register variant names as well as type names. In the word-definition production,
  detect a clause-style body (leading `|` followed by a known variant, per D8) and parse
  clauses; otherwise parse entry-locals + terms as today. Rename `then` to `end` in the
  `if` production. Reject malformed declarations (odd field token count, zero variants),
  clause words whose clauses are not exhaustive/exact (this may be the checker's job;
  the parser at least produces the clause list), and a `|` group that is neither a valid
  clause nor a valid locals block.

## Checker

- **Registration + resolution.** Register each enum so its name resolves as a `Type`;
  a duplicate type name (across structs and enums), an unknown field type, or a variant
  field of an unknown type is a sharp located error. Register each variant's constructor
  word (D2).
- **Layout / recursion.** Compute the tagged layout (tag + max-variant payload) via the
  shared machinery; detect direct and transitive value-recursion (infinite size) as a
  located error (M5).
- **Clause-style elimination.** For a clause word, the top input type must be an enum;
  the clauses must cover every variant of that enum exactly once (missing, duplicate,
  and unknown-variant are distinct sharp errors naming the variant(s)). Each clause is
  type-checked with the variant's fields pushed onto the stack (first deepest) atop the
  remaining declared inputs, and must produce the word's declared output effect (M6).
  Clause-body locals bind per D7; a local named like a variant is rejected (D8).
- **Diagnostics (behaviour, name the types/variants).** At least: unknown field/variant
  type; duplicate type name; recursive enum; non-exhaustive clause word (names the
  missing variant); duplicate/unknown clause; clause-style body on a word whose top
  input is not an enum; constructor arity/type mismatch; `.`/`=`/arithmetic on an enum
  (M2); malformed/zero-variant declaration; local-name-equals-variant-name. Each names
  the enum and/or variant and the rule.

## IR + backend

- **IrType + layout.** `IrType` gains an enum/tagged-aggregate entry with the computed
  layout (tag offset/width, payload offset, per-variant field offsets, total size/
  align), derived once and reusing `compute_layout`. `ir_type_of` maps `Type::Enum` to
  it. `Ptr` stays opaque; all offsets/sizes come from field widths, never a hardcoded
  machine word (word-width-neutrality).
- **QBE aggregate + construction.** Emit a QBE aggregate type per enum sized to
  tag + max payload. A variant constructor allocates a frame slot, stores the
  discriminant, stores the variant's fields at their payload offsets, yields the value.
- **Clause dispatch (elimination).** Load the discriminant, dispatch to the matching
  clause (an `if`-chain or a jump table over the tag; reuse the existing branch
  lowering), and in each clause load that variant's fields from the payload region onto
  the virtual stack (first deepest) before running the clause body. Verify at runtime,
  not by reading IL.
- **Marshalling.** Reuse Slice 3's size-aware carried-stack prologue/epilogue: an enum
  slot carries its whole aggregate (tag + payload) across a word-call or REPL line
  boundary. Scalars and structs unchanged.
- **`dup`/drop for enums.** `dup` copies the aggregate (Copy); drop is a no-op.

## Print + REPL

- `.`, `=`, and arithmetic on an enum route to the sharp-error path (M2).
- REPL residual display renders an enum slot as its `<TypeName>` placeholder (M4),
  reusing the struct placeholder path; scalar display unchanged.

## Out of scope (deferred)

The `_` wildcard / catch-all clause; any inline `match ... end` keyword form; open
multimethods and static overloading (`generic:`/`method:`, dispatch on statically-known
types) (Phase 4); generics / type parameters, so no generic `Option<T>` / `Result<T,E>`
(monomorphic hand-written enums only this slice) (Phase 4); making `Bool` a library enum
(Phase 4); the `Copy` marker and optional / non-null pointers (Slice 7); recursive /
heap data types (need pointers); references / borrows / in-place mutation (Phase 3);
the `?` error short-circuit sugar (Phase 5); struct/enum auto-derived `=` or printing;
mid-body (non-top-of-scope) locals; the bytecode-VM dogfood (Slice 6).

## Goal and exit criteria

Deliver user-declared sum types and their exhaustiveness-checked, clause-style
eliminator: enum declaration, per-variant constructors, clause-style word definition
with payload on the stack, exhaustiveness and branch-join as sharp compile errors, the
`then`->`end` closer rename, clause-body locals, and enum values crossing word/REPL
boundaries, all heap-free and monomorphic. All prior goldens stay green (with the
mechanical `then`->`end` migration); scalar and struct behaviour is otherwise unchanged.

**Exit:**

1. A `type:` declaration with `|` variants parses and registers an enum usable in stack
   effects and word bodies; a `type:` with no `|` still parses as a Slice 3 struct; all
   Phase 0/1/Slice-1/2/3/floats/bitwise/bool goldens still pass.
2. Variant constructors build values in a native binary for a zero-field variant and a
   multi-field variant; the discriminant and payload are correct.
3. A clause-style word eliminates an enum end-to-end in a native binary: it dispatches
   to the correct clause and the variant's payload is on the stack in declared order
   (first deepest), for a multi-field-variant enum and a zero-field-variant enum, with
   a value flowing underneath the scrutinee handled correctly.
4. Exhaustiveness and shape are sharp behavioural diagnostics (assert message text and
   the variant/type names): non-exhaustive clause word (names the missing variant),
   duplicate clause, unknown variant, clause-style body on a non-enum top input, and a
   clause meeting/violating the declared output effect.
5. `if ... else ... end` works and `then` is gone; every migrated golden passes,
   confirming the rename is behaviour-preserving.
6. A clause-body `| names |` block binds the payload by name (extent = the clause);
   word-entry locals are unchanged; a local named exactly like a registered variant is a
   sharp error.
7. An enum value survives a word-call boundary and a REPL line boundary (size-aware
   carried-stack marshalling), including a variant with a large payload; the REPL shows
   `<TypeName>`.
8. Diagnostics are behavioural for `.`/`=`/arithmetic on an enum, a recursive enum
   (compile error, not a hang), and a malformed/zero-variant declaration and a
   constructor arity/type mismatch; and the dogfood `examples/shapes.sth` runs correctly
   as a native binary and in the REPL.

## Dogfood (S8-style)

`examples/shapes.sth`:

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

Exercises: a multi-field-variant enum with float payloads and clause-body locals; a
zero-field variant (`None`) whose clause body is empty (tests the D8 empty-clause
disambiguation, since the following `| Some` is a known variant); a value flowing under
the scrutinee (`unwrap-or`'s default). Runs native and in the REPL. Plus the headline
negatives: a non-exhaustive `area` (missing `Rect`), `.` on a `Shape`, and a recursive
`type:` rejected.

## Current-state codebase anchors (post Slice 3, on `main`, after the `a0c6217` dedupe)

- `src/lexer.rs`: `is_delimiter` matches `; ( ) |` (`:` dropped in Slice 3); `|` is
  already a standalone token. Only the `then`->`end` keyword rename touches control
  flow, and it is parser-level.
- `src/ast.rs`: `Type` (`Int`/`Float`/`Bool`/`Struct(StructId, name)`); the shared free
  function `resolve_type_name(&[StructDecl], name)` (the Slice 3 dedupe; `Module::
  resolve_type_name` is a thin wrapper; `Module::struct_decl` and `Type::is_struct` were
  removed); `StructDecl`; `StackEffect`; `TermKind` (`IntLit`/`FloatLit`/`BoolLit`/
  `Call`/`If`); `WordDef` with `locals: Vec<String>`. The `Type::Enum` variant, the
  `EnumDecl`/`VariantDecl` nodes, the enum registry, the clause-carrying `WordDef` body,
  and the extension of the shared resolver land here.
- `src/parser.rs`: `worddef := ':' Word '(' effect ')' locals? term* ';'`; `locals :=
  '|' Word* '|'` (once, after the effect); the `type:` struct production and the
  struct-name pre-pass. Add enum variants to `type:` and to the pre-pass, the clause-
  style word body, and the `then`->`end` rename.
- `src/check.rs`: struct registration, `compute_layout`/recursion checks, generated-
  word signatures (`struct_generated_sigs`), the `print_requires_printable` guard, the
  recursion DFS (`VisitState`), located diagnostics naming both types; word-entry local
  binding (`Ctx::Word { locals }`, the `locals bind N ... inputs` check). Enum
  registration, variant constructors, clause-style exhaustiveness/branch-join checking,
  enum recursion, enum diagnostics, and clause-body locals land here.
- `src/ir.rs`: `compute_layout` (offsets via `round_up`/`div_ceil`, `scalar_size_align`);
  `carried_slot_bytes`; `Structs::from_structs`; first-field-deepest construction/
  destructure; the getter-aliasing-soundness note; `lower_line` carried-slot marshalling;
  `IrType`; the `if` branch lowering. Enum `IrType`/tagged layout, variant construction,
  and clause dispatch (tag load + branch + payload load) land here, reusing the layout
  and marshalling machinery.
- `src/backend/qbe.rs`: QBE aggregate type emission, `alloc`, field load/store, `blit`,
  branch lowering. Enum aggregate emission and dispatch reuse these.
- `src/repl.rs`: the `<TypeName>` residual placeholder and per-declaration `Box::leak`.
  Enums reuse the placeholder.
- `examples/`: `gcd`, `factorial`, `lerp`, `sign`, `bool_abi`, `rgb`, `rgb_bits`, `mean`,
  `leap`, `vectors`. Migrate `then`->`end` in the six that use `then`; add `shapes.sth`.

## Key risks

- **Clause-vs-locals `|` disambiguation (D8)** is the load-bearing parser change: the
  first `|` of a word body, empty clause bodies, and clause-body locals must all parse
  unambiguously via the variant pre-pass, with a local-named-like-a-variant rejected.
  Cover with goldens: an empty clause followed by another clause; a clause-body local; a
  term word with entry locals still parsing unchanged; and a local-named-like-a-variant
  rejected.
- **Tagged layout** (D3): tag + payload offsets/size/alignment across variants of
  differing size and alignment; the largest variant sizes the payload; mixed int/float
  variant fields read back correctly. Per-variant runtime goldens.
- **Clause dispatch codegen**: discriminant load, branch/jump to the right clause, and
  per-variant payload load onto the stack (first deepest); verify at runtime, not by
  reading IL.
- **`then`->`end` rename must be behaviour-preserving**: only the keyword changes; no
  golden's semantics move.
- **Recursion detection** must terminate and report on a self-referential enum.
- **Carried-stack marshalling** already generalized in Slice 3; confirm an enum
  (especially a large-payload variant) crosses word and REPL boundaries intact.
