# Phase 3 Slice 5 — General locals (implemented)

Design input: [the brief](./phase3-slice5-brief.md). Base: `main` @ `0267843` (700 tests green).
Delivered: `fe2dbc9` (phase 1), `b9662bf` + `c8b476f` + `f9f5c34` (phase 2), `aedcacf` (phase 3).

`| names |` is now a term legal at any point in a body, and REPL lines have locals. Both
are prerequisites for Slice 6: a bare REPL line forms no place, so it can take no borrow.

## What shipped

- **R1/R3 — a binding is a term.** `TermKind::Bind(Vec<String>)` (src/ast.rs:501); it pops
  that many values where it appears, leftmost name taking the deepest. `WordBody::Terms`
  no longer carries a `locals` field: entry locals are just a leading `Bind`. `Clause.locals`
  stays a field (clause payload binding is part of the pattern, not a body term). The
  entry-position diagnostic survives the unification, keyed off a leading `Bind`
  (src/check.rs:1122, message at :1125).
- **R2 — extent is the rest of the enclosing block** (word body, clause body, `if`/`else`
  arm). No closing token. Sibling arms may each bind the same name.
- **R4/R5 — rejections.** Re-binding a name in scope is a located error, applied uniformly
  to linear and Copy values. Binding more than the frame holds reuses the existing
  needs-N-holds-M underflow shape; the floor is the declared inputs in a word and the
  current session stack depth at a REPL line.
- **R6 — block-end linearity site.** `Scope::leave` reports the first (name-sorted)
  unconsumed linear local at its block's terminator; `BlockEnd` distinguishes a body/line
  (cites a line) from an arm (cites the exact terminator token).
- **R7 — REPL line locals**, scoped to the line: `infer_line` builds a fresh `Scope` and
  calls `leave_block` at end of line (src/check.rs:768-773). Session stack persists, names
  do not; the existing transactional rollback is untouched.
