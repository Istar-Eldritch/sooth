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

  **Correction (verified by compiling it, not by reading).** This spec first claimed the
  `EnumWord::Construct` destructure at `control_flow.rs:236` "is fine" and that nothing
  outside `lower_clauses` moves. That is wrong. `EnumWord` (`src/ir/layout.rs:264-266`)
  has exactly **one** variant today, so decision 3's `EnumWord::Eliminate(EnumId)` is a
  breaking change to every site that assumed that. Adding the variant and running
  `cargo check` produces two hard errors, both of which Phase 3 must fix in the same
  phase that adds the variant:

  - `src/ir/func_builder/control_flow.rs:236` — **E0005, refutable pattern in local
    binding**: `let EnumWord::Construct(_, vi) = self.enums.words[&clause.variant];` is
    an irrefutable `let` that stops compiling the moment a second variant exists. It
    needs a `let ... else` (or `match`) whose non-`Construct` path is
    `unreachable!`, since a synthetic clause's `.variant` always keys a constructor
    entry.
  - `src/ir/func_builder/quotation.rs:453` — **E0004, non-exhaustive match** in
    `lower_enum_word`. This one is a *semantic* ruling, not a filler arm: R5's design
    intercepts `EnumWord::Eliminate` in the `calls.rs` dispatch **before**
    `lower_enum_word` is reached, so the correct arm is
    `EnumWord::Eliminate(_) => unreachable!(...)` with that rationale in the message —
    and the `unreachable!` is only sound *because* of the interception order, which
    makes "the `calls.rs` interception precedes `lower_enum_word`" a stated invariant of
    this slice rather than an incidental detail.

  Neither site changes `lower_clauses`'s logic, so the reuse claim holds; what fails is
  the stronger claim that no existing file moves. An implementer following the earlier
  wording literally would meet two compile errors that were not in any phase's scope.

## Decisions (settled in the brief; implemented as-is, not reopened)

1. The eliminator is an ordinary generated word, not new term syntax (an arm is a
   quotation literal, the call is an ordinary call).
2. `QuotAnnot` gains a variant-tag field; no parallel annotation type.
3. No new IR term/instruction kind; extend call-lowering dispatch with
   `EnumWord::Eliminate(EnumId)`, feeding `lower_clauses` synthetic empty-`locals`
   clauses.
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
  `PolyType::Quotation(inputs = [Type::Variant(id, vi)], outputs = [], is_inline = false,
  row_in = Some(a), row_out = Some(b))`, built through Slice 2's `variant_type`
  (`src/check/declarations.rs`) so the leaked `Enum.Variant` display name has one origin.
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

1. **Underflow / arity.** The call needs the scrutinee plus one arm per variant. If the
   stack is too shallow, the exhaustiveness pass (step 3) reports the missing arm by
   name; a genuine operand shortfall below the scrutinee is `underflow_error`. State the
   arity as `1 + variant_count`.
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
   baseline; a later disagreeing arm is the located error, naming both shapes via
   `combinator_branch_output_mismatch_error`, arm-attributed (`arm ( Rect ) disagrees
   with arm ( Circle )`). **"First" is written source order, not enum-declaration order.**
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
  a baseline the declaration-first arm contradicts. The assertion must pin the error to
  the declaration-first arm as the *offender* (baseline = written-first). This test
  **fails under declaration-order iteration** (which would flip which arm is the
  baseline, changing the offender named in the message). Assert on the specific offending
  variant named in the message, not merely that an error occurs — a test that passes
  under both orderings is worthless here.

### R5 — Lowering: `EnumWord::Eliminate` (`src/ir/layout.rs`, `src/ir/func_builder/calls.rs`, `src/ir/func_builder/quotation.rs`)

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
  calls `lower_clauses(&clauses, params, Type::Enum(id, _))`. The scrutinee is the
  params' last value.
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

### R6 — Golden (`examples/` + test harness)

