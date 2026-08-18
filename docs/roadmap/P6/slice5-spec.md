# Spec: Phase 6 Slice 5 — nested tag paths

**Status:** Ready to implement
**Created:** 2026-08-18

Pairs with [slice5-brief.md](./slice5-brief.md) (discovery). Extends the eliminator
shipped in [slice3-spec.md](./slice3-spec.md) and the check-time tag resolution shipped in
[slice3b-spec.md](./slice3b-spec.md). This slice is **surface sugar**: an arm tag may name
a path through nested enums, and the checker desugars it to the exact nested eliminator
call a program writes by hand today. No dispatch lowering, no `ArmBinding`, and no codegen
change (brief decision 1). If the implementation needs a backend change, decision 1 was
wrong and the slice stops.

## Recon confirmation (measured against the tree at `9c3b6ca`, 2026-08-18)

The brief's recon 1 was re-run directly; decision 1's premise holds. The desugar target
(an outer eliminator arm that projects a payload enum and calls a second eliminator over
it) builds and runs in every mode this slice emits:

- **Owning, two levels.** `type: Shape | Circle r i64 | Rect w i64 h i64 ;` and
  `type: Wrapped | Has v Shape | Empty ;` with
  `~[ ( Has ) Has> area ] ~[ ( Empty ) drop 0 ] Wrapped?` (`area` an ordinary
  `( Shape -- i64 )` eliminator). Built clean, printed `75 / 12 / 0`.
- **Reference, two levels.** `~[ ( &Has ) &v area ] ~[ ( &Empty ) drop 0 ] Wrapped?`
  with `area ( &Shape -- i64 )`. Built clean, printed `5`.
