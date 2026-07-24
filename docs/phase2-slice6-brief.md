# Phase 2 Slice 6 — Self-tail-call → loop lowering

Guaranteed constant-stack self-tail-recursion. A word whose body (or any clause
body) ends in a tail call to *itself* compiles to a back-edge jump to a phi'd loop
header instead of a `Call`, so self-tail-recursion runs in constant stack and cannot
overflow. No new surface syntax: existing recursive words simply stop growing the
stack in tail position.

This is a compiler slice, not a language-feature slice: nothing changes in the lexer,
parser, or AST. It precedes the bytecode-VM dogfood (now Slice 7) because that VM's
dispatch loop is self-recursive and would otherwise overflow, and pulling quotations
(Phase 4) forward to get a loop is the larger change.

## Why this is a real slice

- **Unblocks the VM dogfood.** A bytecode interpreter is `while running: dispatch(prog[pc])`.
  In Sooth that is a self-recursive `run` word. Without tail-call elimination the host
  stack grows one frame per executed instruction, so any looping bytecode overflows.
- **Makes recursion honest.** Sooth exists to turn silent failure into sharp failure.
  "This tail call is eliminated, rely on it" (self) vs "this one is not, and we tell you
  so at compile time" (mutual) is exactly that ethos applied to the stack.
- **No new surface area.** It is a lowering transform over IR the compiler already
  emits (`Phi`, back-edge `Jmp`), so it shrinks nothing and adds no keyword. Existing
  tail-recursive words just stop overflowing.

## Locked decisions

- **D1 — Scope: self-tail-call → loop only, and it is a guarantee.** A word whose body
  or clause body ends in a tail call to itself is compiled to a back-edge jump.
  Mandatory whenever it applies (no pragma, no opt-in); code may rely on constant-stack
  behaviour, Scheme-style. Tail calls to *other* words are ordinary calls.
- **D2 — Tail position.** A call is in tail position iff it is the final term of a word
  body or a clause body, its outputs are exactly the enclosing word's declared outputs,
  and nothing follows it before the exit. Tail position **propagates through the arms of
  a terminal `if/else/end`** (an `if` that is itself in tail position hands tail position
  to both arms). A call followed by any stack shuffle, consumer, arithmetic, or further
  call (e.g. `n rec *`) is **not** in tail position.
- **D3 — Mutual tail recursion is a located compile error, this iteration.** A
  tail-call cycle across two or more words (A tail-calls B, B tail-calls A) is detected
  and rejected with a located error naming the cycle, rather than silently overflowing
  at runtime or silently compiling as un-eliminated calls. Tier 2 (SCC contraction into
  one tagged loop, explicitly **not** a trampoline and **not** QBE backend tail calls)
  is a planned follow-on, out of scope here (see DESIGN.md, Open / deferred).
- **D4 — The loop shape.** Introduce a loop-header block. The entry block binds the
  function params and jumps to the header; the header carries a `Phi` for each
  loop-carried value (the entry arm supplies the initial param, each back-edge supplies
  the recursive argument). The body lowers from the header. A tail self-call marshals its
  argument values as the header phis' back-edge inputs and emits `Jmp(header)` instead of
  `Instr::Call`. Base cases still `Ret`.
- **D5 — Reuse existing IR, no new instruction.** Blocks, `Phi(Value, Vec<(BlockId,
  Value)>)`, back-edge-capable `Jmp(BlockId)`, and `seal_block`/`fresh_block`/
  `start_block` are all already emitted for `if`/clause dispatch (`lower_if` emits a phi
  at the join). The transform composes them; it adds no IR instruction or terminator.
  QBE renders loops natively (SSA + phi + back-edge jump).
- **D6 — Locals under the loop.** `| ... |` locals bind at word/clause entry from the
  entry stack; under the loop they bind to the header phi outputs, so each iteration
  rebinds them to that iteration's carried values.
