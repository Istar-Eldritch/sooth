# Spec: Phase 6 Slice 4 — migration off the clause path

**Status:** Ready to implement
**Created:** 2026-08-18

Pairs with [slice4-brief.md](./slice4-brief.md) (discovery). Gated on Slice 3b (done,
merged `9c3b6ca`), which closed the last capability gap (generic-enum elimination) that
made `WordBody::Clauses` load-bearing. This is the payoff slice: every clause-dispatch
site moves to the eliminator, and `WordBody::Clauses` / `parse_clauses` and everything
that exists only to serve them are deleted. Two elimination mechanisms in the language
permanently was never the destination (see [P6-enum-elimination.md](../P6-enum-elimination.md)).

```text
\ before (clause-style word body)
: area ( Shape -- f64 )
| Circle   dup * 3.14159 *
| Rect     | w h |  w h *
;

\ after (eliminator term)
: area ( Shape -- f64 )
  ~[ ( Circle ) Circle> dup * 3.14159 * ]
  ~[ ( Rect )   Rect> | w h | w h * ]
  Shape? ;
```

## Corrections to the brief's recon (verified against `main` at `9c3b6ca`, 2026-08-18)

The brief's recon 4 was re-verified line by line against `src/`. Two claims were wrong or
incomplete and are corrected here rather than silently worked around (project convention:
a falsified claim is visibly corrected).

- **C1 — `check_clause_word` lives in `src/check/word_entry.rs:311`, not
  `src/check/declarations.rs:311`.** The brief misattributed the function to
  `declarations.rs`. Its call site is `word_entry.rs:59` (correct in the brief) and the
  function definition is `word_entry.rs:311`. Its **five unit tests** are in
  `declarations.rs:2973-3025` (correct in the brief). So the retirement unit spans two
  files, not one.

- **C2 — the parser inventory is larger than `parse_clauses` + its call site.** The brief
  listed only `parser.rs:1475` (call site) and `:1784` (`parse_clauses`). Grepped
  directly, the clause path in `src/parser.rs` also owns:
  - `at_clause_start` (`:1775`, the clause-vs-locals discriminator, called at the body
    dispatch `:1474` and inside the clause parse at `:3652`),
  - `parse_clause_body_terms` (`:3646`, called only from `parse_clauses` at `:1802`),
  - the test-support accessor `clauses_body` (`:5481`) with its `WordBody::Clauses` arm,
  - the `terms_body` helper's dead arm (`:3916`: `WordBody::Clauses(_) => panic!(...)`,
    which becomes irrefutable once the variant is gone), and
  - four parser unit tests whose subject is clause parsing:
    `parse_clause_word_multi_field_with_body_locals` (`:5489`),
    `parse_clause_body_mid_body_pipe_produces_bind_term` (`:5508`),
    `parse_clause_word_empty_clause_before_next_clause` (`:5528`),
    `reserved_caret_clause_body_local_is_error` (`:5883`, reference-mode clause
    inference). `parse_term_word_with_leading_locals_is_not_a_clause` (`:5546`) keeps its
    *positive* half (leading locals are a term body) but loses its clause-contrast framing;
    it is retitled, not deleted (R1).

- **C3 — the poly path has two clause-body diagnostics, not one match arm.**
  `poly.rs:196` and `poly.rs:338` each produce a user-facing error
  (`"combines a clause-style body with a polymorphic signature, which is not supported"`).
  Both retire: once a clause body cannot parse, a poly word cannot carry one.

