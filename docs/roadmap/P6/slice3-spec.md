# Phase 6 Slice 3: the eliminator word (spec)

Anchors verified against `main` at `3993f18` (HEAD at spec time). Every `path:line`
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
  (`src/check/declarations.rs:1273`) and `variant_generated_sigs`
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
  `EnumWord::Construct` is registered under at `src/ir/layout.rs:517-519`). With those,
  `lower_clauses`'s **body** is reused unchanged.

  **Correction (verified by compiling it, not by reading; re-corrected again in round 2
  after two independent reviewers reproduced this by building it locally).** This spec
  first claimed the `EnumWord::Construct` destructure at `control_flow.rs:236` "is fine"
  and that nothing outside `lower_clauses` moves. That is wrong. `EnumWord`
  (`src/ir/layout.rs:264-266`) has exactly **one** variant today, so *any* second
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
  - `src/ir/func_builder/quotation.rs:453` — **E0004, non-exhaustive match** in
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

- `( Circle )` — variant only; below-scrutinee inputs, the variant input, and outputs
  all elided.
- `( Circle Push -- Vm )` and the fully spelled `( ..a Vm Push -- ..b )` — the variant
  name (when present as the leading token) sets `variant_tag`, the rest parses as today.

R6's existing rule (`( )` and an arrow-less parenthesized list are located errors) stays;
`parse_quotation_annotation_elided_is_error` (`src/parser.rs:3670`) must still pass. The
new grammar only *adds* the leading-variant-name form; it does not relax the arrow rule
for a non-arm annotation. Unit tests beside the parser: happy path (bare `( Circle )`
parses with `variant_tag = Some("Circle")`), and an error/edge case (a bare `( )` is
still the located elided-form error).

### R2 — Eliminator `PolySig` generator (`src/check/declarations.rs`)

Add `enum_eliminator_sigs(enums: &[EnumDecl]) -> Vec<(String, String, PolySig)>`
alongside `enum_generated_sigs` (`src/check/declarations.rs:1273`). For each enum, emit
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
  materialized quotation body (the known row-combinator-quotation ICE). `is_inline = true`
  is also what keeps R5's `ir_type_of(Type::Variant)` non-reachability argument airtight
  (a materialized arm effect would force a `Type→IrType` conversion at a boundary
  `is_inline = false` does not have).
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
   `resolve_quotation_operand`'s own definition (`combinators.rs:~825`, the
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
   - If the scrutinee slot itself doesn't resolve to `Type::Enum(id, _)` for a registered
     eliminator, `underflow_error` (too few operands below the arms) or the ordinary
     type-mismatch diagnostic (wrong-typed scrutinee) applies, exactly as today.
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
2. **Scrutinee type.** The below-top input must be `Type::Enum(id, _)` (value mode) for
   the registry's `id`. Reject a non-enum scrutinee with the existing type-mismatch
   diagnostic. (Reference-mode `&Enum`/`&!Enum` scrutinees are **out of scope** this
   slice: the surface eliminator is value-mode only; clause-style reference dispatch is
   unchanged and still handled by `check_clause_word`.)
3. **Exhaustiveness + duplication pre-pass**, adapted near-verbatim from
   `check_clause_word` (`src/check/word_entry.rs:288-408`): walk arms in **written
   source order**, look each arm up by its `( Variant )` `variant_tag` against the enum's
   variant list, reject an unknown variant (naming variant and enum), reject a duplicate
   arm (naming the variant), then reject any variant with no arm as non-exhaustive. This
   runs **before** any arm body is checked. An arm with no `variant_tag` (the annotation
   omitted the variant name entirely) is its own located error.
