# Phase 2 — surface syntax and behaviour

Slice 10a, phase 2 of 7. Delivers **R1 rest**, **R2 rest**, **R3**.

## What landed

- `Token::TildeLBracket` (`src/lexer.rs`): `~[` glued with zero intervening
  whitespace lexes as one token, mirroring the existing `|>` glue in the
  word-scan loop. `~ [` (spaced) still lexes as `Word("~")` + `LBracket`, so
  it now hits a parse error (nothing declares a bare `~` word) instead of
  silently falling through to the ordinary-quotation path. Two lexer unit
  tests pin both forms directly.
- `effect_has_variable` (`src/parser.rs`) recognises `Token::TildeLBracket`
  and routes the whole effect to the poly parser, even when the `~` effect is
  otherwise fully concrete — the deliberate "poly-forcing is a choice"
  decision the spec calls out, load-bearing for R9 context 4's later
  unreachability proof.
- **Four parse entry points**, all recognising the token:
  - `parse_poly_slot` dispatches `Token::TildeLBracket` to a new
    `parse_poly_quotation_inner(builder, is_inline)`, split out of
    `parse_poly_quotation` (which now just consumes its own `LBracket` and
    calls the inner entry with `is_inline: false`) so the token that already
    ate the opening bracket has somewhere to resume.
  - The three **concrete** quotation-type gates — `parse_slot` (an
    `extern:`/non-poly word's own unnamed slot), `parse_type_expr` (ref/
    owning-cell referents, nested array elements, nested quotation-effect
    elements), `parse_field_type_expr` (struct fields) — each gained a
    `TildeLBracket` peek at their head that returns a new, shared
    `tilde_quotation_position_error`: a `~` is only legal as a word's own
    declared quotation parameter, never a field, output, referent, or
    extern parameter type. These are the "R2's three located errors" the
    codebase map named.
- `PolyType::Quotation` and `RawTy::Quotation` both grew a trailing `bool`
  (`is_inline`) — the `~`-ness of the effect, carried from the parser's
  `~[`-vs-`[` dispatch through to grounding. Every existing match site
  (`repl.rs`, `check.rs` x6, `ir.rs` test fixtures) updated; all but
  `apply_subst` and the concrete fold ignore the new field via `..` or `_`.
- **Grounding**: `raw_to_poly_type`'s Quotation arm (`src/parser.rs`) folds a
  fully-concrete `~` effect to `Concrete(Type::InlineQuotation(..))` instead
  of `Concrete(Type::Quotation(..))`; a variable-bearing one stays
  `PolyType::Quotation(ins, outs, is_inline)`. `apply_subst`'s Quotation arm
  (`src/check.rs`) does the same at the call-site-grounding step. Both are
  the only two `is_inline`-reading sites; every other match arm treats the
  bool as unread structure.
- `poly_type_str`'s Quotation arm now prints the `~` sigil (`~[ ... ]` vs
  `[ ... ]`), so R3's mismatch diagnostics name both spellings distinctly —
  fixed here because a compile error forced touching the match arm anyway,
  and the R3 goldens below need it (row-awareness proper is phase 3's job,
  named in the existing comment).
- **Nested-in-a-parameter rejection** (`audit_poly_input_quotation`,
  `src/check.rs`): a `~` (or an ordinary quotation) buried inside another
  quotation parameter's effect is now rejected, not silently accepted. Every
  `~` parameter folds to `PolyType::Concrete` (its effect has no row), so the
  variable-bearing `PolyType::Quotation` arm never fired for it; the `Concrete`
  arm returned `Ok` without recursing into the effect. It now recurses through
  `reject_quotation_type_position`, exactly as the monomorphic input walk in
  `audit_word_quotation_positions` already did — closing an R2
  ("`~` is only legal as a word's own direct declared parameter") hole that
  became reachable only once phase 2's `~[` parse path existed. Covers all four
  shapes: `~`-in-ordinary, ordinary-in-`~`, `~`-in-`~`, and either row.
- The fifth materialization boundary (capture admission,
  `check_capture_admission`) gained its own `~`-specific error,
  `captured_inline_quotation_error` ("a `~` quotation cannot be captured"),
  checked before the existing ordinary-quotation-capture deferral so the two
  are never conflated.

## The six behavioural tests

`tests/phase4_slice10a_inline_quotation.rs`:

- `glued_tilde_bracket_parses_as_a_combinator_parameter` / `spaced_tilde_bracket_is_a_parse_error`
  — R1's adjacency requirement, both forms.
- `call_on_inline_quotation_is_accepted` — the sixth test: `call` on a `~`
  still works (prints `12`), so the new guards did not overreach.
