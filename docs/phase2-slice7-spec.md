# Phase 2 Slice 7 — bytecode-VM dogfood (Phase 2 exit) — CONDENSED

## What this slice was

A single worked program, `examples/vm.sth`: a fixed-size bytecode stack machine interpreting a toy instruction set, with native and REPL goldens. The **Phase 2 exit dogfood** — proof that the typed core (fixed-size arrays, `usize`, enums + clause dispatch, structs, Slice 6 self-tail-call dispatch loop) composes into a real data-driven interpreter. A *dogfood, not a feature slice*: no new stage requirement.

### Load-bearing constraint (D1, criterion 9) — HELD

**Zero compiler machinery.** Touched only `examples/vm.sth`, `tests/phase0.rs`, `tests/phase1.rs`. `git diff main...HEAD` shows **no `src/` change**. A `src/` path in any phase would have violated D1; if the VM had been unwritable without a compiler change, that was a *finding to surface*, not a change to make.

## Grounding facts (confirmed against source)

1. **A user word returns at most one value through a call** (`lower_call` materialises a result only when `out_arity == 1`). Generated struct words (`S`/`S>`/…) lower **inline**, so they can push several values. **Consequence:** any helper returning >1 value returns a *bundle struct*, destructured inline at the call site (`Fetched`, `VmPop`).
2. **No `ulimit`/`setrlimit` anywhere.** Constant-stack goldens run a self-tail-recursive word at large N against the default 8 MB host stack; `run_and_capture_stdout` calls `.status.code().expect(...)`, so a stack-overflow SIGSEGV fails the test — the mechanism that turns a no-op tail transform red. No `ulimit` harness (would be new machinery for no benefit).
3. **A conversion word on a `bool` is a checker error.** Bool-to-index must go through `if 1 else 0 end >usize`, never `>i64` on a bool.

Also: `fill` interns any element type (`Halt N fill` seeds `[Op N]`); `get` is non-consuming (drops index, leaves array, pushes element — use `swap drop` to keep the `Op`); `set` is a functional write; a clause word may carry a wide input signature and self-tail-recurse in constant stack.

## Instruction set (D3)

```
type: Op
| Push v i64 | Add | Sub | Mul
| Load addr usize | Store addr usize
| Jz target usize | Jmp target usize | Halt ;
```

Mixed-arity on purpose: `Push`/`Load`/`Store`/`Jz`/`Jmp` carry a payload; `Add`/`Sub`/`Mul`/`Halt` do not — clause dispatch destructures both shapes (criterion 4).

Semantics (operand stack = LIFO of `i64`; `mem` = addressable `[i64 M]`): `Push v` push v, pc+1; `Add/Sub/Mul` pop b, pop a, push a∘b, pc+1; `Load addr` push mem[addr], pc+1; `Store addr` pop x, mem[addr]=x, pc+1; `Jz target` pop x, pc=target if x==0 else pc+1; `Jmp target` pc=target (backward branch drives the loop, criterion 2); `Halt` return top of operand stack. No traps beyond the existing array-bounds trap.

## VM state + dispatch loop

Reference design (D5 primary, the one implemented — keeps structs in the capstone):

```
type: Vm  prog [Op P]  pc usize  stack [i64 S]  sp usize  mem [i64 M] ;
: run ( Vm Op -- i64 )   \ clause-dispatch on top-of-stack Op at entry
```

Every non-`Halt` clause produces an updated `vm'` then `vm' fetch Fetched> run` (the Slice 6 tail back-edge). `Halt` returns without a self-call (base case). The tail-call carries `(Vm, Op)` (`in_arity` 2), so Slice 6 lowers `run` to a two-phi loop header → constant stack (criterion 3).

- `fetch ( Vm -- Fetched )` reads `prog[pc]` with non-consuming `get`, bundles `(Vm, Op)`; `Fetched>` inline yields the pair for the next `run`.
- Operand helpers over `stack`/`sp` (per `examples/stack.sth`): `vm-push ( Vm i64 -- Vm )`, `vm-pop ( Vm -- VmPop )` bundling `(Vm, i64)`.
- `Load`/`Store` are `get`/`set` on `mem` with the `usize` payload as index (criterion 5, bounds-trap path live). `pc += 1`/targets are `usize` arithmetic; literals coerce, computed indices go through `>usize` (criterion 6).

**Blessed fallback (D5 alt, not needed):** thread state as separate loop-carried values (`run ( [Op P] usize [i64 S] usize [i64 M] Op -- i64 )`) if the per-step `Vm` blit made the constant-stack golden slow (> ~3 s). Struct design was picked first and kept.

## Golden program: sum 1..N via bytecode (D4)