- **R9 — evolving locals.** `Ctx::Word`'s `&'a HashMap<String, Type>` is gone. Names live in
  a threaded `&mut Scope` (`bound: Vec<(String, Type)>` plus `Moves`), innermost-last so
  leaving a block truncates to its entry depth (src/check.rs:247-293).
- **R10 — no new `Instr`.** Lowering pops from the lowering stack into a
  `Vec<(String, Value)>` locals list (src/ir.rs:1704) and truncates it at each arm's exit
  (src/ir.rs:2623 and :2630).
- **R11 — nothing crosses a back edge.** Confirmed: a mid-body name's extent ends at the
  block terminator where the tail call sits, so no header phi is added.
- **Dogfood.** `examples/vm.sth`'s `run` word names the first `vm-pop` result mid-body in
  its `Add`/`Sub`/`Mul`/`Store` clauses instead of shuffling it into position with
  `swap`/`over`/`rot`, output byte-identical. ROADMAP/DESIGN updated.

## Deviations from the spec as written

- **R8's "located parse error" was not implementable, and was amended in phase 2.** A
  clause-opening `|` and a binding `|` are token-identical, so the parser has no ground to
  prefer a reading. Delivered instead as a diagnostic from whichever reading was applied,
  each noting the rule: a `|` led by a registered variant name is checked as a clause
  (unknown-variant or duplicate-clause), a `|` led by anything else is parsed as a binding.
  Unknown-variant, duplicate-clause and exhaustiveness checks were hoisted ahead of body
  checking so a misspelt variant swallowed as a binding cannot misattribute the failure to
  a sibling clause, and its missing sibling variant is still reported.
- **`Ctx::Line` gained no locals map** (spec R7/phase 2 said it would). `Scope` is threaded
  independently of `Ctx`, so both contexts share one mechanism and `Ctx` stays purely the
  error-message context. Consequently **src/repl.rs needed no change**: a `Bind` is an
  ordinary term over the session stack.
- **The spec's `build`/`build-into` dogfood was unsatisfiable.** `build-into` never existed
  in `examples/vm.sth` at the base commit; the name came from the brief's narrative about a
  different slice's drafts, and `build` itself has no binding site to rewrite. `run` was
  dogfooded instead (see above).
- **R8's variant-name-collision restriction is global, not clause-body-scoped.**
  `reject_variant_local` fires from the shared `TermKind::Bind` handling, so a local may not
  be named after any registered variant in a plain word body either, not only in a clause
  body. Not a regression (the base commit already refused it at entry, via a
  "clause-style body" error), but wider than the rule's name suggests.
- **`check_clause_word` hoisted its exhaustiveness loop ahead of body-checking**, so a
  clause-style word that is both non-exhaustive and has a body type error in a present
  clause now reports non-exhaustive first (previously the body error, since exhaustiveness
  ran after). Required so a misspelt variant swallowed as a binding (D8) still reports its
  missing sibling instead of the swollen clause's own arity failure; the reordering is a
  side effect that reaches every clause-style word, pinned by
  `check_clause_word_reports_non_exhaustive_before_body_type_error`.
- **The original spec's headline REPL dogfood contradicted its own R7.** The three-line
  session `1 2 3` / `| a b |` / `a b + .` cannot work: R7 makes line locals die with the
  line, so line 2's binding is gone before line 3 reads it. The implementation correctly
  chose R7 over the example. Corrected, working dogfood, with the binding and its use on
  one line so they share a scope, while still reaching values an earlier line left:

  ```
  1 2 3
  | a b | a b + .
  ```

  prints `5` and leaves `1` on the session stack.

## Load-bearing invariants held

Nothing auto-drops (R6 only catches forgetting earlier). A word cannot bind beneath its
declared inputs. No new `Instr` variant. Prefer-the-stack remains a convention, not a limit.

## Tests

Goldens in `tests/phase3_locals.rs`; unit tests beside their stage. Every diagnostic is
asserted on its specific message.

| criterion | test |
|---|---|
| mid-body binding pops; leftmost name takes the deepest value | `mid_body_binding_consumes_from_the_stack`, `mid_body_binding_leftmost_name_takes_deepest_value` |
| arm-scoped extent, and sibling arms may reuse a name | `local_bound_in_if_arm_is_not_visible_after_end`, `name_bound_in_one_arm_can_be_rebound_in_sibling_arm` |
| re-binding in scope / over-binding / empty binding are located errors | `rebinding_a_name_in_scope_is_error`, `binding_more_values_than_frame_holds_is_error`, `empty_binding_with_no_names_is_error` |
| a word cannot bind beneath its inputs; entry diagnostic survives | `binding_cannot_reach_beneath_declared_inputs`, `entry_binding_keeps_its_declared_input_diagnostic` |
| unconsumed linear local errors at the block terminator; consumed is accepted | `unconsumed_linear_local_errors_at_block_end`, `linear_local_bound_and_consumed_in_arm_is_accepted` |
| stage units: bind term, leaked-local report, no new instr, no header phi | `parse_mid_body_binding_produces_bind_term`, `scope_leave_reports_the_unconsumed_linear_local`, `lower_binding_emits_no_new_instr`, `lower_mid_body_binding_adds_no_header_phi` |
| clause bodies bind; the variant-name collision and misspelling diagnostics | `mid_body_binding_in_clause_body_binds`, `clause_body_binding_named_for_a_variant_is_error`, `clause_body_binding_named_for_a_variant_of_the_same_enum_is_error`, `misspelt_variant_in_clause_list_notes_the_binding_read`, `misspelt_variant_swallowed_as_a_binding_reports_the_missing_variant` |
| REPL: binds, reaches earlier lines, session-depth floor, rollback, no name carryover | `repl_line_binds_a_local`, `repl_line_binding_reaches_earlier_line_values`, `repl_line_binding_more_than_the_session_stack_holds_is_error`, `failed_repl_line_after_binding_leaves_stack_intact`, `repl_line_locals_do_not_survive_to_next_line` |
| back edge and dogfood | `mid_body_binding_in_self_tail_recursive_word_loops_correctly`, `vm_with_mid_body_binding_matches_previous_output` |
| non-exhaustive/body-error diagnostic ordering | `check_clause_word_reports_non_exhaustive_before_body_type_error` |

## Residual risks

- **`Moves::join` indexes a `HashMap` directly** (`else_arm.states[name]`, src/check.rs:231)
  and is safe only by construction: `leave_block` always equalises the two arms' key sets
  (by removing exactly the names each arm bound past the join's `depth`) before the join
  runs. A future arm-like construct that binds a name without a matching `leave_block`
  would panic instead of mis-checking. Not changed: nothing in this slice or its tests
  exercises that condition, and CLAUDE.md's convention is not to add handling for a
  condition that cannot occur.
- **A binding's stack consumption has no unit-level cover.** Making `TermKind::Bind` read
  the operands without removing them leaves the whole lib suite green, because the stale
  values sit *below* the top and every later operation still sees the right operands. It
  surfaces only where stack depth itself is read: a self-tail-call's back edge takes its
  operands from a stack that is now too deep, which `vm_dispatch_loop_runs_in_constant_stack`
  catches. The behaviour is covered; the unit layer is not.
- **The no-extra-header-phi property is structurally guaranteed, not empirically at risk.**
  Header phi count is a function of declared input arity and back-edge count, and locals
  live in a list phi construction never reads, so no binding bug can perturb it. Its test
  compares against a binding-free lowering rather than hard-coding the numbers, so it
  cannot pass vacuously, but it should not be read as evidence that the property was
  precarious.

## Out of scope (unchanged)

Closures, a closing token, locals surviving across REPL lines, destructuring, in-place
mutable rebinding, type annotations in a binding. Everything about references is Slice 6,
including the three Slice 6 rules whose reasoning this slice invalidates (R6's "live for
the whole word body", R8's "a parameter is never itself left over", R10's vacuity argument)
and the aggregate-local aliasing hole in `FuncBuilder::lower_call`'s locals lookup, still
unobservable without in-place mutation.