- **D7 — Clause bodies.** A word defined by `|`-clauses has one tail per clause; each
  clause's terminal self-call is a back-edge into the single shared header (the header
  phi gains one incoming arm per back-edge, plus the entry arm). Dispatch stays inside
  the loop body.
- **D8 — Behavioural guarantee.** A self-tail-recursive word over a large N (one that
  would overflow the host stack under naive recursion) runs to completion in constant
  stack. This is tested by a native golden, not just by IR shape.
- **D9 — Drop-at-back-edge (co-design note, vacuous in Phase 2).** The back-edge is the
  defined point at which the outgoing iteration's non-forwarded affine values would have
  their destructors run before the jump. In Phase 2 every type is `Copy`, so the drop set
  is empty and the back-edge emits no drop glue; the point is stated so Phase 3's
  destructors have a home rather than a retrofit.

## Mechanics

- **M1 — Detection.** Tail-position self-call detection is an AST analysis over the word
  body that recurses into terminal `if` arms and clause bodies (D2). The transform needs
  the current word's own name/symbol threaded into the lowering builder, which it does
  not have today.
- **M2 — Mutual detection.** Build a whole-module tail-call graph (edges = tail-position
  calls to user words). A self-loop is tier 1 (handled). Any cycle of length ≥ 2 among
  tail-call edges is the mutual error (D3), reported with the cycle named. This is a
  module-level analysis and belongs in the checker. Non-tail mutual calls are fine (they
  return normally) and must not false-positive.
- **M3 — SSA back-edge / incomplete phi (main risk).** The header phi references values
  produced later, on the back-edge, so the builder must back-patch: lower the body first,
  collect each back-edge's `(block, value-per-carried-slot)`, then finalize the header
  phis. The current builder seals blocks in order and has no deferred-operand path.
- **M4 — Params → header.** Function params are plain value ids, not phis. The transform
  introduces the header whose phis take the params on the entry arm and the recursive
  args on each back-edge; the body reads phi outputs, not the raw params.
- **M5 — Everything else unchanged.** Only the exact self-tail-call site changes. Normal
  calls, non-tail self-calls, struct/enum/array generated words, conversions, and
  builtins lower exactly as today.
- **M6 — REPL parity.** `lower_word` is shared with the REPL, so a self-tail-recursive
  word defined at the REPL gets the same transform (loaded via `dlopen`, runs in constant
  stack). The only REPL-specific need is that the current word's name reaches the same
  lowering path.

## Work by stage

- **Checker (`src/check.rs`):** a tail-position analysis helper; a whole-module
  tail-call graph plus mutual-cycle detection producing a located error (M2). Optionally
  expose, per word, whether it is self-tail-recursive and the set of tail self-call sites,
  or let the lowerer re-derive tail position.
- **IR (`src/ir.rs`):** thread the current word name into `FuncBuilder`; build the
  header-with-phi in `lower_word`; lower a self-tail-call as a back-edge in `lower_call`
  (and inside terminal `if` arms via `lower_if`, and inside clauses via `lower_clauses`);
  back-patch header phis (M3).
- **Backend (`src/backend/qbe.rs`):** expected no change (phi + jmp already emitted);
  verify the loop IL is valid QBE (a phi with a back-edge predecessor) and add a
  structural test.
- **Lexer / parser / AST:** no change (no new syntax); at most a small analysis helper.

## Success criteria (each a runnable golden unless noted)

1. A self-tail-recursive word over a large N (e.g. sum/countdown to 1_000_000) runs to
   completion natively and prints the right result. Would overflow without the transform.
2. IR-level (unit): a self-tail-call lowers to a `Jmp` back to the header with **no**
   `Instr::Call` to self, and the header carries a `Phi` per loop-carried value.
3. A self-tail-recursive word defined with `|`-clauses (multiple tails) runs in constant
   stack; each clause's tail self-call is a back-edge into one header.
4. Tail position through a terminal `if/else/end` (recursive case in one arm, base case
   in the other) runs in constant stack over large N.
