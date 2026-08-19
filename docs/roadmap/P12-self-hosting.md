[← ROADMAP](./ROADMAP.md)

### Phase 12 — Self-hosting  `[XL]`

Stabilise the self-hosting subset S (smaller than before: concrete types + ADTs +
pattern matching, growable collections + strings, words + modules, errors as
values, a modest C FFI for the hosted layer; no inference, no refinements, no effect
rows, no borrow analysis). Rewrite the compiler in S, fixpoint-verify
(bootstrap-compiled == self-compiled), retire/demote the host-language bootstrap.
No metacircular JIT: the self-hosted REPL/build path still runs on the backend.
**Exit:** the compiler compiles itself; fixpoint reached.

### Optional (any time after Phase 2) — WASM sibling backend  `[M]`

A second lowering off the backend-neutral IR, parallel to QBE, not through it: Sooth
IR → WASM (emit, hand to binaryen for optimisation and any structured-control
cleanup). No relooper needed, since the IR already carries structured control flow.
The hosted layer re-ports from libc-FFI to WASI imports; `core`/`fixed` compile
nearly for free. AOT-to-native via `wasm2c` when a native artifact is wanted.
Depends on `Ptr[T]` having been kept abstract since Phase 2.
**Exit:** a Sooth program runs both as a native QBE binary and as a `.wasm` module.

### Committed future target — RISC-V 32

rv32 is a committed eventual target (embedded). QBE gives arm64/x86_64/riscv64 but has no
rv32, so reaching it means patching rv32 into QBE or the hand-written backend, a decision
deferred to **post-bootstrap** (consistent with "reconsider the backend after self-hosting").
Nothing is built for it now; the only present-tense obligation is that the frontend stays
word-width-neutral: the IR never assumes a 64-bit machine word, and `usize`/`isize` arrive as
target-width types with arrays (Slice 5). See DESIGN.md, Codegen and backend.