- **Generic outer, enum payload (OQ1's obligation).** `type: Option 'T | Some v 'T | None ;`
  with `~[ ( Some ) Some> area ] ~[ ( None ) drop 0 ] Option?` over `Option[Shape]`. Built
  clean, printed `75 / 0`. Generic elimination did not parse before Slice 3b; it does now.
- **Multi-field product (decision 6).** `type: Pair | Both a Shape b Shape | Neither ;`,
  routing on both fields via `Both>` then two nested `Shape?` calls with a `swap` between
  them (see R6 for why the reorder is load-bearing). Built clean, printed `17 / 0`.
- **Three levels (OQ3's obligation).** `Outer -> Inner -> Shape`, three nested eliminator
  calls. Built clean, printed `5 / 0 / 0`.

Two findings that shape the spec:

1. **A payload-free inner arm must consume its variant explicitly.** The owning and
   reference probes both failed first with `an arm of Wrapped? leaves Wrapped.Empty on the
   stack ... consume it there`, fixed by `~[ ( Empty ) drop 0 ]`. The linear spine does not
   auto-drop; the desugar must not paper over it (OQ5, R7).
2. **The self-named-variant collision is reachable as a type but not as an outer tag.**
   `type: Box | Box v i64 | Nil ;` builds, but `~[ ( Box ) ... ]` at the leading slot is
   *not* a routing tag: `variant_name_is_visible` (`src/parser.rs:2812`) gives an in-scope
   type name precedence over a same-named variant, so `( Box )` parses as an ordinary
   annotation and fails the missing-`--` rule. The collision this slice must rule on
   (OQ2) is the same name appearing as a *bracket segment*, resolved by new check-time code
   (R4).

## Decisions settling the open questions

- **D-A (OQ1) — the separator is the bracket, resolved against surface names.** Settled by
  brief decisions 6 and 7; the spec's job is the test obligation, met by the `Option[Shape]`
  golden (R9/G3). `( Some Circle )` (space-separated) stays illegal under the existing
  arm-position elision rule (it reads as two stack inputs, then the missing-`--` error).
- **D-B (OQ2) — the type form wins the segment tiebreak.** When a bracket segment's
  identifier names *both* a variant of the field's enum and the field's own declared type
  (only possible when an enum declares a variant named after itself, `type: Box | Box … ;`),
  it reads as the **type form** (leave the field whole). **Reason:** consistency with the
  parse-time precedent (`variant_name_is_visible`, recon finding 2), where an in-scope type
  name already beats a same-named variant; a reader who knows that one rule predicts this
  one. **Consequence:** a variant named after its own enum cannot be individually routed by
  a path segment; route it with a whole `( field-var )` arm carrying a hand-written inner
  eliminator, exactly as today. Tested directly (R9/T-OQ2).
- **D-C (OQ3) — arbitrary depth by construction.** The synthesis is recursive: a routed
  field's segment may itself carry a bracket, and R3's synthesis re-enters. Confirmed to
  compose to three levels (recon). Required goldens: two-level (G1/G2) and three-level
  (G5).
- **D-D (OQ4) — recognition at parse, resolution and synthesis at check.** The parser
  recognizes the bracket and captures its field/selector *tokens* without resolving them
  (a segment's variant-vs-type reading needs the field's instantiated enum, which is
  check-time knowledge, Slice 3b recon 5). The parser owns only *syntactic* diagnostics
  (E9). The checker owns every *resolution* diagnostic (E1–E8) and all synthesis. **Hard
  requirement:** a synthesized inner eliminator call reports against the **written** arm's
  span, never a synthesized one (R3, R8-span).
- **D-E (OQ5) — the synthesized projection changes no drop obligation.** The projection
  the desugar emits leaves the same linear obligations in the same places as the hand form;
  a payload-free inner variant arm consumes its variant explicitly (recon finding 1).
  Witnessed by a payload-free inner arm inside G1 and G2.

## Requirements (traceable)

- **R1 — AST: `VariantTag` carries a field path** (`src/ast.rs:1794`). Extend
  `VariantTag { name, mode }` to `VariantTag { name, mode, fields: Vec<FieldRoute> }`, with
  `FieldRoute { name: String, sel: String, nested: Vec<FieldRoute> }`. `fields` empty is a
  plain arm (today's behaviour, byte-for-byte). A non-empty `fields` is a path arm: each
  `FieldRoute` names one field of the routed variant (`name`), the identifier written for it
  (`sel`, resolved at check to a variant or the field's type), and any further descent
  (`nested`, non-empty only when `sel` resolves to a variant). `name` remains the bare outer
  variant spelling. `mode` continues to carry the outer scrutinee mode; inner modes are
  derived (R6), not spelled.

- **R2 — parser: recognize and capture the bracket** (`parse_leading_variant_slot`,
  `src/parser.rs:2791`). After a recognized leading variant name, if the next token opens a
  `[`, parse a non-empty sequence of `FieldRoute`s: each is a field-name identifier followed
  by a selector identifier, the selector optionally followed by its own `[ … ]`. No type
  resolution here; the identifiers are captured verbatim. The lone-name form and the
  escalated `( V -- … )` form are unchanged. Recognition of the outer name stays exactly
  `variant_name_is_visible` (module-scoped, type-name-precedence); the bracket does not
  change whether the leading token is a tag.

- **R3 — check: group and synthesize** (`check_eliminator_call`, `src/check.rs:2193`). After
  arm collection (`src/check.rs:2232`) and scrutinee/`EnumId` resolution (`src/check.rs`
  `ref_parts`/family gate), partition arms into plain (`fields` empty) and path arms. Group
  path arms by outer variant `name` in written order. For each group, synthesize one outer
  arm whose body is: the mode-correct projection of each routed field (R6), followed by the
  nested eliminator call(s) over the routed fields, whose arms are the group's descents;
  recurse for `nested`. The group's original written arm bodies become the innermost leaf
  arm bodies, re-tagged with their leaf variant. The synthesized outer arm then flows
  through the existing per-arm body check unchanged, so lowering, `ArmBinding`, and codegen
  are untouched (decision 1).
  - **Span discipline (D-D).** Each leaf-arm
    `QuotBody.span` is the **written** arm's span, so `check_eliminator_call`'s
    unknown/duplicate/exhaustiveness diagnostics (which read `prov.quotations[qid].span`)
    point at written source. Synthesized *projection* terms must carry **distinct**
    synthesized spans, never the written arm's span reused N times: `resolved_variant_fields`
    (`src/ast.rs:90`) is keyed by `Span`, so reusing one span across a variant's projections
    collides in that map and misroutes field offsets. The spec requires distinct projection
    spans and a test that a two-field owning group reads both fields' offsets correctly.

- **R4 — check: segment resolution and the tiebreak (D-B).** For each `FieldRoute`, look up
  the field on the routed variant's declaration by `name`, taking the field's declared type
  from the scrutinee's **instantiated** `EnumId` (a generic field's type is the substituted
  argument, not the `'T` header, mirroring Slice 3b recon 5). Then resolve `sel`:
  - `sel` equals the field's own declared type name → **whole** (decline to route; `nested`
    must be empty, else E6). The field is projected and left for the leaf body.
  - else `sel` names a variant of the field's enum → **route** on it; recurse into `nested`.
  - both (self-named variant) → **whole** wins (D-B).
  - neither → **E4**.
  - a non-enum field given a variant form (`sel` is not the field's type and the field's type
    is not an enum) → **E5**.

- **R5 — exhaustiveness is a product, unweakened.** The synthesized inner eliminator calls
  inherit the existing exhaustiveness/duplication pre-pass (`src/check.rs`,
  `eliminator_non_exhaustive_error`/`eliminator_duplicate_arm_error`) with no new check.
  Routing on k fields of enum arities a₁…a_k requires the full ∏ a_i leaf arms (decision 5,
  no wildcard). The whole (type) form declines routing on a field **without** weakening
  exhaustiveness: adding a variant to a *routed* field's enum still breaks every arm that
  routes on that field, because the synthesized inner call over it becomes non-exhaustive
  (R9/T-EXH).

- **R6 — mode is inherited, not decided (recon 2).** The synthesized projection uses the
  call's resolved mode: owning uses the whole destructure `Enum>` (`variant_generated_sigs`,
  `src/check/declarations.rs:1366`); reference uses `&field`/`&!field` through
  `check_reference_word`'s Slice 2 arm and the `resolved_variant_fields` table
  (`src/check/word_families.rs:408`). The inner eliminator call's mode is whatever the
  projection produced (a borrowed parent yields a borrowed field; there is no owning route
  out of a borrow). **Owning multi-field ordering is load-bearing:** `Enum>` pushes fields
  in declaration order, so routing a lower field while a higher one is whole requires the
  synthesis to reorder the stack (the recon probe needed a `swap`); the synthesis must emit
  the shuffle that places each routed field at the scrutinee position for its inner call and
  carries whole fields through. Reference mode needs no shuffle (`&field` projects by name
  independently).

- **R7 — drop/linearity unchanged (D-E).** The synthesized projection leaves the identical
  linear obligations the hand form leaves; a payload-free inner variant arm consumes its
  variant explicitly. No auto-drop is introduced anywhere in the synthesis.

- **R8 — no backend change; span-correct diagnostics.** No file under `src/ir/` changes; the
  as-built acceptance report must show an empty `src/ir/` diff. Every diagnostic E1–E8
  reports against written source spans (D-D). If lowering *must* change, decision 1 was
  wrong and the slice stops rather than growing (brief "Sequencing").

- **R9 — goldens and diagnostic tests** (see Exit criteria).

## Located errors (each mutation-tested, R-MUT)

Wording is illustrative; the tests pin the exact bytes. Each error names itself, never
surfaces as an arity mismatch from the synthesized projection (brief decision 6).

- **E1 — unknown field name.** A `FieldRoute.name` is not a field of the routed variant.
- **E2 — duplicated field.** A field named twice in one bracket.
- **E3 — missing field.** Not every field of the routed variant is named (brief decision 6
  forbids omission).
- **E4 — selector matches neither.** `sel` is neither a variant of the field's enum nor the
  field's declared type.
- **E5 — variant form on a non-enum field.** `sel` routes on a field whose type is not an
  enum.
- **E6 — nested bracket on a whole field.** `sel` resolved to the field's type (D-B/whole)
  but carries a `[ … ]`.
- **E7 — plain + path arm for one outer variant** (brief decision 4). `( Has )` and
  `( Has[…] )` in one call is a duplicated `Has` arm; reuses
  `eliminator_duplicate_arm_error`, and the guard must fire across the plain/path boundary.
- **E8 — inner non-exhaustive / inner duplicate.** A partially covered group
  (`Has[v Circle]` present, `Has[v Rect]` absent) reuses `eliminator_non_exhaustive_error`
  from the synthesized inner call, reported against the written arm span (D-D).
- **E9 — malformed bracket (parse).** Empty `[]`, unbalanced brackets, a field name with no
  selector. Located at parse.

## Mutation-testing obligations (R-MUT)

This project has shipped placebo tests repeatedly; reading a test does not catch them.
Every new rejection must be proven capable of failing when its guard is deleted,
classifying on `test result: FAILED` (not a bare `error` grep, which catches cargo's own
`error: test failed`), committing before mutating, and ending each cycle on an empty
`git status`.

- **M1–M9:** delete the guard behind E1–E9 in turn; the paired diagnostic test must flip to
  `FAILED`. E7's mutation deletes the plain/path duplicate check specifically (a same-outer
  plain+path pair must be rejected, not silently grouped or dropped).