5. A **non-tail** self-call (self-call followed by more work, e.g. classic non-tail
   factorial) still lowers to a real `Call` and still computes correctly at small N
   (correctness preserved; deliberately not eliminated).
6. Mutual tail recursion (A tail-calls B, B tail-calls A) is a located compile error
   naming the cycle. Negative golden.
7. A self-tail-recursive word using `| ... |` locals produces correct results across
   iterations (locals rebind each iteration).
8. REPL: a self-tail-recursive word defined and run at the REPL completes in constant
   stack over large N.

## Dogfood

`examples/countdown.sth` (or similar): a tail-recursive accumulator, e.g.
`sum-to ( acc n -- acc' )` that adds `n` to `acc` and tail-calls itself with `n-1`
until `n` hits zero, invoked with a large `n` and printing the total. It overflows
under naive recursion and completes under the transform, so the example itself
demonstrates the guarantee.

## Out of scope (deferred, some already planned)

- **Mutual TCO / SCC contraction (tier 2):** planned follow-on, not this iteration.
- **Trampolines:** need first-class function values / quotations (Phase 4).
- **QBE backend tail calls:** QBE has none; adding them forks the backend we chose not
  to fork.
- **Quotations, combinators (`each`/`while`/`fold`/`map`/`times`), general loop syntax:**
  Phase 4.
- **Drop glue at the back-edge:** vacuous in all-`Copy` Phase 2; co-designed with
  Phase 3 destructors.
- **Tail calls across extern/module boundaries; optimising non-tail recursion.**

## Key risks

1. **SSA back-edge phi (M3):** the header phi's back-edge operand is defined later;
   the builder must back-patch. Mitigation: lower the body, collect back-edge inputs,
   finalize header phis in a second step. This is the crux of the slice.
2. **Params-as-loop-carried (M4):** getting the header's predecessor set right (one
   entry arm + one arm per back-edge) and reading phi outputs in the body, not raw params.
3. **Tail-position detection correctness (D2):** over-eager misclassification (a call
   followed by shuffles treated as tail) miscompiles; over-conservative misses
   elimination and the VM overflows. Test both boundaries, including the terminal-`if`
   propagation and the "outputs equal word outputs" condition.
4. **Mutual-cycle detection (M2):** identify tail-call cycles without false-positiving on
   non-tail mutual calls (those return normally and are fine).
5. **Coexistence with clause/`if` join phis (Slice 4):** the loop header phi and the
   dispatch join phis share the block/phi bookkeeping; ensure predecessors don't collide.
6. **REPL parity (M6):** the current-word name must reach the REPL lowering path.

## Current-state anchors (verified on `main`; re-verify before editing)

IR (`src/ir.rs`): `BlockId` @542; `Block` @546; `Instr::Call` variant @560, emitted for a
user call @1171 (`sym = (self.resolve)(name)`, default arm of `lower_call`);
`Instr::Phi(Value, Vec<(BlockId, Value)>)` @566; `Terminator::Jnz` @637 / `Jmp(BlockId)`
@638; `lower_word` @848 (entry: `WordBody::Terms { locals, terms }` → `lower_terms`;
`WordBody::Clauses` → `lower_clauses`; then `seal_block(Ret(result))`); `FuncBuilder`
@~900 (fields `env` / `resolve` / `structs` / `enums` / `arrays` — **no current-word
name today**); `fresh_block` @955 / `seal_block` @965 / `start_block` @976; `lower_terms`
@980; `lower_term` @986 (dispatches `TermKind::Call` → `lower_call`, `TermKind::If` →
`lower_if`); `lower_call` @1012 (locals fast-path, builtins, then the user-call default
arm); `lower_if` @1515 (emits `Jnz` + a `Phi` at the join — the pattern to mirror for the
header); `lower_clauses` @1561. Checker (`src/check.rs`): module-level word checking is
where the tail-call graph + mutual-cycle error belongs; `WordDef` / `effect` are in scope.
No lexer / parser / AST change.
