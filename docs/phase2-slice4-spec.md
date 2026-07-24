# Phase 2 Slice 4 — enums / ADTs + clause-style pattern matching

**Status: implemented.** Slice 4 of Phase 2 (typed core), on the scalar core + Slice 3 structs. Added user-declared sum types and their exhaustiveness-checked eliminator. Design locked in `phase2-slice4-brief.md` (D1–D10, M1–M6).

## What shipped

- `type:` extended with `|`-separated variants (D1); each variant = name + zero or more `name type` field pairs; optional leading `|`; `;` terminates; ≥1 variant.
- Per-variant constructor words yielding the *enum* (D2); the Slice 3 struct is the degenerate one-variant case.
- Tagged inline-aggregate representation (D3): one virtual-stack slot = QBE aggregate of a fixed-width `i32` tag (M1) + a max-variant payload, laid out by reused Slice 3 layout machinery. Heap-free, monomorphic, all-Copy (`dup` copies bytes, drop is a no-op).
- Clause-style word definition (D4): a word whose top input is an enum is defined by `|`-led clauses, one per variant, no inline `match`. A clause consumes the scrutinee, pushes the matched variant's fields (first deepest, atop inputs below), runs its body.
- Exhaustive, exact-coverage elimination; branch-join folded into the single declared output effect (D5, M6).
- `then` → `end` control-flow-closer rename (D6), behaviour-preserving; `end` reserved; `if` stays inline.
- `| … |` locals extended to the top of clause bodies (D7).
- Enum values cross word-call and REPL boundaries via reused size-aware carried-stack marshalling.

**Out of scope:** generics, `Option<T>`/`Result<T,E>`, open multimethods, static overloading, `Bool`-as-enum, `_` wildcard, inline `match`, recursive/heap data, `Copy` marker, references, `?` sugar, auto-derived `=`/print, mid-body locals.

## Locked decisions

- **D1–D3.** Enum via `type:` form; variant constructors yield the enum; tagged inline aggregate (Model B), frame-local `alloc`, all Copy, no `Copy` syntax.
- **D4–D5.** Clause-style elimination (not inline `match`); exhaustive, exact coverage, no `_`; missing/duplicate/unknown variant are distinct sharp located errors; every clause checked against the word's single declared effect (no separate join pass).
- **D6.** `then` → `end`; behaviour-preserving; migrate live examples/tests/docs, leave archived per-slice specs on `then`.
- **D7.** Clause-body `| … |` locals (name payload + stack below), extent = the clause; word-entry locals unchanged; no mid-body binding.
- **D8.** Clause-vs-locals disambiguation via the variant pre-pass: `|` + known variant name = clause; `|` + non-variant words closed by `|` = locals. A local/param named as a registered variant is a sharp error. Clause-style words have no word-entry locals.
- **D9.** Aggregates nest freely (any sized value type is a field of any aggregate; struct fields may be enums), sized via the combined registry. Value-recursion forbidden → located layout error, must terminate.
- **D10.** Separate enum registry, shared layout machinery. `Type` gains `Enum`; layout / field load-store / carried-slot / recursion-DFS extracted and generalized to a combined struct+enum graph. Behaviour-preserving: struct goldens unchanged, but shared core moves under both.
- **M1.** Tag = fixed-width `i32` discriminant = declaration index, placed first; payload at max-variant alignment; not user-visible.
- **M2.** `.`, `=`, arithmetic on an enum are sharp located errors naming the enum type.
- **M3.** Single-variant enum allowed (newtype); zero-variant is a malformed-declaration error.
- **M4.** REPL residual display = `<TypeName>` placeholder (reuses struct path).
- **M5.** Recursion/infinite-size detection is a located compile error naming the cycle; terminates.
- **M6.** The clause word's declared output effect is the single join target; clause bodies are ordinary term sequences and may use `if/else/end`.

## Requirements by stage

**Frontend (ast/lexer/parser).** R1 `then`→`end` rename (lexer unchanged, `end` matched positionally, reserved via guard). R2 AST enum nodes (`Type::Enum(EnumId,&str)`, `EnumDecl`, `VariantDecl`, `EnumId`, `Module.enums`). R3 shared `resolve_type_name(structs, enums, name)`: scalars → structs → enums. R4 `WordBody` enum (`Terms{locals,terms}` | `Clauses(Vec<Clause>)`; `Clause{variant,locals,body,span}`). R5 variant pre-pass registers all variant names before word bodies parse. R6 enum `type:` production (variants + fields, malformed/zero-variant errors). R7 clause-style body + D8 disambiguation (empty clause bodies, clause-body locals, no word-entry locals).

**Checker (`check.rs`).** R8 registration + duplicate-name check across both registries (X2). R9 variant constructor `Sig`s `( T1…Tn -- Enum )` (X9). R10 combined-graph value-recursion detection via `VisitState` (X3/M5). R11 clause elimination checking: top input must be enum (X7); exact coverage — missing (X4)/duplicate (X5)/unknown (X6); each clause checked against declared outputs (X8); clause-body locals bind top N; variant-name collision (X12). R12 enum operator/print guards (X10).