4. **Per-arm body check.** For each arm, look up its variant by `variant_tag`, build that
   arm's expected effect directly from the enum's own variant (`variant_type` →
   `Type::Variant(id, vi)` as the arm's below-row-top input, shared `..a`/`..b` rows),
   and pass it to `check_literal_against_declared_effect` (`src/check.rs:1703`) unchanged.
   That helper already carries the `~`/`[` flavour check, the D3 capture restriction,
   tail-position handling, and the directional body check. This sidesteps `apply_subst`
   entirely (decision 4): the arm effects are built from the enum, so no `Type::Variant`
   survives substitution grounding.
5. **Cross-arm output agreement**, following `shape_baseline`'s existing first-wins shape
   (`src/check/combinators.rs:660+`): the first arm **in written order** sets the `..b`
   baseline; a later disagreeing arm is the located error, reported by calling
   `combinator_branch_output_mismatch_error` (`src/check.rs:2019`) **unchanged** — that
   helper's signature is `(ctx, span, word, expected: &[Type], found: &[Type])` and its
   message names the two shapes and a source line, **not an arm or variant**. `expected`
   is the written-first arm's shape, `found` is the offending arm's; that pairing is what
   discriminates written order from declaration order (flipping iteration order swaps
   which shape is `expected`), and it is what decision 4's "call, never re-implement"
   actually buys here — arm-name attribution in the message text is **not** part of this
   slice; adding it would mean forking or extending a helper the live `if`/combinator
   path also depends on, which is its own item with its own test, not a free extension of
   decision 4. **"First" is written source order, not enum-declaration order.**
6. **Outputs.** On success the call produces the shared `..b` outputs (the baseline).
   No `Subst` is threaded out: the eliminator is not a self-tail combinator, so there is
   no back-edge grounding.

Unit tests beside `check_eliminator_call` (naming `thing_condition_expected`):

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
    expects as its receiver — the same pointer `EnumWord::Get`/`Destructure` read
    `payload_offset + field.offset` from.
  - **Open verification, required before this requirement is considered done, not
    assumed:** confirm no code path calls `ir_type_of(Type::Variant(..))` while lowering
    an eliminator call or an arm body under `WholeValue` — the frontend types the arm's
    receiver as `Type::Variant`, but nothing in the `WholeValue` path above actually
    converts that type to an `IrType` (the pushed value is `scrutinee`, already an
    `IrType::Enum`/`IrType::Ptr` from before the call), so this should hold, but it must be
    checked against the actual lowering call graph, not asserted from this description.
    Add a lowering test that exercises an eliminator call end-to-end and would panic on
    `ir_type_of`'s existing `unreachable!("a Type::Variant never reaches the backend")`
    (`src/ir/types.rs:259`) if this assumption is wrong.
- Extend `EnumWord` (`src/ir/layout.rs:264`) with `Eliminate(EnumId)`. Register the
  eliminator's surface/mangled name in `enums.words` alongside the `Construct` entries
  (`src/ir/layout.rs:517-519`), mapping `"{EnumName}?"` → `Eliminate(id)`.
- In the call-lowering dispatch (`src/ir/func_builder/calls.rs`, both the sym-name path
  around `:250` and the name path around `:645`, mirroring the existing
  `enums.words.get(name)` → `lower_enum_word` interception), an `EnumWord::Eliminate`
  hit pops the N quotation operands (via the existing `quot_bodies`/`quot_defs`
  machinery that `branch` already uses at `calls.rs:520`), maps each to its variant by
  the arm's `variant_tag`, wraps each arm body into a synthetic
  `Clause { variant: <registry key for that variant>, locals: vec![], body, .. }`, and
  calls `lower_clauses(&clauses, params, Type::Enum(id, _), ArmBinding::WholeValue)`. The
  scrutinee is the params' last value.
- `EnumWord::Eliminate` in `lower_enum_word` (`src/ir/func_builder/quotation.rs:452`) is
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

### R6 — Variant-accessor IR lowering (`src/ir/layout.rs`, `src/ir/func_builder/quotation.rs`, `src/ir/func_builder/word_families.rs`)

**New in this correction, not in the prior draft.** Slice 2 shipped the checker side of
three variant-accessor mechanisms (scalar/whole-destructure via `Sig`, aggregate getter,
reference-mode getter) but no IR lowering for any of them — `EnumWord`
(`src/ir/layout.rs:264-266`) has exactly one variant, `Construct`, and its own doc comment
still says "Enums have no getter/setter/destructure (elimination is clause-style, Phase
4)", i.e. this gap was explicitly deferred to this slice and the comment was never
updated. Before this requirement, `Type::Variant` cannot survive to the backend at all
(`src/ir/types.rs:259`'s `unreachable!`) because nothing constructs a reachable value of
that type from surface syntax (R2–R5 is what first does). This requirement is the direct
structural mirror of what already exists for `StructWord`— not a new mechanism:

- Extend `EnumWord` with `Get(EnumId, usize /* variant */, usize /* field */)` and
  `Destructure(EnumId, usize /* variant */)`, mirroring `StructWord::Get`/`Destructure`.
- In the enum-word registry build (`src/ir/layout.rs:564-574`, the `ewords` loop that
  today only inserts `EnumWord::Construct`), add the variant-word twin: for every variant
  with fields, insert `"{Variant}>"` → `Destructure(id, vi)` and `"{Variant}>{field}"` →
  `Get(id, vi, fi)` for each field. **This loop does not currently use the dual-key
  `insert` closure the struct registry uses** (`layout.rs:526-530`); it inlines a
  surface-then-mangled pair inline (`layout.rs:568-571`). Adopt the struct registry's
  closure form here rather than inlining a second copy of the same two-line pattern —
  same D7 keying discipline (mangled key always inserted, surface key only when it
  differs), same rationale (an unambiguous single-instantiation call site resolves by
  surface key; an ambiguous one resolves by the checker's mangled `builtin_overloads`
  key). This covers **both** of Slice 2's consuming mechanisms (the `Sig`-dispatched
  scalar getter and whole-destructure, and the checker-direct aggregate getter,
  `check_variant_get_word`): all three are checked differently but resolve to the
  identical call term at this point, and IR lowering does not distinguish aggregate from
  scalar fields any more than `StructWord::Get` already doesn't.
