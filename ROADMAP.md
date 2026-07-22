# Sooth — roadmap

Implementation roadmap for the language in [DESIGN.md](./DESIGN.md). Milestones,
not a schedule.

## Current status / next action

Design phase complete (see DESIGN.md, Decided section). Backend decided: **QBE**
(the joy is the language, not codegen). **Next action: Phase 0**, prototype the
codegen spine to pressure-test the core architectural bet: compile-time virtual
stack → backend-neutral IR → QBE IL → native binary.

Host language: Rust is the sensible default (ADT + pattern-matching-heavy compiler
workload, `no_std` for the runtime/intrinsics library), but nothing now requires
it, since LLVM and Z3 were dropped. Free choice.

## Guiding principles

- **De-risk novel-before-laborious.** Prove the uncertain, novel parts (the codegen
  model, then the affine memory model, which is the whole point of the language)
  early. The larger-but-understood parts (stdlib, self-hosting) can wait.
- **Vertical slices with a dogfood program each phase.** Every phase ends with a
  language you can run a real (if small) program in, and you actually write that
  program. This is the antidote to the failure mode named in DESIGN.md: a beautiful
  half-built compiler no one writes code in. If a phase produces no runnable
  program, the phase isn't done.
- **Liveness early.** A REPL and immediate feedback arrive in Phase 1, not at the
  end, for the same reason.
