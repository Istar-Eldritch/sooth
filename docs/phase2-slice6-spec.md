# Phase 2 Slice 6 — self-tail-call → loop lowering

Slice 6 of Phase 2 (typed core), on the scalar core + Slice 3 structs + Slice 4 enums

+ Slice 5 arrays. A **compiler** slice, not a language-feature slice: nothing changes in
the lexer, parser, or AST. It adds (a) a checker analysis that classifies tail-position
calls and rejects mutual tail-call cycles, and (b) an IR lowering transform that turns a
self-tail-call into a back-edge `Jmp` to a phi'd loop header instead of an `Instr::Call`,
so self-tail-recursion runs in constant stack and cannot overflow. Reuses existing IR
(`Phi`, back-edge `Jmp`, blocks); adds no instruction, terminator, or surface syntax.
Design locked in `phase2-slice6-brief.md` (D1–D9, M1–M6).

## What ships

+ **Self-tail-call → loop, as a guarantee.** A word whose body (or any clause body) ends
  in a tail call to *itself* compiles to a back-edge jump to a phi'd loop header. Constant
  stack, mandatory whenever it applies, no pragma, Scheme-style; code may rely on it (D1).
+ **Tail-position analysis** (D2): a call is in tail position iff it is the final term of
  a word/clause body, its outputs are exactly the enclosing word's declared outputs, and
  nothing follows it before the exit. Tail position **propagates through both arms of a
  terminal `if/else/end`**. A call followed by any shuffle/consumer/arithmetic/further
  call (`n rec *`) is **not** tail.
+ **Mutual tail recursion is a located compile error** (D3): a tail-call cycle of length
  ≥ 2 (A→B→A) is detected on a whole-module tail-call graph and rejected with the cycle
  named, rather than silently overflowing or silently compiling as un-eliminated calls.
  Non-tail mutual calls are fine and must not false-positive.
+ **The loop shape** (D4): an entry block binds the params and `Jmp`s to a header block;
  the header carries one `Phi` per loop-carried value (entry arm = initial param, each
  back-edge arm = the recursive argument); the body lowers from the header; base cases
  still `Ret`.
+ **Reuse existing IR** (D5): `Block`, `Phi(Value, Vec<(BlockId, Value)>)`, back-edge
  `Jmp(BlockId)`, `fresh_block`/`seal_block`/`start_block`. QBE renders loops natively
  (SSA + phi + back-edge jump); backend needs no change beyond a verification test.
+ **Locals + clauses under the loop** (D6, D7): `| … |` locals bind to the header phi
  outputs, so each iteration rebinds them; a `|`-clause word has one shared header with
  one back-edge arm per clause tail; dispatch stays inside the loop body.
+ **REPL parity** (M6): `lower_word` is shared with the REPL, so a self-tail-recursive
  word defined at the REPL gets the same transform and runs in constant stack.
+ **Dogfood** `examples/countdown.sth`: a tail-recursive accumulator over a large `n`
  that overflows under naive recursion and completes under the transform.

**Out of scope:** mutual TCO / SCC contraction (tier 2); trampolines; QBE backend tail
calls; quotations / combinators / general loop syntax; drop glue at the back-edge
(vacuous, all-`Copy`); tail calls across extern/module boundaries; optimising non-tail
recursion.

## Why this is a real slice

+ **Unblocks the VM dogfood (Slice 7).** A bytecode interpreter is a self-recursive
  `run` word; without tail-call elimination the host stack grows one frame per executed
  instruction and any looping bytecode overflows.
+ **Makes recursion honest.** "This tail call is eliminated, rely on it" (self) vs "this
  one is not, and we tell you so at compile time" (mutual) is Sooth's silent-failure →
  sharp-failure ethos applied to the stack.
+ **No new surface area.** A lowering transform over IR the compiler already emits; adds
  no keyword and shrinks nothing. Existing tail-recursive words just stop overflowing.

## Locked decisions (not reopened)

+ **D1** self-tail-call → loop only, and a guarantee (mandatory, no opt-in); tail calls
  to *other* words are ordinary calls.
+ **D2** tail position = final term, outputs equal declared outputs, nothing follows;
  propagates through terminal `if/else/end` arms; a trailing consumer breaks it.
+ **D3** mutual tail recursion is a located compile error this iteration; tier 2 (SCC
  contraction, not a trampoline, not QBE backend tail calls) is deferred.
