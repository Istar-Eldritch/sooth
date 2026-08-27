# Phase 7 Slice 6: surface syntax unification

**Status:** Implemented
**Discovery:** `docs/roadmap/P7/slice6-brief.md`

## Problem Statement

Four unrelated bracket conventions collided in Sooth's type surface. A bare `[` meant an
array (`['T 'N]`) *or* a quotation effect (`[ i64 -- i64 ]`), disambiguated only by a
forward scan (`quotation_type_ahead`) for a depth-1 `--`. `type:`/`trait:` bound their
variables postfix (`type: Box 'T`), unlike every application site (`Box[i64]`). A word's
bounds were written inside its effect (`( 'T: Ord 'T -- Bool )`), where the bound-bearing
token is simultaneously an input slot. The result: `[` was the only bracket in the language
whose meaning depended on its contents, and a bound sat in a position that read like an
annotation but behaved like a slot.

The change is parser, tests, and one emitted-symbol rendering. No AST shape, checker,
lowering, IR or backend *logic* change: `PolySig`, `PolyType`, `RawTy`, `Type::Array`,
`ArrayDecl` and their consumers keep their shapes and receive the identical tree.

## Requirements

- **R1.** The array type is named: `array[ <elem> <count> ]` at every type-position reader
  (`array[i64 4]`, `array['T 'N]`, `array[array[u8 3] 2]`, `&array['T 4]`, `^array[i64 2]`).
  `array` may be followed by `[` with or without intervening space, matching
  `Slice[T]`/`Box[i64]`. The six bracket-dispatch sites (`parse_poly_slot`, `parse_slot`,
  `parse_type_expr`, `quotation_effect_opens_here`, `parse_field_type_expr`,
  `parse_generic_field_shape`) enter the existing array readers past the `array` token; the
  readers themselves are unchanged.
- **R1a.** `&`/`^` are not lexer delimiters, so `&array[i64 4]` lexes as `Word("&array")`
  and never reaches a `[`-dispatch site. Five arms across three functions intercept an
  `array` remainder **by name, ahead of the user type registry**, exactly as
  `resolve_type_or_apply` intercepts `Slice`: `parse_ref_type_expr` and
  `split_owning_cell_word` dispatch into `parse_array_type_expr`; `parse_poly_slot`'s `&`
  and `^` arms dispatch into `parse_poly_array` (not the concrete reader, which cannot hold
  a type-variable element), inside the same peek block as the empty and `'`-prefixed cases
  so the arm is not dead code; `parse_generic_field_shape`'s own `&` and `^` arms dispatch
  into `parse_generic_field_array`, placed *before* their `poly_generic_header` case, which
  would otherwise misreport `unknown type array`. The generic-field pair is a regression
  fix, not a new shape: `&['T 4]` built at HEAD via the bare-sigil recursion.
