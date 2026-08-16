# Phase 6 Slice 3: the eliminator word (brief)

The generated per-enum eliminator (`Shape?` for `type: Shape | Circle r i64 | Rect w i64
h i64 ;`), taking one quotation argument per variant, each annotated with the variant it
handles (`( Circle )`), matched by declared variant not position, exhaustiveness and
duplication as named errors, arm-position effect elision escalating from a bare variant
name up to a full row-polymorphic effect. Lowers to the existing N-way tag dispatch
(`lower_clauses`).

## Recon (measured against the built compiler, 2026-08-17, `main` at `f58e9fa`)

`cargo test` is green at this HEAD (Slices 1-2 done and merged).

1. **The eliminator needs no new `TermKind`.** A body is a flat `Vec<Term>`
   (`src/ast.rs:1585-1614`); nothing there represents a multi-arm construct. But the
   roadmap's own worked example spells elimination as an ordinary call to a generated
   word (`Shape?`) taking N quotation-literal arguments, mirroring the existing `if`
   precedent: `if` (`lib/core.sth:42`, `: if inline ( ..a bool ~[ ..a -- ..b ] ~[ ..a --
   ..b ] -- ..b )`) is a hand-written library word over the builtin `branch`, which the
   backend special-cases by literal name in `ir/func_builder/calls.rs`'s dispatch match
   and lowers with a dedicated `lower_if` (`ir/func_builder/control_flow.rs:99`). No new
   term syntax is implied: an arm is just a quotation literal, and the eliminator call
   is just a call, exactly as `branch then-quot else-quot` is today.

2. **The arm annotation `( Circle )` is not legal syntax today, by explicit design.**
   `parse_quot_annotation` (`src/parser.rs:2362-2381`) requires the full four-part form;
   its own doc comment states "R6: only the full four-part form parses, so `( )` and a
   parenthesized list with no `--` are both located errors," and
   `parse_quotation_annotation_elided_is_error` (`parser.rs:3670`) pins exactly this. A
   bare `( Circle )` is therefore free to claim (nothing legal collides with it today),
   but it is new grammar, not an existing capability slice 1 already built. The roadmap's
   own escalation ladder — `( Circle )` → `( Vm Push -- Vm )` → `( ..a Vm Push -- ..b )`
   — means this can't be a separate bolt-on "tag" token either: it is a strict extension
   of the *existing* four-part grammar, where the variant name can stand in for the
   parts elided (below-scrutinee inputs, outputs, or both), so `QuotAnnot`
   (`src/ast.rs:1566-1576`) most likely needs a variant-name field alongside its existing
   `inputs`/`outputs`, not a parallel type.

3. **Cross-arm agreement already exists and is live; the eliminator's signature is
   expressible in today's vocabulary.** An earlier draft of this brief claimed nothing
   compares two sibling literals against each other. That was wrong, corrected by
   reading the code: `check_poly_combinator_args` (`src/check/combinators.rs:~735`)
   keeps a `shape_baseline: HashMap<u32, Vec<Type>>` keyed by the *shared declared
   output row id*. The first literal checked for a given row sets the baseline; every
   later sibling sharing that row is compared against it and rejected on a contradiction
   with `combinator_branch_output_mismatch_error`, naming both shapes. That is exactly
   the N-arm output agreement this slice needs, and it is what `if`'s two branches run
   through today.

   Nor do per-arm input differences need new machinery.
   `PolyType::Quotation(Vec<PolyType>, Vec<PolyType>, bool, Option<u32>, Option<u32>)`
   (`src/ast.rs:994`) already carries per-slot inputs alongside shared row ids, so the
   generated signature for `Shape?` is spellable as-is:

   ```text
   ( ..a Shape  ~[ ..a Shape.Circle -- ..b ]  ~[ ..a Shape.Rect -- ..b ]  -- ..b )
   ```

   Structurally `if`'s signature plus a scrutinee, with one concrete `Type::Variant`
   input per arm. `..a` carries the shared below-scrutinee prefix, `..b` the shared
   output, and Slice 2's `Type::Variant` is what makes each arm's input differ — which
   is what that type was built for.

   **Consequence: the `( Circle )` annotation is not load-bearing for type checking.**
   The parameter type already says which variant each slot takes. The annotation's jobs
   are (i) letting arms be written in any order, decoupling call sites from declaration
   order, and (ii) error attribution. This slice's depth is therefore in diagnostics and
   arm-to-variant routing, not in inventing a type-level unification.