+ **D4** loop shape = entry block binds params + `Jmp` header; header phi per carried
  value (entry arm + back-edge arms); body from header; tail self-call marshals args as
  back-edge phi inputs + `Jmp(header)`; base cases `Ret`.
+ **D5** reuse blocks / `Phi` / back-edge `Jmp` / `fresh_block`/`seal_block`/`start_block`;
  no new IR instruction or terminator; QBE renders it natively.
+ **D6** `| … |` locals bind to header phi outputs; each iteration rebinds them.
+ **D7** one tail per clause; each clause's terminal self-call is a back-edge into the
  single shared header; dispatch stays inside the loop body.
+ **D8** the guarantee is behavioural: a self-tail-recursive word over a large N runs to
  completion in constant stack; tested by a native golden, not just IR shape.
+ **D9** the back-edge is the defined destructor insertion point; in Phase 2 every type
  is `Copy`, so the drop set is empty and the back-edge emits no drop glue (stated so
  Phase 3 has a home, not a retrofit).

## Requirements by stage

Numbered `Rn`, traceable to brief `Dn`/`Mn`. Diagnostics numbered `Xn` (one negative
behavioural test each, asserting the message text **and** the named identifiers).

### Checker (`src/check.rs`)

+ **R1 — Tail-position analysis helper** (D2, M1). A pure helper over a `WordBody` and
  the word's declared output arity that yields, for a body, its tail-position call sites.
  Rules: the final term of a `Terms` body is a candidate; a candidate `TermKind::Call`
  is in tail position iff its outputs are exactly the word's declared outputs and nothing
  follows; a terminal `TermKind::If` **hands tail position to the final term of both
  arms** (recurse into `then_branch` and `else_branch`); a `Clauses` body applies the
  rule to each clause's `body` (D7). A call followed by any further term is not tail. The
  helper is placed so both the checker (R3) and the lowerer (R7) can call it (shared,
  single source of truth for the classification), so the lowerer re-derives tail position
  rather than the checker threading a site list across stages.
