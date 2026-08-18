# Phase 6 Slice 4: migration (brief)

Every clause-dispatch site moves to the eliminator, and `WordBody::Clauses` /
`parse_clauses` are deleted, along with everything that exists only to serve them. This
is the payoff slice: two elimination mechanisms in the language permanently is worse than
one migration, and Slice 3b (done, merged `9c3b6ca`) removed the last capability gap
(generic enums) that made the clause path load-bearing.

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

## Recon (measured against the built compiler, 2026-08-18, `main` at `9c3b6ca`)

`cargo test`/`clippy -D warnings`/`fmt --check` are all green at this HEAD. Two
pre-existing uncommitted local edits elsewhere in the tree
(`tests/phase4_slice12_partab.rs`, `tests/phase4_slice6g.rs`, a WIP `lib/arrays.sth`
signature change) are unrelated to Phase 6 and untouched by this recon.

1. **Every real clause-bodied site in the tree migrates cleanly.** Grepped for the
   clause-syntax shape (`: word ... | Variant ...`), not estimated:
   - `examples/shapes.sth` (`area`, `unwrap-or`) — plain tag-matched arms, no payload
     aliasing. Migrates directly (shown above).
   - `examples/vm.sth` (`run`, 9 mixed-payload arms) and `examples/vm_table.sth`
     (`decode`, same shape) — each arm ends in a self-tail-call back-edge to the word
     itself. **Probed working**: an eliminator arm's tail call still takes the Slice 6
     self-tail-call back-edge; this was the one mechanism this migration could plausibly
     break and it does not.
   - `examples/list.sth` (`pop`) / `examples/refs.sth` (`pop`, `walk`) — a **recursive,
     boxed** enum, `List | Nil | Cons v i64 next ^List`, in both owning and `&!`
     (mutable-reference) mode. **Not previously probed; built and run this session.**
     Owning mode (`Cons>` through the boxed field, `^>` to unbox) is unsurprising. The
     by-reference case (`walk`, over `&!List`) surfaces a real pattern the brief needs to
     name: an arm's receiver is an anonymous stack value, and projecting *two* fields off
     it (`v`, then `next`) fails if chained directly — the first `&!field` projection
     consumes the receiver. The fix is the same pattern `refs.sth` already uses for a
     bound local: `~[ ( &!Cons ) | c | c &!v 1 +! c &!next &!^ walk ]`. Not a checker
     change, not a new rule — just the arm-authoring pattern for any migrated clause
     whose body touches more than one field of a `&`/`&!` scrutinee. The spec should
     state this pattern explicitly so migrated call sites use it instead of rediscovering
     it independently.
   - `lib/binary_search.sth` is **not in scope**: untracked scratch using syntax that
     does not exist in the language today (`b.low`, `arr[idx]`, `#arr`, `set`,
     `BinSearchArgs>low`). Leave it alone; it is not a migration target.

2. **Forward-declared types still resolve.** Slice 3b moved tag *typing* to check time,
   which changes *when* a tag's enum is known; `generic_enum_elimination_type_declared_after_matching_word`
   (`tests/phase5_generic_enum_elimination.rs:63`) is the clause-path witness for this and
   has no eliminator-form equivalent yet. **Probed this session**: a word using
   `~[ ( Ok ) Ok> ] ~[ ( Err ) Err> 100 + ] Result?` compiles and runs correctly with
   `type: Result` declared *after* the word that eliminates it. The eliminator carries
   the same ordering independence; this needs a golden before the file it's currently
   proven in is deleted (see decision 3).

3. **`tests/phase5_generic_enum_elimination.rs`'s other three tests already have
   eliminator-form equivalents.** `tests/phase6_slice3b.rs` covers: basic two-arm
   dispatch (`generic_enum_eliminator_runs_both_arms`), two independent instantiations
   (`two_asymmetric_instantiations_eliminate_independently_in_one_word` — a stronger
   claim than the clause-path original, since it also covers asymmetric instantiation),
   and non-exhaustive naming the surface variant
   (`non_exhaustive_generic_eliminator_names_the_surface_variant`). Only the
   forward-declaration case (recon 2) is uncovered.