- **M-EXH:** the load-bearing guarantee. A golden that routes on a `Shape`-typed field, then
  a mutation that adds a `Triangle` variant to `Shape`, must make that golden fail to
  compile (inner non-exhaustive). Prove the type/whole form does **not** exempt it: the same
  mutation against a *whole* (type-form) `Shape` field must still compile (the whole field is
  not routed, so it is correctly unaffected), while the routed field breaks. Two directions,
  so a guard that ignored routing entirely is caught.
- **M-SPAN:** mutate the leaf-arm span from the written span to a synthesized one; a test
  asserting E8's reported line equals the written arm's line must flip to `FAILED`.
- **M-DESUGAR-EQUIV:** each golden is paired with its hand-nested equivalent (R9). A
  mutation that changes the synthesis output (e.g. drops a projection) must make the golden's
  runtime output diverge from its hand-written twin.

## Exit criteria (each maps to a named test or golden)

- **G1 — owning two-level golden** (`examples/eliminator_path.sth`, `tests/phase6_slice5.rs`):
  the recon owning probe, `75 / 12 / 0`, with a payload-free `( Empty ) drop 0` arm (D-E).
  Paired with its hand-nested twin asserting identical output.
- **G2 — reference two-level golden** (`examples/eliminator_path_ref.sth`): the recon
  reference probe, `5`, payload-free arm present. Paired with its hand twin.
