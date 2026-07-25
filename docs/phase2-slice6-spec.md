# Phase 2 Slice 6 — self-tail-call → loop lowering (delivered)

A **compiler** slice (no lexer/parser/AST change): a checker analysis that classifies tail-position calls and rejects mutual tail-call cycles, plus an IR lowering that turns a self-tail-call into a back-edge `Jmp` to a phi'd loop header instead of `Instr::Call`. Self-tail-recursion runs in constant stack. Reuses existing IR (`Phi`, back-edge `Jmp`, blocks); adds no instruction, terminator, or syntax.

## What shipped

- **Self-tail-call → loop, guaranteed** (D1): a word/clause body ending in a tail call to *itself* compiles to a back-edge jump to a phi'd header. Mandatory, no pragma; tail calls to *other* words stay ordinary calls.
- **Tail-position analysis** (D2, R1): a `TermKind::Call` is tail iff it's the final term of a `Terms` body, a clause body, or an arm of a **terminal** `if/else/end` (propagates through both arms recursively). Any trailing term (`rec *`, `rec swap`, another call) breaks it; a self-call in a non-terminal `if` is not tail. Output-equality is a *consequence* of syntactic position, not a separate gate. The one syntactic rule is encoded in two lockstep places (the checker's tail-call name-list recursion and the lowerer's positional tail flag), kept in sync by paired comments and tests on each side rather than a single shared helper, because the two consumers need it in genuinely different shapes.
- **Mutual tail recursion → located compile error X1** (D3, R3/R4): whole-module tail-call graph over user words; a self-loop (A→A) is allowed, any cycle ≥ 2 is rejected naming the members in order. Non-tail mutual calls produce no edge, no false-positive.
- **Loop shape** (D4, R6): entry block binds params then `Jmp(header)`; header carries one `Phi` per **input-arity** loop-carried slot (entry arm = param, back-edge arms = recursive args); body lowers from header reading phi outputs, not raw params; base cases `Ret`. Input-arity 0 → zero phis, no special-casing.
- **SSA back-edge back-patching** (R8, the crux): back-edge phi operands are produced later; lower body first collecting per-slot `(pred_block, value)`, then append arms to header phis in a second step. Allocations hoisted to entry block for the constant-stack guarantee.
- **Locals + clauses under the loop** (D6/D7, R9): `| … |` locals bind to header phi outputs (rebind each iteration); a `|`-clause word has one shared header with one back-edge arm per clause tail, dispatch stays in the body. Loop-header phis and Slice-4 clause-join phis keep disjoint predecessor sets (join preds = non-tail clause ends; header preds = entry + tail clause ends).
- **Word name threaded into `FuncBuilder`** (R5): set from `word.name` in `lower_word`; reaches the REPL path with no REPL-specific plumbing (M6).
- **Non-tail preserved** (R10): a self-call not in tail position (`n rec *`, or inside a non-terminal `if`) still lowers to `Instr::Call`; all other calls unchanged.
- **Drop-at-back-edge anchor** (D9, R11): back-edge is the defined destructor insertion point; in all-`Copy` Phase 2 the drop set is empty, recorded as a comment so Phase 3 has a home.
- **Backend**: QBE renders phi + back-edge `Jmp` natively; only a structural verification test added (R12).
- **Dogfood** `examples/countdown.sth`: tail-recursive accumulator over large N, overflows naive / completes transformed. Native + REPL.

## Delivery by phase

1. **Checker** (`src/check.rs`) — R1 tail-position helper, R2 self-tail predicate, R3 module tail-call graph, R4 mutual-cycle detection + X1. Commit `9d6412bd`.
2. **IR prep** (`src/ir.rs`) — R5 current-word-name field, no behaviour change. Commit `5589db15`.
3. **Lowering transform** (`src/ir.rs`) — R6 header+phi, R7 back-edge in `lower_call`/`lower_if`/`lower_clauses`, R8 back-patch, R9 clause tails, R10 unchanged paths, R11 comment. Commits `46236edf`, `39ab7d1c` (entry-block hoist), `39d8f216` (checker-lowerer rule sync docs).
4. **Backend + goldens + REPL** (`src/backend/qbe.rs`, `examples/countdown.sth`, `tests/phase0.rs`, `tests/phase1.rs`) — R12 QBE test, dogfood, native goldens. Commits `3888f7bc`, `8b33be10` (locals-rebind golden), `20c06712` (mixed-clause comment).

## Success criteria (goldens unless noted)

1. Large-N (≥1M) self-tail-recursion in constant stack — `countdown.sth` native.
2. **IR shape** (sole sanctioned structural unit test): tail self-call → `Jmp` to header, no self `Instr::Call`, one header `Phi` per carried (input-arity) slot.
3. Clause multi-tail native golden + mixed-clause (some back-edge, one `Ret`s) — disjoint phi predecessors.
4. Terminal-`if` tail golden + both-arms-tail (two back-edges via `lower_if`, R8 multi-arm).
5. Non-tail factorial still a real `Call` + negative unit test for non-terminal-`if` self-call.
6. Mutual tail recursion rejected (X1, named cycle) + positive non-tail-mutual accepted.
7. Locals rebind per iteration, correct + constant-stack at ≥1M.
8. REPL parity: self-tail-recursive word at REPL, constant stack at ≥1M.

N ≥ 1M throughout so a disabled transform overflows (red), not passes small.

## Invariants held

QBE-only (no LLVM, no native backend); backend-neutral IR (no new instruction/terminator, `Ptr[T]` opaque); affine spine unchanged (all `Copy`, drop set empty at back-edge); `core` stays `no_std`; no in-process JIT / comptime (REPL still `dlopen`). Verified by native + REPL goldens, not IL assertions (except criterion 2). Green each phase (`cargo fmt --check && cargo clippy -- -D warnings && cargo test`).

## Out of scope (deferred)

Mutual TCO / SCC contraction (tier 2); trampolines (need Phase 4 quotations); QBE backend tail calls; quotations/combinators/general loop syntax (Phase 4); drop glue at back-edge (Phase 3, co-designed); tail calls across extern/module boundaries; optimising non-tail recursion.