- Extend `lower_enum_word` (`src/ir/func_builder/quotation.rs`) with `Get`/`Destructure`
  arms mirroring `lower_struct_word`'s (read `payload_offset + field.offset` instead of
  the struct's bare `field.offset`, reusing `load_field_onto_stack`).
- Extend `lower_reference_word` (`src/ir/func_builder/word_families.rs:26+`) with an
  `EnumWord::Get` arm mirroring the existing `StructWord::Get` arm there (`:54`), reading
  the same `payload_offset + field.offset` address and pushing a reference, inserted
  before the locals fallback.

Unit tests beside each lowering site, against hand-built IR state (this mechanism has no
surface-syntax caller until R5 exists in the same slice, so these are exercised directly,
the same way Slice 2 unit-tested its checker side against hand-built checker state before
an `.sth` golden could reach it): a value-mode `Get` on an aggregate field returns the
field's loaded value; a `Destructure` on a zero-field variant pushes nothing (fails if it
panics on an empty field list); a reference-mode `Get` returns a reference at the correct
`payload_offset`-relative address (fails if it forgets the payload offset and computes a
bare `field.offset`, which would collide with the struct case's addressing on the same
byte range).

### R7 — Golden (`examples/` + test harness)

A `.sth` golden exercising a multi-variant enum end-to-end (construct a `Type::Variant`
value via the eliminator, read a field via a Slice-2 accessor inside an arm, produce a
result). This is the slice's exit witness: unlike Slice 2, the eliminator is the
mechanism that first makes a `Type::Variant` value reachable from surface syntax, and R6
is what first gives that value's accessors somewhere to lower to. The golden asserts
observable program output (source in → expected output), not merely that it compiles.
Migration of `examples/vm.sth`/`Bool`/`Result`/`Option` is **Slice 4**.

