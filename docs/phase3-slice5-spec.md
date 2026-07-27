# Phase 3 Slice 5 — General locals (spec)

Design input: [the brief](./phase3-slice5-brief.md). Base: `main` @ `0267843`, 700 tests green.

Two changes, one theme. `| names |` binding becomes legal at any point in a body rather
than only at the top of one, and REPL lines gain locals, which they have none of today.
Both are prerequisites for Slice 6: a bare REPL line can form no place, so it can take no
borrow, so references would be unusable at REPL scope.

## Context: what is already true on the base commit

- Binding consumes. `stack.split_off(stack.len() - take)` at src/ir.rs:1449-1450; the
  clause path at src/ir.rs:2717-2719 is the identical shape. Leftmost name binds the
  deepest value.
- Locals are a **field**, not a term: `WordBody::Terms { locals, … }` at src/ast.rs:266,
  `Clause.locals` at src/ast.rs:278.
- The checker holds locals as a precomputed immutable borrow:
  `Ctx::Word { locals: &'a HashMap<String, Type>, … }` at src/check.rs:268-274.
- REPL lines carry no locals: `Ctx::Line { .. } => None` at src/check.rs:285 and :306.
- A word cannot see beneath its declared inputs: `: f ( i64 -- ) drop drop ;` gives
  "`drop` needs 1 values, but the stack holds 0".
- Nothing auto-drops. An unmoved linear local is already an error at word end, and
  divergent consumption across `if` arms is already an error with its own message.
- The REPL is already transactional: after a failing line, the session stack is unchanged.
- A `|` inside a clause body already disambiguates by lookahead: `at_clause_start` is a `|`
  followed by a **registered variant name**, checked against every enum in scope
  (`is_variant_name`, src/parser.rs:449-458).

## Requirements

**R1 — A binding is a term.** `| names |` is legal at any point in a body and pops that
many values off the stack at the point it appears. Leftmost name binds the deepest value,
identical to the existing entry and clause forms. At least one name is required: `| |` is a
parse error, not a no-op, so that a stray pipe pair cannot silently mean nothing.

**R2 — Extent is the rest of the enclosing block.** The blocks are a word body, a clause
body, and an `if` or `else` arm. A name bound inside an arm is not in scope after `end`.
No closing token is introduced, because the block's existing terminator already marks the
end. Rejected alternative: word-scoped extent with a definite-assignment merge, which
admits more programs at the cost of a "bound on both arms" analysis and worse errors.

**R3 — Entry and clause binding become instances of R1, but the entry diagnostic
survives.** The semantics unify. The message does not: "locals bind 2 value(s), but only 1
input(s) are declared" (src/check.rs:1060) is only formulable at entry, where the declared
effect is known, and it is better than the generic underflow of R5. Keep it for the entry
position and do not let the unification flatten it.

**R4 — Re-binding a name already in scope is a located error.** For a linear value the
rejection is forced regardless, since the earlier binding would become unreachable and
leak. Applying it uniformly, rather than only to linear values, keeps one rule and one
message. Two sibling arms each binding the same name is **not** re-binding: the first
name's extent ended at the first arm's terminator.

**R5 — Binding more values than the frame holds is a located error, and the frame floor is
context-dependent.** In a word the floor is the declared inputs, already enforced. At a
REPL line the floor is the current session stack depth, because consuming values left by
earlier lines is the REPL's model. Reuse the existing "needs N values, but the stack holds
M" shape rather than inventing a second underflow message.

**R6 — The linearity check gains a block-end firing site.** A linear value bound inside a
block and not consumed before that block's terminator is an error at the terminator, not at
word end. The message must name where the scope ended; a bare "never consumed" that points
at the word would be worse than the existing word-end message it replaces for this case.

**R7 — REPL lines gain locals, scoped to the line.** `Ctx::Line` carries a locals map. The
session stack persists across lines; names do not. Existing transactional behaviour is
preserved: a line that fails after binding leaves the session stack as it was.

**R8 — A clause-body binding may not lead with a registered variant name.** The parser
disambiguates a `|` in a clause body by looking at the next token: a registered variant
name opens the next clause, anything else opens a binding. Since `is_variant_name` scans
every enum in scope, not just the scrutinee's, the restriction is global. State it as a
located parse error rather than letting the binding be silently reparsed as a clause.

**R9 — The checker's locals map must evolve during the walk.** `Ctx::Word` currently holds
`&'a HashMap<String, Type>` computed before the body is visited. Mid-body binding makes the
map change as terms are walked, so the borrow cannot stand. This is the main structural
change in the slice and the main regression risk to the 700 existing tests.

**R10 — No new IR instruction.** A binding is a compile-time rebinding of existing SSA
values: pop from the lowering stack, insert into the locals map. Scope teardown truncates
the map to its length at block entry. Values are SSA and outlive the name.

**R11 — Nothing crosses a back edge.** In a self-tail-recursive body a binding's extent
ends at its block's terminator, which is where the tail call sits, so no mid-body name is
live across the back edge and no new header phi is required. Verify rather than assume.

## Test discipline