+ **R2 — Self-tail-recursion predicate** (M1). Using R1, expose per word whether it
  contains ≥ 1 tail-position self-call (name equals the word's own name). The lowerer
  uses this to decide whether to build the loop shape (R6) at all.
+ **R3 — Whole-module tail-call graph** (M2). After signature registration and before /
  alongside body checking in `check` (`src/check.rs:154`), build a directed graph over
  user words: an edge `A → B` exists iff `A` has a tail-position call (R1) to user word
  `B`. Builtins, generated struct/enum/array words, conversions, and *non-tail* calls
  produce no edge.
+ **R4 — Mutual-cycle detection** (D3, M2). On the R3 graph, a self-loop (`A → A`, length
  1) is tier 1 and allowed. Any cycle of length ≥ 2 is the mutual-tail-recursion error
  X1, reported located and naming the cycle members in order. Non-tail mutual calls
  produce no edge, so a pair of words that mutually call each other in non-tail position
  must not be flagged.

### IR (`src/ir.rs`)

+ **R5 — Thread the current word name into `FuncBuilder`** (M1, M6). Add a field to
  `FuncBuilder` (`src/ir.rs:897`) carrying the word being lowered (its name/symbol), set
  from `word.name` in `lower_word` (`src/ir.rs:849`). The REPL path (`repl.rs:407`) also
  calls `lower_word(&word, …)`, so the name reaches the REPL lowering with no
  REPL-specific plumbing (M6).
+ **R6 — Header-with-phi setup** (D4, M4, risk 2). In `lower_word`, when the word
  contains a tail self-call (R2), build the loop shape: the entry block binds the param
  values, then `seal_block(Jmp(header))`; `start_block(header)`; the header carries one
  `Phi` per loop-carried value seeded with the entry arm `(entry_block, param)`; the body
  lowers from the header reading the **phi output values**, not the raw params.
  `| … |` locals and clause payloads bind from the header phi outputs (D6, D7). When the
  word has no tail self-call, lower exactly as today (no header, no phi) (R10, M5).
+ **R7 — Lower a tail self-call as a back-edge** (D4). Reached in the user-call default
  arm of `lower_call` (`src/ir.rs:1170`–`1171`), and inside terminal `if` arms via
  `lower_if` (`src/ir.rs:1515`) and inside clause bodies via `lower_clauses`
  (`src/ir.rs:1561`): when the called name equals the current word name (R5) **and** the
  site is in tail position (R1), pop the argument values, record them as a back-edge
  `(cur_block, [value per carried slot])`, and `seal_block(Jmp(header))` instead of
  emitting `Instr::Call`. Base cases fall through to the existing `Ret` in `lower_word`.
+ **R8 — SSA back-edge / incomplete-phi back-patching** (M3, risk 1; **the crux**). The
  header phi's back-edge operands are produced later (on the back-edges), so the builder
  cannot finalize them when it emits the header. Add a deferred-operand path to
  `FuncBuilder`: lower the body first, collecting each back-edge's per-carried-slot
  `(pred_block, value)`; then in a second step append those incoming arms to the header
  phis. The current builder seals blocks in order and has no such deferred path (R5's
  field plus a back-edge accumulator make it possible without a new IR node).
+ **R9 — Clause tails into one shared header** (D7, risk 5). A `|`-clause word gets a
  single header (R6); each clause's terminal self-call is one back-edge arm into that
  header. The header phis gain one incoming arm per back-edge plus the entry arm. The
  clause dispatch join phis (Slice 4, `lower_clauses`) and the loop header phi must not
  collide: predecessor sets stay disjoint (dispatch join predecessors are the
  non-tail clause ends; the header's predecessors are the entry block + the tail clause
  ends).
+ **R10 — Non-tail self-calls and everything else unchanged** (M5, risk 3). A self-call
  **not** in tail position (followed by more work, e.g. `n rec *`) still lowers to a real
  `Instr::Call`. Normal calls to other words, generated struct/enum/array words,
  conversions, and builtins lower exactly as today; only the exact tail self-call site
  changes.
+ **R11 — Drop-at-back-edge point defined, vacuous** (D9). The back-edge (R7) is the
  defined destructor insertion point for the outgoing iteration's non-forwarded affine
  values. In Phase 2 every type is `Copy`, so the drop set is empty and no drop glue is
  emitted; recorded as a code comment at the back-edge site so Phase 3 has a home.

### Backend (`src/backend/qbe.rs`)

+ **R12 — Verify, do not change** (D5). QBE already emits `Phi` and back-edge `Jmp`, so
  no codegen change is expected. Add one structural verification test that a loop IL (a
  header `Phi` with a back-edge predecessor + a back-edge `Jmp`) renders to valid QBE.

### Lexer / parser / AST

+ **No change.** No new syntax. At most the R1 analysis helper reads existing AST nodes.

### Diagnostics

+ **X1 — mutual tail recursion.** A tail-call cycle of length ≥ 2 (A tail-calls B, B
  tail-calls A) is a located compile error naming the cycle members (e.g.
  `` error: mutual tail recursion `a` -> `b` -> `a` `` at the offending word's span).
  Negative behavioural test asserts the message and the named words. Non-tail mutual
  calls between the same words must **not** trigger X1 (a second test).

## Success criteria (each a runnable golden unless noted)

Every criterion is a native binary or REPL golden, not an IL-string assertion, except
criterion 2, the single sanctioned structural unit test.

1. **Large-N self-tail-recursion runs in constant stack** (D8, R6–R8) → native golden:
   `examples/countdown.sth` (sum/countdown to 1_000_000) runs to completion and prints
   the right total. Overflows without the transform.
2. **IR shape** (R7, R8; unit) → `#[cfg(test)] mod tests` in `src/ir.rs`: a tail
   self-call lowers to a `Jmp` back to the header with **no** `Instr::Call` to self, and
   the header carries a `Phi` per loop-carried value.
3. **Clause-style multiple tails** (D7, R9) → native golden: a `|`-clause self-tail-
   recursive word runs in constant stack over large N; each clause tail is a back-edge
   into one header.
4. **Tail position through a terminal `if/else/end`** (D2, R1) → native golden: recursive
   case in one arm, base case in the other, constant stack over large N.
5. **Non-tail self-call preserved** (M5, R10) → native golden: a classic non-tail
   factorial (self-call followed by `*`) still computes correctly at small N (still a
   real `Call`, deliberately not eliminated).
6. **Mutual tail recursion rejected** (D3, X1) → negative golden: A tail-calls B, B
   tail-calls A, located error naming the cycle. Plus a positive test that non-tail
   mutual recursion is accepted (R4 no-false-positive).
7. **Locals rebind per iteration** (D6, R6) → native golden: a self-tail-recursive word
   using `| … |` locals produces correct results across iterations.
8. **REPL parity** (M6, R5) → scripted REPL golden: a self-tail-recursive word defined
   and run at the REPL completes in constant stack over large N.

## Dogfood — `examples/countdown.sth`

A tail-recursive accumulator, e.g. `sum-to ( acc n -- acc' )` that adds `n` to `acc` and
tail-calls itself with `n-1` until `n` hits zero, invoked with a large `n` and printing
the total. It overflows under naive recursion and completes under the transform, so the
example itself demonstrates the guarantee. Native and REPL.

## Non-functional

Green each phase (`cargo fmt --check && cargo clippy -- -D warnings && cargo test`).
Every stage function touched (checker analysis, lowering) gets `#[cfg(test)] mod tests`
beside it: the happy path plus at least one error/edge case (naming
`thing_condition_expected`). Diagnostics tested as behaviour (right message + named
identifiers). Invariants held: **QBE-only backend, no LLVM, no native backend started**;
**backend-neutral IR** (no new instruction/terminator; `Ptr[T]` stays opaque); the
**affine spine** unchanged (all types still `Copy`, drop set empty at the back-edge);
`core` stays `no_std`; **no in-process JIT / comptime interpreter** (REPL still loads via
`dlopen`). Verify by native + REPL goldens, not IL assertions (the one sanctioned
structural unit test is criterion 2).

## Out of scope (deferred, some already planned)

+ **Mutual TCO / SCC contraction (tier 2):** planned follow-on (see DESIGN.md,
  Open / deferred), not this iteration.
+ **Trampolines:** need first-class function values / quotations (Phase 4).
+ **QBE backend tail calls:** QBE has none; adding them forks the backend we chose not
  to fork.
+ **Quotations, combinators (`each`/`while`/`fold`/`map`/`times`), general loop syntax:**
  Phase 4.
+ **Drop glue at the back-edge:** vacuous in all-`Copy` Phase 2; co-designed with
  Phase 3 destructors (D9).
+ **Tail calls across extern/module boundaries; optimising non-tail recursion.**

## Key risks

1. **SSA back-edge phi (R8, M3):** the header phi's back-edge operand is defined later;
   the builder must back-patch. Mitigation: lower the body, collect back-edge inputs,
   finalize header phis in a second step. The crux of the slice.
2. **Params-as-loop-carried (R6, M4):** get the header's predecessor set right (one
   entry arm + one arm per back-edge) and read phi outputs in the body, not raw params.
3. **Tail-position detection correctness (R1, D2):** over-eager (a call followed by
   shuffles treated as tail) miscompiles; over-conservative misses elimination and the VM
   overflows. Test both boundaries, including terminal-`if` propagation and the
   "outputs equal word outputs" condition.
4. **Mutual-cycle detection (R3, R4, M2):** identify tail-call cycles without
   false-positiving on non-tail mutual calls (those return normally and are fine).
5. **Coexistence with clause/`if` join phis (Slice 4, R9):** the loop header phi and the
   dispatch join phis share block/phi bookkeeping; ensure predecessors don't collide.
6. **REPL parity (R5, M6):** the current-word name must reach the REPL lowering path.

## Current-state anchors (verified on `main`; re-verify before editing)

IR (`src/ir.rs`): `BlockId` @542; `Block` @545; `Instr::Call` variant @560, emitted in
the user-call default arm @1170–1171 (`let sym = (self.resolve)(name);` /
`Instr::Call(ret, sym, args)`); `Instr::Phi(Value, Vec<(BlockId, Value)>)` @566;
`Terminator::Ret` @636 / `Jnz` @637 / `Jmp(BlockId)` @638; `lower_word` @849 (entry:
`WordBody::Terms { locals, terms }` → `lower_terms`; `WordBody::Clauses` →
`lower_clauses`; then `seal_block(Ret(result))`); `FuncBuilder` struct @897 (fields
`env` / `resolve` / `structs` / `enums` / `arrays` / `blocks` / `cur_id` /
`cur_instrs` / `stack` / `locals` / `value_types` / `const_vals` — **no current-word
name today**); `fresh_block` @955 / `seal_block` @966 / `start_block` @976; `lower_terms`
@980; `lower_term` @986 (dispatches `TermKind::Call` → `lower_call`, `TermKind::If` →
`lower_if`); `lower_call` @1012; `lower_if` @1515 (emits `Jnz` + a `Phi` at the join —
the pattern to mirror for the header); `lower_clauses` @1561 (N-predecessor join phis).
Checker (`src/check.rs`): `check` @154 (registers signatures, then iterates
`words.iter()` calling `check_word` @582 — where the tail-call graph + mutual-cycle
error belong); `check_word` dispatches `WordBody::Terms` → `check_terms_word` @603 and
`WordBody::Clauses` → `check_clause_word` @654. AST (`src/ast.rs`): `WordDef` @198
(`name` / `effect` / `body`); `WordBody` @209 (`Terms { locals, terms }` | `Clauses`);
`Clause` @222 (`variant` / `locals` / `body` / `span`); `Term` @412 / `TermKind` @415
(`Call(String)`, `If { then_branch, else_branch }`). REPL: `lower_word` shared call at
`src/repl.rs:407`. No lexer / parser / AST change.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "checker-tail-position-analysis-and-mutual-cycle-detection",
      "effort": "M",
      "difficulty": "hard",
      "summary": "R1 tail-position analysis helper over WordBody (terminal-if propagation, clause bodies, outputs-equal-declared-outputs); R2 self-tail-recursion predicate; R3 whole-module tail-call graph; R4 mutual-cycle detection producing located error X1 naming the cycle, with no false-positive on non-tail mutual calls.",
      "changes": ["src/check.rs"],
      "tests": ["tail-position helper unit tests (tail vs non-tail, terminal-if arms, trailing consumer)", "X1 mutual-cycle negative test asserting message + named words", "positive test that non-tail mutual recursion is accepted"],
      "exit": "criterion 6 negative golden passes; tail-position helper covered happy + edge; no regressions."
    },
    {
      "phase": 2,
      "focus": "thread-current-word-name-into-funcbuilder",
      "effort": "S",
      "summary": "R5 add a current-word-name field to FuncBuilder, set from word.name in lower_word; confirm the REPL path (repl.rs:407) carries it with no REPL-specific plumbing. Pure prep for the transform, no behaviour change.",
      "changes": ["src/ir.rs"],
      "tests": ["unit test that FuncBuilder receives the word name for both build and REPL lowering paths"],
      "exit": "builds green; name reaches lower_call; no behaviour change (existing goldens unchanged)."
    },
    {
      "phase": 3,
      "focus": "self-tail-call-back-edge-phi-lowering-transform",
      "effort": "L",
      "difficulty": "hard",
      "summary": "R6 header-with-phi setup in lower_word (params to loop-carried phis, body reads phi outputs, locals/clauses bind from phis); R7 lower a tail self-call as a back-edge Jmp(header) instead of Instr::Call, inside lower_call/lower_if/lower_clauses; R8 SSA back-edge / incomplete-phi back-patching (lower body, collect back-edge inputs, finalize header phis); R9 clause tails into one shared header without predecessor collision; R10 non-tail self-calls and everything else unchanged; R11 vacuous drop-at-back-edge comment anchor.",
      "changes": ["src/ir.rs"],
      "tests": ["criterion 2 IR unit test: tail self-call -> Jmp to header, no Instr::Call to self, header Phi per carried value", "unit test that a non-tail self-call still lowers to Instr::Call (R10)", "unit test for clause-tail back-edge into one header (R9)"],
      "exit": "criterion 2 unit test passes; IR shape correct for terms/if/clause tails and non-tail preserved; green."
    },
    {
      "phase": 4,
      "focus": "backend-verification-goldens-dogfood-and-repl-parity",
      "effort": "M",
      "summary": "R12 QBE structural verification test (header Phi with back-edge predecessor + back-edge Jmp renders valid QBE); examples/countdown.sth dogfood; native goldens for criteria 1, 3, 4, 5, 7; scripted REPL golden for criterion 8; full suite green.",
      "changes": ["src/backend/qbe.rs", "examples/countdown.sth", "tests/phase0.rs", "tests/phase1.rs"],
      "tests": ["criterion 1 large-N native golden (constant stack)", "criterion 3 clause multi-tail native golden", "criterion 4 terminal-if tail native golden", "criterion 5 non-tail factorial native golden", "criterion 7 locals-rebind native golden", "criterion 8 REPL constant-stack golden", "R12 backend structural test"],
      "exit": "all 8 success criteria pass as runnable goldens; countdown.sth native + REPL; full suite green (fmt + clippy -D warnings + test)."
    }
  ]
}
```