## OQ1 artifact: `check_poly_combinator_args` rule set vs `check_eliminator_call`

The mitigation for decision 4's accepted cost (a second copy of rules that already
exist). Each rule enforced by `check_poly_combinator_args`
(`src/check/combinators.rs:580-756`), and what `check_eliminator_call` does with it. A
reviewer diffs the two paths against this table: a rule present there and silently absent
here is a defect unless a **Reason** column explains it.

| # | Rule in `check_poly_combinator_args` | Anchor | `check_eliminator_call` | Reason |
|---|---|---|---|---|
| 1 | Underflow: `stack.len() < n` → `underflow_error` | `combinators.rs:618` | **Arm-flavoured variant.** Arity is `1 + variant_count`; a missing arm is reported by the exhaustiveness pass (R4.3) by name, a genuine sub-scrutinee shortfall by `underflow_error` | Exhaustiveness gives a better-attributed message than a bare count mismatch |
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

### Phase 3 — variant-accessor IR lowering

Scope: R6. Add `EnumWord::Get`/`Destructure`, register them in `enums.words` alongside
`Construct` (mirroring `StructWord`'s registration exactly), and extend
`lower_enum_word`/`lower_reference_word` with the corresponding arms. Independently
green and independently testable: these variants are constructed (registration) and
matched (the new arms) within this phase regardless of whether anything calls them yet
via surface syntax — not `dead_code`, the same chicken-and-egg Slice 2 already resolved
on the checker side by unit-testing against hand-built state. This phase adds no surface
way to reach a variant accessor; that arrives in Phase 4.

**Required in this same phase, not Phase 4 (round-2 review correction, reproduced by two
independent reviewers building it locally):** `EnumWord` has exactly one variant
(`Construct`) before this phase. Adding `Get`/`Destructure` here is what first turns it
into a multi-variant enum, so it is **this** phase, not Phase 4's `Eliminate`, that
trips `src/ir/func_builder/control_flow.rs:236`'s irrefutable
`let EnumWord::Construct(_, vi) = self.enums.words[&clause.variant];` (E0005: refutable
pattern). An earlier draft attributed this breakage to Phase 4's `Eliminate` variant;
that is wrong — *any* second `EnumWord` variant trips it, and `Get`/`Destructure` are
the second and third. Fix it here: replace the irrefutable `let` with a `let ... else`
(or `match`) whose non-`Construct` path is `unreachable!("a clause always dispatches to
a variant constructor")`. `src/ir/func_builder/quotation.rs:453`'s `lower_enum_word`
match stays exhaustive through this phase (`Get`/`Destructure` get real arms here), so it
does **not** break in Phase 3 — only in Phase 4, when `Eliminate` is added without a
corresponding `lower_enum_word` arm (by design, R5).

Exit criteria (breakable assertions):

- `cargo build`/`cargo clippy -- -D warnings` pass with all three `EnumWord` variants
  present (`Construct`, `Get`, `Destructure`) — fails if `control_flow.rs:236`'s fix is
  deferred, per the correction above.
- A value-mode `Get` lowering test on an aggregate field, asserting **both** the loaded
  value's address (`payload_offset + field.offset` for the correct variant, not an
  adjacent variant sharing the same field index) **and** its loaded `IrType`. Use a test
  enum whose variants have *different* field layouts at the tested index (not two
  same-shaped variants) — otherwise a wrong-variant mutation (reading `variants[vi']`
  for the wrong `vi'`) is invisible, and an address-only assertion never catches a
  wrong load width/signedness.
- A `Destructure` lowering test on a **multi-field** variant asserting it pushes exactly
  N values in field order, **plus** a companion test on a zero-field variant asserting
  it pushes nothing and does not panic. The zero-field case alone does not catch a
  mutation that always pushes nothing regardless of field count.
