# Phase 6 Slice 3: the eliminator word (spec)

Anchors re-verified against `main` at `7186a19` (current HEAD, post-P7.S1-merge and
post-decision-6; supersedes the original `3993f18` baseline). Every `path:line`
below was read at this HEAD; re-confirm before editing if HEAD has moved.

## Summary

Generate a per-enum eliminator word `Shape?` for
`type: Shape | Circle r i64 | Rect w i64 h i64 ;`. It takes one quotation-literal
argument per variant; each arm is annotated with the variant it handles
(`( Circle )`), matched by declared variant, not by position. Missing and duplicated
arms are named errors. Arm annotations escalate from a bare variant name up to a full
row-polymorphic effect. The call lowers to the existing N-way tag dispatch
(`lower_clauses`).

This slice consumes Slice 2's `Type::Variant` and its field accessors and must not
change them. It leaves every existing clause-style path (`WordBody::Clauses`,
`parse_clauses`, `check_clause_word`, `lower_clauses`) working and green. All migration
(moving `examples/vm.sth`, `Bool`, `Result`/`Option` off clause-style dispatch, deleting
`WordBody::Clauses`/`parse_clauses`) is Slice 4 and out of scope here.

## Recon re-verification (done at this HEAD)

- **Recon 4 (no production `PolySig` generator) — CONFIRMED.** The only non-parser
  `PolySig { ... }` constructions are test fixtures: `src/check/declarations.rs:1929`
  is inside `#[test] fn overload_generic_and_concrete_overlap_is_module_scoped`'s local
  `poly_word` helper; `src/check/poly.rs:2562,2659,2682` are all under `#[cfg(test)]`.
  The parser sites (`src/parser.rs:985` `finish`, `parse_poly_effect`) build a `PolySig`
  from surface syntax, not programmatically from an enum. `enum_generated_sigs`
  (`src/check/declarations.rs:1323`) and `variant_generated_sigs`
  (`src/check/declarations.rs:1300+`) emit concrete monomorphic `Sig`s only. So this
  slice writes the **first declaration-time `PolySig` generator**.

- **Recon 5 (`lower_clauses` needs no change) — CONFIRMED with one caveat.**
  `lower_clauses` (`src/ir/func_builder/control_flow.rs:194`) takes
  `clauses: &[Clause]`, `params: &[Value]`, `scrutinee_ty: Type`, resolves the enum id
  from `scrutinee_ty` (not from a name scan), maps each clause to its variant index via
  `self.enums.words[&clause.variant]` returning `EnumWord::Construct(_, vi)`
  (`control_flow.rs:236`), and threads `clause.locals`
  (`control_flow.rs:271`, `let take = clause.locals.len();`). Caveat confirmed: an arm's
  synthetic `Clause` **must** carry `locals: vec![]` (Slice 2 accessors read fields by
  name inside the arm body, so no positional binding is needed) and its `variant` field
  must be the registry key `enums.words` holds (the mangled registry spelling, same key
  `EnumWord::Construct` is registered under at `src/ir/layout.rs:548-550`). With those,
  `lower_clauses`'s **body** is reused unchanged.

  **Correction (verified by compiling it, not by reading; re-corrected again in round 2
  after two independent reviewers reproduced this by building it locally).** This spec
  first claimed the `EnumWord::Construct` destructure at `control_flow.rs:236` "is fine"
  and that nothing outside `lower_clauses` moves. That is wrong. `EnumWord`
  (`src/ir/layout.rs:259-261`) has exactly **one** variant today, so *any* second
  variant — whether decision 3's `Eliminate(EnumId)` or R6's `Get`/`Destructure` — is a
  breaking change to every site that assumed exactly one. Two hard errors result, and
  **they land in different phases**, because R6's `Get`/`Destructure` is what actually
  adds the *second* variant; `Eliminate` (R5) adds a fourth on top of an already
  multi-variant enum:

  - `src/ir/func_builder/control_flow.rs:236` — **E0005, refutable pattern in local
    binding**: `let EnumWord::Construct(_, vi) = self.enums.words[&clause.variant];` is
    an irrefutable `let` that stops compiling the moment a second variant exists at all.
    Fixed in **Phase 3** (R6), the phase that introduces that second variant — not
    Phase 4, as an earlier draft of this correction wrongly attributed it. It needs a
    `let ... else` (or `match`) whose non-`Construct` path is `unreachable!`, since a
    synthetic clause's `.variant` always keys a constructor entry.
  - `src/ir/func_builder/quotation.rs:407-443` — **E0004, non-exhaustive match** in
    `lower_enum_word`. Fixed in **Phase 4** (R5), because R6 (Phase 3) adds real
    `Get`/`Destructure` arms there and keeps the match exhaustive; only `Eliminate`
    (Phase 4) is deliberately left unhandled. This one is a *semantic* ruling, not a
    filler arm: R5's design intercepts `EnumWord::Eliminate` in the `calls.rs` dispatch
    **before** `lower_enum_word` is reached, so the correct arm is
    `EnumWord::Eliminate(_) => unreachable!(...)` with that rationale in the message —
    and the `unreachable!` is only sound *because* of the interception order, which
    makes "the `calls.rs` interception precedes `lower_enum_word`" a stated invariant of
    this slice rather than an incidental detail.

  Neither site changes `lower_clauses`'s logic, so the reuse claim holds; what fails is
  the stronger claim that no existing file moves. An implementer following the earlier
  wording literally would meet compile errors in phases that didn't scope them.

## Decisions (settled in the brief; implemented as-is, not reopened)

1. The eliminator is an ordinary generated word, not new term syntax (an arm is a
   quotation literal, the call is an ordinary call).
2. `QuotAnnot` gains a variant-tag field; no parallel annotation type.
3. No new IR term/instruction kind; extend call-lowering dispatch with
   `EnumWord::Eliminate(EnumId)`, feeding `lower_clauses` synthetic empty-`locals`
   clauses under a new `ArmBinding::WholeValue` mode (R5's correction: existing
   clause-style callers are unaffected, still `Decompose`).
4. A dedicated `check_eliminator_call`, **not** a permutation into
   `check_poly_combinator_args`. See the rule-set table below (OQ1 artifact).
5. Mixed-level sibling arms are legal by construction (each arm reconciles against its
   own slot); cross-arm output agreement stays first-wins on **written source order**,
   not enum-declaration order.
6. **The eliminator's scrutinee may be owning (`Shape`) or a reference (`&Shape`/
   `&!Shape`); every arm's receiver mode matches the scrutinee's, uniformly across all
   arms of one call, and the arm's own annotation must spell that mode** (settled with
   the user, 2026-08-17, after two rounds of narrowing). An owning scrutinee gives every
   arm an owning `Type::Variant`, exactly as originally specced. A reference scrutinee
   gives every arm a reference to the narrowed variant (`&Shape.Circle`/`&!Shape.Circle`)
   — not new machinery: `check_field_projection` already branches this way for ordinary
   fields (`ref_parts`, P7.S1 R1) and `lower_clauses` already branches this way for
   real clause-style words (`control_flow.rs:207-222`); the eliminator reuses both
   branches rather than inventing a third. Mode can never legally vary between sibling
   arms of one call (one scrutinee, one mode), so it is **not** a per-arm independent
   choice — but the arm's annotation must still write it correctly, because `( Circle )`
   is elided sugar for the arm's actual declared effect (R1: `( ..a Shape.Circle -- ..b )`),
   and an annotation that misstates its own quotation's type is exactly what
   `check_literal_against_declared_effect` already rejects for every other quotation in
   the language — no new mismatch-checking logic, this falls out of decision 4's "call,
   never re-implement" for free. Concretely: `( &Circle )`/`( &!Circle )` for a
   reference scrutinee, bare `( Circle )` only for an owning one.

   **This reopens two things settled differently earlier.** R4.2 previously ruled
   reference-mode scrutinees explicitly out of scope ("the surface eliminator is
   value-mode only"); that carve-out is dropped. And admitting a reference to a
   narrowed variant means `Type::Variant` now has to be **interned as a real reference
   referent**, which forces the one thing R5's `WholeValue` correction had flagged as an
   open verification rather than assumed: `src/ir/layout.rs:556`'s
   `ir_type_of(d.referent)` runs **unconditionally over every interned reference type at
   build time**, for the whole program, not lazily — so the moment a program contains a
   reference to a narrowed variant, `ir_type_of(Type::Variant)`'s current
   `unreachable!` (`src/ir/types.rs:286`) is a live, forced panic, not a hypothetical
   one. P7.S1 already hit this exact wall for ordinary variant-field references
   (`&r` on a `Type::Variant` receiver is check-legal today, per P7.S1's own R4, but
   "a build/run golden would panic on the missing lowering arm") and deliberately kept
   its tests check-only to avoid it. This slice can no longer defer it: see R7.

## Requirements

### R1 — Arm-annotation grammar (`src/parser.rs`, `src/ast.rs`)

Extend `QuotAnnot` (`src/ast.rs:1566`) with a variant-tag field:

```rust
pub variant_tag: Option<String>,
```

The field is `pub` on a `pub` struct, so it does not trip `dead_code` in its own phase
even before a reader exists (see Phase plan). A plain (non-arm) literal leaves it `None`;
`parse_quot_annotation` (`src/parser.rs:2362`) must set `variant_tag: None` on the
existing full-form path.

Arm-annotation grammar is a strict extension of the existing four-part form
(`parse_quot_annotation`), where a leading bare variant name can stand in for the parts
elided:

- `( Circle )` — variant only, owning mode; below-scrutinee inputs, the variant input,
  and outputs all elided.
- `( &Circle )` / `( &!Circle )` — same elision, reference mode (decision 6). The
  leading token lexes as one `Word` starting with `&`/`&!`, exactly like an ordinary
  reference-typed slot elsewhere in an annotation (`parse_type_expr`,
  `src/parser.rs:2325`, already branches into `parse_ref_type_expr` on that prefix —
  this is not new lexing or new grammar, only teaching the elision's leading-token
  check to recognize the same prefix it already recognizes in the general four-part
  form's type slots). `variant_tag` stores the **bare** variant name (`"Circle"`, sigil
  stripped) — routing (R4 step 3) matches variant names, which never carry a sigil in
  the `type:` declaration — while the elision's *expansion* carries the sigil through
  into the annotation's declared input type (`&Shape.Circle`/`&!Shape.Circle`, not bare
  `Shape.Circle`), so `check_literal_against_declared_effect` sees the arm's true
  declared type and can reject a mode mismatch the ordinary way (decision 6).