- **No calendar estimates** (they'd be fiction). Effort weights (S/M/L/XL) are
  relative, to show where the mass is.

## Phases

### Phase 0 — Codegen spine  `[L]`  `[highest risk: go/no-go on the architecture]`
Lexer/parser for a minimal concrete-typed core (`: ;`, literals, arithmetic,
`if/else/then`, `begin/until`, `| locals |`). Compile-time virtual stack → a
backend-neutral IR → **QBE** IL → `qbe` → system assembler + linker → native binary.
No LLVM, no hand-written native backend. Keep the IR's `Ptr[T]` abstract from the
start so a WASM sibling lowering can be added later. Static stack-effect (arity)
checking. One concrete int type, no heap.
**Exit:** `gcd` and `factorial` compile to a standalone native binary and run
correctly. Proves the virtual-stack → IR → QBE → native path end-to-end.

### Phase 1 — REPL and liveness  `[M]`
No in-process JIT (that left with LLVM). REPL that compiles each snippet to a temp
shared object and loads it, or batch-recompiles; word definition, execution, and
redefinition (name→latest-symbol table). Compile-time / immediate words run in a
small **interpreter** over the IR, not as native code.
**Exit:** define/test words interactively; redefinition works; the first
throwaway-but-real interactive session exists.
**Dogfood:** a tiny interactive calculator or turtle-graphics doodle.

### Phase 2 — Typed core (monomorphic)  `[L]`
`(value, type)` slot from day one, concrete types only. Numeric tower (i8..i64,
u8..u64, f32/f64; i128/u128 synthesised in the frontend if on QBE; `*/` widening
primitive; literal defaults). Records/structs, enums/ADTs, exhaustiveness-checked
pattern matching. Non-null pointers + explicit optional type. The **`Copy` vs
affine distinction** as a built-in property of types (primitives Copy; anything
owning a resource affine), so Phase 3 has it to build on. Stack-effect checking now
unifies **type and arity** at branch/loop join points. Still heap-free: value types
+ fixed-size arrays only.
**Exit:** typed programs with structs/enums/match; type and arity errors are sharp
compile errors.
**Dogfood:** a small parser or a fixed-size VM for some toy bytecode.

### Phase 3 — The affine spine  `[XL]`  `[highest novelty: this is the point of the language]`
Move semantics as the default; `dup` is the explicit copy, gated on `Copy`;
deterministic drop (destructor at the statically-known end of ownership). Hylo-style
mutable value semantics: parameter conventions (`let`/`inout`/`sink`/`set`) and
second-class references (can't be stored, can't escape scope), so no borrow checker
and no lifetimes. Opt-in RC (`Rc`/`Arc`-equivalent). **Heap arrives here**, under
ownership. Resources (fds, later locks) modelled as affine values; `dup` on them is
a compile error.
**Exit:** memory-safe heap programs, no GC, deterministic destruction, resources as
affine values that can't be duplicated or leaked.
**Dogfood:** a program that opens/reads/closes files and manages owned buffers,
with the compiler catching a deliberate double-use.

### Phase 4 — Minimal polymorphism + quotations  `[L]`
Not full HM inference. Type variables (`'T`) and a row variable (`..s`) so
`dup`/`swap`/`max` and user words have honest polymorphic signatures; monomorphise
per concrete stack shape, force-inline the small core words. Required operations
(e.g. `>` for `max`) resolved at the concrete instantiation, Kitten-style, no formal
trait system. Quotations and higher-order words (`map`/`filter`/`each`). Escaping
quotations use the uniform-runtime-stack fallback and depend on the alloc layer
(Phase 6).
**Exit:** polymorphic `dup`/`swap`/`max`, generic operations over collections,
higher-order combinators; monomorphisation verified.
**Dogfood:** rewrite an earlier program using `map`/`filter` and a couple of
user-defined polymorphic words.

### Phase 5 — Errors as values  `[S]`
Result/Either as an ordinary ADT (mostly free from Phase 2), plus the `?`-style
short-circuit sugar and the convention that fallible words return it. Branch-on-
result codegen, no unwinding. FFI/C error returns map to Result at the (later)
safe-wrapper layer.
**Exit:** Result-based error handling with `?` sugar; no exception/unwind path
exists anywhere.

### Phase 6 — Stdlib and `no_std` layering  `[L]`  `[where it becomes usable for real programs]`
The four layers from DESIGN.md, with boundaries and the allocator *interface* fixed
now even though hosted is built first: **core** (already accreting), **fixed**
(allocation-free fixed-capacity vec/map/string/ringbuffer), **alloc** (growable
Vec/Map/String, Box, opt-in Rc/Arc, escaping closures, bignum, against core's
allocator interface), **hosted** (files, stdio, time, FFI-to-libc via safe
wrappers). Tag every stdlib word with the layer it needs.
**Exit:** real hosted programs using libc via safe wrappers; a usable standard
library; the `fixed` layer works with no allocator present.
**Dogfood:** a genuinely useful small tool (a line-oriented text utility, a small
static-site or markdown thing) written entirely in Sooth.

### Phase 7 — Concurrency (library)  `[M]`
Core intrinsics only: **atomics + memory ordering** (LL/SC on arm64, or FFI to C11
atomics on QBE) and a **spawn** primitive (thin FFI to `pthread_create` at the
hosted layer). Everything else is library: split-endpoint channels, mutexes,
pools, and actors (mailbox + loop + move-only messages). Data-race freedom is
inherited from the affine spine (send = move) and non-escaping refs, no separate
`Send`/`Sync` apparatus. Ship two libraries: the convenient hosted one and a
constrained `no_std`/RT one (static topology, fixed mailboxes, no escaping
captures).
**Exit:** concurrent programs that are data-race-free by construction; a deliberate
attempt to alias a sent value is a compile error.
**Dogfood:** a small worker-pool or a producer/consumer pipeline.

### Phase 8 — Bare metal  `[M]`  `[the craft milestone: own the vertical to the metal]`
Cross-compile to arm64 (or Cortex-M) bare metal: per-target intrinsics
(memcpy/memset, integer-divide/soft-float helpers), linker script, entry point,
`no_std` core + `fixed` layer on-device, soft-float lint. Soft-real-time works out
of the box; demonstrate hard-RT-by-discipline (fixed layer + static-topology
concurrency, no allocation or spawning on the hot path) if you want it.
**Exit:** a program running on real hardware or QEMU with no OS and no allocator,
blinking an LED or driving a sensor, from your own source language down to the
machine code you emit.

### Phase 9 — Self-hosting  `[XL]`
Stabilise the self-hosting subset S (smaller than before: concrete types + ADTs +
pattern matching, growable collections + strings, words + modules, errors as
values, a modest C FFI for the hosted layer; no inference, no refinements, no effect
rows, no borrow analysis). Rewrite the compiler in S, fixpoint-verify
(bootstrap-compiled == self-compiled), retire/demote the host-language bootstrap.
Compile-time immediate words run in the same interpreter, no metacircular JIT.
**Exit:** the compiler compiles itself; fixpoint reached.

### Optional (any time after Phase 2) — WASM sibling backend  `[M]`
A second lowering off the backend-neutral IR, parallel to QBE, not through it: Sooth
IR → WASM (emit, hand to binaryen for optimisation and any structured-control
cleanup). No relooper needed, since the IR already carries structured control flow.
The hosted layer re-ports from libc-FFI to WASI imports; `core`/`fixed` compile
nearly for free. AOT-to-native via `wasm2c` when a native artifact is wanted.
Depends on `Ptr[T]` having been kept abstract since Phase 2.
**Exit:** a Sooth program runs both as a native QBE binary and as a `.wasm` module.

## Cross-cutting — Tooling and diagnostics  `[ongoing from Phase 0]`
Not a terminal phase. Good, localised compile errors start at Phase 0, for the
author's own write-run-fix loop and for legibility, not for any LLM-authorability
goal (dropped). A formatter and an auto-generated reference doc (word list + stack
effects) once the surface stabilises around Phase 4. An LSP is optional and low
priority for a craft language; add it only if you're using it enough to want it.

## Shape of the risk

- The mass and the risk are in **Phase 0** (go/no-go on the codegen architecture:
  does the virtual-stack → IR → QBE path hold, small but decisive) and **Phase 3**
  (the affine memory model,
  the most novel work and the reason the language exists). Do both carefully.
  **Phase 9** (self-hosting) is the other large lift but is well understood.
- Phases 4-8 are more independent than the numbering implies. Errors (5) is nearly
  free once ADTs exist (2). Concurrency (7) needs the affine model (3) but little
  else. Bare metal (8) needs the `fixed` layer (6) but not the hosted one. Reorder
  within that band by what you want to play with first, which for a craft project is
  a legitimate way to choose.