- A reference-mode `Get` lowering test asserting the returned reference's address
  equals `payload_offset + field.offset` **as an absolute value**, not merely "matches
  the value-mode case's result" — a relative comparison would pass if both modes shared
  the same regression (e.g. both dropping `payload_offset`).

### Phase 4 — eliminator lowering + golden

Scope: R5, R7. Add the `ArmBinding` parameter to `lower_clauses` (`Decompose` default,
unchanged for every existing caller; `WholeValue` new), `EnumWord::Eliminate(EnumId)`,
register it in `enums.words`, intercept it in the `calls.rs` dispatch (before
`lower_enum_word`), build synthetic empty-`locals` clauses from the N quotation operands,
and feed `lower_clauses` under `ArmBinding::WholeValue`. Add the end-to-end `.sth` golden.
This phase is where an eliminator call first compiles and runs, and where Phase 3's
accessor lowering is first exercised by a real program.

**Required in this same phase** (corrected in round 2: `control_flow.rs:236` was already
fixed in Phase 3, since `Get`/`Destructure` tripped it first — only the `lower_enum_word`
match remains to update here):

- `src/ir/func_builder/quotation.rs:453` — E0004: `lower_enum_word` gains
  `EnumWord::Eliminate(_) => unreachable!(...)`, sound only because the `calls.rs`
  interception precedes it. Order the interception before `lower_enum_word` deliberately,
  and say so in the arm's message.

Exit criteria (breakable assertions):

- An IR-lowering test (`lower_src`) asserting the eliminator dispatch reaches
  `lower_clauses` and emits the tag-dispatch/phi-join shape (one join predecessor per
  non-tail arm) — fails if the synthetic-clause construction is wrong (e.g. non-empty
  `locals` mis-binds, or the variant key mismatches `enums.words`).
- A `Decompose`-mode existing clause-style test (e.g. an existing `Bool`/`Result` clause
  lowering test) is unchanged and still passes — proves `ArmBinding` is additive, not a
  behaviour change to real clause words.
- The `.sth` golden asserts observable output for a multi-variant enum eliminated
  end-to-end with a Slice-2 accessor read inside an arm — fails if elimination or
  field access regresses. This is the first test that exercises Phase 3's accessor
  lowering through a real surface program rather than hand-built IR state.

## Out of scope (restated)

- Migrating `examples/vm.sth`, `Bool`, `Result`/`Option` off clause-style dispatch, or
  deleting `WordBody::Clauses`/`parse_clauses` — Slice 4.
- Any change to `Type::Variant`, the **checker-side** accessor mechanisms, or their
  generated `Sig`s — Slice 2's checker is done; this slice only consumes it. (R6 adds
  their *missing* IR lowering, which is new code, not a change to existing code — Slice 2
  never shipped an IR side to modify.)
- Any change to `EnumLayout`/`VariantLayout` or `dispatch_on_tag`. `lower_clauses`
  gains an additive `ArmBinding` parameter (R5); every existing caller keeps its current
  behaviour bit-for-bit under `Decompose` — this is not the block/phi dispatch shape
  changing, only which values populate the stack before a clause body runs.
- Reference-mode (`&Enum`/`&!Enum`) surface eliminators — value mode only this slice;
  clause-style reference dispatch stays in `check_clause_word`.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "arm-annotation grammar: QuotAnnot.variant_tag and the leading-variant-name annotation form",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "frontend eliminator PolySig generator, registry, and check_eliminator_call with variable-arity arm collection and written-order baseline",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "variant-accessor IR lowering: EnumWord::Get/Destructure mirroring StructWord, unit-tested against hand-built IR state",
      "difficulty": "standard"
    },
    {
      "phase": 4,
      "focus": "eliminator lowering via a new ArmBinding::WholeValue mode on lower_clauses, EnumWord::Eliminate, plus the end-to-end golden",
      "difficulty": "hard"
    }
  ]
}
```
