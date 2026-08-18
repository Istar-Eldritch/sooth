# Phase 6 Slice 3b: check-time arm-tag resolution (brief)

An eliminator arm's `( Ok )` tag is currently resolved to a concrete `Type::Variant` by
the *parser*, against the concrete enum registry. That makes the eliminator unable to
eliminate a generic enum at all, because a bare tag carries no type arguments and a
generic header contributes no concrete variant to scan. This slice moves tag *typing*
from parse time to check time, where the scrutinee operand's `EnumId` already says which
enum (and which instantiation) is being eliminated. Tag *recognition* stays at parse
time, where it belongs, and becomes generic-aware by reusing the predicate the
clause path already uses.

The surface syntax does not change. That is the point: the generic case must read exactly
like the concrete case.

```text
: to-int ( Result[i64 i64] -- i64 )
  ~[ ( Ok )  Ok>  ]
  ~[ ( Err ) Err> 100 + ]
  Result?
;
```

## Why this is a slice and not a Slice 4 sub-task

Slice 4 deletes `WordBody::Clauses`. `tests/phase5_generic_enum_elimination.rs` is four
tests whose entire subject is eliminating a generic enum through a clause-style word, a
capability Phase 5 Slice 1 shipped deliberately ("this closes that gap so `Result`/`Option`
become usable in Slice 2"). Clause bodies are therefore the *only* working generic-enum
elimination mechanism today. Deleting them before the eliminator can replace them makes
generic enums constructible but not eliminable — a capability regression, not a migration.
This slice is the prerequisite that makes Slice 4 a migration.

## Recon (measured against the built compiler, 2026-08-18, `main` at `9049b0a`)

`cargo test` is green at this HEAD (Slice 3 done and merged), except two pre-existing
uncommitted local edits in `tests/phase4_slice12_partab.rs` / `tests/phase4_slice6g.rs`
unrelated to Phase 6.

1. **The gap is real and reproduced.** A bare `( None )` arm over an imported generic
   `Option[i64]` fails with an "unknown type `None`" parse error, not a located
   eliminator diagnostic. The tag never becomes a tag: `parse_leading_variant_slot`
   (`src/parser.rs:2788`) returns `Ok(None)` when the leading word does not resolve,
   so the token falls through to the ordinary input-slot reader, which then reports an
   unknown *type*. The failure mode is therefore also a bad diagnostic, not only a
   missing capability.

2. **Recognition and typing are currently fused in one call, and only typing is the
   problem.** `parse_leading_variant_slot` does both jobs at once: it decides whether the
   leading token is a routing tag *by* resolving it to a type
   (`resolve_variant_type`, `parser.rs:2825` → `find_variant_type_in_module`,
   `parser.rs:2849`), which scans `self.enums` — the concrete registry — and returns
   `variant_type(..)`. A generic enum's variants are not in there, so recognition fails as
   collateral damage from typing failing.

3. **The generic-aware recognition predicate already exists and is already used for
   exactly this.** `is_variant_name` (`src/parser.rs:1757`) checks the concrete registry
   **and** `self.generics.enums`. It is what `at_clause_start` (`:1775`) uses, which is
   why the clause path can match `| Ok` on a generic enum while the eliminator cannot.
   The generic headers are populated by `parse_generic_typedefs` before any word body is
   parsed, which is what makes the clause path's declaration-order independence work
   (`generic_enum_elimination_type_declared_after_matching_word`,
   `tests/phase5_generic_enum_elimination.rs:63`). So this slice does not need to invent
   generic-aware recognition; it needs to *use* it.

   Nuance to carry into the spec: `is_variant_name` compares `v.name == name`, while
   `check_eliminator_call` routes on `generic_surface_name(&v.name) == tag`
   (`src/check.rs:~2320`). Those are the same string for a concrete enum and can differ
   for an instantiated one; the spec must pick one spelling for the tag and use it on
   both sides rather than letting recognition and routing disagree.

4. **The check side already builds the arm's real expected type itself, so the
   parse-time type is nearly vestigial already.** `check_eliminator_call`
   (`src/check.rs:2215`) resolves the variant from the scrutinee's own `EnumId` and
   builds each arm's declared effect from scratch: `variant_type(ctx.enums(), id, *vi)`,
   then `intern_ref_type(refs, owned, mutable)` under decision 6's resolved mode, then
   `inline_quotation_type`. Nothing there consults the annotation's parse-time input
   slot. This is the same operand-`EnumId`-driven rule Slice 2 settled for accessors; the
   parser is the one place that still name-scans.

5. **But the parse-time type is load-bearing in exactly one place, and that is the whole
   design problem.** For a *tagged* literal, `check/terms.rs:852` skips
   `check_literal_against_annotation` entirely (the arm has no standalone fixed point).
   The written annotation is consulted only later, by
   `reconcile_annotation_with_parameter` (`src/check.rs:1840`), which compares
   `annot.inputs == eff.inputs` under strict structural equality. That comparison is what
   catches decision 6's mode mismatch — an arm spelling `( Circle )` under a `&Shape`
   call — and it is what renders the diagnostic's "annotated `~[ Shape.Circle -- ]` /
   `Shape?` declares it `~[ &Shape.Circle -- ]`" wording. If the parser stops producing a
   leading input slot, this comparison silently loses its only input and the mode check
   goes with it. Any design that just deletes the parse-time resolution regresses
   decision 6 to nothing, and the existing `check_eliminator_call_mode_mismatch_is_error`
   test (`src/check.rs:4355`) is the guard that must keep failing under that mistake.

6. **The R3 registry-key collision dies as a side effect, and is otherwise waiting to
   bite.** `eliminator_registry` (`src/check/declarations.rs:1459`) keys by
   `generic_surface_name`, so `Result[i64 i64]?` and `Result[bool i64]?` collapse to one
   `"Result?"` entry. Slice 3's reviewers flagged this as dormant *because* generic
   elimination cannot parse. This slice unblocks exactly that, so the collision stops
   being dormant on the same commit — it is in scope here, not deferrable again.

7. **Nothing in-repo currently spells generic elimination the eliminator way**, so there
   is no migration debt inside this slice: `lib/option.sth` and `lib/result.sth` only
   declare and export the types. The four clause-form witnesses live in
   `tests/phase5_generic_enum_elimination.rs` and stay on the clause path until Slice 4
   moves them.

## Decisions (settled here, not reopened by the spec)

1. **Split recognition from typing.** Parse time recognizes a tag by *name only*
   (recon 3's `is_variant_name`, generic-aware) and records the tag plus its mode. Check
   time resolves the tag to a `Type::Variant` against the scrutinee's `EnumId` (recon 4's
   existing code, unchanged). This is the rule Slice 2 already settled for accessors,
   applied to the one site that still violates it.

2. **Apply it uniformly, not as a generic-only fallback.** The parser stops resolving arm
   tags to types *for every enum*, concrete and generic alike. A concrete/generic
   divergence here would mean two routing paths, two mode-check paths, and a
   concrete-only test suite that never exercises the generic one — the exact shape of
   defect this phase keeps finding. One path, exercised by every existing Slice 3 test.

3. **The mode must be recorded explicitly, since it can no longer ride on an interned
   `Type::Ref`.** Today `&`/`&!`/owning is encoded by what
   `parse_leading_variant_slot` interns. With typing deferred, the annotation carries the
   tag *and* a three-state mode (owning / `&` / `&!`) as data. Recon 5 is why this is
   non-negotiable: the mode is the only thing decision 6's mismatch check compares.

4. **`check_eliminator_call` synthesizes the annotation's leading input slot from
   (resolved variant, recorded mode) before reconciliation runs**, so
   `reconcile_annotation_with_parameter` keeps comparing two fully-formed effects and the
   existing diagnostic wording is preserved byte-for-byte. Rejected alternative: a new
   bespoke mode-comparison branch inside `check_eliminator_call`. That would be a second
   copy of a rule that already exists, which is precisely the risk Slice 3's decision 4
   named as its primary hazard; the mode mismatch should keep coming out of the shared
   comparison, not a parallel one.

5. **Key `eliminator_registry` by the instantiated (mangled) enum spelling, not the
   generic surface name** (recon 6). Last-write-wins across instantiations is a live bug
   the moment this slice lands.

## Open questions for the spec

- **OQ1 — what discriminates a tag from an ordinary first input slot in the escalated
  forms?** The lone form `( Circle )` is unambiguous (a parenthesized annotation with one
  word and no `--` is otherwise a located error). The escalated `( Circle -- i64 )` is
  the question: with typing deferred, `is_variant_name` is the only discriminator, so a
  variant and a type sharing a name (legal today? recon does not settle it) decides
  whether the leading slot is a tag or an input. `resolve_variant_type` currently
  side-steps this by returning `None` when the name also resolves as a type name
  (`parser.rs:2825`, its first branch). The spec must state the replacement rule and
  test the collision case directly, in both spellings.

- **OQ2 — does a generic enum's variant tag need `generic_surface_name` normalization on
  the recognition side, the routing side, or both?** Recon 3's nuance. One spelling,
  stated once, tested with an instantiated enum whose mangled and surface variant names
  differ.

- **OQ3 — does the deferred typing change what `tagged_literal_reaches_an_eliminator_call`
  (`check/terms.rs:~865`) can see?** That guard exists so a tagged literal that never
  reaches an eliminator call is not silently unchecked. It runs before any scrutinee is
  known; confirm it still works when the tag no longer carries a resolved type, and that
  `eliminator_arm_outside_call_error` still fires for a *generic* enum's stray tag, which
  today cannot even parse.

## Out of scope

- Migrating any clause-style site or deleting `WordBody::Clauses`/`parse_clauses`:
  Slice 4. This slice must leave the clause path working, including
  `tests/phase5_generic_enum_elimination.rs`, so the two mechanisms coexist for exactly
  one slice.
- Any change to `Type::Variant`, accessor resolution, or the generated accessor `Sig`s:
  Slice 2 shipped them and recon 4 finds them already operand-driven.
- Any change to `lower_clauses`, `ArmBinding`, or tag-dispatch codegen. This is a
  frontend resolution-timing change; the IR sees the same resolved variant it sees today.
- Generic-enum *construction*, inference of an instantiation from an arm rather than from
  the scrutinee, or eliminating a generic enum whose instantiation is not determined by
  the call site's operand type.

## Sequencing

Gated on Slice 3 (done, `main` at `9049b0a`). Blocks Slice 4, which cannot delete the
clause path until this lands. Touches `src/parser.rs` (`parse_leading_variant_slot` and
`parse_quot_annotation` stop typing the tag; `resolve_variant_type`/
`find_variant_type_in_module` likely lose their only caller), `src/ast.rs`
(`QuotAnnot`'s tag field gains the mode; the leading input slot is no longer synthesized
here), `src/check.rs` (`check_eliminator_call` synthesizes the annotation input per
decision 4), and `src/check/declarations.rs` (decision 5's registry key).

## Exit

A generic enum is eliminable through the eliminator with bare tags, spelled identically
to the concrete case, with the instantiation coming from the scrutinee. `Result[i64 i64]`
and `Result[bool i64]` eliminate independently in one word (decision 5's registry key,
and the asymmetric-instantiation rule: instantiate multi-variable generics asymmetrically
so a swap of the two arguments cannot pass as agreement). Every existing Slice 3
diagnostic still fires with unchanged wording, decision 6's mode mismatch included
(recon 5's test is the guard). The clause path is untouched and
`tests/phase5_generic_enum_elimination.rs` still passes.

## Ready to spec?

**Yes.** The mechanism is settled (decisions 1-4) and its one genuine hazard is
identified with the test that guards it (recon 5). OQ1 is the only question with design
content left, and it is narrow: a naming-collision discriminator rule, not an
architecture choice.

The spec should re-verify recon 4 and 5 directly rather than inheriting them: 4 is the
claim that the check side already ignores the parse-time type (which, if wrong anywhere,
changes decision 4's shape), and 5 is the claim that
`reconcile_annotation_with_parameter` is the *only* consumer of the annotation's leading
input slot for a tagged literal. Both are cheap to check and expensive to be wrong about.