- **C4 — decision 2's `Bool` arm bodies must *consume* the receiver.** The brief's
  decision 2 said the generated `Bool` print word gets "an eliminator-term body" but did
  not state that each owning arm must consume its narrowed variant. A naive
  `~[ ( False ) "false\n" . ]` fails with `eliminator_variant_escape_error` (the `False`
  value is left on the stack). Probed directly: the correct body is
  `~[ ( False ) drop "false\n" . ] ~[ ( True ) drop "true\n" . ] bool?`. The same
  correction applies to every zero-field owning arm in the `.sth` migrations (R7): a
  payload-free arm must `drop` its receiver, since the linear spine does not auto-drop
  (the clause path's decompose binding did this implicitly).

The rest of recon 4 (the `WordBody::Clauses` production match sites, `audits.rs`'s guard,
the `globals.rs` clause-constructing test, `ArmBinding::Decompose`'s sole caller) verified
correct, modulo small line drift noted per requirement below.

## Design as intended

Delete, do not deprecate (decision 1). `WordBody::Clauses` and its whole service graph are
removed outright: no flag, no parseable-but-warned grace period. Every in-tree call site
migrates in the same slice, so a deprecation window buys nothing.

- **R1 — the clause word body stops parsing.** Delete `parse_clauses` (`parser.rs:1784`),
  its call site branch (`parser.rs:1474-1476`, leaving the body as an unconditional
  term-body parse), `at_clause_start` (`parser.rs:1775`), and `parse_clause_body_terms`
  (`parser.rs:3646`). A source using `| Variant ...` at body position is then an ordinary
  parse error (a `|` is only ever a binding term), which is the located rejection for the
  REPL path too (`repl.rs:2227` is wired through `assemble_module`, so per the standing
  rule that anything there is unenforced at the REPL, correctness comes from the syntax no
  longer parsing, not a new located check). The `terms_body` test helper (`parser.rs:3912`)
  loses its now-irrefutable `WordBody::Clauses` arm. Retire the four clause-parsing unit
  tests (C2) and the `clauses_body` accessor; retitle
  `parse_term_word_with_leading_locals_is_not_a_clause` to keep its surviving assertion
  (leading `| a b |` parses as entry-locals of a term body).

- **R2 — the `WordBody::Clauses` variant is deleted, with every match arm that reads it.**
  Grepped exhaustively against `src/`:
  - `resolve.rs:435`, `ir/func_builder/mod.rs:792` (arm at `:792`, whose body passes
    `ArmBinding::Decompose` at `:803`), `repl.rs:2227` — production match arms, deleted
    with the variant.
  - `check/combinators.rs:146`, `check/drop_graph.rs:88` and `:706`,
    `check/globals.rs:90`, `check/word_entry.rs:59` and `:100`, `check/poly.rs:196` and
    `:338` (C3), `check/audits.rs:249` — checker match arms and guards, deleted with the
    variant.
  Once the variant is gone, `WordBody` has a single case (`Terms { terms }`). Collapsing
  `WordBody` to a bare `Vec<Term>` (mirroring decision 5's `ArmBinding` latitude) is a
  mechanical cleanup with an identical observable result and is left to the implementer;
  keeping it a one-variant enum is equally acceptable.

- **R3 — paired retirement units.** Two deletions are each a function-plus-dependents unit:
  - `check_clause_word` (`word_entry.rs:311`, C1) retires with its call site
    (`word_entry.rs:59`) and its five unit tests (`declarations.rs:2973-3025`).
  - `clause_bodied_quotation_word_error` (`audits.rs:455`) retires with its sole guard
    (`audits.rs:249-255`). Its only reason to exist is that a clause body cannot be spliced
    by the inliner (`combinators.rs:173` documents this) — a fact that stops being true of
    anything once no word can have a clause body.
  Surviving comments in `check.rs:2304` and `:2565` that say "adapted from
  `check_clause_word`" / "mirroring `check_clause_word`" go stale and are reworded to name
  the behaviour, not the retired function.

- **R4 — `ArmBinding` collapses to one case.** `ArmBinding::Decompose`
  (`ir/func_builder/control_flow.rs:16`, matched at `:303`) has exactly one caller, the
  clause-body arm at `mod.rs:803`, which R2 deletes. `ArmBinding::WholeValue` (used at
  `calls.rs:774`, the eliminator's own lowering, since Slice 3) is then the only case.
  Per decision 5, whether `ArmBinding` becomes a `WholeValue`-only marker or is removed
  entirely (dropping `lower_clauses`'s binding-mode parameter) is an implementation choice
  with an identical observable result; either ships.

- **R5 — the generated `Bool` print word gets an eliminator-term body (decision 2, C4).**
  `bool_print_word_def` (`ast.rs`, currently constructing
  `WordBody::Clauses(vec![clause("False", "false\n"), clause("True", "true\n")])`) is
  rebuilt to construct a `WordBody::Terms` holding two tagged inline-quotation arms and a
  trailing `bool?` call. The `WordBody::Terms` is assembled the same way any hand-built
  compiler-internal word body is: each arm is a
  `Term { kind: TermKind::Quotation(body, /*is_inline*/ true, Some(annot)), .. }`, where
  `annot: QuotAnnot` carries `variant_tag: Some(VariantTag { name: "False".into(), mode:
  VariantTagMode::Owning })`, empty `inputs`/`outputs`, both rows `None`, and empty name
  tables — the exact shape the parser produces for `~[ ( False ) ... ]` (verified against
  `parse_leading_variant_slot`'s output per slice3b R1/R2). Each arm body is
  `[ Call("drop"), StrLit("false\n"), Call(".") ]` (C4: the owning arm must consume its
  narrowed variant before printing). The trailing term is `Call("bool?")` — the eliminator
  env key for the `bool` enum, since `eliminator_registry` keys each enum by
  `generic_surface_name(name)` suffixed with `?` (`declarations.rs:1465`, so `bool` keys
  `bool?`), and `bool_enum_decl` is injected into every module.
  - **OQ1 answer.** The migration changes only *how* the body is built. The word keeps its
    name (`.`), its declared effect (`( bool -- )`, printing `"false\n"` / `"true\n"`), and
    its `builtin_overloads`-dispatched call sites. No caller depends on anything else about
    it, so no mangled-symbol or call-site change follows. Verified by probe: an all-unit
    two-variant enum with `~[ ( Off ) drop "off\n" . ] ~[ ( On ) drop "on\n" . ] Flag?`
    checks, lowers, and prints correctly (owning-mode scalar elimination runs; the
    slice3b "scalar enum by reference dies in the backend" gap is reference-mode only).

- **R6 — every clause-bodied `.sth` site migrates to the eliminator.** Grepped for the
  clause shape (`: word ... | Variant ...`), not estimated:
  - `examples/shapes.sth` (`area`, `unwrap-or`): plain tag-matched arms. `unwrap-or`'s
    `None` arm must `drop` its receiver (C4): `~[ ( None ) drop ] ~[ ( Some ) Some> swap drop ] MaybeInt?`.
  - `examples/vm.sth` (`run`, nine mixed-payload arms) and `examples/vm_table.sth`
    (`decode`, same shape): each arm ends in a self-tail-call back-edge to the word.
    **Correction (post-implementation review): the original probe claim here was
    false.** It asserted an eliminator arm's tail call already takes the Slice 6
    self-tail-call back-edge with no checker change needed. It does not: `TailWalk`
    (`src/check/drop_graph.rs`, the syntactic pass both `has_self_tail_call` and the
    tail-call cycle graph read) had fallen out of lockstep with `lower_eliminator`,
    which has threaded `tail` into arms since Slice 3 (`calls.rs:740`) -- `walk` had
    no case for an eliminator dispatch call, so it never saw `run`'s/`decode`'s tail
    call inside an arm as inheriting tail position. Confirmed by reverting the fix in
    isolation: `vm_dispatch_loop_runs_in_constant_stack` fails. The fix, in scope for
    this phase since the migration is what made the gap reachable, extends `walk` to
    recognize the run of tagged quotation-literal arms immediately preceding an
    eliminator dispatch call the same way it already recognizes `call`'s/`branch`'s
    quotation operands (`src/check/drop_graph.rs`, with unit coverage:
    `tail_position_eliminator_arm_self_call_is_tail`,
    `tail_position_eliminator_arm_run_stops_at_untagged_operand`). One consequence:
    a *mutual* tail-recursion cycle routed entirely through eliminator arms, previously
    invisible to `check_tail_call_cycles`, is now correctly rejected the same way an
    `if`-arm mutual cycle already is (`tail_mutual_recursion_through_eliminator_arms_
    is_error`). This **is** a behavior change: such a cycle compiled before, but with
    unbounded stack growth (the pre-fix build segfaults on one at depth 1e6), which is
    exactly what the rule exists to prevent. Programs that previously compiled into a
    stack overflow are now rejected at compile time.
  - `examples/list.sth` (`pop`) and `examples/refs.sth` (`pop`, `walk`): a recursive,
    boxed enum `List | Nil | Cons v i64 next ^List`, in both owning and `&!` mode.
    **Migrated and run this session; output matches the clause form byte-for-byte** (R7).
  - `lib/binary_search.sth` is **not in scope** (untracked scratch using syntax the
    language does not have: `b.low`, `arr[idx]`, `#arr`, `set`, `BinSearchArgs>low`). It is
    left untouched; it is not a migration target.
  Each migrated file's header prose also references "clause dispatch" / "clause-style"
  (`vm.sth:3,73`, `vm_table.sth:4,5,192`, `shapes.sth:21`); those comments are rewritten to
  the eliminator form as part of the migration. **OQ3 answer:** a grep pass found no
  `.sth` or doc file that quotes the clause-path *error text* (non-exhaustive /
  duplicate-clause wording), so no diagnostic reference goes stale; only these descriptive
  comments in the migrated files themselves do.

- **R7 — the multi-field `&`/`&!`-mode arm-authoring pattern is stated, not rediscovered
  (decision 4).** An owning arm decomposes with the whole destructure (`Cons>`) and unboxes
  a boxed field with `^>`. A reference-mode arm cannot: projecting more than one field off
  an anonymous receiver fails, because the first `&!field` projection consumes the
  receiver. Bind the receiver to a local first, then project each field off the local:

  ```text
  ~[ ( &!Cons ) | c | c &!v 1 +! c &!next &!^ walk ]
  ```

  This is not a checker change and needs none; the diagnostic if you get it wrong is a
  generic "does not borrow a place" error that does not name the pattern, so the spec
  states it so every migrated reference-mode arm uses it directly. **Verified for every
  migrated example, not only the probed one:** `list.sth`'s owning `pop`
  (`Cons> | v next | next ^> v Popped`) and `refs.sth`'s by-reference `walk` (the pattern
  above) both build and produce identical output to their clause forms (`list` prints `6`;
  `refs` prints `72 / 90 / 2 / 2`).

- **R8 — the one uncovered clause-path capability gets an eliminator-form golden before
  its file is deleted (decision 3).** `tests/phase5_generic_enum_elimination.rs` is deleted
  as a file (its subject, clause-style generic elimination, no longer exists). Three of its
  four tests already have eliminator-form equivalents in `tests/phase6_slice3b.rs`
  (`generic_enum_eliminator_runs_both_arms`,
  `two_asymmetric_instantiations_eliminate_independently_in_one_word` — a stronger claim
  than the clause original, since it also covers asymmetric instantiation, and
  `non_exhaustive_generic_eliminator_names_the_surface_variant`). The fourth,
  `generic_enum_elimination_type_declared_after_matching_word` (`phase5_generic_enum_elimination.rs:63`,
  forward-declared generic type resolution), has no eliminator-form witness. Its
  replacement, `forward_declared_generic_type_eliminates_after_the_matching_word`, goes in
  `tests/phase6_slice3b.rs` alongside its three siblings (structurally verified: the file's
  `build_and_run(name, src)` helper and `assert_eq!(stdout, ...)` shape fit it exactly).
  **Probed:** `~[ ( Ok ) Ok> ] ~[ ( Err ) Err> 100 + ] Result?` with
  `type: Result 'T 'E | Ok val 'T | Err val 'E ;` declared *after* the word compiles and
  prints `42 / 107`, the same output the clause original asserts. The direct
  `WordBody::Clauses`-constructing unit test `direct_set_reaches_into_a_clause_body`
  (`globals.rs:480-491`) retires with the mechanism (it is deleted, not "fixed"), and the
  parser clause tests (C2) are deleted with `parse_clauses`.

## Guards (tests the phase adds or must keep green)

- **R5 golden.** `bool_print_word_prints_false_and_true` (or the migrated Slice 9 `bool`
  print test, if one already asserts the output): a program that prints a `false` and a
  `true` still emits `"false\ntrue\n"`. This is the runnable witness that the internally
  generated eliminator-term body checks, lowers, and runs — a placebo-proof point, since it
  is the one non-`.sth` clause body in the compiler.
- **R6/R7 goldens.** Every migrated example keeps its existing golden (`tests/phase*`
  covering `shapes`/`vm`/`vm_table`/`list`/`refs`, or a direct build-and-run) and must
  print exactly what it printed as a clause body — `vm.sth`'s bytecode-interpreter dogfood
  included, self-tail-call back-edges intact. These are regression guards, not new
  behaviour: a migration that changed output would fail them.
- **R8 golden.** `forward_declared_generic_type_eliminates_after_the_matching_word`
  (`tests/phase6_slice3b.rs`), asserting `"42\n107\n"`, added **before**
  `tests/phase5_generic_enum_elimination.rs` is deleted.
- **`TailWalk` eliminator-arm unit coverage.** `src/check/drop_graph.rs` gets a unit
  test per this phase's new tail rule, mirroring the existing splice-recognition tests:
  `tail_position_eliminator_arm_self_call_is_tail` (a self-tail-call inside a tagged arm
  is recognized) and `tail_position_eliminator_arm_run_stops_at_untagged_operand` (the
  reverse scan for the tagged-arm run halts at the first non-tagged operand, so a
  self-tail-call sitting beyond it is not reached). `tail_mutual_recursion_through_
  eliminator_arms_is_error` covers the new-rejection consequence noted above.
- **Deletion is proved by absence, not by a new test.** The exit criterion "no
  `WordBody::Clauses` anywhere in `src/`" is checked mechanically (grep) and by the tree
  building green; there is no located "clause body rejected" diagnostic to test, because
  the rejection is a plain parse error (R1). Do **not** add a placebo test that asserts a
  clause body fails to parse with specific wording; the failure is the generic
  `|`-is-a-binding parse error, not a Phase 6 diagnostic.
- **Unchanged regressions.** All of `tests/phase6_slice3.rs`, the surviving
  `tests/phase6_slice3b.rs` suite, and `examples/eliminator{,_ref,_mid_body}.sth` stay
  green — this slice removes the redundant mechanism and repoints call sites; it does not
  touch the eliminator, its mode resolution, or `eliminator_registry`.

## Delivery plan

Two phases. The seam (OQ2 answer) is forced by the type system, not chosen for taste: an
enum-variant removal cannot compile until *every* arm that reads it is gone, so R2's
variant deletion and all its match-arm/guard/test deletions are **one atomic commit**. But
the producer migration (R5, R6) and the golden move (R8) are separable and must land
**first**, so that when phase 2 deletes the variant there is nothing left constructing or
matching it. Within phase 2, R3's two paired units and R4's `ArmBinding` collapse ride the
same commit as the variant removal (they are unreachable the instant the variant is gone).

- **Phase 1 — migrate producers, add the golden (variant still present, tree green).**
  Rebuild `bool_print_word_def` to a `WordBody::Terms` eliminator body (R5); migrate
  `examples/{shapes,vm,vm_table,list,refs}.sth` and their header comments (R6/R7); add
  `forward_declared_generic_type_eliminates_after_the_matching_word` to
  `tests/phase6_slice3b.rs` (R8). At phase exit no `.sth` site and no compiler-internal
  word constructs a clause body, but the `WordBody::Clauses` variant and `parse_clauses`
  still exist and every migrated golden passes.
- **Phase 2 — delete the clause path (one atomic commit, tree green without the variant).**
  Delete `parse_clauses` and its helpers and call site (R1); delete the `WordBody::Clauses`
  variant and every production match arm (R2); retire `check_clause_word` + its five tests
  and `clause_bodied_quotation_word_error` + its guard (R3); collapse `ArmBinding` (R4);
  delete `tests/phase5_generic_enum_elimination.rs`, the parser clause tests, and
  `direct_set_reaches_into_a_clause_body` (R8). At phase exit `grep -r 'WordBody::Clauses'
  src/` returns nothing and the tree is green.
  Three items outside `src/` ride this commit, all falsified by the variant's removal:
  - **`DESIGN.md`.** `:456`, `:512` and `:1032` declare clause-bodied definition "the
    sole enum eliminator"; `:63` and `:510` name "a clause body" as a locals-extent
    scope. All five are false once the form is gone; they must name the eliminator
    (`~[ ( V ) .. ] Shape?`) and its arm bodies instead.
  - **`tests/phase4_quotations.rs:1042`.** The M5 guard
    `matches!(word.body, WordBody::Terms { .. })` becomes **irrefutable** the instant
    `WordBody` drops to one variant, i.e. a silent placebo asserting nothing. R2's
    inventory swept only `src/`, so it does not catch this. Re-point it at what the
    guard actually means (a `vm_table.sth` handler contains no eliminator dispatch
    call) and mutation-test the replacement by inlining a dispatch back into a handler.
  - **Stale prose.** `tests/phase4_quotations.rs:996-997` and
    `tests/phase3_locals.rs:456-459` still describe the migrated `vm.sth` bodies as
    "clauses".

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Migrate producers (R5-R8): rebuild the generated Bool print word as a WordBody::Terms eliminator body, migrate examples/{shapes,vm,vm_table,list,refs}.sth to eliminator terms, and add the forward-declaration golden to tests/phase6_slice3b.rs. WordBody::Clauses and parse_clauses still exist at exit; every migrated golden and the new golden pass.",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "Delete the clause path in one atomic commit (R1-R4, R8): remove parse_clauses/at_clause_start/parse_clause_body_terms and their call site and tests in src/parser.rs, delete the WordBody::Clauses variant and every production match arm it forces (resolve.rs, ir/func_builder/mod.rs, repl.rs, and the check/{combinators,drop_graph,globals,poly,audits,word_entry} sites), retire check_clause_word plus its five declarations.rs tests and clause_bodied_quotation_word_error plus its audits.rs guard, collapse ArmBinding, and delete tests/phase5_generic_enum_elimination.rs and the direct_set_reaches_into_a_clause_body unit test. Also outside src/: update DESIGN.md:63,456,510,512,1032 (they call clause-bodied definition the sole enum eliminator and name a clause body as a locals scope), re-point the now-irrefutable M5 guard at tests/phase4_quotations.rs:1042 (matches!(word.body, WordBody::Terms { .. }) asserts nothing once WordBody has one variant) and mutation-test its replacement, and fix the stale \"clause\" prose at tests/phase4_quotations.rs:996-997 and tests/phase3_locals.rs:456-459. Exit: grep -r 'WordBody::Clauses' src/ returns nothing and the tree is green.",
      "difficulty": "hard"
    }
  ]
}
```

## Exit

The clause-style word body no longer parses. The tree builds green without
`WordBody::Clauses` anywhere in `src/`: no dead match arm, no retired-but-present
diagnostic, no test that still constructs one. Every migrated `.sth` example prints the
same output it did as a clause body (`examples/vm.sth`'s nine-variant `Op` dispatch
included, self-tail-call back-edges intact), and the generated `Bool` print word still
emits `"false\n"` / `"true\n"` from an eliminator-term body. The one clause-path
capability without a prior eliminator-form witness (forward-declared generic type
resolution) has a golden in `tests/phase6_slice3b.rs` before
`tests/phase5_generic_enum_elimination.rs` is deleted.

## Out of scope

- **Slice 5** (nested tag paths, `( Some[v Circle] )`): a separate slice, already
  briefed and specced (`docs/roadmap/P6/slice5-brief.md`, `slice5-spec.md`, both committed
  `485e27f`). Verified no collision: slice5-brief.md:209-210 states it is "Independent of
  Slice 4 — nothing here touches the clause path", and its spec references no
  `WordBody::Clauses` / `parse_clauses` / `check_clause_word`. The two do not contend for
  the same section of the phase doc.
- Any change to eliminator semantics, mode resolution, or `eliminator_registry`: Slice 3b
  shipped and reviewed all of that; this slice removes the redundant mechanism and
  repoints call sites, nothing more.
- The pre-existing scalar-enum-**by-reference** backend crash (documented in
  slice3b-spec.md's "Known gaps"): unaffected by this migration (the `Bool` word and every
  migrated scalar arm is owning-mode) and not fixed here.
- `lib/binary_search.sth`: not a real program, not a migration target (R6).

## References

- [slice4-brief.md](./slice4-brief.md) — discovery, recon, decisions 1–5.
- [slice3b-spec.md](./slice3b-spec.md) — check-time arm-tag resolution (the generic-enum
  capability this migration relies on).
- [slice3-spec.md](./slice3-spec.md) — the eliminator word, `ArmBinding`, `Type::Variant`.
- [P6-enum-elimination.md](../P6-enum-elimination.md) — Phase 6 plan.
