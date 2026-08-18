# Spec: Phase 6 Slice 3b — check-time arm-tag resolution

**Status:** Ready to implement
**Created:** 2026-08-18
**Base:** `main` at `78d8cee`

Pairs with [slice3b-brief.md](./slice3b-brief.md) (discovery; decisions 1–5 settled there).
Modifies what [slice3-spec.md](./slice3-spec.md) shipped: eliminator arm tags stop being
typed at parse time and are typed at check time against the scrutinee's `EnumId`, uniformly
for concrete and generic enums. Deleting `WordBody::Clauses`/`parse_clauses` and migrating
`tests/phase5_generic_enum_elimination.rs` is **Slice 4** and out of scope; the clause path
stays fully working through this slice.

## Recon re-verification (against `78d8cee`, not inherited)

All line anchors below are re-read at this HEAD. The brief's anchors were taken at `9049b0a`
and drift by a few lines; the corrected anchors are used throughout this spec.

- **Recon 1 — reproduced.** Building a generic-enum eliminator arm fails with a *type*
  error, not a located eliminator diagnostic:
 `: to-int ( o::Option[i64] -- i64 ) ~[ ( None ) 0 ] ~[ ( Some ) Some> ] o::Option? ;`
  → `error: unknown type \`None\` at line 3, col 8`. Confirmed by build. The tag never
  becomes a tag because `parse_leading_variant_slot` (`src/parser.rs:2788`) returns
  `Ok(None)` for a name the concrete registry cannot type, and the token falls through to
  the ordinary input-slot reader.
- **Recon 2 — confirmed.** `parse_leading_variant_slot` (`src/parser.rs:2788`) fuses
  recognition and typing: it recognizes a tag *by resolving it to a `Type`* via
  `resolve_variant_type` (`src/parser.rs:2825`) → `find_variant_type_in_module`
  (`src/parser.rs:2849`), which scans `self.enums` (concrete registry) matching on
  `name_static`. A generic enum's variants are not in there.