4. **The retirement inventory is exhaustive, not partial.** Every `WordBody::Clauses`
   match site, grepped directly against `src/`:
   - `src/parser.rs:1475` (`parse_clauses` call site) and `:1784` (`parse_clauses` itself)
     — deleted.
   - `src/ast.rs:850` — the generated `Bool` print word (`False`/`True` clauses) is the
     one non-test, non-check production use; it needs an eliminator-term equivalent, not
     just deletion (see decision 2).
   - `src/resolve.rs:435`, `src/ir/func_builder/mod.rs:792`, `src/repl.rs:2227` — each a
     `WordBody` match arm that becomes unreachable once the variant is gone; deleted with
     the variant, not left as a dead arm.
   - `src/check/combinators.rs:146`, `src/check/drop_graph.rs:88,706`,
     `src/check/globals.rs:90`, `src/check/poly.rs:196,338`, `src/check/audits.rs:249`,
     `src/check/word_entry.rs:59,100` — checker-side match arms and guards, deleted with
     the variant. `word_entry.rs:59` is `check_clause_word`'s call site;
     `check_clause_word` itself (`declarations.rs:311`) and its five unit tests
     (`declarations.rs:2973-3025`) retire as a unit. `audits.rs:249`'s guard *is*
     `clause_bodied_quotation_word_error` (recon: its sole reason to exist is that a
     clause body cannot be spliced by the inliner — a fact that stops being true of
     anything once no word has a clause body); the rule and its error constructor retire
     together.
   - `src/check/globals.rs:480-490` is a **unit test that constructs a `WordBody::Clauses`
     value directly** to prove global-set inference walks into a clause body. It retires
     with the mechanism, not just its production callers — a mechanical scope point, but
     worth naming so it doesn't get "fixed" instead of deleted.
   - `ArmBinding::Decompose` (`src/ir/func_builder/control_flow.rs:16`, used at `:303`):
     confirmed its **only** caller is the clause-dispatch path. The eliminator uses
     `ArmBinding::WholeValue` exclusively (`calls.rs:774`) and always has, since Slice 3.
     `ArmBinding` collapses to a single case; whether it stays a one-variant enum or is
     deleted in favor of a plain call is an implementation choice, not a design one.
   - REPL (`src/repl.rs:2227`): per this project's standing rule that anything wired into
     `assemble_module` is unenforced at the REPL, confirm the REPL path for a clause body
     is a parse-time rejection once `parse_clauses` is gone (a syntax that no longer
     parses, not a new located check to write).

## Decisions

1. **Delete, don't deprecate.** `WordBody::Clauses` and `parse_clauses` are removed
   outright in this slice, not gated behind a flag or left parseable-but-warned. Two
   permanent elimination mechanisms was never the destination (see the phase intro); a
   deprecation period buys nothing since every call site in-tree migrates in the same
   slice.

2. **The generated `Bool` print word (`src/ast.rs:850`) gets an eliminator-term body,
   not a hand-maintained special case.** It is the one production (non-test) clause body
   in the compiler itself. Migrating it the same way every `.sth` site migrates keeps the
   retirement uniform: no `WordBody::Clauses` construction survives anywhere, including
   the one the compiler builds internally.

3. **Add the missing forward-declaration golden before deleting the file it currently
   lives in.** `tests/phase5_generic_enum_elimination.rs` is deleted as a file (its
   subject, clause-style generic elimination, no longer exists), but its one claim without
   an eliminator-form witness (recon 2) does not get to disappear silently. The
   replacement golden goes in `tests/phase6_slice3b.rs`, alongside its three siblings.

4. **State the multi-field `&`/`&!`-mode arm-authoring pattern in the spec, don't leave
   it to be rediscovered per call site.** Recon 1's `walk` finding (bind the receiver to
   a local before projecting more than one field off it) is not a checker change and
   needs none — but every migrated clause site with more than one field access under a
   reference-mode arm hits it, and the diagnostic if you get it wrong (consuming an
   already-consumed receiver) is a generic "does not borrow a place" error, not one that
   names the pattern. Migrated call sites should use it directly rather than costing each
   migration its own rediscovery.

5. **`ArmBinding`'s collapse to one variant is cleanup, not a design decision.** Whether
   it becomes `ArmBinding::WholeValue`-only (kept as a marker for future extension) or
   removed entirely in favor of `lower_clauses` no longer taking a binding-mode parameter
   is left to whoever implements; either is a mechanical simplification with an identical
   observable result, and re-litigating it does not change what ships.