- `( Circle Push -- Vm )` and the fully spelled `( ..a &Vm Push -- ..b )` — the variant
  name (when present as the leading token, itself optionally `&`/`&!`-prefixed) sets
  `variant_tag`, the rest parses as today, sigil and all, via the existing
  `parse_type_expr` path.

R6's existing rule (`( )` and an arrow-less parenthesized list are located errors) stays;
`parse_quotation_annotation_elided_is_error` (`src/parser.rs:3670`) must still pass. The
arrow may be omitted **only** for the lone-variant-name form (`( Circle )` or its
`&`/`&!`-prefixed twin, exactly one token); any additional token requires the `--` as
today, so `( Circle Push )` (two tokens, no arrow) is the same located elided-form error
as a bare `( )`, not a partial arm annotation. The new grammar only *adds* the
leading-variant-name form (bare or reference-prefixed); it does not relax the arrow rule
for a non-arm annotation. Unit tests beside the parser: happy path (bare `( Circle )`
parses with `variant_tag = Some("Circle")` and an owning declared input type;
`( &!Circle )` parses with the **same** `variant_tag = Some("Circle")` but a mutable
reference declared input type — the sigil must not leak into the routing name), and an
error/edge case (a bare `( )` is still the located elided-form error, and
`( Circle Push )` with no arrow is rejected the same way, not accepted as a partial arm).

### R2 — Eliminator `PolySig` generator (`src/check/declarations.rs`)

Add `enum_eliminator_sigs(enums: &[EnumDecl]) -> Vec<(String, String, PolySig)>`
alongside `enum_generated_sigs` (`src/check/declarations.rs:1323`). For each enum, emit
one entry keyed by surface name `"{EnumName}?"` (lowering symbol = the mangled registry
spelling, following `enum_generated_sigs`' D7 keying: env key is bare surface, symbol is
mangled). The generated `PolySig` uses the **minimal subset** (OQ3, below):

- `row_in: Some(a)` — the shared below-scrutinee prefix `..a`.
- `inputs`: the shared prefix row `..a`, then the concrete scrutinee `Type::Enum(id, _)`,
  then N per-arm quotation inputs. Each arm input is
  `PolyType::Quotation(inputs = [Type::Variant(id, vi)], outputs = [], is_inline = true,
  row_in = Some(a), row_out = Some(b))`, built through Slice 2's `variant_type`
  (`src/ast.rs:297`) so the leaked `Enum.Variant` display name has one origin.
  **`is_inline = true`, matching `if`/`branch` (`parser.rs:3639`, `~[ ]` ⇒
  `is_inline == true`) and the brief's own `~[ ... ]` arm spelling — not `false`. An
  `is_inline = false` (materializable `[` quotation) input would admit exactly the
  forwarded-abstract-quotation arm R4 step 1's correction rules out, and would let a
  real runtime `(code, env)` value reach `lower_clauses`, which cannot inline-splice a
  materialized quotation body (the known row-combinator-quotation ICE).

  **Corrected claim (decision 6 made the old one false): `is_inline = true` no longer
  keeps `ir_type_of(Type::Variant)` unreachable — nothing does, and nothing needs to.**
  An earlier draft argued `is_inline = true` was what kept that path airtight (avoiding a
  `Type→IrType` conversion a materialized effect would force). Decision 6 forces the
  conversion anyway, through a different route entirely (`intern_ref_type`'s `RefDecl`
  table, forced by a reference-mode arm's declared type, independent of `is_inline`), so
  `ir_type_of(Type::Variant)` now has a real implementation regardless (R6). The
  `is_inline = true` choice above still stands, but only on its own original grounds
  (matching `if`/`branch`, avoiding the materialized-quotation ICE) — not as a load-bearing
  reason `Type::Variant` never reaches the backend, which is no longer true.

  **This `PolySig`'s scrutinee slot (`Type::Enum(id, _)`) and its arm inputs
  (`Type::Variant(id, vi)`, owning) do not encode decision 6's mode choice, and are not
  meant to.** `check_eliminator_call` (R3) intercepts before this `PolySig` is ever
  unified against via the ordinary poly-call path — it exists for registration/env
  presence, not for per-call type-checking. Mode resolution (owning vs `&` vs `&!`) is
  entirely `check_eliminator_call`'s own job, resolved per call site from the concrete
  operand on the stack (R4 step 2), the same way `check_field_projection` resolves
  receiver mode per call site rather than encoding it in a `Sig`.
- `row_out: Some(b)` — the shared output row `..b`.
- `outputs: vec![]` (the `..b` row carries outputs).
- `bounds: vec![]`, `len_var_names: vec![]`, `ty_var_names: vec![]`,
  `row_var_names: vec!["..a".into(), "..b".into()]`.

**OQ3 resolved:** no `bounds`, no `len_var_names`, no type variables. Each arm's input is
a concrete `Type::Variant`, so nothing per-arm unifies; the only free variables are the
two shared rows. Do not build the full `PolySig` generality.

Unit test beside the generator: assert the generated `PolySig` for a two-variant enum has
exactly two row var names, no bounds, no len vars, no ty vars, and the arm inputs are the
two distinct `Type::Variant`s in declaration order (this is a shape assertion that breaks
if the generator regresses to a `Type::Enum` arm input or drops a row).

### R3 — Eliminator registry + call interception (`src/check.rs`, `src/check/terms.rs`)

Build a checker-side eliminator registry `HashMap<String, EnumId>` (bare surface
`"{EnumName}?"` → `EnumId`) alongside the env registration at `src/check.rs:516`. Thread
it into the term-checking dispatch. In `src/check/terms.rs` (the call dispatch around
`:628`, where combinator interception and poly-call interception live), intercept a call
whose name hits the eliminator registry **before** the ordinary `env`/combinator/poly
paths, and route it to `check_eliminator_call`. The eliminator word is not a user
`Combinator` (it has no body) and must not fall into `inline_combinator`.

### R4 — `check_eliminator_call` (`src/check.rs`)

A dedicated checker for the eliminator call. It **calls, never re-implements**, the
shared helpers. Behaviour, in order:

1. **Arm collection is variable-arity, not a fixed `n = 1 + variant_count` pop, and
   forwarded abstract quotation arms are out of scope this slice.**
   `check_poly_combinator_args`'s underflow guard (`combinators.rs:599`) pops a fixed
   arity and cannot distinguish "an arm is missing" from "the stack is short below the
   scrutinee" — with a fixed pop, a missing arm always presents as underflow before the
   exhaustiveness pass (step 3) ever runs, so R4.1's original promise ("the missing arm
   is named by the exhaustiveness pass") is unimplementable and its test would collapse
   to asserting `underflow_error`'s generic text, a placebo against a deleted
   exhaustiveness scan.

   **Correction: an earlier draft of this step tried to let a forwarded abstract
   quotation stand in for a tagged arm ("or is a forwarded abstract quotation — see
   step 4"). That is self-contradictory and is dropped.** A forwarded operand, by
   `resolve_quotation_operand`'s own definition (`combinators.rs:348`, the
   non-`Literal` branch), carries no `variant_tag` — tags live on a quotation
   *literal's* annotation. So "stop collecting at the first untagged operand" (needed to
   find the scrutinee) and "a forwarded operand is accepted as an arm" (needed for R4
   step 4 as originally drafted) cannot both hold: the collector would either swallow a
   forwarded arm as the scrutinee slot, or reject it as "requires a variant tag" before
   step 4 ever sees it. **Ruling: an eliminator arm must be a quotation *literal*
   carrying a `variant_tag`; a forwarded abstract quotation is rejected the same way an
   untagged literal is** ("eliminator arm requires a variant tag or a literal
   quotation"). Step 4's forwarded-arm acceptance and OQ1 table rows 7/10 are removed
   accordingly — this is a real capability gap (an abstract-quotation-typed arm), not an
   oversight, and is deferred rather than silently ICE'd at lowering (see R5's `is_inline`
   note).

   Collection: pop operands off the top of the stack while each one is a quotation
   *literal* carrying a `variant_tag`, stopping at the first operand that is not a
   tagged quotation literal at all (untagged literal, forwarded quotation, or
   non-quotation value) or when the stack is exhausted. That stopping operand (or the
   exhausted stack) is the scrutinee slot.
   - If the scrutinee slot itself doesn't resolve to an owning or referenced enum for a
     registered eliminator (step 2), `underflow_error` (too few operands below the arms)
     or the ordinary type-mismatch diagnostic (wrong-typed scrutinee) applies, exactly as
     today.
   - Once the scrutinee is confirmed, exhaustiveness (step 3) runs over the arms
     actually collected and names any variant with no collected arm — regardless of how
     many arms were popped, so `variant_count - 1` popped arms genuinely reaches
     exhaustiveness and names the missing one.
   - A collected tagged quotation literal whose tag doesn't resolve to a variant, or an
     untagged/forwarded/non-literal operand that stopped collection early enough to
     starve the arm count, both surface through the ordinary exhaustiveness/unknown-tag
     diagnostics in step 3 — no separate "requires a variant tag" error class is needed
     once forwarded arms are rejected the same way untagged ones are.

   **Collection order.** The checker pushes operands in written source order, so
   popping from the top of the stack yields arms in **reverse written order**. Before
   running exhaustiveness (step 3) or the output-baseline walk (step 5), **reverse the
   collected vector back to written source order.** This is not optional bookkeeping:
   steps 3 and 5 both require a written-order walk, and a pop-loop that skips the
   reversal silently makes the baseline the written-*last* arm instead of the
   written-*first* one — the same class of accidental-implementation trap decision 5's
   "written order, not declaration order" ruling was written to prevent, just arrived at
   through stack-pop order instead of enum-declaration order. Add a test with three arms
   whose written order, declaration order, and stack-pop order are pairwise different,
   asserting the baseline is the written-*first* arm — the existing written-vs-declaration
   test does not catch a reversal bug, since it never varies pop order independently.