- **Recon 3 — confirmed with a correction.** `is_variant_name` (`src/parser.rs:1757`)
  consults **both** `self.enums` and `self.generics.enums`, comparing `v.name == name`, and
  `at_clause_start` (`src/parser.rs:1775`) uses it via `token_at_is_variant`. **Correction
  (finding F1):** `is_variant_name` is *not module-scoped* — it matches a variant of any
  enum in any module. The clause path tolerates that (the clause is later typed against the
  word's declared enum), but `parse_leading_variant_slot_other_module_variant_is_not_visible`
  (`src/parser.rs:4325`) proves the eliminator path must **not**: its whole point is that a
  variant declared in an unimported module must not capture this module's annotation slot
  ("the pre-fix bug let any variant name anywhere in the program capture every annotation's
  leading slot"). Reusing bare `is_variant_name` for recognition would reintroduce exactly
  that regression. Recognition must therefore keep `resolve_variant_type`'s module scoping
  (own module, then a selectively-imported variant's target module) **and** gain generic
  awareness — it cannot be `is_variant_name` verbatim. See R1/OQ2.
- **Recon 4 — confirmed.** `check_eliminator_call` (`src/check.rs:2193`) builds each arm's
  declared effect from the scrutinee's own `EnumId`: `variant_type(ctx.enums(), id, *vi)`,
  then `intern_ref_type(refs, owned, mutable)` under the call's resolved mode, then
  `inline_quotation_type(vec![narrowed], vec![])`. It never reads `annot.inputs`. It routes
  arms to variants on `generic_surface_name(&v.name) == tag` (`src/check.rs:2280`, in the
  exhaustiveness pre-pass). So decision 1's premise holds: the check side is already
  operand-`EnumId`-driven.
- **Recon 5 — confirmed.** For a tagged literal, `src/check/terms.rs:854` skips
  `check_literal_against_annotation` (the `resolved.variant_tag.is_none()` branch is the
  only caller of that standalone check). The only remaining consumer of a tagged literal's
  annotation leading input is `reconcile_annotation_with_parameter` (`src/check.rs:1840`),
  reached via `check_literal_against_declared_effect` (`src/check.rs:1919`, calling reconcile
  at `src/check.rs:1978`). That comparison (`annot.inputs == eff.inputs`,
  `src/check.rs:1852`) is what catches decision 6's mode mismatch and renders the
  `annotated \`~[ Shape.Circle -- ]\`` / `\`Shape?\` declares it \`~[ &Shape.Circle -- ]\``
  wording. **Account of every reader of `annot.inputs`/` AnnotEffect.inputs` on the
  tagged-literal path:** `resolve_annotation` → `resolve_annot_slots` (`src/check.rs:1630`)
  transforms `annot.inputs` into the resolved effect (a pass-through, no comparison);
  `check_literal_against_annotation` (`src/check.rs:1767`,`:1803`,`:1806`) is **skipped**
  for a tagged literal; `reconcile_annotation_with_parameter` (`src/check.rs:1850`/`:1852`)
  is the sole comparison. Every other`.inputs` reader grepped is `word.effect.inputs` or an
  untagged path. Recon 5 stands: reconcile is the only thing catching the mode mismatch.
- **Recon 6 — confirmed, with a mechanism correction (see decision 5 below).**
  `eliminator_registry` (`src/check/declarations.rs:1469`) keys by
  `format!("{}?", generic_surface_name(&decl.name))`, so two instantiations of one generic
  (`Result__m0[i64 bool]` and `Result__m0[bool i64]`) both key `"Result__m0?"`, last write
  wins. The collision is real.

**F1 and the decision-5 mechanism correction are the two places this spec departs from the
brief; both are argued in-line and neither reopens a settled *decision*, only its stated
mechanism.**

## OQ rulings

### OQ1 — the tag-vs-input discriminator (with typing deferred)

**Measured, not reasoned:** a variant and an ordinary type *can* share a name today. Built
`type: Shape | Circle | Rect ;` alongside `type: Circle r i64 ;` (both declaration orders):
no duplicate-name error, both names usable. So `is_variant_name` alone cannot be the
discriminator — a shadowing type name must still win.

**Replacement rule (R1).** In an annotation's leading slot, a token (sigil-stripped) is a
routing **tag** iff **all** of:

1. it does **not** resolve as an ordinary type name in scope
   (`resolve_type_name_in_module`, the first branch of today's `resolve_variant_type`) —
   preserves the type-precedence the current code has; **and**
2. it names a variant of a concrete enum **visible in this module's scope** (own module, or
   a selectively-imported variant's target module — the `find_variant_type_in_module` search
   of `resolve_variant_type`, read as a boolean), **or** a variant of a generic enum visible
   in this module's scope (`self.generics.enums`, filtered by `module`/selective import the
   same way — the new capability).

Otherwise it is an ordinary first input slot. This rule is module-scoped (F1) and
generic-aware, and is applied identically in both spellings: the lone form `( Circle )` and
the escalated form `( Circle -- i64 )` (where `Circle` matching the rule makes the slot a
tag and the annotation's declared inputs empty, its outputs `i64`).

**Required collision tests (R1):** with a struct `Circle` and an enum `Shape` whose variant
is `Circle`, in scope together —

- `( Circle )` records `variant_tag == None` and a single input `Type::Struct(Circle)`
  (type wins);
- `( Circle -- i64 )` records `variant_tag == None`, inputs `[Type::Struct(Circle)]`,
  outputs `[i64]` (type wins in the escalated spelling too).
Both are `assert_eq!` on the parsed `QuotAnnot`, extending
`parse_leading_variant_slot_struct_of_same_name_takes_precedence` (`src/parser.rs:4260`)
with the escalated spelling.

### OQ2 — one tag spelling for recognition and routing

**Ruling: the bare surface variant name**, on both sides. Recognition (parse) records
`variant_tag = <bare name>` matched against the (concrete or generic) declaration's bare
variant name. Routing (check) already compares `generic_surface_name(&v.name) == tag`
(`src/check.rs:2280`); for an instantiated enum `v.name` is the instantiation-suffixed
`Some[i64]` and `generic_surface_name` strips the `[...]` back to `Some`. Verified that
resolve does **not** `__mN`-mangle variant names (only enum *type* names), so
`generic_surface_name(&v.name)` yields exactly the bare surface name the parser records — no
demangling needed on the routing side, and no `generic_surface_name` needed on the
recognition side (the generic decl already holds the bare name pre-instantiation).

**Required test (R4/R6):** an instantiated enum whose stored (instantiation-suffixed)
variant name differs from its surface spelling — e.g. `Result[i64 bool]`, whose variant is
stored `Ok[i64 bool]` — eliminates through a bare `( Ok )` arm and runs, `assert_eq!` on
program output. This fails if either side stops normalizing to the bare name.

### OQ3 — the deferred typing and the stray-tag guard

**Ruling: both guards keep working, and are extended to the generic case.**
`tagged_literal_reaches_an_eliminator_call` (`src/check/terms.rs:939`) and the skip that
precedes it key off `variant_tag.is_some()`, not off any resolved type, so a tag carrying no
resolved type still flows through the existing branch unchanged. Because recognition is now
generic-aware, a *generic* enum's stray tag (which previously could not parse) now parses,
is recognized, and — if it does not reach an eliminator call — must fire
`eliminator_arm_outside_call_error` (`src/check/terms.rs:954`).

**Required tests (R3):**

- a generic `Option`'s `~[ ( Some ) .. ]` written with no following `Option?` call produces
  `error: this quotation is annotated \`( Some )\`, an eliminator-arm tag, but it is not
  consumed by a call to a generated eliminator` (exact prefix; the message continues with
  the in-word suffix and line);
- a generic `Option`'s tagged arm *does* reach `Option?` and checks (the positive OQ3 case,
  covered by R6's runnable golden).

## Requirements

All anchors verified at `78d8cee`.

- **R1 — parse-time recognition, name-only, module-scoped, generic-aware, type-precedent.**
  `parse_leading_variant_slot` (`src/parser.rs:2788`) stops returning a `Type`. It becomes a
  recognizer that, on the OQ1 rule, records `variant_tag` **and the mode** (see R2) and
  consumes the token, or leaves the token for the ordinary slot reader. `resolve_variant_type`
  / `find_variant_type_in_module` (`src/parser.rs:2825`/`:2849`) are **repurposed to boolean
  recognizers** (name-only, retaining their module scoping and `name_static` match) and
  extended to consult `self.generics.enums` filtered by module/selective import; they are not
  deleted (their module-scoped search is exactly what F1 requires), so no caller is left dead
  and no import goes unused. `parse_quot_annotation` (`src/parser.rs:2735`) no longer pushes a
  synthesized leading input `PolyType` — `annot.inputs` for a tagged arm is empty (lone form)
  or holds only the ordinary post-`--` slots (escalated form). Surface syntax is unchanged.
- **R2 — explicit three-state mode on the annotation.** Because the mode can no longer ride
  on an interned `Type::Ref`, `QuotAnnot` (`src/ast.rs:1770`) carries it as data. Replace
  `variant_tag: Option<String>` with `variant_tag: Option<VariantTag>` where
  `struct VariantTag { name: String, mode: VariantTagMode }` and
  `enum VariantTagMode { Owning, Ref, RefMut }`. `AnnotEffect.variant_tag` (`src/check.rs:256`)
  and `resolve_annotation` (`src/check.rs:1633`) mirror the new shape. Every reader
  (`src/check.rs:2231` in collection, `src/check/terms.rs:854`/the `else` branch, the parser
  tests) is updated to the struct.
- **R3 — check-time synthesis of the leading input slot.** In `check_eliminator_call`
  (`src/check.rs:2193`), after resolving each arm's `vi` and building `narrowed` (variant
  type in the **call's** mode, unchanged), synthesize the annotation's leading input from
  `(variant_type(ctx.enums(), id, vi), recorded mode)` — the mode the *user wrote*, from
  R2's `VariantTag.mode`, interned through the same `intern_ref_type` — and write it as the
  arm annotation's sole input before `check_literal_against_declared_effect` runs. Concretely:
  set `prov.quotations[qid].annot`'s `inputs` to `[synthesized]` for the arm being checked.
  `reconcile_annotation_with_parameter` then compares `[synthesized-in-user-mode]` against
  `eff.inputs == [narrowed-in-call-mode]`: equal when the modes agree (both go through the
  same `variant_type`/`intern_ref_type`, so identical interned `Type `s), unequal — and the
  existing `annotation_parameter_mismatch_error` wording — when they disagree. No new
  mode-comparison branch is added (decision 4). The `tagged_literal_reaches_an_eliminator_call`
  guard and `eliminator_arm_outside_call_error` (`src/check/terms.rs:939`/`:954`) are
  updated only for R2's field shape and keep firing (OQ3).
- **R4 — recognition and routing agree on the bare surface name** (OQ2). No change to the
  routing comparison at `src/check.rs:2280`; the requirement is that recognition record the
  bare name, verified by R6's differing-stored-name test.
- **R5 — operand-driven `EnumId`; registry as a base-family gate (decision 5, corrected).**
  **Mechanism correction:** the brief's literal "re-key `eliminator_registry` by the
  instantiated (mangled) enum spelling" is unworkable and would break routing. Proof: the
  eliminator call site resolves to the **bare mangled base** `Enum__mN?` with *no*
  instantiation suffix (`resolve.rs:878`, `eliminator_call_site_mangles_to_match_the_enum_based_key`,
  asserts `Shape__m0?`; the source call carries no type arguments and resolve is syntactic).
  Keying the registry by `Result__m0[i64 bool]?` would make `poly.eliminators.get("Result__m0?")`
  return `None`, so the eliminator would not be recognized at all. Decision 5's **intent**
  (independent elimination of two instantiations in one word, no last-write-wins) is instead
  achieved the way every other generic generated word already achieves it — operand-driven
  (`enum_generated_sigs`/`variant_generated_sigs` at `src/check.rs:561` already key env by
  bare name and disambiguate per instantiation by operand type):
  - `check_eliminator_call` takes the operative `EnumId` from the **scrutinee slot's own
    `Type::Enum(id, _)`** (via `ref_parts` on `scrutinee.ty`), not from the registry value;
  - the registry stays keyed by `generic_surface_name(&decl.name)` (so the resolved call name
    `Enum__mN?` still matches) and is used as a **family gate**: the call is an eliminator,
    and the scrutinee's enum must belong to the same base family, checked by
    `generic_surface_name(module.enums[scrutinee_id].name) == generic_surface_name(module.enums[gate_id].name)`;
    a scrutinee of the wrong family is the existing `type_mismatch_error` (`src/check.rs:2266`).
  This removes the last-write-wins dependence entirely: whichever instantiation the registry
  happened to store is irrelevant, because the scrutinee names the exact instantiation to
  eliminate.
- **R6 — goldens** (`tests/phase6_slice3b.rs`, new): a runnable generic-enum eliminator,
  spelled identically to the concrete case, instantiation from the scrutinee; plus the
  decision-5 two-instantiation golden. See test requirements.

## Test requirements (breakable assertions only)

- **T1 (R3, primary hazard — must keep passing, unchanged wording).**
  `check_eliminator_call_mode_mismatch_is_error` (`src/check.rs:4395`) keeps passing with its
  two assertions byte-for-byte: `err.contains("annotated \`~[ Shape.Circle -- ]\`")
  && err.contains("\`Shape?\` declares it \`~[ &Shape.Circle -- ]\`")`, and the `&`/`&!`
  sibling variant. **Mutation check (T1a):** with R3's synthesis of the leading input slot
  removed (so `annot.inputs` stays empty and reconcile compares `[] == [narrowed]`), this
  test must **fail** with `test result: FAILED`. State this in the phase's exit criteria and
  run it (edit out the synthesis line, `cargo test check_eliminator_call_mode_mismatch_is_error`,
  observe FAILED, restore).
- **T2 (R1/OQ1 collision).** Extend
  `parse_leading_variant_slot_struct_of_same_name_takes_precedence` (`src/parser.rs:4260`):
  `( Circle )` → `annot.variant_tag == None`, `annot.inputs == [Concrete(Type::Struct(Circle))]`;
  and a new escalated case `( Circle -- i64 )` → `annot.variant_tag == None`,
  `annot.inputs == [Concrete(Type::Struct(Circle))]`, `annot.outputs == [Concrete(Type::I64)]`.
- **T3 (R1/F1 module scoping).** `parse_leading_variant_slot_other_module_variant_is_not_visible`
  (`src/parser.rs:4325`) keeps passing: a `Circle` variant declared only in module 1 does
  **not** become module 0's tag — assert the module-0 parse errors as an unknown *type*
  (`err.contains("unknown type \`Circle\`")`), i.e. recognition declined. This is the guard
  that a bare-`is_variant_name` recognizer would break.
- **T4 (R2 parser).** Migrate `parse_quotation_annotation_variant_tag_owning_ok`
  (`src/parser.rs:4188`) and`..._mut_ref_ok` (`src/parser.rs:4217`): owning →
  `variant_tag == Some(VariantTag { name: "Circle", mode: Owning })` and
  `annot.inputs.is_empty()`; `&!` → `mode: RefMut` and `annot.inputs.is_empty()` (no
  synthesized `Type::Ref`). The bare name (no sigil) invariant stays asserted.
- **T5 (R1/R3 generic single-instantiation, runnable).** `tests/phase6_slice3b.rs`: a
  generic `Option`, eliminated through `~[ ( None ) 0 ] ~[ ( Some ) Some> ] Option?` with
  the instantiation from the scrutinee's signature type; `assert_eq!(stdout, ...)` on a
  distinguishing output (both arms exercised).
- **T6 (R5 decision-5, asymmetric, two instantiations, one word).** `tests/phase6_slice3b.rs`:
  **one word** that eliminates both `Result[i64 bool]` and `Result[bool i64]` (asymmetric —
  never `Result[i64 i64]`, which cannot distinguish `Ok 'T | Err 'E` from its swap).
  `assert_eq!` on an output that differs per instantiation and per arm (e.g. the `i64` arm of
  `Result[i64 bool]` prints a number, the `bool` arm of `Result[bool i64]` prints a different
  number), so a last-write-wins regression prints the wrong value or errors as a type
  mismatch rather than routing to the scrutinee's own instantiation. **Mutation check (T6a):**
  reverting R5 (use the registry's stored id instead of the scrutinee's) makes one of the two
  instantiations fail as `type_mismatch_error` — state and run.
- **T7 (OQ2 differing stored name).** Covered by T6: `Result[i64 bool]`'s stored variant name
  is `Ok[i64 bool]`, surface `Ok`; routing through bare `( Ok )` proves both sides normalize
  to the surface name. Assert the run succeeds and prints the expected value (not a
  placebo "does not panic").
- **T8 (OQ3 stray generic tag).** `tests/phase6_slice3b.rs` (or a `check_src`/`build_err`
  case): a generic `Option`'s `~[ ( Some ) 0 ]` with no following `Option?` →
  `err.contains("this quotation is annotated \`( Some )\`, an eliminator-arm tag, but it is
  not consumed by a call to a generated eliminator")`.
- **T9 (regression — clause path untouched).** `cargo test` includes
  `tests/phase5_generic_enum_elimination.rs` (all four:
  `generic_enum_clause_elimination_runs`, `generic_enum_elimination_type_declared_after_matching_word`,
  `two_generic_enum_instantiations_eliminate_independently`,
  `non_exhaustive_generic_enum_clause_names_surface_variant`) unchanged, and the whole
  `tests/phase6_slice3.rs` concrete golden suite unchanged.

## Phase plan

Each phase is independently green: `cargo fmt --check && cargo clippy -- -D warnings &&
cargo test`. No import or helper is introduced before its first use.

### Phase 1 — recognition/typing split, explicit mode, check-time synthesis

Delivers decisions 1–4. Parser recognizes tags by name (R1, module-scoped, generic-aware,
type-precedent), records `variant_tag: Option<VariantTag>` with the three-state mode (R2),
and stops synthesizing the leading input slot; `check_eliminator_call` synthesizes it from
`(variant, recorded mode)` before reconciliation (R3). Concrete enums behave identically;
generic enums with a **single** instantiation now recognize, type, and eliminate (the
registry's one entry equals the scrutinee's id, so decision 5 is not yet needed). All
`resolve_variant_type`/`find_variant_type_in_module` callers are repurposed, not orphaned
(no dead code / unused import).

**Exit criteria (tests):** T1 (unchanged wording) + T1a (mutation FAILED); T2, T3, T4
(parser); T5 (generic single-instantiation runs); T8 (stray generic tag); T9 (clause path +
concrete suite unchanged). Full `cargo test` green.

### Phase 2 — operand-driven `EnumId`, registry as family gate

Delivers decision 5 (corrected, R5): `check_eliminator_call` derives the operative `EnumId`
from the scrutinee and uses the registry only as a base-family gate. Enables two
asymmetric instantiations in one word.

**Exit criteria (tests):** T6 (two asymmetric instantiations, one word, distinguishing
output) + T6a (mutation: reverting to the registry id fails one instantiation as a type
mismatch); T7 (differing stored name, via T6); everything from Phase 1 still green.

## Out of scope

Deleting `WordBody::Clauses`/`parse_clauses` and migrating
`tests/phase5_generic_enum_elimination.rs` (Slice 4); any change to `Type::Variant`,
accessors, `lower_clauses`, `ArmBinding`, or tag-dispatch codegen; generic-enum
construction; inferring an instantiation from an arm rather than from the scrutinee operand.

## Machine-readable phases

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "recognition-typing split, explicit arm mode, check-time input synthesis",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "operand-driven EnumId with registry as base-family gate",
      "difficulty": "standard"
    }
  ]
}
```