- **R1b.** A slot *named* `array` needs no special-case code. R1's dispatch predicate is
  `array` **followed by `[`**, and `parse_slot`'s name-then-optional-`:type` read never
  resolves a slot name as a type, so `array : i64` cannot trip R2. No `:`-lookahead
  exemption (mirroring `owning`'s) was added; the coverage test is kept as plain regression
  coverage and flagged not mutation-testable.
- **R2.** `array` in a type position with no following `[` is a located
  `array_without_bracket_error` naming `array[T N]`. **One raise site**:
  `resolve_type_or_apply`, the single funnel for resolving a bare word as a type, beside the
  `Slice`/`!Slice` arm. Every reader including R1a's arms funnels through it, so `&array --`
  reports the shape error rather than `unknown type array`.
- **R3.** `ARRAY_TYPE_NAME = "array"` sits beside `SLICE_TYPE_NAME` and is the one spelling
  every reader compares against. `reject_reserved_name` gains an `array` arm gated on `type`
  and `variant`. A word, local, field or slot named `array` stays legal.
- **R4.** A bare `[` in a type position is a quotation effect, unconditionally.
  `quotation_type_ahead` is deleted and its six call sites collapse to an `LBracket` peek;
  `quotation_effect_opens_here` survives as the named predicate the `owning` readers call.
- **R4a.** Since a quotation reader is now entered without knowing a `--` exists, the
  deleted depth walk is rewritten as a validator
  `require_top_depth_arrow(depth_base: i32) -> Result<(), String>` with **exactly two call
  sites at two different bases**. Detecting the missing `--` inside
  `parse_quot_type_list`/`parse_poly_quot_list` cannot work: those loops dispatch a bare
  count token to `parse_type_expr` and fail before the loop observes the `]`.
  - **(i) Depth base is entry-point-dependent.** `parse_quotation_effect_rows` *is*
    positioned on the `[`, so it calls the validator at base `0`, before its `expect`.
    `parse_poly_quotation_inner` is entered *past* its bracket by all three of its callers
    (`parse_poly_slot`'s `~[` arm, its `owning` arm, and `parse_poly_quotation`), so the
    validator is called once on that function's first line at base `1`, never in the three
    callers. At base `0` there, a legal `~[ 'T -- Bool ]` runs to EOF and false-rejects every
    inline combinator in `lib/combinators.sth`.
  - **(ii) The walk counts `Token::TildeLBracket`.** The old walk incremented only on
    `LBracket` while the matching `]` still decremented, so a nested `~[ … ]` made the
    validator **fail open**, passing vacuously on the inner quotation's `--`.
  - **(iii) The error drops its `array[T N]` clause for a `~[` opener.**
    `quotation_effect_missing_arrow_error` takes `opened_with_tilde`, computed by the
    validator itself as `depth_base > 0 && the previous token is TildeLBracket` (sound
    because every base-1 caller consumed exactly one opener immediately before entry). `~[`
    has no array reading anywhere, so that advice would send the author somewhere the parser
    refuses.
- **R5.** `type:` and `trait:` bind their variables in brackets (`type: Box['T]`,
  `type: Result['T 'E]`, `trait: Ord['T]`). The bracket is permanently **optional** for
  `type:`/`:` (a bare name is a concrete declaration) and **mandatory at slice exit** for
  `trait:`. An empty bracket is a located error; duplicates keep
  `duplicate_generic_ty_var_error`; a second `trait:` variable keeps
  `multi_variable_trait_error`; a non-`'`, non-`]` token inside a header bracket is a located
  error, never a silent break. No bounds on a header bracket (bounds are a word feature).
  The REPL generic-`type:` rejection fires on the bracketed form; `skip_typedef` and the
  pipe/variant scans stay correct with a bracket present (asserted, not assumed).
- **R5a.** `header_ty_var_count` is replaced by
  `header_is_generic = bracket_follows(..) || header_ty_var_count(..) > 0` at all three
  callers, and `parse_generic_header_vars` keeps its postfix loop as a second arm beside the
  bracket reader. Both survive phases 2–3 untouched: narrowing either earlier misclassifies
  the whole un-migrated corpus as concrete.
- **R5b.** `parse_trait_decl` does **not** call `header_ty_var_count` — it carries its own
  inline peek — so R5a's OR does not reach it and it needs the equivalent treatment of its
  own: a `LBracket` arm, the legacy `'`-prefixed arm unchanged, and the neither-form case
  (**two** arms: located and EOF) retargeted in message text only to name `trait: Name['T]`.
  Retargeting only the located arm would leave a truncated `trait: Ord` still advising the
  postfix form.
- **R6.** A word's bounds live in a bracket after the optional `inline` keyword and before
  `(`: `: mymax inline ['T: Copy Ord] ( 'T 'T -- 'T )`. The same bracket is admitted on a
  `trait:` member declaration. Two rules keep it a spelling change:
  - **The bound-bearing occurrence is a stack slot and stays one.** `parse_poly_ty_var`
    returns `RawTy::Var(id)`, so `( 'T: Ord 'T -- Bool )` has *two* inputs. Moving a bound
    into the bracket never removes a slot: the bracket adds the declaration, the effect keeps
    every mention with the `'T:` prefix stripped to a bare `'T`. A migration that deletes the
    bound-bearing slot changes arity and is a bug. (The brief's worked examples got this
    wrong; corrected here.)
  - **Ids stay effect-derived.** The bracket parser must not pre-intern into `PolyBuilder`:
    bracket-order ids would change `PolySig.ty_var_names` order and therefore
    `instantiation_symbol` output. It parses into a local side table and attaches bounds to
    the ids the effect interned. A bracket variable absent from the effect is a located
    error. An unbounded variable still binds at first mention and needs no bracket.
- **R6a.** Bracket grammar: `'[' var_decl+ ']'`, `var_decl := TYVAR [ ':' bound_list ]`
  (colon glued or spaced), `bound_list := bound+`, terminating at the next `'`-prefixed word
  or `]`. Termination is positional and total, with no next-slot fallback, so
  `parse_capabilities` gains a bracket mode in which its `None => break` arm errors instead;
  `parse_impl_bounds` (the `where`-clause caller) is unchanged. Two existing tests lose their
  subject to this and are the slice's one sanctioned retarget/retire:
  `parse_capabilities_stops_before_a_following_type_slot` becomes
  `parse_bound_bracket_ends_at_close_and_effect_follows` with byte-identical `sig.inputs`
  assertions, and `..._unbound_qualifier_after_a_bound_is_the_next_slot` is replaced by
  `parse_bound_bracket_unknown_name_after_a_bound_is_an_error` plus a companion keeping its
  real-world case alive on the effect side.
- **R7.** A bound inside an effect is a located error, raised from **inside**
  `parse_poly_ty_var` the moment `bound_follows` is true, selected on `builder.forbid_bounds`:
  `false` (word-def / trait-member) gives the new `bound_in_effect_error`, `true` (`impl:`
  target) gives `impl_target_bound_error`. Moving the diagnostic makes
  `parse_impl_target`'s post-hoc `!builder.bounds.is_empty()` check dead, and it is deleted —
  in that order, or `impl: Show for 'T: Copy` reports the word-def message.
  `bound_on_use_error` becomes unreachable and is deleted; its one message-pinning test is
  retargeted to `parse_trait_decl_member_bound_in_effect_is_error` (subject preserved: a
  member-signature bound must not misreport `unknown capability Copy`), and
  `parse_impl_bounds`' doc comment, which named it in prose, is reworded.
- **R8.** `impl: Show for array['T 'N]` falls out of R1 through `parse_poly_slot`;
  `forbid_bounds` and the row-variable rejection are unchanged. `intern_array_type` mints
  `array[i64 4]`, so every renderer reaching through `name_static` follows automatically,
  including the REPL's `<…>` array placeholder. Array-length diagnostic prose is updated.
- **R8a.** This is **not** display-only. `instantiation_symbol` builds from
  `sanitize(ty.name())`, so array-typed monomorph symbols change
  (`sooth_mono_w__t0__i64_4_` → `sooth_mono_w__t0_array_i64_4_`). Accepted; measured blast
  radius zero (no test pins an array-typed `sooth_mono_*`), with a re-run of the symbol grep
  a required migration step rather than an assumption.
- **R8b.** Three renderers build the array shape by hand with `format!("[{} {}]", …)`,
  bypassing `name_static` *and* invisible to a `[T N]`/`[elem count]` prose grep.
  `poly_type_str` (user-facing poly diagnostics) and `generic_field_type_str` (a generic
  field's surface spelling) move to `array[…]`. `poly_type_shape_str` is **exempt**: its own
  doc comment establishes it is a compiler-internal spelling never shown to the user, and it
  keys synthesized `member;Trait;Type` word names. `type_arg_key` is likewise exempt: it is a
  generic-instantiation registry key, not a surface spelling. The resulting cosmetic
  divergence (`Box[[i64 4]]` in an instantiation name) is a known wart, not fixed here. A
  second sweep, `grep -rn 'format!("\[{} {}\]"' src/`, reconciles every hit against this
  ruling; without it, exit criterion 7 is asserted rather than enforced.
- **R9.** The migration is exhaustive and semantics-free across `lib/`, `examples/`
  (including `experiments/`), `tests/*.rs` and the `#[cfg(test)]` fixtures under `src/`. No
  test deleted, weakened, or retargeted to a different subject; a migrated fixture that stops
  reproducing its subject is reported, not adjusted. `examples/*.tmpsth` (gitignored scratch)
  is excluded by glob.
- **R10.** At slice exit each legacy form is a **located** error naming its replacement: a
  bare `[` with no top-depth `--` gets R4a's error; a bound in an effect gets R7's; a postfix
  `type:`/`trait:` header variable gets `postfix_header_var_error`, raised where
  `header_is_generic` narrows to `bracket_follows` and, separately, where `parse_trait_decl`
  drops R5b's postfix disjunct (its neither-form arm keeps "expected a type variable").

## Success Criteria

- No `[` opens a **type** without a preceding name (`array[`, `Slice[`, `Box[`, `Result[`),
  including behind `&`, `&!` and `^`. R5/R6's brackets are binding sites, not types, and are
  preceded by a declaration or word name.
- `quotation_type_ahead` does not exist; a bracket with no top-depth `--` yields a located
  error from `require_top_depth_arrow` at base `0` from `parse_quotation_effect_rows` and
  base `1` from `parse_poly_quotation_inner`, counting `TildeLBracket`.
- `array` is reserved as a `type:`/`variant` name, unparseable without its bracket, and still
  legal as a slot/field/word name with no special-case code.
- `type:`/`:` bind variables in an optional bracket, `trait:` in a mandatory one; a postfix
  header variable is a located error naming the bracket form.
- A word's bounds parse only in its bracket; a bound in an effect is a located error; the
  bracket never changes a word's effect arity.
- `impl: Show for array['T 'N]` parses; `impl:` targets still forbid bounds and rows.
- `name_static` renders `array[i64 4]`; `poly_type_str` and `generic_field_type_str` follow;
  `poly_type_shape_str` and `type_arg_key` are the ruled-on exemptions; both sweeps reconcile
  and the `sooth_mono_*` grep is clean.
- The whole `.sth` and fixture corpus reads in the new syntax; full green; the P7 goldens
  (`gcd.sth`, `factorial.sth`, `lib/cmp.sth`, `lib/combinators.sth`) pass.

## Scope & Boundaries

**In scope:** the four surface changes and their diagnostics; R1a's five interception arms;
the corpus migration; the `name_static` rendering and R8b's two renderer changes.

**Out of scope:** `tree-sitter-sooth/grammar.js` (tokenises `bracket_group` generically, so
`array[` needs no grammar change); `docs/book/`, which is uncompiled and already teaches
rejected syntax (separately tracked); `examples/experiments/binary_search.sth`, left
byte-for-byte as-is — its own first line says `hypothetical grammar`, its `Slice['T: Ord 'N]`
passes two type arguments to a one-argument `Slice`, so it does not parse at HEAD and is
compiled by no test.

## Design Decisions & Rationale

**Dual acceptance is a within-slice scaffold, not a kept convenience.** The mechanism change
and the ~1 543-occurrence migration cannot land in one commit without a wall of unrelated
churn, so the parser accepted both spellings through phases 1–2, the corpus migrated in phase
3 against a parser that already took the new form, and phase 4 retired the old forms. During
phases 1–3, behaviour for un-migrated sources was bit-identical.

**R5a's OR and R5b's `trait:` twin are separate mechanisms, both load-bearing.** Fixing only
the three `type:`-side callers would have left 217 postfix `trait:` occurrences — including
`lib/cmp.sth`, imported by the P7 goldens — failing for the whole of phase 2. Dual acceptance
had to reach the type-variable *reader* too, not just the classifier: replacing
`parse_generic_header_vars` outright satisfies `header_is_generic` while still breaking
`lib/result.sth:1` and `lib/option.sth:1` three phases early.

**R4a is the only behaviour-changing edit, and its entry point is the trap.** A validator
seeded at depth 0 inside `parse_poly_quotation_inner` false-rejects every legal inline
combinator; a walk ignoring `TildeLBracket` fails *open*. Both are pinned by named tests
(`parse_poly_quotation_legal_inline_effect_still_parses`,
`require_top_depth_arrow_counts_a_nested_tilde_bracket`), and the two missing-arrow tests are
pinned to *different* openers so R4a(iii)'s conditional clause is independently covered.

**Migration inventory, measured rather than estimated.** ~1 543 occurrences in 92 files
across four grep patterns (arrays 620, bounds 522, postfix `type:` 184, postfix `trait:` 217).
Only **five** postfix headers exist in `.sth` at all (`lib/result.sth:1`, `lib/option.sth:1`,
`lib/cmp.sth:38`, `examples/traits.sth:25` and `:29`); the bulk is Rust-side fixtures, and
**837 of the 935 `src/` occurrences sit inside `#[cfg(test)]` blocks** — so a `tests/`-only
sweep looks complete and is not.

**Mutation testing was a gate, not a nicety.** This project has shipped placebo tests
repeatedly, and a test asserting "a bare `[ i64 4 ]` is rejected" can pass off an older
upstream blocker, so every new gate (R2, R3, R4a's two halves, R6's unused-variable error,
R6a's bracket-mode unknown name, R7, R10, R3's reserved-name arm) was reverted one at a time
against a pinned message.

## Phase 4 Verification (recorded post-merge)

- **`src/parser.rs` split signals, re-run (success criterion: bounds parse only in their
  bracket / whole corpus migrated).** The file grew from 10,516 to 11,786 lines (+1,270) and
  is now 6,286 non-test lines with 3 `use` lines and no would-be circular dependency, so
  import divergence and the circular-dependency signal do not fire. The other three fire: the
  file mixes driver-facing pre-passes, token-level peeks and the REPL line readers (high- and
  low-level code in one place); it does several unrelated things (bracket-header parsing,
  bound-bracket parsing, effect/slot parsing, REPL line entry points); and some of its
  functions never call each other (e.g. the REPL line readers vs. the module pre-passes). **3
  of 5, the same posture as the deferred `poly.rs` split**: deferred, not split, with no
  candidate cut identified yet.
- **Undisclosed test deletion, corrected.** `parse_x3_bound_on_use_occurrence_is_error`
  (present at `9c13878`, asserting `: f ( 'T: Copy 'T: Copy -- 'T ) drop ;` raised a located
  "must be written at its binding" error) is gone as of `64b4d4e`. The deletion is
  defensible, not a silent weakening: that fixture now raises `bound_in_effect_error` (a bound
  cannot appear in an effect at all any more, use-occurrence or not), and its subject survives
  as `parse_worddef_bound_in_effect_is_error` (`src/parser.rs:10727`), whose own doc comment
  records the retirement. The phase's own licence for the deletion ("no dedicated test exists
  for it") was wrong; this entry is the correction the commit message should have carried.

## Open Questions

None outstanding.

## Implementation

| Area | Commit | Key files |
|------|--------|-----------|
| R1/R1a/R1b/R2/R3: the named array type, six dispatch sites plus five interception arms | `95ed2475` | `src/parser.rs` |
| R5/R5a/R5b/R6/R6a: header and bound brackets, dual acceptance | `308fb4ab` | `src/parser.rs` |
| R9/R8: corpus migration and array rendering | `9fac97b1`, `97785507` | `examples/*.sth`, `src/ast.rs`, `src/parser.rs`, `src/check/poly.rs`, `src/check/declarations.rs`, `src/check/audits.rs`, `src/driver.rs`, `src/repl.rs`, `tests/phase5_slice1.rs` |
| R4/R4a/R7/R10: retire the legacy spellings | `64b4d4e3` | `src/parser.rs`, `src/ast.rs`, `src/check.rs`, `src/check/word_families.rs`, `tests/phase4_combinators.rs`, `tests/phase4_quotations.rs`, `tests/phase4_slice10a_inline_quotation.rs`, `tests/phase4_slice11_inline.rs` |
| Pin R6a's bracket-mode gate to its discriminating message | `57b53c68` | `src/parser.rs` |
| Docs: P7.S6 done; old spellings retired from README/DESIGN | `fa8533d1` | `DESIGN.md`, `README.md`, `docs/roadmap/P7-language-prereqs.md`, `src/ast.rs`, `src/parser.rs` |
| Review-feedback fixes (cycle 2) | `727d8270`, `0a89ae5f` | `src/parser.rs`, `src/check/declarations.rs`, `src/check/poly.rs`, `tests/phase7_slice3b_follow.rs`, `docs/roadmap/P7/slice6-spec.md` |
