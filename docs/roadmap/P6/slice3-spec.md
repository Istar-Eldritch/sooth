# Spec: Phase 6 Slice 3 — the eliminator word

**Status:** Implemented
**Created:** 2026-08-17
**Timestamp:** 2608171659

Pairs with [slice3-brief.md](./slice3-brief.md) (discovery). Consumes Slice 2's
`Type::Variant`/accessors and P7.S1's receiver-directed `&field` projection; migrating
`examples/vm.sth`/`Bool`/`Result`/`Option` off clause-style dispatch and deleting
`WordBody::Clauses`/`parse_clauses` is **Slice 4**.

## What shipped

A generated per-enum eliminator word `Enum?` taking one quotation-literal arm per variant,
each routed to its variant by an annotation tag (`( Circle )`), not by slot position.
Missing, duplicated, unknown-variant, untagged, and stray-tagged arms are all named errors.
The scrutinee may be owning or a reference; the mode is a property of the call and every
arm's expected effect is built in it. The call lowers to the existing N-way tag dispatch
(`lower_clauses`) under a new arm-population mode. Three `.sth` goldens.

Every clause-style path (`WordBody::Clauses`, `parse_clauses`, `check_clause_word`,
`lower_clauses` under `ArmBinding::Decompose`) is untouched behaviourally.

## Requirements (as delivered)