A `.sth` golden exercising a multi-variant enum end-to-end (construct a `Type::Variant`
value via the eliminator, read a field via a Slice-2 accessor inside an arm, produce a
result). This is the slice's exit witness: unlike Slice 2, the eliminator is the
mechanism that first makes a `Type::Variant` value reachable from surface syntax. The
golden asserts observable program output (source in → expected output), not merely that
it compiles. Migration of `examples/vm.sth`/`Bool`/`Result`/`Option` is **Slice 4**.

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
| 7 | `resolve_quotation_operand`: Literal vs Forwarded vs None | `combinators.rs:692,741,746` | **Called (shared).** Literal is the arm's normal case; a forwarded abstract quotation arm is accepted the same way; `None` → `quotation_argument_required_error` | Identical operand resolution |
| 8 | `check_literal_against_declared_effect` (flavour `~`/`[`, D3 capture, tail handling, directional body check) | `combinators.rs:695` | **Called (shared), unchanged.** Per arm, against the effect built in rule 5's replacement | The point of decision 4's "call, never re-implement" |
| 9 | Cross-sibling output agreement via `shape_baseline` (first-wins per shared row id) → `combinator_branch_output_mismatch_error` | `combinators.rs:716-733` | **Arm-flavoured variant.** One shared `..b` row, so one baseline; first **written** arm sets it; message is arm-attributed | Decision 5: keeps first-wins, adds arm-name attribution; **must** iterate in written order (R4.5) |
| 10 | Forwarded abstract quotation acceptance (`found.ty == concrete`) | `combinators.rs:735-745` | **Called (shared).** | Same acceptance for a forwarded arm |
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

### Phase 3 — lowering + golden

Scope: R5, R6. Add `EnumWord::Eliminate(EnumId)`, register it in `enums.words`, intercept
it in the `calls.rs` dispatch (before `lower_enum_word`), build synthetic empty-`locals`
clauses from the N quotation operands, and feed `lower_clauses` (body unchanged). Add the
end-to-end `.sth` golden. This phase is where an eliminator call first compiles and runs.

**Required in this same phase** (see the recon-5 correction above; verified by compiling
the variant addition, not by reading): adding `EnumWord::Eliminate` breaks two existing
sites, and the phase is not green until both move.

- `src/ir/func_builder/control_flow.rs:236` — E0005: the irrefutable
  `let EnumWord::Construct(_, vi) = ...` becomes a `let ... else`/`match` with an
  `unreachable!` non-constructor path.
- `src/ir/func_builder/quotation.rs:453` — E0004: `lower_enum_word` gains
  `EnumWord::Eliminate(_) => unreachable!(...)`, sound only because the `calls.rs`
  interception precedes it. Order the interception before `lower_enum_word` deliberately,
  and say so in the arm's message.

Exit criteria (breakable assertions):

- An IR-lowering test (`lower_src`) asserting the eliminator dispatch reaches
  `lower_clauses` and emits the tag-dispatch/phi-join shape (one join predecessor per
  non-tail arm) — fails if the synthetic-clause construction is wrong (e.g. non-empty
  `locals` mis-binds, or the variant key mismatches `enums.words`).
- The `.sth` golden asserts observable output for a multi-variant enum eliminated
  end-to-end with a Slice-2 accessor read inside an arm — fails if elimination or
  field access regresses.

## Out of scope (restated)

- Migrating `examples/vm.sth`, `Bool`, `Result`/`Option` off clause-style dispatch, or
  deleting `WordBody::Clauses`/`parse_clauses` — Slice 4.
- Any change to `Type::Variant`, the accessor mechanisms, or their generated `Sig`s —
  Slice 2 is done; this slice only consumes them.
- Any change to `EnumLayout`/`VariantLayout`, `dispatch_on_tag`, or `lower_clauses`'
  block/phi shape — recon 5 finds no lowering gap beyond the new call-site entry point.
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
      "focus": "frontend eliminator PolySig generator, registry, and check_eliminator_call with written-order baseline",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "lowering EnumWord::Eliminate through synthetic clauses into lower_clauses plus the end-to-end golden",
      "difficulty": "standard"
    }
  ]
}
```
