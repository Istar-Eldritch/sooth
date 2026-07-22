# Phase 1 brief — REPL and liveness

Input for spec-writer. Resolves the decisions for the Phase 1 slice (the interactive
live loop). Read alongside [../DESIGN.md](../DESIGN.md), [../ROADMAP.md](../ROADMAP.md),
and [../CLAUDE.md](../CLAUDE.md). Everything here is scoped to Phase 1 only, and builds
on the Phase 0 compiler already on `main` (lexer/parser/checker/IR/QBE-emit/driver).

## Goal and exit criteria

Deliver liveness: an interactive session where you define words, evaluate expressions,
redefine words, and watch the data stack evolve, all running through the real backend
(no interpreter, no JIT).

**Exit:**

1. `cargo run -- repl` starts a session that reads lines from stdin.
2. Defining a word (`: sq ( int -- int ) dup * ;`) loads it; a later line can call it.
3. A bare expression (`5 sq`) runs and leaves its result on a **stack that persists
   across lines**; the session shows the residual stack after each line.
4. Redefining a word (`: sq ( int -- int ) dup dup * * ;`) takes effect for
   subsequently compiled lines.
5. A bad line (lex/parse/check/compile error) prints the diagnostic and the session
   continues with the stack unchanged.
6. **Dogfood:** a real interactive session, e.g. a small calculator, driven end to end.

Phase 1 stays **`int`-only** (the Phase 0 surface). No new language types.

## Execution model: dlopen on the existing backend (Route C)

No in-process JIT and no IR interpreter (see DESIGN; there are no immediate words). The
REPL runs on the QBE backend:

- Each **word definition** line is compiled through the normal pipeline to a **shared
  object** and `dlopen`'d into the session process. The REPL keeps a symbol table:
  word name -> (loaded symbol, declared effect).
- Each **bare expression** line is wrapped in a synthesized entry, compiled to a shared
  object, loaded, and called immediately.
- Objects are loaded so that symbols from earlier definitions are visible to later ones
  (POSIX `RTLD_GLOBAL`-style), so a new definition can call previously loaded words by
  symbol.

This is Factor's in-image feel minus the sub-millisecond per-word compile: there is an
assembler + linker + load round-trip per line, which is acceptable for a craft REPL.
Sub-millisecond would require owning a backend, which is deferred (DESIGN Open/deferred).

## Persistent data stack: a byte buffer plus the carried virtual stack

The stack persists across lines, but **no runtime data stack lives inside compiled word
bodies** (the compile-time-virtual-stack invariant holds: word bodies still compute in
SSA/registers). The persistence is a REPL-driver artifact that bridges separately
compiled lines. It is two things held by the driver:

- a growable **byte buffer** holding the raw values, and
- the **carried virtual stack**: the checker's typed-slot stack (type + offset of each
  live slot), carried forward line to line. This is compile-time state, not runtime
  data; it is what lets the next line be compiled against the current stack shape.

Each expression line compiles to a synthesized wrapper with a **uniform signature**
(roughly `fn(stack: *mut u8, top: usize) -> usize`): a prologue loads its N inputs from
the top of the buffer using the carried virtual stack's layout, the body runs in
registers exactly like any word, an epilogue writes its M outputs back to the buffer and
returns the new top. The driver then advances the carried virtual stack by the line's
net effect. Consequences that keep this simple:

- Results go through the buffer, not the C return value, so **every line has the same
  signature regardless of arity** — no multi-value-return ABI, no per-arity host
  marshalling.
- The **compiler emits the load/store marshalling** from layouts it already knows, so
  the design generalizes to future types with no host-side reflection.
- **Word bodies are unchanged** from Phase 0; only the synthesized line wrapper touches
  the buffer. The buffer is an additive REPL concept, not a change to how words compile.

Phase 1 fills this in with a single type: fixed **8-byte (`int`) slots**. The
buffer-and-carried-virtual-stack shape is the general one; the byte-level layout logic
for richer types is Phase 2's problem.

This runtime stack buffer is a preview of the "uniform runtime stack" DESIGN reserves
for escaping quotations (Phase 4); record it in DESIGN so it is not mistaken for a
breach of the compile-time-only-stack invariant.

## Line surface: accept both defs and bare expressions

A REPL line may be a word definition (`: name ( effect ) | locals | body ;`) or a bare
expression (a sequence of terms, e.g. `5 sq` or `2 3 + .`). A def is compiled and
loaded but touches no stack; an expression gets the buffer-marshalling wrapper and runs.
This needs:

- **Parser:** accept a top-level term sequence, not only word defs + `main`
  (Phase 0 parses `Module { words }`).
- **Checker:** **infer** the net effect of a bare term sequence (a small extension of
  the existing depth simulation in `check_terms`, `src/check.rs`), rather than only
  verifying a declared effect. The inferred entry depth is the current carried stack;
  underflow against the persisted stack is a normal, reported error.

## Redefinition

Latest-symbol binding: a redefinition takes effect for **subsequently compiled** lines;
already-loaded code keeps the callee it was compiled against. Concrete mechanism: mangle
each definition's exported symbol with a generation counter (e.g. `sq__gen3`), and have
the REPL resolve a callee name to its **current** generation at compile time. This
avoids symbol clashes under `RTLD_GLOBAL` and makes "loaded callers keep the old callee"
fall out naturally.

**Deferred (intended, not a Phase 1 exit):** recompiling dependents so a redefinition
propagates to existing words. It needs a dependency graph and cascading recompiles;
Phase 1 does not do it, but the REPL should gain it later.

## Driver and session mechanics

- New driver subcommand `repl` alongside Phase 0's `build`/`run` (`src/driver.rs`,
  `src/main.rs`, `src/lib.rs`).
- New **compile-to-shared-object** path (Phase 0 only compiles to a binary): emit
  `.ssa`, run `qbe` -> `.s`, `cc -shared` -> `.so`. Load via a crate such as
  `libloading`, or raw libc `dlopen`.
- **Output:** after each line, show the residual stack (Factor-style), since carrying
  the stack is the whole point; not Forth `ok`-only.
- **Input:** bare stdin line reading for Phase 1. Readline/history is polish, deferred.
- **Error recovery:** any compile-stage error prints the diagnostic and the session
  continues, stack unchanged. Caveat: a *runtime* crash in loaded code takes the process
  down (unavoidable in-process); low risk while the surface is `int`-only and statically
  checked.
- **Platform:** Linux + macOS.

## Out of scope for Phase 1

New language types (`bool`, the numeric tower, structs/enums, pointers), heap,
affine/move semantics, polymorphism, quotations/combinators, tail-call optimisation,
recompiling dependents on redefinition, readline/history, sub-millisecond compile / an
owned native backend, WASM, `no_std` packaging, and comptime/immediate words (declined,
see DESIGN). All later phases or explicitly declined.

## Test plan

- **Session goldens:** feed a scripted sequence of REPL lines on stdin and assert the
  emitted output (residual-stack display + `.` output). At least: define + call across
  lines; carry the stack across lines; redefine and observe the new behaviour; a bad
  line reports and the session survives. A small calculator session is the dogfood
  golden.
- **Unit tests** beside each new/extended piece (per CLAUDE.md: happy path + at least
  one error/edge case), named `thing_condition_expected`:
  - checker: net-effect inference of a bare term sequence; underflow against the carried
    stack is the right error.
  - the synthesized line-wrapper lowering (prologue/epilogue buffer marshalling).
  - the compile-to-`.so` driver path.
  - redefinition/generation symbol resolution (a later line binds the current
    generation of a redefined word).
- Diagnostics are behaviour: bad lines assert the *right* error text, not just failure.