## Open questions for the spec

- **OQ1 — does the `Bool` print word's migration change its mangled symbol name or call
  sites?** Decision 2 changes *how* the word's body is built, not its signature; the spec
  should confirm no caller depends on anything about it besides its declared effect
  (`( Bool -- )`, printing `"false\n"`/`"true\n"`).
- **OQ2 — is there a clean order for the retirement inventory (recon 4), or does the
  `WordBody::Clauses` variant removal have to be one atomic commit like Slice 3b's Phase
  1?** Unlike Slice 3b's type change, most of these sites are independent match arms with
  no shared reader graph forcing atomicity — but `check_clause_word`'s deletion and its
  five tests, and `clause_bodied_quotation_word_error`'s deletion and `audits.rs:249`'s
  guard, are each a paired unit. The spec should say whether the whole slice is one phase
  or several, and if several, where the seams are.
- **OQ3 — does anything downstream of `check_clause_word` depend on its non-exhaustive /
  duplicate-clause diagnostics having distinct wording from the eliminator's equivalent
  errors?** If a migrated `.sth` example or doc quotes the clause-path error text
  anywhere, that reference goes stale; worth a grep pass in the spec rather than
  assuming none exists.

## Out of scope

- Anything from Slice 5 (nested tag paths, `( Some[Circle] )`): a separate slice, already
  briefed/specced independently in this tree
  (`docs/roadmap/P6/slice5-brief.md`/`slice5-spec.md`, untracked as of this recon —
  confirm with whoever owns that work before this slice's spec locks its own "Exit",
  since S5 sits immediately after S4 in the roadmap and the two should not collide on the
  same section of `docs/roadmap/P6-enum-elimination.md`).
- Any change to eliminator semantics, mode resolution, or `eliminator_registry`: Slice 3b
  shipped and reviewed all of that; this slice only removes the now-redundant mechanism
  and repoints call sites, it does not touch the eliminator itself.
- `lib/binary_search.sth`: not a real program (recon 1), not a migration target.
- The pre-existing scalar-enum-by-reference backend crash (documented in Slice 3b's spec,
  reproduced independently, wants its own slice): unaffected by this migration and not
  fixed here.

## Sequencing

Gated on Slice 3b (done, `main` at `9c3b6ca`). Touches `src/parser.rs` (delete
`parse_clauses` and its call site), `src/ast.rs` (`WordBody::Clauses` variant deleted,
`Bool`'s print word rebuilt per decision 2), `src/resolve.rs`, `src/ir/func_builder/mod.rs`,
`src/repl.rs`, `src/ir/func_builder/control_flow.rs` (`ArmBinding` collapse),
`src/check/{combinators,drop_graph,globals,poly,audits,word_entry,declarations}.rs`
(match-arm deletions, `check_clause_word` and `clause_bodied_quotation_word_error`
retirement), `examples/{shapes,vm,vm_table,list,refs}.sth` (migrated), and
`tests/phase5_generic_enum_elimination.rs` (deleted, with its one uncovered claim moved
to `tests/phase6_slice3b.rs` per decision 3).

## Exit

The clause-style word body no longer parses. The tree builds green without
`WordBody::Clauses` anywhere in `src/` — no dead match arm, no retired-but-present
diagnostic, no test that still constructs one. Every migrated `.sth` example prints the
same output it did as a clause body (`examples/vm.sth`'s bytecode-interpreter dogfood
included, self-tail-call back-edges intact). The one clause-path capability without a
prior eliminator-form witness (forward-declared generic type resolution) has a golden in
`tests/phase6_slice3b.rs` before `tests/phase5_generic_enum_elimination.rs` is deleted.

## Ready to spec?

**Yes.** Every clause-bodied site in the tree has been traced to a concrete eliminator
form, including the one genuinely untested combination (recursive/boxed enum by mutable
reference), which was built and run rather than assumed. The retirement inventory (recon
4) is a grep-verified list, not an estimate. The only open design content is sequencing
(OQ2) and two narrow confirmations (OQ1, OQ3); nothing here is an architecture choice the
spec still has to make.