Goldens go in a new `tests/phase3_locals.rs`; unit tests sit beside the stage code they
cover (`src/parser.rs`, `src/check.rs`, `src/ir.rs`) per CLAUDE.md. Every diagnostic
requirement is tested for the *specific* message, not merely for failure. The 700 existing
tests are a delivery gate on every phase, not a criterion: R9's refactor is the risk and a
green suite is how it is discharged.

## Load-bearing invariants (must survive)

- Nothing auto-drops. R6 adds a place where forgetting is caught earlier, never a place
  where something is dropped for you.
- A word cannot see beneath its declared inputs (R5).
- No new `Instr` variant (R10).
- Prefer-the-stack remains the culture. This slice removes a language-enforced limit, not
  the convention; the ROADMAP note at Phase 2 Slice 4 already records that trade.

## Delivery phases

1. **Mid-body binding, block scope, and its rejections.** R1, R2, R4, R5, R6, R9, R10 over
   word bodies and `if` arms. R9's `Ctx` change lands here, so the existing suite is the
   gate. R3 preserves the entry diagnostic through the unification.
2. **Clause bodies and REPL lines.** R8's disambiguation and its parse error, then R7's
   line locals with the session-depth frame floor and preserved transactionality.
3. **Dogfood and docs.** R11's back-edge check, the `examples/vm.sth` rewrite, and the
   ROADMAP/DESIGN updates.

## Criterion → test map

| # | criterion | phase | test |
|---|---|---|---|
| 1 | a binding mid-body pops and names the top of stack | 1 | `mid_body_binding_consumes_from_the_stack` |
| 2 | leftmost name takes the deepest value, mid-body as at entry | 1 | `mid_body_binding_leftmost_name_takes_deepest_value` |
| 3 | a name bound in an `if` arm is not in scope after `end` | 1 | `local_bound_in_if_arm_is_not_visible_after_end` |
| 4 | sibling arms may each bind the same name (teardown works) | 1 | `name_bound_in_one_arm_can_be_rebound_in_sibling_arm` |
| 5 | re-binding a name still in scope is a located error | 1 | `rebinding_a_name_in_scope_is_error` |
| 6 | binding more values than the frame holds is a located error | 1 | `binding_more_values_than_frame_holds_is_error` |
| 7 | a word still cannot bind beneath its declared inputs | 1 | `binding_cannot_reach_beneath_declared_inputs` |
| 8 | the entry-position diagnostic is unchanged by the unification | 1 | `entry_binding_keeps_its_declared_input_diagnostic` |
| 9 | a linear value bound in an arm and unconsumed errors at the arm's end, naming the scope | 1 | `unconsumed_linear_local_errors_at_block_end` |
| 10 | a linear value bound in an arm and consumed there is accepted | 1 | `linear_local_bound_and_consumed_in_arm_is_accepted` |
| 11 | a binding that names nothing is a parse error, not a no-op | 1 | `empty_binding_with_no_names_is_error` |
| 12 | the parser produces a binding term at a mid-body position | 1 | `parse_mid_body_binding_produces_bind_term` (unit, src/parser.rs) |
| 13 | the checker's locals map is restored at block exit | 1 | `check_block_exit_restores_locals_map` (unit, src/check.rs) |
| 14 | lowering emits no new instruction for a binding | 1 | `lower_binding_emits_no_new_instr` (unit, src/ir.rs) |
| 15 | a mid-body binding inside a clause body binds | 2 | `mid_body_binding_in_clause_body_binds` |
| 16 | a clause-body binding leading with a variant name is a located parse error | 2 | `clause_body_binding_named_for_a_variant_is_error` |
| 17 | a REPL line binds and uses a local | 2 | `repl_line_binds_a_local` |
| 18 | a REPL line's binding reaches values left by earlier lines | 2 | `repl_line_binding_reaches_earlier_line_values` |
| 19 | a REPL line that fails after binding leaves the session stack intact | 2 | `failed_repl_line_after_binding_leaves_stack_intact` |
| 20 | a REPL line's locals do not survive into the next line | 2 | `repl_line_locals_do_not_survive_to_next_line` |
| 21 | a mid-body binding in a self-tail-recursive word loops correctly | 3 | `mid_body_binding_in_self_tail_recursive_word_loops_correctly` |
| 22 | no header phi is added for a mid-body name | 3 | `lower_mid_body_binding_adds_no_header_phi` (unit, src/ir.rs) |
| 23 | `examples/vm.sth` rewritten with a mid-body binding produces identical output | 3 | `vm_with_mid_body_binding_matches_previous_output` |

## Dogfood

The REPL session is the one that matters, because it is the thing that cannot be written
today at all and the thing Slice 6 depends on:

```
1 2 3
| a b |          \ binds b=3, a=2, reaching into a value left by the previous line
a b + .
```

Second, `examples/vm.sth`'s `build`, which Slice 6's drafting twice restructured
incorrectly for want of a binding point. Rewriting it with a mid-body binding removes the
`build-into` helper that exists only to provide one, and the golden asserts the program's
output is byte-identical to before.

## Explicitly out of scope