4. **No production code builds a poly `Sig` (`PolySig`) programmatically today.** The
   only `PolySig { ... }` construction outside the parser
   (`src/check/declarations.rs:1929`, `parse_poly_effect` at `parser.rs:~1650`) is a test
   fixture, not a generator. `enum_generated_sigs`/`struct_generated_sigs`/
   `variant_generated_sigs` (`src/check/declarations.rs`) are all concrete-only: one
   fixed monomorphic `Sig` per enum/struct/variant. The eliminator's generated word is
   unavoidably poly (recon 3's signature carries two rows and a per-arm variant input),
   so this slice must write the first declaration-time `PolySig` generator, not mirror an
   existing one.

5. **Lowering has a low-risk, already-precedented shape.** `lower_clauses`
   (`ir/func_builder/control_flow.rs:194-314`) already does exactly the N-way
   tag-dispatch-and-join this slice needs, keyed off `enums.words[&clause.variant]` via
   `EnumWord::Construct` (not a hardcoded string), the same registry-lookup precedent
   `Type::Variant` accessors (Slice 2) already use instead of a global name scan. An
   `EnumWord::Eliminate(EnumId)` registry entry recognized in the call-lowering dispatch
   (alongside `"branch"`/`"tag"`'s literal-name special cases in
   `ir/func_builder/calls.rs`), popping N quotation operands via the existing
   `quot_bodies`/`quot_defs` machinery `branch` already uses and wrapping each into a
   synthetic `Clause { locals: vec![], body, .. }`, likely reuses `lower_clauses`
   unchanged. Arm bodies need no positional locals: Slice 2's `Circle>r`-style
   accessors already read fields by name, so an arm's `Clause.locals` is always empty —
   confirm this rather than assume it, since `lower_clauses` still threads `locals`
   through.

6. **Exhaustiveness/duplication diagnostics have a direct template.** `check_clause_word`
   (`src/check/word_entry.rs:288-408`) already does exactly this pre-pass — validate
   every clause's variant identity, reject a duplicate before checking any body,
   scan the declared variant list for a missing one — over `Clause`'s `.variant: String`
   field. The eliminator's arms are a different value shape (quotation literals with
   `( Variant )` annotations, not `Clause`s), so this is a second implementation of the
   same three checks, not a call to the existing one, but the wording, ordering
   (identity/duplication before any body check, D8's misspelt-variant-eats-terms
   concern), and error shape can be reused near-verbatim.

## Decisions (settled here, not reopened by the spec)

1. **The eliminator is an ordinary generated word, not new term syntax.** `Shape?`
   is registered the same way `enum_generated_sigs` registers a constructor — one
   generated `Sig`/`PolySig` per enum — and called like any other combinator taking
   quotation arguments. Recon 1 and 5 both support this: nothing else in the compiler
   would need to change to make a mid-body, quotation-nesting elimination construct
   exist, since ordinary calls already compose mid-body and nest inside quotations.

2. **`QuotAnnot` gains a variant-tag field rather than a new parallel annotation type.**
   Recon 2's escalation ladder is a strict extension of the existing four-part grammar
   (a variant name can stand in for the parts elision drops), so one type serving both
   an ordinary literal's annotation and an arm's annotation is the smaller change; a
   plain (non-arm) literal never sets the new field.