- **G3 — generic two-level golden** (`examples/eliminator_path_generic.sth`): `Option[Shape]`,
  `75 / 0` (D-A). This is the case that would expose a segment accidentally matching a
  mangled instantiated variant name; it must route by surface name (brief decision 7).
- **G4 — multi-field product golden** (`examples/eliminator_path_product.sth`): `Pair` routed
  on both `Shape` fields, the full four-arm product, `17 / 0` (R5/R6). A companion asserts
  the one-routed-one-whole form (`Both[a Circle b Shape]`) type-checks and reads `b` whole.
- **G5 — three-level golden** (`examples/eliminator_path_deep.sth`): `Outer -> Inner ->
  Shape`, `5 / 0 / 0` (D-C).
- **T-E1 … T-E9** (`tests/phase6_slice5.rs`): each located error, asserting the exact
  wording, each with its M1–M9 mutation.
- **T-OQ2** (`tests/phase6_slice5.rs`): `type: Box | Box v i64 | Nil ;` used as a field enum;
  a segment `Box` on that field resolves as the whole (type) form (D-B), witnessed by the
  field reaching the leaf body whole; and the same-named variant is confirmed unroutable via
  a path segment.
- **T-EXH** (`tests/phase6_slice5.rs`): M-EXH, both directions.
- **T-SPAN** (`tests/phase6_slice5.rs`): E8 reports the written arm's line (M-SPAN).
- **Regression:** all of `tests/phase6_slice3.rs`, `tests/phase6_slice3b.rs`,
  `tests/phase5_generic_enum_elimination.rs`, and `examples/eliminator{,_ref}.sth` pass
  unchanged; every Slice 3/3b diagnostic fires with unchanged wording.
