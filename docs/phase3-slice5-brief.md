# Phase 3 Slice 5 — General locals (brief)

`| names |` binding is confined to the top of a word body or a clause body. This slice
lifts that confinement in two directions: binding becomes legal at any point in a body,
and REPL lines gain locals, which they have none of today.

This reverses the mid-body half of Phase 2 Slice 4's locked decision ("no mid-body
binding, no closer: factor a word instead", ROADMAP.md:198-216). The evidence for the
reversal came from specifying Slice 6: six separate places wanted to name a value
computed partway through, and "factor a word instead" produced `run` and `build-into`,
words that exist purely as binding sites rather than as meaningful abstractions. The
discipline failed on its own terms. The **no-closing-token** half of that decision
stands unchanged, for the reason given below.

The slice is also a prerequisite rather than a convenience. A bare REPL line has no
locals, so it can form no place, so it can take no borrow: without this slice, Slice 6's
references are unusable at REPL scope and array `get`/`set` can never be retired.

## Recon: what already works today (measured, not assumed)

- **Binding consumes.** `stack.split_off(stack.len() - take)` at src/ir.rs:1449-1450.
  Confirmed at the surface: `: f ( i64 -- ) | a | ;` compiles clean, while
  `: f ( i64 -- i64 ) | a | ;` fails with "body leaves 0 values, but ( … ) declares 1
  outputs". The value is off the stack and reachable only through the name.
- **Leftmost binds deepest**, from zipping the names against the split-off tail. `| a b |`
  binds `b` to the top of stack.
- **Clause-body locals bind through identical code**: src/ir.rs:2717-2719 is the same
  `take`/`split_off`/`zip` shape as the word-entry path. They are already a stack pop, so
  they collapse into a general rule rather than surviving as a special case.
- **No per-block local scope exists in any form.** Locals go into one flat map for the
  whole word body. This is the only genuinely new machinery the slice needs.
- **Divergent move state across `if` arms is already handled**, with a located error:
  "linear value `b` is not consumed on every path … it is consumed on one `if` arm but not
  the other". Block-scoped bindings do not need this built; it exists.
- **Nothing auto-drops, locals included.** An unmoved linear local is an error today:
  "linear value `b` is never consumed in `f` … drop it or return it (nothing is dropped
  for you)".
- **A word cannot reach beneath its declared inputs.** `: f ( i64 -- ) drop drop ;` gives
  "`drop` needs 1 values, but the stack holds 0"; the caller's deeper stack is invisible to
  the checker. The frame floor already exists and is already enforced.
- **Locals are a field, not a term**: `WordBody::Terms { locals, … }` at src/ast.rs:266 and
  `Clause.locals` at src/ast.rs:278. Mid-body binding needs them representable as a term.
- **Both target forms are parse errors today.** Mid-body `| b |` and a REPL line's `| a |`
  each give "parse error: unexpected token Pipe". REPL lines carry no locals at all:
  `Ctx::Line { .. } => None` at src/check.rs:285 and :306.

## Decided (locked, one at a time)

- **D1. Binding is legal at any point in a body, and pops from the stack at that point.**
  Leftmost still binds deepest. This is the same operation the entry and clause forms
  already perform, at an arbitrary position.
- **D2. Extent is the rest of the enclosing block**: a word body, a clause body, or an `if`
  arm. This is why no closing token is needed, and why Phase 2 Slice 4's no-closer half
  survives intact. The rejected alternative is word-scoped extent with a definite-assignment
  merge, which is more permissive, needs a "bound on both arms" analysis, and produces worse
  errors for the case it admits.
- **D3. Entry binding keeps its specialized diagnostic.** The semantics unify with D1, but
  "locals bind 2 value(s), but only 1 input(s) are declared" (src/check.rs:1060) is a better
  message than a generic underflow and is only available at entry, where the declared effect
  is known. Unifying the semantics must not flatten the diagnostic.
- **D4. Clause-body locals become an instance of the general rule**, not a parallel feature.
  Their lowering is already identical.
- **D5. Re-binding a name that is in scope is a located error.** For a linear value the
  rejection is forced anyway, since the earlier binding would become unreachable and leak.
  Rejecting uniformly, rather than only for linear values, keeps one rule.
- **D6. Binding more values than the frame holds is a located error, and the frame floor
  differs by context.** In a word the floor is the declared inputs, which the checker already
  enforces. At a REPL line the floor is the current session stack depth, because operating on
  values left by earlier lines is the REPL's whole model. The error reuses the existing
  "needs N values, but the stack holds M" shape rather than introducing a new one.
- **D7. REPL lines gain locals, scoped to the line.** The session stack persists across
  lines; names do not.
- **D8. The linearity check gains a block-end firing site.** Today an unconsumed linear local
  is caught at word end. A linear value bound inside an `if` arm and not consumed there must
  be caught at the arm's end.

## Open questions the spec must answer

- What exactly does the block-end linearity error say, and does it name the block? The
  existing word-end and every-path messages are both good; a third that says "never consumed"
  without saying where the scope ended would be worse than either.
- Is a scope teardown needed in IR lowering, or does truncating the locals map to its
  entry length suffice? Values themselves are SSA and outlive the name.
- Does a mid-body binding inside a self-tail-recursive body interact with the loop header's
  phis? The extent ends at the block end, which is where the tail call sits, so the name
  should die before the back edge and need no phi. Confirm rather than assume.
- Is `| |` with no names legal? Reject, no-op, or parse error.
- If a REPL line fails after binding, is the session stack restored to its pre-line state?
  This is existing REPL transactionality, but D7 gives it a new way to be observed.
- **Hand back to Slice 6, do not solve here:** reference-typed locals with block extent
  invalidate the reasoning in three of that slice's rules. R6 defines a reference-typed
  local as live for the whole word body; R8 exempts one from the surplus check because "a
  parameter is never itself left over"; and R10's round-3 justification rests on a
  reference-typed local being a parameter, "identical on both arms by construction". Once an
  arm can bind a reference, none of those three hold as stated.
- **Do not fix here:** naming an aggregate local reuses the same value id (src/ir.rs:1709),
  so two locals can denote one region. General locals makes that hole easier to reach but
  not observable, since observing it needs the in-place mutation Slice 6 introduces. It stays
  Slice 6's open question.

## Dogfood

A REPL session that binds a local at line scope and uses it, which cannot be written today
at all. This is the exit condition that matters, because it is what unblocks references and
the `get`/`set` retirement at REPL scope.

Second, one existing word rewritten to use a mid-body binding in place of a binding-site
helper, demonstrating the workaround the restriction was forcing. `examples/vm.sth`'s
`build`, which Slice 6's drafting twice restructured incorrectly for want of a binding
point, is the honest candidate.