- Five materialization-boundary rejections, each showing the `~` is
  rejected **before** reaching its boundary because the *declaration
  position* is banned upstream:
  - `inline_quotation_as_word_output_is_error` (boundary 1, word output).
  - `inline_quotation_as_ref_referent_is_error` (boundary 2, `&!` store —
    unreachable because the referent position itself is banned).
  - `tilde_bearing_signature_always_routes_to_the_poly_parser` (boundary 3,
    declared parameter of a *mono* combinator — unreachable because `~` is
    always poly-forced; asserted directly on `WordDef.poly`, since "no other
    error" would not distinguish routing from coincidence).
  - `inline_quotation_as_word_output_is_error` again covers boundary 4 (the
    `if`-join), which the spec's own audit says is unreachable by the same
    upstream word-output/ref-referent bans, not a distinct mechanism.
  - `check::tests::check_capture_admission_rejects_captured_inline_quotation`
    (boundary 5, capture admission) — pinned as a **direct unit test**, not
    a source golden: `reject_poly_quotation_anywhere` (pre-dates this slice)
    already bans an ordinary quotation from *any* poly-signature position but
    a direct top-level parameter, so no `.sth` program can drive a `~` local
    into an escaping-closure boundary this slice. Exercised the same way
    phase 1 pinned its routing predicates.
- `inline_quotation_as_struct_field_is_error` / `inline_quotation_as_array_element_is_error`
  / `inline_quotation_as_extern_parameter_is_error` — the remaining
  materializing-declaration rejections R2 names by name.

## R3, both directions, at the call site

Struct-field/output tests show one direction (an ordinary quotation output
is legal, a `~` one is not — R3's structural inequality falling out of the
type system for free). The interesting direction is at the **abstract
forward** (`check_poly_combinator_args`'s let-else, already accessor-routed
by phase 1): a combinator forwards its own declared quotation parameter to a
nested combinator's declared parameter.

- `forwarding_inline_quotation_into_an_ordinary_declared_parameter_is_error`
  and its mirror `forwarding_ordinary_quotation_into_an_inline_declared_parameter_is_error`:
  forwarding a `~`-typed abstract parameter into an ordinary-declared one (and
  vice versa) is a located mismatch naming both spellings exactly —
  ``` `takes_ordinary` expects a quotation `[ i64 -- i64 ]` here, found `~[ i64 -- i64 ]` in `outer` ```
  — because `Type` derives structural equality and `InlineQuotation(eff) !=
  Quotation(eff)` even though `eff` is identical.
- `forwarding_inline_quotation_into_a_matching_inline_declared_parameter_runs`
  is the positive control: when both sides declare `~`, the forward is
  accepted and runs, so the negative goldens are catching a real mismatch,
  not merely rejecting every forward.
- `variable_bearing_inline_quotation_grounds_through_apply_subst` and
  `variable_bearing_inline_quotation_still_mismatches_ordinary` repeat the
  positive/negative pair with a `'T`-bearing (non-folded) effect: every other
  golden here is fully concrete and folds to `Concrete(InlineQuotation)` at
  *parse* time, never reaching `apply_subst`'s `PolyType::Quotation` arm. A
  `'T` keeps the effect `PolyType::Quotation` through checking, so grounding
  it against a live call site is what actually exercises `apply_subst`'s
  `is_inline` branch (confirmed by mutation below — without this pair, the
  mutation below is invisible).

No `~` value literal exists (`~[` is a type-position-only sigil; a `~[` in
term position hits the generic "unexpected token" parse error), so R3's
"goldens in both directions" is expressed as declared-parameter forwarding,
not literal-vs-declaration coercion.

## R20 — mutation evidence

Every new guard, reverted and confirmed to flip a test, then restored:

| Guard | Reverted to | Flips |
| --- | --- | --- |
| Lexer `~[` glue | drop the `TildeLBracket` push | `glued_tilde_bracket_parses_as_a_combinator_parameter` |
| `effect_has_variable`'s `TildeLBracket` arm | remove the match arm | `tilde_bearing_signature_always_routes_to_the_poly_parser` |
| `parse_slot`'s gate | remove the peek | `inline_quotation_as_extern_parameter_is_error` |
| `parse_type_expr`'s gate | remove the peek | `inline_quotation_as_array_element_is_error` (and the ref-referent golden) |
| `parse_field_type_expr`'s gate | remove the peek | `inline_quotation_as_struct_field_is_error` |
| Capture admission's `~` branch | remove the `InlineQuotation` check | `check_capture_admission_rejects_captured_inline_quotation` |
| `audit_poly_input_quotation`'s `Concrete`-arm recursion | collapse back to `Concrete(_) \| Var(_) => Ok(())` | `inline_quotation_nested_in_a_quotation_parameter_is_error` |
| `apply_subst`'s `is_inline` branch | always ground to `quotation_type` | `variable_bearing_inline_quotation_still_mismatches_ordinary` |
| `raw_to_poly_type`'s fold `is_inline` branch | always fold to `Concrete(Type::Quotation)` | `inline_quotation_type_differs_from_ordinary_at_the_output_boundary`, both forwarding-mismatch goldens |

Each mutation was applied individually (via a scratch copy of the file,
diffed back to a clean restore after), confirmed to fail the named test(s)
without touching any other guard, then reverted before moving to the next.

## Green

`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
all pass: 773 lib tests, 16 new phase-2 integration tests, full existing
suite (including `qbe_baseline`) unchanged. `lib/combinators.sth` is
byte-unchanged; `git diff --stat` touches only `src/{ast,check,ir,lexer,
parser,repl}.rs` and adds the one new test file.