2. **Scrutinee type and mode (decision 6, replacing the earlier value-mode-only
   ruling).** The below-arms input must resolve, after stripping any reference, to
   `Type::Enum(id, _)` for the registry's `id` — the same `ref_parts`-based split
   `check_field_projection` already does for an ordinary field receiver
   (`src/check/word_families.rs:300-303`), and the same split `lower_clauses` already
   makes on its `scrutinee_ty` (`control_flow.rs:211-215`). Three legal shapes:
   `Type::Enum(id, _)` (owning), `Type::Ref(rid, false, _)` referencing an `Enum(id)`
   (`&Enum`), `Type::Ref(rid, true, _)` referencing an `Enum(id)` (`&!Enum`). Reject a
   non-enum (and non-reference-to-enum) scrutinee with the existing type-mismatch
   diagnostic. The resolved mode (owning / `&` / `&!`) is recorded once per call and
   used uniformly for every arm in step 4 — it is a property of the call, not of any
   individual arm.
3. **Exhaustiveness + duplication pre-pass**, adapted near-verbatim from
   `check_clause_word` (`src/check/word_entry.rs:288-408`): walk arms in **written
   source order**, look each arm up by its `( Variant )` `variant_tag` against the enum's
   variant list, reject an unknown variant (naming variant and enum), reject a duplicate
   arm (naming the variant), then reject any variant with no arm as non-exhaustive. This
   runs **before** any arm body is checked. An arm with no `variant_tag` (the annotation
   omitted the variant name entirely) is its own located error.