Closures and any closing token for a binding (R2 makes both unnecessary). Locals persisting
across REPL lines. Pattern-destructuring in a binding. Mutable rebinding of a name in
place. Type annotations in a binding. Anything about references, which is Slice 6, including
the three rules there whose stated reasoning this slice invalidates: R6's "live for the
whole word body", R8's "a parameter is never itself left over", and R10's vacuity argument.
Those are handed to Slice 6's rewrite, not patched here. The aggregate-local aliasing hole
(src/ir.rs:1709) likewise stays Slice 6's open question: this slice makes it easier to reach
and still cannot make it observable, since observing it needs in-place mutation.

## Phases

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "mid-body-binding-block-scope-and-rejections",
      "difficulty": "hard",
      "summary": "Make `| names |` a term legal at any point in a word body or `if` arm, popping from the stack at that point with leftmost binding deepest. Extent is the rest of the enclosing block, with the checker's locals map evolving during the walk instead of being a precomputed immutable borrow. Adds the rejections: re-binding a name in scope, binding more than the frame holds, an empty binding, and a linear value left unconsumed at its block's terminator. Preserves the entry-position diagnostic through the unification.",
      "changes": [
        "src/ast.rs: locals become representable as a term rather than only a field on WordBody::Terms and Clause",
        "src/parser.rs: parse `| names |` at any term position; reject an empty binding with a located error",
        "src/check.rs: Ctx::Word's locals stop being a precomputed `&'a HashMap<String, Type>` and become a map that evolves as terms are walked, with block entry/exit save and restore; reject re-binding a name in scope; reject binding more values than the frame holds, reusing the existing needs-N-holds-M shape; fire the linearity check at a block terminator, naming the scope that ended; keep the entry-position 'locals bind N value(s), but only M input(s) are declared' message",
        "src/ir.rs: lower a binding as a pop from the lowering stack plus an insert into the locals map, and truncate the map at block exit; no new Instr variant"
      ],
      "tests": [
        "mid_body_binding_consumes_from_the_stack",
        "mid_body_binding_leftmost_name_takes_deepest_value",
        "local_bound_in_if_arm_is_not_visible_after_end",
        "name_bound_in_one_arm_can_be_rebound_in_sibling_arm",
        "rebinding_a_name_in_scope_is_error",
        "binding_more_values_than_frame_holds_is_error",
        "binding_cannot_reach_beneath_declared_inputs",
        "entry_binding_keeps_its_declared_input_diagnostic",
        "unconsumed_linear_local_errors_at_block_end",
        "linear_local_bound_and_consumed_in_arm_is_accepted",
        "empty_binding_with_no_names_is_error",
        "parse_mid_body_binding_produces_bind_term",
        "check_block_exit_restores_locals_map",
        "lower_binding_emits_no_new_instr"
      ]
    },
    {
      "phase": 2,
      "focus": "clause-bodies-and-repl-line-locals",
      "difficulty": "standard",
      "summary": "Extend mid-body binding to clause bodies, where a `|` already disambiguates by lookahead against every registered variant name, and reject a binding that leads with a variant name rather than letting it be silently reparsed as the next clause. Give REPL lines locals scoped to the line, with the frame floor being the current session stack depth so a binding may consume values left by earlier lines, and preserve the existing transactional rollback on a failing line.",
      "changes": [
        "src/parser.rs: apply the existing at_clause_start lookahead at every `|` in a clause body, not only the first; a binding leading with a registered variant name is a located parse error",
        "src/check.rs: Ctx::Line gains a locals map, replacing the two `Ctx::Line { .. } => None` arms; the REPL frame floor is the current session stack depth rather than a declared input list",
        "src/repl.rs: thread line-scoped locals through a line's evaluation and discard them at end of line, leaving the existing session-stack rollback on failure intact"
      ],
      "tests": [
        "mid_body_binding_in_clause_body_binds",
        "clause_body_binding_named_for_a_variant_is_error",
        "repl_line_binds_a_local",
        "repl_line_binding_reaches_earlier_line_values",
        "failed_repl_line_after_binding_leaves_stack_intact",
        "repl_line_locals_do_not_survive_to_next_line"
      ]
    },
    {
      "phase": 3,
      "focus": "back-edge-check-dogfood-and-docs",
      "difficulty": "standard",
      "summary": "Confirm that a mid-body binding in a self-tail-recursive body needs no header phi, since its extent ends at the block terminator where the tail call sits. Rewrite examples/vm.sth's build to use a mid-body binding, removing the build-into helper that exists only to provide a binding point, and assert byte-identical output. Update ROADMAP.md and DESIGN.md.",
      "changes": [
        "examples/vm.sth: build uses a mid-body binding; the build-into helper that existed only as a binding site is removed",
        "ROADMAP.md: mark Phase 3 Slice 5 done and repoint Next action at Slice 6",
        "DESIGN.md: update the locals characterization so it no longer implies top-of-scope-only binding"
      ],
      "tests": [
        "mid_body_binding_in_self_tail_recursive_word_loops_correctly",
        "lower_mid_body_binding_adds_no_header_phi",
        "vm_with_mid_body_binding_matches_previous_output"
      ]
    }
  ]
}
```