**IR + backend (`ir.rs`, `backend/qbe.rs`).** R13 `IrType::Enum` + `EnumLayout`/`VariantLayout` via reused field-placement core (i32 tag first, payload at max-variant alignment, word-width-neutral); combined-registry nested sizing; `carried_slot_bytes`/`scalar_size_align` gain Enum arms. R14 enum registry + variant-word map threaded into lowering (`lower_call`), not merged with structs. R15 opaque-blob QBE aggregate (`type :E = align A { b N }`); variant constructor = `Alloc` + tag `FieldStore` + field stores (first deepest); `dup` blits, drop no-op. R16 clause dispatch: discriminant into a *temp* (not pushed), N-way `Cmp(Eq)`-tag chain via `Jnz`, exhaustiveness makes the last variant terminal fall-through (no default/trap), per-variant first-deepest payload load, single join block with one `Phi` per declared output merging all N predecessors — an N-predecessor/M-output join, *not* the 2-pred `lower_if`. Single-variant enums skip the discriminant load. R17 marshalling: enum slot treated as struct slot (blit at `carried_slot_bytes`).

**REPL (`repl.rs`).** R18 `Session.enums` registry; `eval_typedef` branches on `Pipe`; append-then-rollback discipline; D8 variant set at REPL scope = union of session's accumulated + current line's variants; `typed_env` adds constructor sigs; `format_stack` renders `<TypeName>` and advances by enum size.

## Diagnostics (assert message text + type/variant names)

X1 unknown variant field type · X2 duplicate type name across registries · X3 recursive enum/infinite size (terminates) · X4 non-exhaustive (names missing variant) · X5 duplicate clause · X6 unknown variant in clause · X7 clause body on non-enum top input · X8 clause violating declared output effect · X9 constructor arity/type mismatch · X10 `.`/`=`/arithmetic on enum · X11 malformed/zero-variant declaration · X12 local/param named as a registered variant.

## Success criteria → goldens (runtime/behavioural, not IL)

1. Declaration + registration, no regressions (X1, X2; migrated suites green).
2. Constructors build correct values natively, incl. an align-8 padded payload (exercises round-up).
3. Clause elimination end-to-end natively (`area` multi-field, `unwrap-or` zero-field + value under scrutinee) + nested-aggregate golden (struct payload in variant; enum as struct field).
4. Exhaustiveness/shape diagnostics (X4–X8).
5. `then` gone, `end` works; negative `then`-unmatched test.
6. Clause-body locals + X12 collision; term-word-with-entry-locals still parses.
7. Enum crosses word + REPL boundaries incl. large payload (multiple i64s); REPL shows `<TypeName>`; scripted REPL session declares enum then defines a clause word on a *later* line (REPL-scope D8 seeding).
8. X10, X3 (direct + mutual cycle), X11, X9; `examples/shapes.sth` native + REPL; headline negatives (non-exhaustive `area`, `.` on `Shape`, recursive `type:`).

## Dogfood — `examples/shapes.sth`

```
type: Shape | Circle r f64 | Rect w f64 h f64 ;
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
  5 None unwrap-or .         \ 5
  5 7 Some unwrap-or . ;     \ 7
```

Exercises float-payload multi-field variants + clause-body locals; a zero-field variant with an empty clause body (D8 empty-clause disambiguation); and a value flowing under the scrutinee.

## Non-functional

Green each phase (`cargo fmt --check && cargo clippy -- -D warnings && cargo test`). Invariants held: QBE-only backend; backend-neutral IR (tag is fixed `i32`, not machine word; QBE spellings derived only in backend); frontend `Type` / backend `IrType` stay distinct; no JIT (REPL via `dlopen`); `core` `no_std`. Reuse Slice 3 layout/carried-slot machinery — no new memory model. Verify by native + REPL goldens, not IL assertions.

## Implementation summary

- **Phase 1** (`then`→`end`): `561c45aa`, docs `14e023fa` — DESIGN, README, examples, `backend/qbe.rs`, `check.rs`.
- **Phase 2** (enum frontend + checker registration): `4106d0a1` (delegate struct dup-check), `7d1f342c` — `ast.rs`, `check.rs`, `ir.rs`, `parser.rs`.
- **Phase 3** (tagged layout + construction + marshalling): `140a31a6`, native golden `7016da76` — `backend/qbe.rs`, `check.rs`, `ir.rs`, `parser.rs`, `repl.rs`, `tests/`.
- **Phase 4** (clause elimination): `12ea619a`, single-variant discriminant-skip fix `9ed63d9a`, if/else/end-in-clause test `e8da9639` — `ast.rs`, `check.rs`, `ir.rs`, `parser.rs`, `repl.rs`, `examples/shapes.sth`, `tests/`.
- **Phase 5** (dogfood + REPL plumbing + full suite): `e4503620`, REPL shapes golden `95ede5c2` — README, ROADMAP, `tests/phase1.rs`.

## Key risks (all runtime-verified)

D8 `|` disambiguation (empty clauses, clause-body locals, variant-named local rejected); tagged layout across differing-size/alignment variants; N-way clause dispatch (3+ variants, not 2-pred `lower_if` shape); opaque-blob aggregate ABI/alignment; behaviour-preserving rename; recursion termination (self + mutual); large-payload marshalling across word/REPL boundaries.