4. **Per-arm body check.** For each arm, look up its variant by `variant_tag`, build that
   arm's expected effect directly from the enum's own variant (`variant_type` →
   `Type::Variant(id, vi)`), then apply step 2's resolved scrutinee mode **uniformly**:
   owning mode uses `Type::Variant(id, vi)` as-is; `&`/`&!` mode wraps it via
   `intern_ref_type(refs, Type::Variant(id, vi), mutable)` (decision 6) before it becomes
   the arm's below-row-top input (shared `..a`/`..b` rows either way). Pass the built
   effect to `check_literal_against_declared_effect` (`src/check.rs:1769`) unchanged.
   That helper already carries the `~`/`[` flavour check, the D3 capture restriction,
   tail-position handling, and the directional body check — including comparing the
   arm's *written* declared type against this built one, which is what rejects an arm
   whose annotation spells the wrong mode (e.g. bare `( Circle )` under a reference
   scrutinee): no separate mode-mismatch diagnostic needed, this is the same check every
   other quotation literal in the language is already held to. This sidesteps
   `apply_subst` entirely (decision 4): the arm effects are built from the enum, so no
   `Type::Variant` survives substitution grounding via that path — it now does survive
   into a different registry (`intern_ref_type`'s `RefDecl` table) when reference mode is
   in play; see R6's `ir_type_of` requirement, which this is why that requirement is no
   longer optional.
5. **Cross-arm output agreement**, following `shape_baseline`'s existing first-wins shape
   (`src/check/combinators.rs:660+`): the first arm **in written order** sets the `..b`
   baseline; a later disagreeing arm is the located error, reported by calling
   `combinator_branch_output_mismatch_error` (`src/check.rs:2085`) **unchanged** — that
   helper's signature is `(ctx, span, word, expected: &[Type], found: &[Type])` and its
   message names the two shapes and a source line, **not an arm or variant**. `expected`
   is the written-first arm's shape, `found` is the offending arm's; that pairing is what
   discriminates written order from declaration order (flipping iteration order swaps
   which shape is `expected`), and it is what decision 4's "call, never re-implement"
   actually buys here — arm-name attribution in the message text is **not** part of this
   slice; adding it would mean forking or extending a helper the live `if`/combinator
   path also depends on, which is its own item with its own test, not a free extension of
   decision 4. **"First" is written source order, not enum-declaration order.**

   **Step 5b — no `Type::Variant` on an arm's exit row** (the Phase 4 ruling; see that
   phase's entry for the alternative it rejects). A second rejection on the same row,
   run per arm before the agreement walk above. An arm that does not consume its
   variant leaves it on the caller's stack; reject that, and a reference to it, at the
   arm's literal span. Only a single-variant enum reaches the check — with two or more
   variants the arms leave different variant types and the agreement walk fires first
   — but it is soundness, not tidiness: `is_copy` (`src/check/builtins.rs:233`) is
   written over `Type::Enum` and falls through to `true` for a `Type::Variant`, so an
   escaped variant of a *linear* enum passes `dup`'s `Copy` gate that its own parent
   enum fails, and its payload's `drop` runs twice. Slice 2's R8 ("a `Type::Variant`
   value has no legal destination outside the arm that bound it") is otherwise
   enforced only by unspellability, which stops the value crossing a word boundary
   (`( W -- W.One )` is `unknown type W.One`) but not sitting on `main`'s stack.
6. **Outputs.** On success the call produces the shared `..b` outputs (the baseline).
   No `Subst` is threaded out: the eliminator is not a self-tail combinator, so there is
   no back-edge grounding.

**Review fix (Phase 2 code review): what an arm's exit does to caller move-state and
borrow provenance, left unspecified above because `if` gets both free from splicing.**
An eliminator never splices, so step 4's per-arm check is the *only* accounting the
checker ever does for that arm — unlike a combinator's argument pre-check (`if`,
`check_poly_combinator_args`), whose own `check_literal_against_declared_effect` probe is
discardable because the splice that follows re-checks whichever arm actually runs, for
real. Two consequences, both settled the same way `check_branch_join`'s two-arm join
already settles them for `if`, generalized from two arms to N:

- **Move-state.** Each arm is checked against its own clone of the caller `Scope`
  (mirroring `then_scope`/`else_scope`), so one arm consuming an outer linear local does
  not leak into a sibling arm's check. Every arm's ending move-state is then joined
  (`Moves::join`, folded pairwise across N arms) into the caller's real `Scope`: a local
  consumed on every arm stays `Moved`, consumed on none stays `Live`, and consumed on
  some but not all becomes `MaybeMoved` — exactly `if`'s join, not silently forgotten.
- **Borrow provenance.** The exit-row baseline (R4 step 5) carries each surviving
  position's real `Slot` (deriv, alias, surviving set), not a type-only re-derivation from
  the pre-call row: two arms that leave *type-agreeing* but differently-rooted borrows
  (or one arm leaving a live borrow the other doesn't) are rejected via the same
  `borrow_join_disagreement_error` `check_branch_join`'s merge already uses, one position
  at a time. Erasing to a provenance-free slot instead (the original implementation's
  defect) let an escaped `&!` alias a second, independently-taken one.

**Review fix (Phase 2 code review, cycle 2): the same erasure on the way *in*, which the
fix above missed by covering only arm outputs.** `check_literal_against_declared_effect`
seeds a boundary's declared inputs as `Slot::computed`, so an arm built from
`inline_quotation_type(vec![narrowed], vec![])` received a `&!Shape.Circle` rooted at
nothing: a reference projected out of it inside the arm left the call unrooted, and two
live `&!` to the same caller place were accepted (the identically-shaped spliced `if`,
whose operand rides in `row` and keeps its real slots, rejects it). Step 4 therefore
hands the arm the caller's **own scrutinee `Slot`, retyped to `narrowed`** — an
input-slot override on the shared helper (`input_slots`, `None` at every other call
site), not a change to the declared effect, so the mode-mismatch comparison is
untouched. This is the linear spine, not a lowering concern: it belongs to this phase,
not Phase 4.

Unit tests beside `check_eliminator_call` (naming `thing_condition_expected`):

- `check_eliminator_call_reference_arm_keeps_the_scrutinee_borrow_rooted` and
  `_arm_cannot_consume_the_borrowed_scrutinee_root` — the override above, from both
  sides: a reference projected out of the scrutinee inside an arm is still rooted at the
  caller's place after the call (a second `&!` conflicts), and the root cannot be
  consumed inside an arm that holds such a projection. Both are accepted if the arm's
  input is seeded provenance-free.
- `check_eliminator_call_sibling_arms_may_each_consume_one_outer_local` — the per-arm
  `Scope` clone above, in its *positive* form: two arms may each consume the same outer
  linear local, because only one arm runs. Fails if the clone is hoisted out of the arm
  loop (which the move-state join test alone does not catch).

- `check_eliminator_call_missing_arm_names_missing_variant` — an omitted arm names the
  missing variant (mirror `check_clause_word_non_exhaustive_names_missing_variant`).
- `check_eliminator_call_duplicate_arm_is_error` — two arms for one variant is the
  duplicate error.
- `check_eliminator_call_unknown_variant_names_it_and_enum` — a `( Squircle )` tag names
  the variant and enum.
- `check_eliminator_call_arm_output_disagreement_is_error` — two arms with disagreeing
  exit shapes trip `combinator_branch_output_mismatch_error`.
- `check_eliminator_call_written_order_sets_baseline` — **decision 5's mandatory test**:
  an enum whose declaration order and arm order differ, where the written-first arm sets
  a baseline the declaration-first arm contradicts. The two arms' `..b` exit shapes must
  be **genuinely distinct concrete types** (e.g. one exits `i64`, the other `bool`), and
  the assertion must pin the *exact* structural `expected`/`found` `Vec<Type>` **passed
  to** `combinator_branch_output_mismatch_error` — not the rendered diagnostic string
  (two distinct `Type`s can `Display` identically, which would let this test pass under
  both orderings) and not "a variant named in the message" (the helper never names one,
  see step 5). `expected` = written-first arm's shape, `found` = declaration-first arm's
  shape. This test **fails under declaration-order iteration**, which would swap
  `expected` and `found`.
- `check_eliminator_call_missing_arm_is_error_not_underflow` — a stack with
  `variant_count - 1` correctly-tagged arms below the scrutinee reaches exhaustiveness
  and names the missing variant, rather than tripping `underflow_error`. This is the
  breakable form of step 1's variable-arity design: fixing the arm collection back to a
  fixed-arity pop makes this test fail (it would report underflow's generic text
  instead of the missing variant's name).
- `check_eliminator_call_forwarded_arm_is_error` — a forwarded abstract quotation
  standing in for one arm is rejected the same way an untagged literal is, not silently
  accepted (step 1's correction). Fails if the collector's tag check is loosened back to
  admitting a forwarded operand.
- `check_eliminator_call_pop_order_does_not_set_baseline` — three arms whose written
  order, enum-declaration order, and stack-pop order are pairwise different; asserts the
  baseline is the written-*first* arm's shape. Fails if the collected-arms vector is used
  in pop order (reverse written order) without the required reversal — the
  written-vs-declaration test above does not catch this, since it never varies pop order
  independently of declaration order.
- `check_eliminator_call_reference_scrutinee_types_arms_by_reference` — decision 6: a
  `&Shape` scrutinee (arms annotated `( &Circle )`/`( &!Rect )` matching the caller's own
  `&`/`&!`) type-checks, with each arm's built expected effect a reference to
  `Type::Variant`, not owning. Fails if step 2 rejects a reference scrutinee outright (the
  old value-mode-only ruling) or if step 4 always builds an owning effect regardless of
  mode.
- `check_eliminator_call_mode_mismatch_is_error` — a `&Shape` scrutinee with an arm
  annotated bare `( Circle )` (owning, not `&Circle`) is rejected by
  `check_literal_against_declared_effect`'s existing declared-vs-expected comparison, the
  same way any other quotation literal misdeclaring its own type would be. No new
  diagnostic path; fails if step 4 silently coerces the mismatch instead of building the
  mode-correct expected effect and letting the shared helper reject the disagreement.

### R5 — Lowering: `EnumWord::Eliminate` (`src/ir/layout.rs`, `src/ir/func_builder/calls.rs`, `src/ir/func_builder/quotation.rs`, `src/ir/func_builder/control_flow.rs`)

**Correction (found by an independent scope reviewer, then reproduced by reading, not
trusted at face value): the earlier wording of this requirement was unbuildable.** It fed
synthetic empty-`locals` clauses into `lower_clauses` **unchanged** on the premise that an
arm body reads its payload via a Slice-2 accessor (R6, below — renumbered from the prior
draft's R6 Golden, now R7). But `lower_clauses` (`control_flow.rs:194-274`)
unconditionally **decomposes** the matched variant's fields onto the IR stack before
running any clause body (`control_flow.rs:253-269`, `load_field_onto_stack`/
`push_reference` per field) — `clause.locals.len()` only controls how many of the
*already-decomposed* values get bound to names; with `locals: vec![]` the decomposed
fields are left stranded, and no `Type::Variant` handle for an accessor to operate on ever
exists. An accessor call inside such an arm body has nothing to lower to: it hits the
generic call path with an unminted symbol. This is now fixed by giving `lower_clauses` a
second, narrowly-scoped arm-population mode rather than by decomposing at all for the
eliminator's arms.

- **`lower_clauses` gains an `ArmBinding` parameter** (`Decompose` | `WholeValue`),
  threaded through its existing signature (`clauses`, `params`, `scrutinee_ty`, plus this
  new arg). Every existing call site (real clause-style words — `Bool`, `Result`,
  `Option`, `examples/vm.sth`) passes `Decompose` and hits the exact code path that exists
  today, byte-for-byte: this is an additive parameter, not a behaviour change to any
  existing caller, so it carries none of Slice 4's migration blast radius. Only the
  eliminator's call passes `WholeValue`.
  - `Decompose` (existing code, unchanged): the per-field loop at `control_flow.rs:253-269`
    runs as today.
  - `WholeValue` (new): skip the per-field loop entirely. In value mode, push `scrutinee`
    itself back onto `self.stack` (no decomposition, no local binding — `clause.locals`
    is asserted empty for this mode, matching the eliminator's `locals: vec![]`). In
    reference mode, `scrutinee` is already the `IrType::Ptr` reference established at
    entry (`control_flow.rs:207-213`); push it unchanged. Either way, the value an arm
    body starts with is the *whole* aggregate/reference the accessor lowering added by R6
    expects as its receiver — the same pointer `resolved_variant_fields`-driven
    projection (owned or reference mode) and `EnumWord::Destructure` read
    `payload_offset + field.offset` from.
  - **Superseded by decision 6, no longer merely an open verification: `WholeValue`
    itself never calls `ir_type_of(Type::Variant(..))`, but reference-mode arms now
    force it elsewhere, on a different path.** The original claim here was that
    `WholeValue` pushes an already-computed `IrType` (`scrutinee`'s own
    `IrType::Enum`/`IrType::Ptr`), never converting `Type::Variant` itself — that part
    still holds by the same reasoning. But decision 6 admits `&Shape.Circle` as an arm's
    *declared type*, which the checker interns via `intern_ref_type(refs,
    Type::Variant(id, vi), mutable)` (a genuinely new `RefDecl` entry, since it differs
    from the caller's own `&Shape` entry). `src/ir/layout.rs:556`'s
    `ref_referents: Vec<IrType> = refs.iter().map(|d| ir_type_of(d.referent)).collect()`
    runs this conversion **unconditionally over every interned reference type, for the
    whole program, at build time** — not lazily, and not only for code lowering actually
    reaches. So a reference-mode eliminator call forces `ir_type_of(Type::Variant)` for
    real, every time one appears anywhere in the program, whether or not that particular
    call is ever executed. See R6 for the fix this now requires (no longer optional).
- Extend `EnumWord` (`src/ir/layout.rs:259-261`) with `Eliminate(EnumId)`. Register the
  eliminator's surface/mangled name in `enums.words` alongside the `Construct` entries
  (`src/ir/layout.rs:548-550`), mapping `"{EnumName}?"` → `Eliminate(id)`.
- In the call-lowering dispatch (`src/ir/func_builder/calls.rs`, both the sym-name path
  around `:250` and the name path around `:645`, mirroring the existing
  `enums.words.get(name)` → `lower_enum_word` interception), an `EnumWord::Eliminate`
  hit pops the N quotation operands (via the existing `quot_bodies`/`quot_defs`
  machinery that `branch` already uses at `calls.rs:520`), maps each to its variant by
  the arm's `variant_tag`, wraps each arm body into a synthetic
  `Clause { variant: <registry key for that variant>, locals: vec![], body, .. }`, and
  calls `lower_clauses(&clauses, params, Type::Enum(id, _), ArmBinding::WholeValue)`. The
  scrutinee is the params' last value.
- `EnumWord::Eliminate` in `lower_enum_word` (`src/ir/func_builder/quotation.rs:407`) is
  not reachable there (it is intercepted in `calls.rs`, not the inline
  alloc/tag-store path); the match arm should `unreachable!` with a note, or the
  interception should be structured so `lower_enum_word` never sees `Eliminate`. Prefer
  the latter (intercept in `calls.rs` before `lower_enum_word` is called), so
  `lower_enum_word` keeps only `Construct`.

**OQ4 resolved:** a one-variant enum's eliminator uses the general path (single synthetic
clause, `lower_clauses` with `n == 1` builds a one-arm dispatch and a join with one
predecessor); it is not special-cased. A zero-variant enum is not constructible (no
constructor exists to make a value), so its eliminator is unreachable and needs no
special handling. One-line ruling: no degenerate special-casing; the general path covers
both.

Unit test beside the lowering: an eliminator IR-lowering test (`lower_src`) that asserts
the dispatch emits the same tag-dispatch/phi-join shape a clause word does (reuse the
`lower_clauses` test helpers), proving the synthetic-clause path reaches `lower_clauses`.
A second test, reference-mode: an eliminator call over `&Shape`/`&!Shape` reaches the
same dispatch shape and does not panic on `ir_type_of(Type::Variant)` (see R7) — this is
the test that actually exercises the `ref_referents` build-time conversion decision 6
now forces, not a hypothetical.

### R6 — Variant-accessor IR lowering (`src/ast.rs`, `src/check.rs`, `src/check/word_families.rs`, `src/ir/layout.rs`, `src/ir/func_builder/quotation.rs`, `src/ir/func_builder/word_families.rs`, `src/ir/driver.rs`)

**Re-pointed after P7.S1 merged (`3b9bbce`), which retired the fused accessor spelling
this requirement originally targeted.** The prior draft of R6 planned `EnumWord::Get`
and `Destructure` as a name-fused mirror of `StructWord::Get`/`Destructure`
(`Variant>field` → a registry lookup by name). P7.S1 deleted that whole *mechanism* for
structs — `StructWord::Get`/`Set`/`Peek` are gone (`slice1-spec.md` D3) — and replaced
field access with a **receiver-directed projection**, `&field`/`&!field`, resolved per
call site against the receiver's *type*, not a name fused with the aggregate's own name.
P7.S1 shipped this for structs and *already extended the checker side to variants*
(`check_field_projection`, `src/check/word_families.rs:268-`, its `Type::Variant` arm at
`:314-317`) — a variant projection like `&r` on a `Type::Variant` receiver is
**check-legal today**. P7.S1's own spec explicitly flags this requirement by name: "P6.S3's
R6 ... must be re-pointed at this shape before implementation, including its own
`EnumId`-keyed lowering-side table rather than a widened `resolved_fields`"
(`docs/roadmap/P7/slice1-spec.md`, R4). This is that re-pointing.

**What P7.S1 left deliberately unfinished, and what this requirement now closes.**
`resolved_fields` (`src/ast.rs:82`, `HashMap<Span, (StructId, usize)>`) is the side table
a struct projection's resolution rides from checker to lowering (P7.S1 R2): the checker
can't route by name (the name is a bare field like `hp`, ambiguous across every struct
that has an `hp` field), so it records `span → (StructId, field index)` at the call site
instead, threaded through `check.rs` → `ast.rs`'s `Module::resolved_fields` →
`ir/driver.rs` → `FuncBuilder`, and `lower_reference_word`'s `_` arm
(`word_families.rs:52-70`) reads it back per span rather than consulting a name registry.
`check_field_projection`'s `Type::Variant` arm already resolves a variant field the same
way (same function, same fields/name lookup, just a different decl table) — but it
**never inserts into `resolved_fields`**, because that table is `StructId`-keyed and has
no shape for an `EnumId`. There is a standing test proving this is deliberate, not an
oversight (`word_families.rs:2181-2222`, asserting `resolved_fields.is_empty()` after
checking a variant projection). So today a variant projection **type-checks and then has
nowhere to lower to** — exactly the representation gap round 1 of this spec's review
found for the retired `Circle>r` spelling, now relocated to `&r`.

This requirement supplies the missing table and its lowering read, mirroring P7.S1's own
R2/R6 structurally rather than reusing `resolved_fields` itself (per P7.S1's own
instruction: "its own `EnumId`-keyed lowering-side table"):

- Add `resolved_variant_fields: HashMap<Span, (EnumId, usize /* variant */, usize /* field */)>`
  alongside `resolved_fields`: a `Module` field (`src/ast.rs:82`), a `PolyCtx`-riding
  scratch field in `check.rs` next to `resolved_fields` (`check.rs:140`, same rationale —
  it must survive an `if`-arm clone), threaded through `ir/driver.rs` and `FuncBuilder`
  (`ir/func_builder/mod.rs:180` and every call site that threads `resolved_fields`)
  identically to how `resolved_fields` is threaded, so this is additive plumbing along an
  existing, proven path, not a new one.
- In `check_field_projection`'s `Type::Variant` arm (`word_families.rs:314-317`), insert
  `(span, (id, vi, fi))` into `resolved_variant_fields` — the one line P7.S1 explicitly
  did not add. **The existing canary test
  (`word_families.rs:2181-2222`, `resolved_fields.is_empty()`) must be updated, not
  deleted**: it should now assert the entry lands in `resolved_variant_fields` and that
  `resolved_fields` (the struct table) stays empty — preserving the real invariant it
  guards (a variant must never be misrouted into the `StructId`-keyed table) while
  reflecting that variant projection is no longer checker-only.
- In `lower_reference_word`'s `_` arm (`word_families.rs:52-70`), add an
  `else if let Some(&(id, vi, fi)) = self.resolved_variant_fields.get(&span)` branch
  beside the existing `resolved_fields` lookup, following the identical owned/reference
  consuming logic already there (`ref_inner.contains_key(&base)` decides whether the
  receiver is popped, per P7.S1 D2) — the only difference is the addressed field:
  `self.enums.layouts[id.index()].variants[vi].fields[fi]` at
  `payload_offset + field.offset`, not the struct arm's bare `field.offset`.
- The whole-variant destructure (`Circle>`) is unaffected by P7.S1 — it is a globally
  unique fused name (`variant_generated_sigs` still registers only `"{surface}>"`,
  `src/check/declarations.rs:1356-1379`, confirmed: the per-field scalar-getter entries
  that loop used to also emit are gone), so it keeps the original name-registry design:
  extend `EnumWord` (`src/ir/layout.rs:259-261`) with **`Destructure(EnumId, usize)`
  only** (no `Get` variant — that mechanism moved to `resolved_variant_fields` above),
  register `"{Variant}>"` → `Destructure(id, vi)` in the `ewords` loop
  (`layout.rs:564-574`, adopting the struct registry's dual-key `insert` closure rather
  than inlining a second copy — `layout.rs:526-530`), and extend `lower_enum_word`
  (`quotation.rs`) with the one `Destructure` arm, reading every field at
  `payload_offset + field.offset` in order (mirroring `lower_struct_word::Destructure`).
- **`ir_type_of` gets a real `Type::Variant` case (`src/ir/types.rs:286`), replacing
  the `unreachable!("a Type::Variant never reaches the backend (Slice 3)")`.** This is
  new and load-bearing per decision 6, not part of the original R6 scope: admitting a
  reference-mode eliminator scrutinee means an arm's declared input type can be
  `&Shape.Circle`, interned via `intern_ref_type(refs, Type::Variant(id, vi), mutable)`
  — a real `RefDecl` entry, not a hypothetical one — and `src/ir/layout.rs:556`'s
  `ref_referents` computes `ir_type_of` over **every** interned referent unconditionally
  at build time, whether or not that reference is ever exercised at runtime. Erase
  `Type::Variant(id, _, _)` to the same `IrType::Enum(id)` its parent enum already gets
  (`ir_type_of(Type::Enum(id, _))`'s existing arm) — a variant is represented identically
  to its enum at the backend; only the frontend distinguishes them, which is exactly the
  erasure R2's own note already assumed ("`Type::Variant` maps to the same
  `IrType::Enum(id)` at the backend") before this decision made it load-bearing instead
  of incidental. The retired `#[should_panic]` test at `src/ir/types.rs:536-541`
  (asserting this exact `unreachable!` fires) must be replaced with a positive assertion
  that `ir_type_of(Type::Variant(id, vi, name)) == IrType::Enum(id)` for the same `id` a
  plain `Type::Enum(id, _)` erases to — not merely that it no longer panics.

Unit tests beside each lowering site, against hand-built IR state (this mechanism has no
surface-syntax caller until R5 exists in the same slice, so these are exercised directly,
the same way P7.S1 and Slice 2 both unit-tested a checker-only mechanism against
hand-built state before a lowering arm existed):

- A projection lowering test asserting `resolved_variant_fields` is read and produces a
  reference at `payload_offset + field.offset` for the correct variant — use a test enum
  whose variants have *different* field layouts at the tested index, so a wrong-variant
  mutation (reading `variants[vi']` for the wrong `vi'`) is visible rather than masked by
  identical layouts.
- Owned-receiver vs reference-receiver consuming behaviour: the owned case leaves the
  receiver on the stack (per D2), the reference case pops it — asserted by stack-depth
  after lowering, not merely that a reference is produced.
- A `Destructure` test on a **multi-field** variant asserting it pushes exactly N values
  in field order, plus a companion zero-field-variant test asserting it pushes nothing
  without panicking (the zero-field case alone doesn't catch a mutation that always
  pushes nothing).
- The updated canary test (`word_families.rs:2181-2222`): a variant projection populates
  `resolved_variant_fields` and leaves `resolved_fields` empty — fails if a future change
  routes a variant into the struct table (the original misdispatch risk P7.S1's version
  of this test existed to rule out).

### R7 — Golden (`examples/` + test harness)

**Two `.sth` goldens, not one — decision 6 makes both modes exit witnesses, not just one
owning-mode example.** Neither mode is a strict subset of the other's coverage: owning
mode exercises `Type::Variant` reachability from surface syntax for the first time ever
(unlike Slice 2); reference mode exercises the `ir_type_of(Type::Variant)` real
implementation (R6) and the `intern_ref_type`/`ref_referents` build-time path decision 6
forces, which the owning-mode golden never touches at all.

- **Owning-mode golden**: construct a `Type::Variant` value via the eliminator, read a
  field via a P7.S1-style `&field`/`&!field` projection inside an arm (the spelling
  P7.S1 retired the fused `Circle>field` form in favour of), produce a result.
- **Reference-mode golden**: call the eliminator over `&Shape`/`&!Shape` (arms annotated
  `( &Circle )`/`( &!Rect )` accordingly), read/write a field through the narrowed
  reference inside an arm, and confirm the original `Shape` is still owned by whatever
  held the reference — nothing was consumed by the call.

Both assert observable program output (source in → expected output), not merely that
they compile. Migration of `examples/vm.sth`/`Bool`/`Result`/`Option` is **Slice 4**.

## OQ1 artifact: `check_poly_combinator_args` rule set vs `check_eliminator_call`

The mitigation for decision 4's accepted cost (a second copy of rules that already
exist). Each rule enforced by `check_poly_combinator_args`
(`src/check/combinators.rs:580-756`), and what `check_eliminator_call` does with it. A
reviewer diffs the two paths against this table: a rule present there and silently absent
here is a defect unless a **Reason** column explains it.

| # | Rule in `check_poly_combinator_args` | Anchor | `check_eliminator_call` | Reason |
|---|---|---|---|---|
| 1 | Underflow: `stack.len() < n` → `underflow_error` | `combinators.rs:618` | **Arm-flavoured variant.** Nominal arity is `1 + variant_count`, but collection is variable-arity (R4.1): a missing arm is reported by the exhaustiveness pass (R4.3) by name, a genuine sub-scrutinee shortfall by `underflow_error` | Exhaustiveness gives a better-attributed message than a bare count mismatch |
| 2 | Pass 1: unify non-quotation inputs → `θ` (`unify_poly_input`) | `combinators.rs:636` | **Reduced to a scrutinee type check.** The only non-quotation input is the scrutinee; match it against `Type::Enum(id, _)` via the shared type-mismatch path | No per-arm tyvar exists to solve; arm inputs are concrete `Type::Variant` |
| 3 | Deferred i64-literal coercion (D8: bare `Var` filled by a fresh literal, unified last against `usize`/`isize`) | `combinators.rs:634,645-660` | **Omitted.** | No bare `Var` input parameter exists; the scrutinee is a concrete enum, no literal-coercion slot |
| 4 | `reject_quotation_argument` for a non-quotation slot given a quotation | `combinators.rs:641` | **Called (shared).** The scrutinee slot must not be a quotation | Same guard applies to the scrutinee |
| 5 | Pass 2: `apply_subst` to ground each quotation parameter's declared effect | `combinators.rs:665` | **Omitted deliberately.** Each arm's expected effect is built directly from the enum's variant (`variant_type`), not ground through `θ` | Decision 4: sidesteps `apply_subst` so no `Type::Variant` survives substitution grounding |
| 6 | Row reconstruction (`row = stack[..base]` for a row-bearing param) | `combinators.rs:680` | **Arm-flavoured variant.** The shared `..a` grounds to the below-scrutinee region for every arm, computed once | Same grounding, one shared prefix across all arms |
| 7 | `resolve_quotation_operand`: Literal vs Forwarded vs None | `combinators.rs:692,741,746` | **Reduced: only the `Literal` outcome is accepted.** A `Forwarded` outcome is rejected the same way an untagged literal is (R4 step 1's correction); `None` → `quotation_argument_required_error` | Forwarded abstract quotation arms are out of scope this slice — see R4 step 1 and rule 10 |
| 8 | `check_literal_against_declared_effect` (flavour `~`/`[`, D3 capture, tail handling, directional body check) | `combinators.rs:695` | **Called (shared), unchanged.** Per arm, against the effect built in rule 5's replacement | The point of decision 4's "call, never re-implement" |
| 9 | Cross-sibling output agreement via `shape_baseline` (first-wins per shared row id) → `combinator_branch_output_mismatch_error` | `combinators.rs:716-733` | **Called (shared), unchanged.** One shared `..b` row, so one baseline; first **written** arm sets it; the helper's message names shapes and a line, not an arm — no arm-attribution is added this slice | Decision 5: keeps first-wins verbatim; the only change is iterating arms in written order (R4.5), not the message |
| 10 | Forwarded abstract quotation acceptance (`found.ty == concrete`) | `combinators.rs:735-745` | **Omitted.** A forwarded operand is rejected in rule 7, never reaches this acceptance step | Deferred capability, not an oversight: an abstract-quotation-typed eliminator arm needs its own design (routing by tag when there is no literal annotation to carry one) and is left for a future slice rather than silently accepted and ICE'd at lowering |
| 11 | `quotation_argument_required_error` for a non-quotation operand | `combinators.rs:746` | **Called (shared).** An arm slot given a non-quotation | Same guard |
| 12 | Returns `Subst` (`θ`) for the caller's back-edge grounding | `combinators.rs:754` | **Omitted.** Returns unit/outputs; the eliminator is not a self-tail combinator | No back-edge to ground |
| — | (absent) exhaustiveness + duplication pre-pass | — | **New**, from `check_clause_word` (`word_entry.rs:288-408`) | The eliminator's arms are a value shape `check_poly_combinator_args` never sees |
| — | (absent) arm-to-variant routing by `( Variant )` tag | — | **New.** Arms matched by `variant_tag`, not by slot position | Decouples call sites from declaration order (recon 3's consequence) |

## Phase delivery plan

Each phase is independently green (`cargo fmt --check && cargo clippy -- -D warnings &&
cargo test`). Plumbing is never scheduled ahead of its first call site.

### Phase 1 — arm-annotation grammar

Scope: R1. Add `QuotAnnot.variant_tag: Option<String>` and the leading-variant-name
arm-annotation grammar in `parse_quot_annotation`. The field is **`pub` on the `pub`
`QuotAnnot`**, so it survives this phase's clippy `-D warnings` gate with no reader yet
(pub struct fields are not `dead_code`). Set `variant_tag: None` at the existing
non-arm construction site. Parser unit tests: `( Circle )` parses with
`variant_tag = Some("Circle")` (happy path); `( )` is still the located elided-form
error (edge). Exit: parser tests green; `parse_quotation_annotation_elided_is_error`
still passes.

Exit criteria (breakable assertions):

- A test parsing `[ ... ( Circle ) ]` asserts `variant_tag == Some("Circle")` on the
  literal's annotation — fails if the field is never populated.
- A test asserting `( )` yields the arrow-missing/elided error — fails if the new
  grammar wrongly accepts the bare form.

### Phase 2 — frontend: generator, registry, `check_eliminator_call`

Scope: R2, R3, R4. Add `enum_eliminator_sigs` (the first declaration-time `PolySig`
generator), the checker-side eliminator registry, the call interception in
`src/check/terms.rs`, and `check_eliminator_call` (calling the shared helpers per the
OQ1 table). Every item added here has a call site within this phase: the generator feeds
env registration and the registry, the registry feeds the interception, the interception
calls `check_eliminator_call`. No lowering yet, so tests are `check_src`-level (no
`.sth` build of an eliminator call). Nothing existing regresses: registering `Shape?`
in env/registry does not affect programs that never call it, and mints no IR symbol.

Exit criteria (breakable assertions):

- `check_eliminator_call_missing_arm_names_missing_variant`, `_duplicate_arm_is_error`,
  `_unknown_variant_names_it_and_enum`, `_arm_output_disagreement_is_error` — each
  asserts the specific message text (variant/enum names, both shapes), not just that an
  error occurred.
- `check_eliminator_call_written_order_sets_baseline` — decision 5's test: an enum whose
  declaration order and arm order differ, written-first arm sets the baseline, and the
  assertion pins the **declaration-first** arm as the named offender. Fails under
  declaration-order iteration (which would name the other arm).
- A `PolySig`-shape test for `enum_eliminator_sigs`: two row vars, no bounds/len/ty
  vars, arm inputs are the two distinct `Type::Variant`s. Fails if the generator
  regresses to `Type::Enum` arm inputs or drops a row.
- `check_eliminator_call_missing_arm_is_error_not_underflow` and
  `_forwarded_arm_is_error` — the variable-arity collection design (R4 step 1): a
  correctly-tagged short stack reaches exhaustiveness and names the missing variant
  rather than tripping `underflow_error`, and a forwarded abstract quotation is rejected
  the same way an untagged literal is, not silently accepted.
- `check_eliminator_call_pop_order_does_not_set_baseline` — three arms whose written,
  declaration, and stack-pop order are pairwise different; the baseline must be the
  written-*first* arm. Fails if the collected-arms vector is used without the
  required written-order reversal.
- **Decision 6, least-reviewed, both required here:**
  `check_eliminator_call_reference_scrutinee_types_arms_by_reference` — a `&Shape`
  scrutinee types every arm's built expected effect as a *reference* to `Type::Variant`,
  not owning; fails if step 2 rejects a reference scrutinee outright or step 4 always
  builds an owning effect regardless of mode. `check_eliminator_call_mode_mismatch_is_error`
  — a `&Shape` scrutinee with a bare `( Circle )` arm (wrong mode) is rejected by
  `check_literal_against_declared_effect`'s existing declared-vs-expected comparison;
  fails if step 4 silently coerces the mismatch instead of building the mode-correct
  expected effect and letting the shared helper reject the disagreement.

**As shipped, deviating from the letter above (all reviewed and accepted):**

- **Registering `Shape?` is not free after all.** `Shape?` reaches `resolve` as a name
  whose trailing `?` `split_destructure_suffix` must recognize (so the *type* branch
  mangles it to the registry's own `Shape__m0?` key). That made every ordinary word whose
  name ends in `?` (`ok?`, `zero?`) look generated to the four name-table branches that
  skipped a non-empty suffix, breaking a cross-module call to one that worked before this
  slice. A generated word is identified by its type prefix naming a real type — that
  branch returns on its own — so the suffix alone no longer gates the qualified-word,
  own-word, static-borrow, or selective-import branches
  (`word_named_with_a_generated_suffix_resolves_in_every_branch` pins all four).
  **Eligibility is per suffix, not per "names a type at all"** (cycle 3): each generated
  suffix comes from exactly one kind of type — `>` is the struct destructure, `?` the enum
  eliminator — so `NameTables` keeps structs and enums in separate sets and the type
  branch asks `names_a_type(module, head, suffix)`. Under the looser gate a plain word
  called `P?` beside a *struct* `P` was mangled to `P__m0?`, a generated name nothing
  generates, and the call became `unknown word`; the mirror hole (`E>` beside an *enum*
  `E`) predates this slice and closes with it
  (`word_named_for_another_kinds_generated_suffix_stays_a_word`).
  The REPL's import rewrite (`rewrite_import_call`) had the same regression from the same
  cause: it split the suffix before consulting `import_aliases`, so an imported word named
  `ok?` missed the alias installed under its whole name. It now tries the whole spelling
  first and splits only if that misses — and R15's `not exported` wording follows the same
  rule, naming the *type* for a generated word (`q::P>` → `P`, gated as one unit) and the
  word itself for a suffix-spelled one (`q::ok?` → `ok?`)
  (`import_call_to_a_word_named_like_a_generated_one_resolves`).
- **The REPL needs its own rejection, twice.** Eliminator interception runs ahead of the
  env lookup at a session line too, so `check_no_word_shadows_eliminator` is called from
  the `Line::Def` fan-out; and because a session declares one thing per line, the reverse
  ordering (a `type:` line whose eliminator name a session word already holds) is caught
  in `eval_enum_typedef` against the session's own word names.
- **A tagged literal must reach its call by *written* adjacency.** The literal-side check
  that a `( Circle )` tag is actually consumed as an arm is syntactic, so a stack-neutral
  term written between two arms is rejected even though the stack-based collection would
  accept it. The looser alternative (scan forward past anything) re-opens the hole the
  check exists to close, so the rule stands and the diagnostic states it.
- **An untagged literal and a forwarded quotation share one error class**
  (`eliminator_untagged_arm_error`) rather than reusing `quotation_argument_required_error`
  as OQ1 row 7 had it.
- **Decision 5's pairing is pinned by a pure function, not a test-only side channel.**
  The cross-arm comparison is `arm_exit_row_mismatch(baseline, arm) -> Option<(Vec<Type>,
  Vec<Type>)>`, whose unit test
  (`arm_exit_row_mismatch_pairs_baseline_first`) asserts the returned pair by structure:
  the baseline is `expected`, the arm under check is `found`. `_written_order_sets_baseline`
  and `_pop_order_does_not_set_baseline` then assert the rendered message only, which is a
  sound ordering discriminator because their two baselines (`bool`/`i64`) `Display`
  distinctly. (Cycle 2 shipped a `cfg(test)` thread-local capture written from inside the
  production error helper for this; cycle 3 removed it — a global mutable side channel in a
  helper the live `if` path also calls, for an assertion a pure split gets directly.)
- **The eliminator's `PolySig` is registered in `poly_env`, and nothing reads it this
  phase.** `check_term`'s interception precedes every env/poly lookup unconditionally, and
  a colliding user word is rejected outright by `check_no_word_shadows_eliminator`, so the
  registration changes no dispatch and no diagnostic: removing it leaves the whole suite
  green. It stays because it is the generator's only production consumer in this phase
  (removing it makes R2's `enum_eliminator_sigs` dead code, which is clippy-fatal under
  `-D warnings`), and the signature's paired lowering symbol becomes load-bearing in Phase
  4. Note the asymmetry for whoever touches env assembly: the three other assembly sites
  (the REPL's, and the two poly entry points) build only `eliminator_registry`.

### Phase 3 — variant-accessor IR lowering

Scope: R6 (re-pointed at P7.S1's receiver-directed projection shape, not the retired
fused spelling). Add `resolved_variant_fields` and thread it from
`check_field_projection`'s `Type::Variant` arm through to `FuncBuilder`, mirroring
`resolved_fields`'s existing threading exactly; extend `lower_reference_word` with the
read of it; add `EnumWord::Destructure` (the whole-variant fused word only — no `Get`
variant, that mechanism is the side table) and its `lower_enum_word` arm. Independently
green and independently testable: `resolved_variant_fields` is populated and read within
this phase regardless of whether anything calls a variant projection yet via surface
syntax, the same chicken-and-egg P7.S1 and Slice 2 both already resolved by unit-testing
against hand-built state. This phase adds no surface way to reach a variant projection;
that arrives in Phase 4.

**Required in this same phase, not Phase 4** (this attribution held up under round-2
review, reproduced by two independent reviewers building it locally — re-verify after the
R6 re-pointing, since `EnumWord` now gains only one new variant instead of two, but the
underlying mechanism is unchanged): `EnumWord` has exactly one variant (`Construct`)
before this phase. Adding `Destructure` here is what first turns it into a multi-variant
enum, so it is **this** phase, not Phase 4's `Eliminate`, that trips
`src/ir/func_builder/control_flow.rs:236`'s irrefutable
`let EnumWord::Construct(_, vi) = self.enums.words[&clause.variant];` (E0005: refutable
pattern) — *any* second `EnumWord` variant trips it, and `Destructure` is the second.
Fix it here: replace the irrefutable `let` with a `let ... else` (or `match`) whose
non-`Construct` path is `unreachable!("a clause always dispatches to a variant
constructor")`. `src/ir/func_builder/quotation.rs:407-443`'s `lower_enum_word` match stays
exhaustive through this phase (`Destructure` gets a real arm here), so it does **not**
break in Phase 3 — only in Phase 4, when `Eliminate` is added without a corresponding
`lower_enum_word` arm (by design, R5).

Exit criteria (breakable assertions):

- `cargo build`/`cargo clippy -- -D warnings` pass with both `EnumWord` variants present
  (`Construct`, `Destructure`) — fails if `control_flow.rs:236`'s fix is deferred, per
  the correction above.
- A projection lowering test asserting `resolved_variant_fields` is read and the pushed
  reference's address is `payload_offset + field.offset` for the correct variant **and**
  the correct `IrType`. Use a test enum whose variants have *different* field layouts at
  the tested index (not two same-shaped variants) — otherwise a wrong-variant mutation
  (reading `variants[vi']` for the wrong `vi'`) is invisible, and an address-only
  assertion never catches a wrong load width/signedness.
- An owned-vs-reference-receiver consuming test: the owned case leaves the receiver on
  the stack (asserted by stack depth after lowering, per D2), the reference case pops it.
- A `Destructure` lowering test on a **multi-field** variant asserting it pushes exactly
  N values in field order, **plus** a companion test on a zero-field variant asserting
  it pushes nothing and does not panic. The zero-field case alone does not catch a
  mutation that always pushes nothing regardless of field count.
- The updated canary test (`word_families.rs:2181-2222`, see R6): a variant projection
  populates `resolved_variant_fields` and leaves `resolved_fields` (the struct table)
  empty — fails if a future change misroutes a variant into the struct-keyed table.
- `ir_type_of(Type::Variant(id, vi, name)) == IrType::Enum(id)` (R6) — a positive
  equality assertion, not "does not panic." Replaces the retired
  `#[should_panic]` test at `src/ir/types.rs:536-541`. Fails if the erasure regresses to
  a variant-specific `IrType` instead of collapsing to the same representation as its
  parent enum.

### Phase 4 — eliminator lowering + golden

Scope: R5, R7. Add the `ArmBinding` parameter to `lower_clauses` (`Decompose` default,
unchanged for every existing caller; `WholeValue` new), `EnumWord::Eliminate(EnumId)`,
register it in `enums.words`, intercept it in the `calls.rs` dispatch (before
`lower_enum_word`), build synthetic empty-`locals` clauses from the N quotation operands,
and feed `lower_clauses` under `ArmBinding::WholeValue`. Add the end-to-end `.sth` golden.
This phase is where an eliminator call first compiles and runs, and where Phase 3's
accessor lowering is first exercised by a real program.

**Required in this same phase** (corrected in round 2, re-verify after the R6
re-pointing: `control_flow.rs:236` was already fixed in Phase 3, since `Destructure`
tripped it first — only the `lower_enum_word` match remains to update here):

- `src/ir/func_builder/quotation.rs:407-443` — E0004: `lower_enum_word` gains
  `EnumWord::Eliminate(_) => unreachable!(...)`, sound only because the `calls.rs`
  interception precedes it. Order the interception before `lower_enum_word` deliberately,
  and say so in the arm's message.
- Between Phase 2 and this one, a *checked* eliminator program panics when built:
  `Shape?` passes the checker and then reaches the generic call path with no minted
  symbol (`src/ir/func_builder/calls.rs`, `checked user word exists`). No stopgap guard
  is added for it — the interception this phase installs is the fix — but the golden
  below is what proves it gone, so it must build and run, not merely check. The same
  panic hits at the REPL, where it kills the session rather than printing an error.
- **Write the golden's arms adjacently.** Phase 2's tag-consumption check is syntactic
  (see its `as shipped` notes): a stack-neutral term written *between* two arms is
  rejected, even though the stack-based arm collection would accept it. A golden needing
  the looser form would be asking for a rule change, not exercising this phase.
- **Ruled: a `Type::Variant` may not leave the call** (R4 step 5b). The open alternative
  was to allow it, on the grounds that R6's erasure gives `Type::Variant` the same
  `IrType` as its parent enum (`src/ir/types.rs:286`'s `unreachable!` is a real case
  since Phase 3) and the value is representationally just the enum. That is false as a
  frontend argument: representational identity is not *type-rule* identity, and every
  type-directed predicate outside the eliminator is written over `Type::Enum` with a
  fall-through default. `is_copy` is the one that bites — it reads a `Type::Variant` as
  trivially `Copy`, so `1 R One ~[ ( One ) ] W? dup drop drop` (over a linear
  `type: W | One a R ;`) built and ran `R`'s `drop` twice, while the identical `dup` on
  `W` itself is rejected as linear. Allowing the escape would therefore mean auditing
  every such predicate for a `Type::Variant` arm and making the type first-class, which
  is the opposite of what Slice 2 declared it to be. Rejecting is the smaller rule and
  the one R8 already states; it costs only the degenerate no-op arm on a single-variant
  enum.
- **Phase 2 leaked the mangled enum name into three diagnostics** (found in Phase 4's
  review, fixed there since no later phase owns it). `check_eliminator_call` reads the
  enum's name from `EnumDecl::name`, which `resolve` mangles — unlike `name_static`,
  which every `Type::Enum` render already uses — so a real build named the missing
  variant's enum `Shape__m0`. Phase 2's own unit tests could not see it: `check_src`
  skips `resolve_modules`, so the name is bare there whatever the renderer does. Fixed
  at the single read rather than the three render sites, and guarded by a
  `resolve_modules`-then-`check` test beside the sibling one that already covers the
  *call* name.

Exit criteria (breakable assertions):

- An IR-lowering test (`lower_src`) asserting the eliminator dispatch reaches
  `lower_clauses` and emits the tag-dispatch/phi-join shape (one join predecessor per
  non-tail arm) — fails if the synthetic-clause construction is wrong (e.g. non-empty
  `locals` mis-binds, or the variant key mismatches `enums.words`).
- A `Decompose`-mode existing clause-style test (e.g. an existing `Bool`/`Result` clause
  lowering test) is unchanged and still passes — proves `ArmBinding` is additive, not a
  behaviour change to real clause words.
- Checker tests that a single-variant enum's arm may not leave its variant, nor a
  reference to it, plus one pinning *why*: the same program's parent enum fails `dup`'s
  linearity gate that the escaped variant would have passed — so the rejection cannot be
  weakened back to a tidiness rule without a failing test.
- The `.sth` golden asserts observable output for a multi-variant enum eliminated
  end-to-end with a `&field`/`&!field` projection read inside an arm — fails if
  elimination or field access regresses. This is the first test that exercises Phase 3's
  projection lowering through a real surface program rather than hand-built IR state.

## Out of scope (restated)

- Migrating `examples/vm.sth`, `Bool`, `Result`/`Option` off clause-style dispatch, or
  deleting `WordBody::Clauses`/`parse_clauses` — Slice 4.
- Any change to `Type::Variant`, `variant_generated_sigs`, or `check_field_projection`'s
  existing struct behaviour — Slice 2's checker-side variant work and P7.S1's
  receiver-directed projection are both done; this slice only consumes them. R6's one
  addition to checker code (the `resolved_variant_fields` insert in
  `check_field_projection`'s already-shipped `Type::Variant` arm) is filling in a gap
  P7.S1 explicitly left as a no-op for this slice to close (P7.S1's own R4), not
  reopening either slice's design.
- Any change to `EnumLayout`/`VariantLayout` or `dispatch_on_tag`. `lower_clauses`
  gains an additive `ArmBinding` parameter (R5); every existing caller keeps its current
  behaviour bit-for-bit under `Decompose` — this is not the block/phi dispatch shape
  changing, only which values populate the stack before a clause body runs.
- Forwarded abstract quotation arms (an eliminator arm must be a quotation *literal*
  carrying a `variant_tag`; see R4 step 1, OQ1 rows 7/10) — a real capability gap,
  deferred rather than accepted and left to ICE at lowering.
- **Self-tail recursion *through* an eliminator arm (found in Phase 4, and a Slice 4
  blocker).** A clause word whose every clause ends in a self-call becomes a loop (R6/R7);
  the same recursion written as eliminator arms does not, because the self-tail analysis
  that opens the loop header does not look inside an arm's quotation literal. The arm's
  self-call lowers to a real call, which is *correct* but not a loop, so deep recursion
  exhausts the C stack (measured: a 2,000,000-deep count segfaults). Phase 4 only
  guarantees the arms of a **non**-tail eliminator call never back-edge — they must not,
  or the arm would skip every term after the call, which is the miscompile that threading
  the call's own tail flag into `lower_clauses` fixes. Slice 4's migration of
  `examples/vm.sth` needs the missing half: `run` is a self-tail clause word whose every
  clause tail-calls `run`, and it would lose its loop on migration.
- **Reference-mode dispatch over an all-unit-variant enum crashes the backend, not the
  compiler (pre-existing, not this phase's).** `&Toggle` over `type: Toggle | On | Off ;`
  reaches QBE and fails there (`invalid type for first operand ... in add`). It reproduces
  identically through the pre-existing clause-style path (`| On | ... |`), so the
  eliminator only adds a second door to it, not the bug: `dispatch_on_tag`'s
  `scrutinee_is_value` correctly loads the tag through the pointer for this case (see the
  IR-level unit test `lower_eliminator_call_over_a_reference_to_a_scalar_enum_loads_the_tag`),
  but something downstream in codegen still mistreats the scalar enum's representation.
  Needs its own fix in whichever future slice next touches scalar-enum codegen or
  reference-mode dispatch, not this one.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "arm-annotation grammar: QuotAnnot.variant_tag with owning and &/&! reference-mode leading-variant-name forms",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "frontend eliminator PolySig generator, registry, and check_eliminator_call with variable-arity arm collection, owning/reference scrutinee mode, and written-order baseline",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "variant-projection IR lowering: resolved_variant_fields side table mirroring P7.S1's resolved_fields, EnumWord::Destructure, and a real ir_type_of(Type::Variant) case, unit-tested against hand-built IR state",
      "difficulty": "standard"
    },
    {
      "phase": 4,
      "focus": "eliminator lowering via a new ArmBinding::WholeValue mode on lower_clauses, EnumWord::Eliminate, plus owning- and reference-mode end-to-end goldens",
      "difficulty": "hard"
    }
  ]
}
```