- **R1 — arm-annotation grammar** (`src/ast.rs:1664`, `src/parser.rs:2564`).
  `QuotAnnot.variant_tag: Option<String>`. `parse_leading_variant_slot` consumes a leading
  `Variant`/`&Variant`/`&!Variant` token *only if* the sigil-stripped word resolves to a
  known variant, records the **bare** name as the tag (routing never carries a sigil), and
  returns the slot's declared type — owning, or through the same `intern_ref_type` an
  ordinary reference slot uses. `( Circle )` alone (one token, then `)`) is the sole place
  the `--` is elidable; `( Circle Push )` is still R6's located arrow-missing error, as is
  `( )`.
  - Resolution is **module-scoped** (`resolve_variant_type`, `find_variant_type_in_module`):
    own module first, then a selectively-imported variant's target module, matching on
    `name_static` (slice 5b R8d: a REPL-spliced enum's `.name` carries an import epoch, its
    variants' `name_static` does not). An in-scope struct/enum of the same name takes
    precedence — a variant name is a routing tag only where no ordinary type resolves. A
    generic enum's variant does not resolve: a bare token supplies no type arguments.
- **R2 — first declaration-time `PolySig` generator** (`enum_eliminator_sigs`,
  `src/check/declarations.rs:1412`): `( ..a Enum ~[ ..a Enum.V1 -- ..b ] … -- ..b )`. Arms
  are `is_inline = true` (matching `if`/`branch`; a materializable `[` arm would reach
  `lower_clauses` as a runtime `(code, env)` value it cannot splice), inputs built through
  `variant_type`, and the **minimal** subset: two row vars, no bounds, no len vars, no ty
  vars — nothing per-arm unifies. D7 keying: bare surface env key, mangled lowering symbol.
  This sig is registration only; `check_eliminator_call` intercepts before it is ever
  unified against, which is why its arm inputs are owning regardless of call mode.
- **R3 — registry + interception** (`eliminator_registry`,
  `src/check/declarations.rs:1465`; `src/check/terms.rs:495`). Bare surface `Enum?` →
  `EnumId`, threaded on `PolyCtx.eliminators`, consulted ahead of the env/combinator/poly
  paths so an eliminator is never spliced as a `Combinator`.
  - **Known gap, unreachable today:** the registry key is `generic_surface_name`-only, so
    `Result[i64 i64]?` and `Result[bool i64]?` both collapse to `"Result?"`, last write
    wins. Unreachable while generic-enum elimination cannot parse at all (the standing
    blocker); whoever closes that parse gap must key this by the mangled spelling.
- **R4 — `check_eliminator_call`** (`src/check.rs:2153`). Calls, never re-implements:
  `check_literal_against_declared_effect` per arm, `combinator_branch_output_mismatch_error`
  for disagreement. In order:
  1. **Variable-arity collection.** Pop while the top is a quotation *literal* carrying a
     tag; the first operand that is not is the scrutinee. A fixed `1 + variant_count` pop
     could not distinguish a missing arm from a short stack, making the exhaustiveness
     message unreachable. A forwarded abstract quotation carries no annotation, so it is
     rejected exactly like an untagged literal (`eliminator_untagged_arm_error`) — an
     abstract-quotation arm is a deferred capability, not an oversight.
     `arms.reverse()` undoes pop order once: both later passes walk **written** order.
  2. **Scrutinee mode** via `ref_parts`: `Enum` / `&Enum` / `&!Enum`, one mode per call,
     applied uniformly. Anything else is the ordinary `type_mismatch_error`.
  3. **Exhaustiveness + duplication pre-pass** before any body is checked, adapted from
     `check_clause_word`. Diagnostics name the variant and the enum — the enum name is run
     through `demangle_word(generic_surface_name(..))`, since an `EnumDecl.name` is the
     per-module mangled spelling and stripping `[...]` alone would leave `Shape__m0`.
  4. **Per-arm body.** Expected input is `variant_type(id, vi)`, wrapped by
     `intern_ref_type` in reference mode. An arm annotation spelling the wrong mode is
     rejected by the shared declared-vs-written comparison — no mode-specific diagnostic.
     The arm receives the caller's **own** scrutinee slot retyped to the narrowed variant,
     not a provenance-free one: otherwise a reference projected inside an arm left the call
     unrooted, and a second independent `&!` to the same place was accepted alongside it.
  5. **Cross-arm agreement.** Written-first arm sets the `..b` baseline (`arm_exit_row_mismatch`
     is a split-out pure function so a test can pin the `expected`/`found` pairing
     structurally — two `Type`s can `Display` identically). Type agreement is not enough:
     `merge_arm_output_slot` reconciles borrow suspension per position and rejects a
     disagreement rather than erasing it.
  6. **Variant escape is rejected** (`eliminator_variant_escape_error`): no `Type::Variant`
     or reference to one may leave the call. Only a single-variant enum gets this far, but
     it is unsound, not untidy — every type predicate outside the eliminator is written over
     `Type::Enum`, so `is_copy` reads an escaped variant as trivially `Copy` and `dup`
     double-drops its payload.
  7. **Outputs.** The merged baseline, provenance included; each arm is checked against its
     own `scope` clone and every arm's real move-state is joined into `scope`
     (`Moves::join` reduced over N arms). No `Subst`: there is no back-edge to ground.

  Two supporting changes fell out: `LiteralBoundary` gained `finalize` (arms need their real
  consumed move-state to survive, since nothing later re-checks the arm that runs — every
  pre-existing caller passes `finalize: false`), and a **tagged literal that never reaches
  an eliminator call is now an error** (`tagged_literal_reaches_an_eliminator_call`,
  `src/check/terms.rs:926`). The skip that lets an arm defer its annotation check to the
  call site otherwise silently left a typo'd-call or stray tagged literal unchecked
  entirely. The rule is **written adjacency** (a run of tagged literals ending in an
  eliminator call), deliberately stricter than the stack-based collection: the looser rule
  reopens the hole.
- **R5 — `EnumWord::Eliminate`** (`src/ir/layout.rs:584`, `src/ir/func_builder/calls.rs:729`,
  `control_flow.rs`). `lower_clauses` gained `ArmBinding` (`Decompose` = today's per-field
  loop, byte-for-byte for every existing caller; `WholeValue` = skip decomposition, push the
  whole aggregate/reference the arm's `&field` projections address). `lower_enum_call`
  intercepts `Eliminate` ahead of `lower_enum_word`, whose `Eliminate` arm is `unreachable!`
  *because of* that ordering — a stated invariant, not a filler arm. Arms become synthetic
  `Clause { locals: vec![], .. }` keyed by tag; the call's mode is recovered from the first
  arm's declared receiver type (`quot_arm_tags`), needing no new checker→IR side table.
  - **Two mid-body corrections** an eliminator call forced on `lower_clauses`, which
    previously only ever lowered a whole word body: locals are truncated to the entry depth
    per arm and restored after, instead of `clear()`ed (a caller's `| k |` was being wiped),
    and arm tail position is `tail && self.header.is_some()` (an arm of a mid-body call must
    emit a real call, not back-edge past the rest of the enclosing word).
  - OQ4: no degenerate special-casing. One variant takes the general path; a zero-variant
    enum has no constructible value.
- **R6 — variant-projection lowering.** `resolved_variant_fields: HashMap<Span, (EnumId,
  usize, usize)>` (`src/ast.rs:90`) mirrors P7.S1's `resolved_fields` threading exactly
  (`PolyCtx` scratch → `Module` → `ir/driver.rs` → `FuncBuilder`), populated by the one line
  P7.S1 left out (`src/check/word_families.rs:408`) and read in `lower_reference_word`'s `_`
  arm at `payload_offset + field.offset`. `EnumWord::Destructure(EnumId, usize)` is
  registered through the struct registry's dual-key closure and lowered in `lower_enum_word`.
  `ir_type_of(Type::Variant(id, ..)) == IrType::Enum(id)` (`src/ir/types.rs:290`) replaces
  the `unreachable!`: a reference-mode arm's declared `&Shape.Circle` interns a real
  `RefDecl`, and `ref_referents` converts **every** interned referent at build time whether
  or not it executes.
  - `Destructure` — not `Eliminate` — is the second `EnumWord` variant, so it is Phase 3
    that turned `control_flow.rs`'s irrefutable `let EnumWord::Construct(_, vi) = …` into a
    `let … else … unreachable!`.
- **R7 — goldens** (`tests/phase6_slice3.rs`): `examples/eliminator.sth` (owning: first
  program to put a `Type::Variant` on the stack from surface syntax), `eliminator_ref.sth`
  (reference: forces `ir_type_of(Type::Variant)` at build time, and shows the call consumes
  nothing — `grow` mutates in place through `&!`, `area` reads it after), and
  `eliminator_mid_body.sth` (a local bound before the call survives inside an arm and past
  it). All assert program output.

## Placebo-proofing that shaped the tests

- Owning golden uses variants with **different field counts**, so a mis-routed arm reads
  wrong offsets rather than a wrong value.
- The projection lowering test's two variants differ in field *type* at index 0 with both at
  offset 0, so the `ref_inner` type assertion — not the address assertion — is what catches
  a wrong variant index.
- `Destructure` is tested on a multi-field variant *and* a zero-field one: the zero-field
  case alone cannot catch a mutation that always pushes nothing.
- `ir_type_of` is a positive equality against the parent enum's erasure, not "does not
  panic".
- Baseline ordering has two tests: written-vs-declaration order, plus one where written,
  declaration, and stack-pop order are pairwise different (the first cannot see a missing
  `arms.reverse()`).
- `_missing_arm_is_error_not_underflow` breaks if collection regresses to a fixed-arity pop.

## Success criteria (met)

- [x] `Enum?` checks, lowers, and runs in owning and reference mode; arms routed by tag.
- [x] Missing / duplicate / unknown / untagged / stray-tagged arm and variant escape each
      produce their own named diagnostic.
- [x] `ArmBinding::Decompose` leaves every clause-style caller's behaviour unchanged.
- [x] `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green per phase.
- [x] No `EnumLayout`/`VariantLayout`/`dispatch_on_tag` change; no change to Slice 2's
      `Type::Variant` or P7.S1's struct projection.

## Deferred (with reasons)

- **Migration** of `examples/vm.sth`, `Bool`, `Result`/`Option`; deleting
  `WordBody::Clauses`/`parse_clauses`: **Slice 4**.
- **Forwarded abstract-quotation arms.** Routing needs a tag, and a non-literal operand has
  no annotation to carry one. Rejected explicitly rather than accepted and left to ICE.
- **Generic-enum elimination.** Blocked upstream at parse (`( Ok )` → "unknown type Ok");
  the registry collision above waits behind it.
- **Arm-name attribution in the output-mismatch message.** `combinator_branch_output_mismatch_error`
  names two shapes and a line; extending it forks a helper the live `if`/combinator path
  depends on. Its own item, with its own test.

## As-built map

| Location | Symbol |
|---|---|
| `src/ast.rs:1664` | `QuotAnnot.variant_tag` |
| `src/parser.rs:2564`, `:2652` | `parse_leading_variant_slot`, `resolve_variant_type` |
| `src/check/declarations.rs:1412`, `:1465` | `enum_eliminator_sigs`, `eliminator_registry` |
| `src/check/terms.rs:495` | eliminator interception, ahead of env/combinator/poly |
| `src/check/terms.rs:926`, `:954` | `tagged_literal_reaches_an_eliminator_call` + its error |
| `src/check.rs:2153` | `check_eliminator_call` |
| `src/check.rs:2414`, `:2440` | `arm_exit_row_mismatch`, `merge_arm_output_slot` |
| `src/check.rs:2479+` | the four eliminator diagnostics + variant-escape |
| `src/check.rs:1875` | `LiteralBoundary.finalize` |
| `src/check/word_families.rs:408` | `resolved_variant_fields` insert (P7.S1's no-op, closed) |
| `src/ir/func_builder/control_flow.rs:16`, `:213` | `ArmBinding`, `lower_clauses` |
| `src/ir/func_builder/calls.rs:729`, `:743` | `lower_enum_call`, `lower_eliminator` |
| `src/ir/func_builder/word_families.rs:79` | variant projection lowering |
| `src/ir/layout.rs:584` | `EnumWord::Eliminate` registration (dual-key) |
| `src/ir/types.rs:290` | `ir_type_of(Type::Variant)` → `IrType::Enum` |
| `examples/eliminator{,_ref,_mid_body}.sth`, `tests/phase6_slice3.rs` | goldens |

## References

- [slice3-brief.md](./slice3-brief.md) — discovery, recon, settled decisions 1–6.
- [slice2-spec.md](./slice2-spec.md) — `Type::Variant` and the accessors this consumes.
- [P7/slice1-spec.md](../P7/slice1-spec.md) — the receiver-directed projection R6 re-points at.
- [ROADMAP.md](../ROADMAP.md) — Phase 6 plan.
