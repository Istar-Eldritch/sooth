[← ROADMAP](./ROADMAP.md)

### Phase 12 — Self-hosting  `[XL]`

Stabilise the self-hosting subset S (smaller than before: concrete types + ADTs +
pattern matching, growable collections + strings, words + modules, errors as
values, a modest C FFI for the hosted layer; no inference, no refinements, no effect
rows, no borrow analysis). Progressive takeover, not a rewrite-and-cutover: each
compiler stage (lex, parse, check, lower, emit) is reimplemented in S and swapped in
one at a time, front-end first, while the not-yet-ported stages keep running as
host-language (Rust) code called across the FFI boundary — the Zig self-hosted-compiler
model, not Nim's permanent-hybrid one. Stages are retired from the host side as their
Sooth replacement proves out; the host-language bootstrap only disappears once every
stage has been ported. No metacircular JIT: the self-hosted build path still runs
on the backend.

**Prerequisite:** [P8.S4](./P8-packages-modules.md) (richer `extern:` payloads, unmangled
exports). The FFI boundary this phase depends on is one-directional today (host calling
Sooth, via `driver::Library`'s `dlopen` over a `compile_so` output); a progressive port
additionally needs Sooth calling host code with non-trivial payloads (token streams, AST
nodes, tagged unions), which is module/linkage machinery pulled forward into Phase 8
rather than designed here, since it blocks self-hosting sequencing, not stdlib content.

**Exit:** the compiler compiles itself; fixpoint reached (bootstrap-compiled ==
self-compiled).

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