Counter in `mem[0]`, accumulator in `mem[1]`; loops with `Jz`/`Jmp`; halts with the sum on top. Assembled by a builder word via `fill` + per-index `set` (no array literal, D2):

```
L: Load 0 ; Jz E ; Load 1 ; Load 0 ; Add ; Store 1 ;
   Load 0 ; Push 1 ; Sub ; Store 0 ; Jmp L ;
E: Load 1 ; Halt
```

Initial `mem[0]=N`, `mem[1]=0`. **N rule:** the criterion is *dispatch steps*, not loop trips; a body of ~11 opcodes over T trips runs S ≈ 11·T steps — choose N so S ≥ 1_000_000.
- Correctness (Phase 3): small N (e.g. 10 → 55), fast, exercises every opcode + backward branch.
- Constant-stack (Phase 4): **N = 100_000 → sum 5000050000**, S ≈ 1.1M > 1M; confirmed the step count clears 1M.

## Criterion → test map

| # | Proven where |
|---|---|
| 1 | `vm_dogfood_compiles_and_runs` (phase0) |
| 2 | same native golden; asserted sum only reachable via the `Jz`/`Jmp` loop |
| 3 | `vm_dispatch_loop_runs_in_constant_stack` (phase0), N=100_000, overflow caught by `.code()` |
| 4 | sum program uses payload *and* no-payload variants; no separate test |
| 5 | builder + operand helpers; native golden |
| 6 | `Vm`/`run` signatures + `Load`/`Store`/jump handling; native golden |
| 7 | `vm_dogfood_runs_in_repl` (phase1) |
| 8 | `enum_get_from_carried_array_clause_dispatch_constant_stack` (phase0) — built first |
| 9 | `git diff` shows no `src/` change; enforced per-phase |

## Implementation (commits)

| Phase | Focus | Commit(s) | Files |
|---|---|---|---|
| 1 | Smoke-test the crux (criterion 8): `get` an `Op` from a carried `[Op 2]` array, clause-dispatch, tail-recurse 1M× in constant stack. Stop + surface if it needs `src/`. | `e28ec5c` | tests/phase0.rs |
| 2 | VM data model: `Op` enum, `Vm` struct, `Fetched`/`VmPop` bundles, `vm-push`/`vm-pop`; minimal `main` round-tripping push/pop. | `280b63a` | examples/vm.sth, tests/phase0.rs |
| 3 | `fetch`, nine-clause `run`, `fill`+`set` sum-1..N builder, real `main`; native correctness golden at small N. | `b93d35e`, `8afba64` (clarify: sum never multiplies) | examples/vm.sth, tests/phase0.rs |
| 4 | Scale committed example to N=100_000; native constant-stack golden (default-stack-overflow mechanism, no `ulimit`). | `a56477d` | examples/vm.sth, tests/phase0.rs |
| 5 | REPL parity golden through the `dlopen` path, asserting the same sum; no REPL-specific plumbing. | `b962d78` | tests/phase1.rs |

## Phase 1 smoke test (criterion 8, the crux)

The residual unproven composition: `get` an `Op` out of the *carried* program array, clause-match, tail-recurse in constant stack. Inline temp-source `tests/phase0.rs` test (mirrors Slice 6 temp-source goldens). Carries a `[Op 2]`, a countdown, a step accumulator; loops 1_000_000× reading the enum out of the array each step, then halts. `idx` uses `if 1 else 0 end >usize`; `fetch` uses non-consuming `get` + `swap drop`. Expected stdout `1000000\n`, exit code 0. Marked `difficulty: hard` — de-risks the whole slice. If unpassable without `src/`, stop and surface (that is itself the Phase 2 exit verdict).

## Test placement notes

Phases 3 and 4 share the same committed `examples/vm.sth` at N=100_000; the Phase 3 fast small-N correctness check uses an inline temp-source copy so both coexist without the example carrying two mains. Phase 5 REPL golden uses the same N so "the same result" is literal.

## Findings logged (not fixed here)

- **Program-construction verbosity (D2):** `fill`+per-index `set` is verbose — signal for a Phase 4 ergonomics slice (array/opcode literals).
- **Per-step aggregate blit:** the `Vm` struct blits the whole record each step (constant-stack, but a real memcpy). The separate-loop-carried-values design would remove the `prog` copy; in-place mutation/borrow is a separate slice.
- **Single-return-value call ABI tax:** every multi-result helper needs a bundle struct + inline destructure (grounding note 1).

## Out of scope

Any `src/` change (new opcode generality, in-place array mutation, array literals, error opcodes beyond the bounds trap); bytecode-level functions/calls (Phase 4 quotations/combinators); mutual tail recursion / trampolines / SCC contraction (the VM is a *single* self-tail-recursive `run`); making program construction nicer (logged as a finding).
