# Sooth — codegen and backend

Design detail for codegen and backend, split from [DESIGN.md](../../DESIGN.md).

## Codegen and backend

Codegen model (unchanged from first principles, it's the good part): don't model
the data stack at runtime. Simulate it at compile time as an array of typed slots;
push/pop manipulate the array, and when IR is emitted the slots become ordinary
SSA/register values. Each word compiles to a function taking N stack args and
returning M results. The `branch` primitive becomes basic blocks and a conditional
jump; there are no loop keywords (see [control flow and iteration](./control-flow.md)), and iteration
lowers to an internal loop primitive with a back-edge. Branch and loop join points unify the virtual-stack state
(depth and type) across predecessors; mismatched depth or type across arms is a
compile error.

**No LLVM, and not a hand-written backend either. Decided: QBE.** The joy in this
project is the language and writing programs in it, not emitting machine code, so
codegen is offloaded to the smallest backend that stays legible. QBE (~15k lines you
could actually read) gives arm64/x86_64/riscv64 plus C-ABI struct classification for
free, and can carry essentially the entire design: everything interesting (linear
analysis, monomorphisation of the small polymorphic core, deterministic drop) is
frontend/runtime work QBE is agnostic to. A hand-written native backend (own the
vertical, direct syscalls) was the craft-purist alternative, set aside for
now, and reconsidered after self-hosting, because it optimises for building the
compiler, which isn't the point at this stage. LLVM was
rejected outright: too large and opaque for a hold-in-head project, a perpetual
dependency tax, and product-grade output the language doesn't need. Wanting LLVM's
full-service codegen is a tell that the project has drifted back to product-think,
where the honest answer was "use Rust."

**RISC-V 32 is a committed eventual target; the backend to reach it is deferred to
post-bootstrap.** QBE gives arm64/x86_64/riscv64, but has no rv32 target (and assumes a
64-bit machine word in places), so emitting rv32 will mean either patching an rv32 target
into QBE or the hand-written backend, a call taken after the language self-hosts, consistent
with the "reconsidered after self-hosting" stance above. The commitment is recorded now not
to build anything, but so the frontend stops accruing 64-bit assumptions before then (see
next).

QBE's costs, accepted: it emits assembly text, so you depend on the system assembler

- linker (a cross-toolchain + sysroot when cross-compiling the hosted layer); it has no
volatile or atomic primitives, patched in rather than worked around (see
[embedded](./embedded.md) and [concurrency](./concurrency.md)). Its modest optimiser is a feature, not a bug: more predictable than
LLVM's aggressive passes and friendlier to any later WCET work.

**QBE is a tracked fork, not a system dependency.** `~/code/qbe` tracks canonical upstream
(`git://c9x.me/qbe.git`) at v1.3 plus a few, matching the installed binary's target list
(`amd64_sysv/apple/win`, `arm64`/`arm64_apple`, `rv64`) exactly. Forking it for volatile
and atomics is accepted; that does not extend to a Thumb/ARMv6-M target, a full new
backend and an unrelated order of magnitude, argued on its own terms rather than
inheriting "we already patch QBE" as a precedent.

**WASM is a sibling lowering, not routed through QBE.** Sooth's IR is already
stack-shaped with structured control flow, exactly what WASM wants, so WASM hangs off
the neutral IR in parallel to QBE (emit WASM, hand to binaryen for optimisation),
never downstream of it (going through QBE would flatten the stack/structured-control
shape only to rebuild it with a relooper). The "uxn that grew up" target: portable,
AOT-to-native via wasm2c when a native binary is wanted.

**Enabling decision, load-bearing from Phase 2:** keep pointer size and memory model
abstract in the IR (`Ptr[T]` is an opaque handle, not a native `u64`), so the QBE
(native pointers) and WASM (linear-memory offsets) lowerings each concretise it. A
native-pointer assumption leaking into shared IR is the one thing that makes WASM
chafe later.

The same rule extends to integer width: **the IR never assumes a 64-bit machine word.**
Word, pointer, and `usize`/`isize` width are a target parameter, not a constant, exactly as
`Ptr[T]` is opaque. This is not abstract tidiness: the committed rv32 target (above) has
32-bit pointers and makes `i64`/`u64` double-word there (synthesised as register pairs in
the frontend), so `usize` is genuinely 32-bit there. `usize` is a target-width type
introduced with fixed-size arrays (Phase 2, Slice 5), where indexing is its first real
consumer; `isize` mirrors it but waited for Phase 3 Slice 3, since it had no consumer
until recursive/heap data existed. Both resolve to 64-bit on current targets but must
never be *assumed* 64-bit in shared IR. A corollary worth revisiting under a 32-bit target: the
current "integer literals default to `i64`" stance is 64-bit-centric, since on rv32 the
natural machine word is 32-bit, not `i64`.

**Dropping LLVM means no in-process JIT, and it turns out to cost nothing.** LLVM's
ORC would have let an interactive host-loading path and a compile-time evaluator
share one native engine. Two decisions remove the need for it outright:

- **No compile-time execution.** There is no immediate-word / macro facility (see
  [Declined](../../DESIGN.md#declined)), so nothing runs Sooth at compile time and there is no comptime
  interpreter to build.
- **Any host-loading path runs on the backend, not an interpreter.** `driver::Library`
  keeps a `dlopen`/`dlsym` wrapper over a `compile_so` output as the load-bearing
  primitive for this: a compiled word is a shared object loaded in-process, so the
  process holds live, natively-compiled code it can call at once. Whole-program
  `run`/watch takes the simpler compile-to-binary + subprocess path. One execution
  semantics either way, with nothing to keep in sync against a second, interpreted
  one. Sub-millisecond per-definition loading would require owning a backend (see
  [Open / deferred](../../DESIGN.md#open--deferred)); not now.

Word bodies compute entirely in SSA/registers; the compile-time-virtual-stack
invariant holds regardless of which loading path a compiled word reaches the
process through. The **uniform runtime stack** reserved for escaping quotations
(Phase 4) is the same shape as any such loading path's stack-bridging buffer --
marshal to/from a byte buffer at a compiled boundary -- reused there for closures
that must cross into `alloc` rather than for bridging separately-compiled units.
Neither case puts a runtime stack inside a word body.