3. **No new IR term/instruction kind; extend the call-lowering dispatch with an
   `EnumWord::Eliminate(EnumId)` registry entry**, following the `EnumWord::Construct`
   precedent, feeding the existing `lower_clauses` with synthetic empty-`locals`
   `Clause`s built from the eliminator call's N quotation operands.

4. **The eliminator call is checked by a dedicated `check_eliminator_call`, not by
   permuting arms into `check_poly_combinator_args`** (settled with the user,
   2026-08-17, against the alternative of reusing the generic combinator path with a
   tag-driven pre-pass). The call is recognized by an `EnumWord::Eliminate(EnumId)`
   registry hit — the same registry-lookup precedent decision 3 uses for lowering — and
   each arm is looked up by its `( Variant )` tag directly rather than reordered into
   declaration order. Bought with this: full control of arm-attributed diagnostics
   ("arm `( Rect )` disagrees with arm `( Circle )`" rather than a generic "the
   quotation passed to `Shape?`"), exhaustiveness and duplication checks living beside
   the arm walk that needs them, and no permutation layer between what the programmer
   wrote and what an error message points at. It also sidesteps `apply_subst` entirely:
   the arms' `QuotEffect`s are built directly from the enum's own variants, so no
   `Type::Variant` ever has to survive substitution grounding.

   **The accepted cost is a second copy of rules that already exist**, and the spec must
   treat that as the slice's primary risk, not an afterthought. `check_eliminator_call`
   must *call*, never re-implement: `check_literal_against_declared_effect`
   (`src/check.rs:1703`, which already carries the `~`/`[` flavour check, the D3 capture
   restriction, tail-position handling, and the directional body check),
   `resolve_quotation_operand` (literal vs forwarded abstract quotation), and
   `match_slot` for every type comparison. Only two things are genuinely new here: the
   arm-to-variant routing, and the cross-arm output comparison — and the latter should
   follow `shape_baseline`'s existing first-wins-baseline shape (recon 3) rather than
   inventing a different agreement rule. A reviewer should diff the two paths' rule sets
   explicitly: a rule present in `check_poly_combinator_args` and absent here is a
   defect unless the spec says why it does not apply.

5. **Mixed-level sibling arms are legal, and cross-arm output agreement stays first-wins
   on written order** (settled with the user, 2026-08-17). Two parts:

   *Declared level.* Each arm's annotation, at whatever level of the ladder, is
   reconciled against **its own slot** in the generated signature (recon 3), which
   already fixes that arm's variant input. This is slice 1's existing
   literal-vs-parameter reconciliation applied per arm, with no arm-to-arm interaction:
   if every arm agrees with its own slot, no two arms can conflict with each other.
   Mixed levels (one arm bare `( Circle )`, a sibling fully spelled
   `( ..a Vm Push -- ..b )`) are therefore legal by construction, not by an added
   permissive rule.

   *Body exit shape.* The one genuine cross-arm comparison keeps `shape_baseline`'s
   existing behaviour unchanged (recon 3): the first arm sets the baseline, a later
   disagreeing arm is the located error, and the message names both shapes. Rejected
   alternative: letting an explicitly output-annotated arm claim the baseline regardless
   of position, so blame never depends on arm order. It buys only attribution — the
   existing error already prints both shapes, so even a misattributed message shows the
   reader everything needed — and that is not worth a second baseline-selection rule in
   a language this small. The convention that replaces it: **an arm that wants to state
   its output shape should be written first.**

   *The precision this depends on.* "First" must mean **first in written source order,
   not enum-declaration order.** Decision 4 looks arms up by their `( Variant )` tag, so
   iterating variants in `type:`-declaration order is the easy accidental
   implementation — and it would silently make the baseline "whichever variant the enum
   declares first" rather than the arm the programmer wrote first, breaking the
   convention above and reporting errors in an order unrelated to the source. Written
   order is also the better span ordering. This needs a test that fails under
   declaration-order iteration: an enum whose declaration order and arm order differ,
   with the written-first arm establishing a baseline the declaration-first arm
   contradicts.

## Open questions for the spec

- **OQ1 — settled by decision 4** (dedicated `check_eliminator_call`). What remains for
  the spec is mechanical, not architectural: enumerate the rule set
  `check_poly_combinator_args` enforces and state, per rule, whether
  `check_eliminator_call` calls the shared helper, deliberately omits it (with a
  reason), or needs an arm-flavoured variant of it. That enumeration is the artifact a
  reviewer checks the implementation against.

- **OQ2 — settled by decision 5** (mixed-level siblings legal; first written arm sets the
  output baseline).

- **OQ3 — does the generated `PolySig` (recon 4) need bounds, a length variable, or just
  a shared unbound output type variable?** The simplest shape (one output tyvar, N
  concrete-per-arm input lists, shared below-scrutinee prefix) may not need `PolySig`'s
  full generality (`bounds`, `len_var_names`) at all; the spec should confirm the
  minimal subset before writing a generator for the rest.

- **OQ4 — is a zero-variant enum (if one can even be declared) or a one-variant enum a
  degenerate case the eliminator must handle, and does a single-arm eliminator collapse
  to something simpler than `lower_clauses`'s join-block machinery?** Not seen in any
  existing `.sth` file; worth a one-line ruling rather than silence.

## Out of scope

- Migrating `examples/vm.sth`, `Bool`, or Phase 5's `Result`/`Option` off clause-style
  dispatch, or deleting `WordBody::Clauses`/`parse_clauses`: Slice 4.
- Any change to `Type::Variant`, the accessor mechanisms, or their generated `Sig`s:
  Slice 2 is done; this slice only consumes what it built.
- Any change to `EnumLayout`/`VariantLayout` or the underlying tag-dispatch codegen
  (`dispatch_on_tag`, `lower_clauses`'s block/phi shape): recon 5 finds no lowering gap
  beyond a new call-site entry point into machinery that already exists.

## Sequencing

Gated on Slice 2 (done, `main` at `f58e9fa`) for the field-accessor words an arm body
calls, and on Slice 1 (done) for the underlying annotation-parsing/reconciliation
infrastructure `QuotAnnot` extends. Touches `src/parser.rs` (new arm-annotation grammar),
`src/ast.rs` (`QuotAnnot`'s new field, and the generated-word/`EnumWord::Eliminate`
registration), `src/check/declarations.rs` (the new poly-`Sig` generator, alongside
`enum_generated_sigs`), `src/check.rs` (the new cross-arm reconciliation/unification
logic), and `src/ir/func_builder/calls.rs` + `control_flow.rs` (the new
`EnumWord::Eliminate` dispatch case feeding `lower_clauses`).

## Exit

A match is a term usable mid-body and inside a quotation (recon 1: true by construction,
since it is an ordinary call); reordering a `type:` declaration's variants leaves every
call site correct (recon 5: variant identity, not position, drives both the frontend
match-to-arm mapping and `lower_clauses`'s tag dispatch). A missing or duplicated arm is
a named error (recon 6's template). Dogfood is deferred to Slice 4's migration
(`examples/vm.sth`'s `Op` dispatch), per the top-level phase doc; this slice's own exit
witness is a `.sth` golden exercising a multi-variant enum end-to-end, since — unlike
Slice 2 — the eliminator is exactly the mechanism that makes a `Type::Variant` value
reachable from surface syntax for the first time.

## Ready to spec?

**Yes.** Both semantic questions are settled (decisions 4 and 5). OQ3/OQ4 are narrow and
can default (minimal `PolySig` subset; a one-variant enum handled by the general path,
not special-cased, until something demonstrates it needs to be).

One recon claim in this brief was wrong on first writing (recon 3, corrected in place
after reading `check_poly_combinator_args`). The spec should re-verify recon 4 and 5
directly rather than inheriting them: 4 is a negative claim ("no production code builds a
`PolySig`") and 5 asserts `lower_clauses` needs no change, both of which are the kind of
claim that is cheap to check and expensive to be wrong about.