- **Green** per phase: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

## Phased delivery plan

Each phase is independently green and runnable.

**Phase 1 — AST and parser (standard).** R1, R2, E9. `VariantTag.fields` and the recursive
bracket parse, with parse-time syntactic diagnostics only. `check_eliminator_call` gains a
single located rejection of any non-empty-`fields` tag (`nested tag paths are not yet
lowered`), so the tree stays green and nothing reaches unimplemented synthesis; this
rejection is *live* in this phase and removed in Phase 2 (no pre-staged dead plumbing).
Parser unit tests for the bracket shape
and E9; the temporary check rejection has its own test. Plain tags unchanged.

**Phase 2 — single-field synthesis and resolution (hard).** R3, R4, R6 (single field, both
modes), R7, R8-span, plus E1, E4, E5, E6, E7, E8 and D-B (OQ2). Removes the Phase 1
rejection. Goldens G1, G2, G3; tests T-E1, T-E4, T-E5, T-E6, T-E7, T-E8, T-OQ2, T-SPAN; the
span-collision test from R3. Mutations M1, M4–M8, M-SPAN, M-DESUGAR-EQUIV for these goldens.
This is the core: it establishes the desugar and every resolution diagnostic on the
single-field path.

**Phase 3 — multi-field product and the whole form (hard).** R5, R6 (multi-field ordering
and whole-field carry-through), E2, E3. Golden G4 and its one-routed-one-whole companion;
tests T-E2, T-E3; the exhaustiveness-product test T-EXH (both directions). Mutations M2, M3,
M-EXH. Depends on Phase 2's single-field synthesis being green.

**Phase 4 — depth, dogfood, and audit (standard).** D-C: golden G5 (three levels), proving
the recursion composes. Re-run the full regression set and the growth-structure signals
(CLAUDE.md) against `check.rs`/`parser.rs` as they now stand. Final mutation sweep confirming
every guard from Phases 1–3 still flips to `FAILED` after the tree settled, ending on an
empty `git status`.

```json
{
  "phases": [
    { "phase": 1, "focus": "AST and parser bracket path", "difficulty": "standard" },
    { "phase": 2, "focus": "single field synthesis and resolution", "difficulty": "hard" },
    { "phase": 3, "focus": "multi field product and whole form", "difficulty": "hard" },
    { "phase": 4, "focus": "arbitrary depth dogfood and audit", "difficulty": "standard" }
  ]
}
```

## Out of scope

- `cond`/guard/literal dispatch (brief decision 8), in any spelling.
- Wildcards or catch-all arms at either level (brief decision 5).
- Any change to dispatch lowering, `ArmBinding`, or codegen (brief decision 1).
- Any change to accessor generation or `Type::Variant` (Slice 2 shipped both).
- Nesting through a *struct* field: structs have no variants, so there is nothing to route
  on; a struct field is reached by ordinary projection.
- Routing on a variant named after its own enum via a path segment (D-B): unreachable by
  design, use a whole arm with a hand-written inner eliminator.
