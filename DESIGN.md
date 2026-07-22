# Sooth — design notes

A small, statically-checked concatenative language in the Forth/Factor/Kitten
lineage, compiled straight to native code with no external runtime. Working notes,
not a spec.

## What this is: a craft language

Sooth is built for the pleasure of building and writing it, not for a market. That
is a deliberate scope decision, made after arguing the alternative and rejecting
it (see "Why craft, not product" below). The consequences run through every choice
here: the language stays small enough to hold in one head, the compiler stays small and legible, and its one backend dependency
(QBE) is itself small enough to read rather than an opaque black box. Where a decision
trades reach or peak
performance for simplicity and legibility, simplicity wins.

The one intellectual bet that makes Sooth more than a tidy Forth clone: **in a
stack language the stack discipline already is move semantics.** Every word
consumes its inputs and produces its outputs, so a value is moved by default and
keeping a copy requires an explicit `dup`. Affine types therefore drop in for
free: `dup` is the copy, and drop is a statically-known destructor point. That
single idea pays off three times over (resource safety, deterministic destruction,
data-race-free concurrency) and is the reason to write programs in Sooth rather
than in Forth or Rust.

## Why craft, not product

Recorded so the scope decision isn't re-litigated by accident.

- **No market gap for a general-purpose version.** Every axis a serious version
  would compete on is already held: memory-safe no-GC systems work by Rust,
  proven-real-time by Ada/SPARK, refinement+SMT by F*/Dafny/Liquid Haskell,
  effects by Koka, data-race-free actors by Pony. A new general-purpose language
  faces a brutal adoption bar against incumbents that improve for free every model
  generation.
- **"For agents, not humans" does not land on this language.** A language
  optimised for LLM authoring wants maximal familiarity (transferable corpus),
  explicit named intermediates, and cheap human/agent review. Concatenative is
  near-pessimal on all three: near-zero corpus, implicit global stack state (the
  exact thing models track worst), and point-free density that makes review
  expensive. The honest derivation of "best language for agent-authored safe code"
  lands on "a familiar C/Python-shaped surface with a strong effect/contract layer
  and a great structured-diagnostic protocol," which is a tooling project on top
  of a mainstream language, not Sooth.
- **Therefore, for production, use a mainstream language** (Rust when its
  guarantees are needed, Go/typed-Python when they aren't) and invest in the
  agent-loop tooling around it. Sooth is the hobby, kept honestly separate.

Nothing below is justified by market need. It's justified by being interesting to
build and to write in.

## Surface language

Concatenative, Forth-flavoured, with two non-negotiable ergonomics that Forth
lacks: statically-checked stack effects and named locals.

`gcd`, to fix the shape. `| a b |` binds the top items left-to-right in the same
order as the effect comment, and a word calls itself directly (no `recurse`):

```forth
: gcd ( a:int b:int -- int )
  | a b |
  b 0 = if
    a
  else
    b  a b mod  gcd
  then ;
```

Checked stack effects are the cheap, high-value feature: Forth's signature failure
mode (a silent underflow producing a wrong number at runtime) becomes a compile
error.

```forth
: oops ( a:int -- int )
  | a | a a + + ;
```
```
error: stack effect mismatch in `oops`
  declared ( a:int -- int ), but body has net effect ( a:int -- ⊥ )
  a a + +
        ^ `+` needs 2 values, stack holds 1 here (one `+` too many)
```

## The affine spine

Plain data is `Copy`: reuse is free and `dup` is ordinary.

```forth
: square ( n:int -- int )
  | n | n n * ;          \ int is Copy: naming n and using it twice is fine
```

A value is *moved* by default, and a resource is affine (not `Copy`). `dup` on
something that owns a resource is a type error. This is the whole point, and it is
where Sooth diverges from both Forth and Rust:

```forth
: leak ( f:File -- File File )
  | f | f dup ;
```
```
error: cannot `dup` a value of type File
  f dup
    ^^^ File is affine: it owns an OS handle and has no Copy instance
  note: `dup` on plain data copies bits; there are no bits to copy here.
        thread the File through, or open a second handle explicitly.
```

There is no borrow checker. Operations that don't consume a resource take it and
hand it back, which in a stack language is just normal data flow
(`size-of ( File -- File int )` returns the File):

```forth
: report ( path:str -- )
  | path |
  path open-read         \ ( -- File )         acquire ownership
  size-of                \ ( File -- File int ) hands the File back
  print                  \ ( File int -- File ) print consumes the int
  close ;                \ ( File -- )          destructor runs HERE
```

Deterministic drop, with no GC and no finalizer. Forget `close` and the program is
still correct: the File is still owned at end of scope, so the compiler inserts its
destructor at that statically-known point.

```forth
: report ( path:str -- )
  | path |
  path open-read
  size-of
  print ;                \ File never explicitly closed;
                         \ compiler drops it (runs close) exactly here.
```

## Type system: deliberately small

The value is in a few sharp, cheap features, not in a research-grade type theory.

**In:**
- Checked stack effects (the compile-time virtual-stack pass, needed for codegen
  anyway).
- Concrete monomorphic types: the numeric tower, `bool`, fixed arrays, slices,
  string slices, records/structs, enums/ADTs.
- Enough parametric polymorphism to give `dup`/`swap`/`max` honest signatures:
  type variables (`'T`) and a row variable (`..s`, the rest of the stack). This is
  Kitten-style row polymorphism, kept minimal.
- The `Copy` marker distinction (copyable vs affine). This is the load-bearing
  bit for the memory model and must exist early.

**Out, and why:**
- **Full HM inference**: not required for a craft language. Annotate stack effects
  explicitly (they double as documentation and as the LLM-nothing-to-do-here
  legibility win). Keeping each stack slot as `(value, type)` from day one leaves
  the door open to add inference later without a data-structure rewrite, but it is
  not a goal.
- **Refinement types + SMT (Z3)**: dropped. Great for a safety product, far too
  much machinery (and a heavyweight solver dependency) for a hold-in-head craft
  language. Division-by-zero and bounds become ordinary runtime checks or `Result`
  returns.
- **Effect rows**: dropped as a core feature. Effects are not tracked in the type
  system. (Concurrency safety is recovered structurally instead, see below.)
- **Dependent types**: never. Research-level over a concatenative calculus, and
  the payoff doesn't fit a craft language.

## Memory model

Ownership + affine types, deterministic drop, **no tracing GC**. Reference counting
is opt-in only (`Rc`/`Arc`-equivalent), reached for knowingly when shared ownership
is genuinely needed, because dropping the last ref cascades frees synchronously.

References are **second-class**, in the Hylo (mutable value semantics) mould, not
Rust's borrow checker: refs can be passed into a word but cannot be stored and
cannot escape their scope. Because they can't escape, no lifetime system is needed
to track them. Lifetimes attach to named bindings; stack values are anonymous and
shuffled by `swap`/`rot`, so a borrow checker is the worst possible fit here and is
deliberately avoided. Affine values plus non-escaping refs give most of the safety
with none of the lifetime apparatus.

Pointers are non-null by default; nullability is an explicit optional type. The
return stack is hidden or balance-checked; raw return addresses are never exposed.
FFI is the explicit unsafe hole, wrapped in safe words that establish invariants
(same discipline as Rust std over libc), and only exists at the hosted layer.

## Control flow and iteration

Boolean branching is `if ... else ... then` (which later becomes a combinator, see
below); structural dispatch is `match`. There are deliberately **no loop keywords** (no `begin/until`, `do/loop`);
dropping them keeps the surface small and matches the Factor/Kitten lineage, where
iteration is expressed with combinators rather than syntax.

The iteration story, top to bottom:

- **Quotations are the only iteration primitive.** A quotation `[ ... ]` is a
  first-class code value; `call` invokes it. This is the one piece that must be built
  into the language (new syntax + a value kind); it cannot be a library.
- **An internal loop primitive gives constant-stack iteration.** The IR has a
  loop/back-edge construct (not surface syntax, not user-facing). It is what makes
  iterating a large collection not overflow, independently of whether the TCO pass
  exists.
- **Combinators are library words, not keywords.** `each`, `map`, `filter`, `fold`,
  `while`, `times` are ordinary higher-order words that take quotations, written in
  Sooth on top of the loop primitive. A thin floor (one or two intrinsic combinators)
  bottoms out on the loop primitive; the rest are pure library. Reserving them as
  keywords would bloat the core for no reason.
- **Combinators are inlined.** The compiler inlines the common combinators and their
  quotation arguments at the call site, so `[ ... ] each` lowers to a tight loop with
  the body inlined, not a higher-order `call` per element. This is what makes "loops
  are a library" perform as well as loop syntax would have.
- **Raw recursion is legal but not the idiom.** A word may call itself; it is just a
  word. But threading the stack across a self-call by hand is fiddly, so combinators
  are the normal tool. Tail-call optimisation is therefore an optimisation for
  hand-written recursion, not the lifeline for iteration (see Open / deferred).

Iteration lands with quotations in Phase 4; Phases 0-3 have only shallow recursion,
which is enough for their goldens.

**Conditionals and dispatch.** Boolean branching is `if ... else ... then`. Structural
dispatch on ADTs is `match`, exhaustiveness-checked (a missing case is a compile
error). Multi-way branching is a **`cond` combinator** (a library word taking
`[ pred ] [ body ]` pairs), not syntax, so nested `if`s aren't the only option.
Haskell-style clause-based definitions with guards were considered and rejected: they
fit a stack language badly (Haskell matches named positional arguments, while Sooth's
inputs are anonymous stack values, the same named-vs-position tension that rules out
dependent types), and they replace the tiny `if` construct with a larger machine
(literal patterns + guards + clause sugar) without shrinking the language, since the
condition still has to be written somewhere.

**`if` becomes a combinator once quotations exist (Phase 4).** Phases 0-3 keep
`if/else/then` as syntax because they predate quotations. Once quotations land, `if`
is redefined as an ordinary combinator (`cond [ then ] [ else ] if`, Factor-style) and
stops being a keyword. This shrinks the core the honest way, by making `if` a word
rather than by replacing it with a bigger feature.

## Codegen and backend

Codegen model (unchanged from first principles, it's the good part): don't model
the data stack at runtime. Simulate it at compile time as an array of typed slots;
push/pop manipulate the array, and when IR is emitted the slots become ordinary
SSA/register values. Each word compiles to a function taking N stack args and
returning M results. `if`/`then` become basic blocks and branches; there are no loop
keywords (see Control flow and iteration), and iteration lowers to an internal loop
primitive with a back-edge. Branch and loop join points unify the virtual-stack state
(depth and type) across predecessors; mismatched depth or type across arms is a
compile error.

**No LLVM, and not a hand-written backend either. Decided: QBE.** The joy in this
project is the language and writing programs in it, not emitting machine code, so
codegen is offloaded to the smallest backend that stays legible. QBE (~15k lines you
could actually read) gives arm64/x86_64/riscv64 plus C-ABI struct classification for
free, and can carry essentially the entire design: everything interesting (affine
analysis, monomorphisation of the small polymorphic core, deterministic drop) is
frontend/runtime work QBE is agnostic to. A hand-written native backend (own the
vertical, direct syscalls) was the craft-purist alternative, set aside precisely
because it optimises for building the compiler, which isn't the point here. LLVM was
rejected outright: too large and opaque for a hold-in-head project, a perpetual
dependency tax, and product-grade output the language doesn't need. Wanting LLVM's
full-service codegen is a tell that the project has drifted back to product-think,
where the honest answer was "use Rust."

QBE's costs, accepted: it emits assembly text, so you depend on the system assembler
+ linker (a cross-toolchain + sysroot when cross-compiling the hosted layer);
i128/u128 are synthesised in the frontend (not QBE base types); atomics lower via FFI
to C11 atomics rather than a QBE primitive. Its modest optimiser is a feature, not a
bug: more predictable than LLVM's aggressive passes and friendlier to any later WCET
work.

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

**The one real casualty of dropping LLVM: no in-process JIT.** LLVM's ORC would have
let the REPL and AOT share one engine and let compile-time immediate words run as
native code. Without it:
- Compile-time / immediate words run in a small **interpreter** over the same IR,
  not JIT-compiled to native. This is a normal choice (most languages interpret
  macros) and keeps the comptime path simple.
- The REPL either batch-compiles snippets or compiles to a temp shared object and
  loads it. Higher latency than a JIT, acceptable for craft.

## Concurrency: a library, not a core feature

Only two things must be core intrinsics; everything else is a library.

**Core intrinsics (cannot be synthesised from below):**
- **Atomics + memory ordering** (compare-and-swap, acquire/release). Codegen must
  respect them as barriers. On the from-scratch backend, emit LL/SC (arm64) or
  `LOCK`/CAS (x86) directly; on QBE, lean on FFI to C11 atomics or hand-written asm
  (QBE's atomics story is thin).
- **A spawn primitive.** Can be a thin FFI to `pthread_create` at the hosted layer
  rather than a language feature.

**Free from the affine spine:** data-race freedom. Sending a message *moves* the
payload (the same `dup`-is-the-copy rule across a thread boundary), so there is no
shared mutable aliasing by construction. Second-class refs can't cross a thread
boundary because they can't escape a scope at all, so the dangerous case is already
forbidden by machinery built for resources. No separate `Send`/`Sync` apparatus
needed.

**Library (the whole model as the user sees it):** channels, mutexes, condvars,
pools, futures, and actors (a mailbox + a loop + move-only messages). Split-endpoint
channels so nothing needs to be `dup`ed:

```forth
\ intrinsics: spawn ( q -- Thread ), cas ( p a b -- bool ). the rest is library.

: worker ( ch:Recv[Job] -- )
  | ch |
  begin
    ch recv            \ ( -- ch Job )  ownership of the Job MOVES to us
    handle
  again ;

: main ( -- )
  chan                          \ ( -- Send[Job] Recv[Job] )  two affine ends
  swap [ worker ] spawn drop    \ Recv end moves into the spawned quotation
  ... ;                         \ Send end stays here, still owned, race-free
```

Note `spawn [ ... ]` uses an escaping quotation, so the convenient concurrency
library is an alloc/hosted citizen. A `no_std`/real-time concurrency library is a
distinct, more constrained one (static topology, fixed mailboxes, no escaping
captures). Concurrency-as-a-library is two libraries, not one.

## Real-time: capable, not guaranteed

The core is a strong real-time foundation, better than most, because the two hardest
RT properties come for free: no GC (no stop-the-world pauses) and deterministic drop
(destruction at a statically-known time). Errors-as-values means no unwind-path
jitter either.

What was *not* built is the RT *guarantee* machinery:
- **Soft real-time** (audio, games, robotics tolerant of occasional misses) works
  essentially out of the box.
- **Hard real-time** (deadline miss = failure) is achievable by discipline, not
  turnkey. You add a heap-free layer, a bounded-mailbox/static-topology concurrency
  library, and carry the WCET reasoning yourself. The enemies to keep off hot paths
  are dynamic dispatch through escaping quotations and spawning during the loop;
  the fix is static topology (spawn at init). Nothing in the design fights this; it
  just isn't enforced by a type/effect system the way a product version would.

The RT-safe subset is exactly: core + the fixed (no-alloc) layer + the no-escape
concurrency library.

## `no_std` core and layering

The core is `no_std` and everything else layers on top. This is the honest shape of
the language, not a concession: a tiny `no_std` core is both the hold-in-head craft
object and the thing that runs on a microcontroller with no OS. The hosted layer
(files, threads) is the optional convenience, not the foundation.

```
core       stack semantics, numeric tower, bool, fixed arrays + slices,
           string slices (rodata), affine/move/drop, checked stack effects,
           control flow, modules, non-escaping quotations, atomics,
           second-class refs, and the allocator *interface* (not an impl).
           assumes: a few compiler intrinsics, no allocator, no OS.

fixed      fixed-capacity vec/map/string/ringbuffer, bounded mailboxes.
(no-alloc) allocation-free, real-time-safe. the embedded sweet spot.

alloc      growable Vec/Map/String, Box, opt-in Rc/Arc, escaping closures,
           bignum. needs an allocator satisfying core's interface.

hosted     files, stdio, time, net, OS thread spawning, FFI-to-libc,
           blocking channels. needs an OS.
```

Placements worth noting: atomics are core but spawning threads is hosted; string
slice is core but growable `String` is `alloc`; a non-escaping quotation is core but
an escaping one is `alloc`; the allocator *interface* is core, its *implementations*
(arena, pool, malloc) are not.

Discipline: fix the layer boundaries and the allocator interface on day one and tag
every stdlib word with the layer it needs, even though the hosted layer is built
first (that's where dogfooding happens). Carving `no_std` out later is the retrofit
tax Rust paid early; avoid it. And `no_std` core is not "runs on nothing": it still
assumes a handful of intrinsics (memcpy/memset for moves, integer-divide and
soft-float helpers where there's no hardware, the atomics) plus a per-target linker
script and entry point.

## Liveness and the craft discipline

The failure mode with this project's name on it is a beautiful half-built compiler
that no one ever writes a program in, because building the compiler is more
immediately rewarding than using the language. The antidote is a hard requirement:

- A REPL / immediate feedback from day one.
- Write real small programs in Sooth (a raymarcher, a text adventure, the classic
  demos, an LED blink on bare metal) *before* the compiler is "done."

If it's fun to play with early, it gets finished. If it's only fun to build, it
won't.

## Bootstrap and implementation

Self-hosting is a milestone, not a day-one constraint (rustc/Go/TypeScript
precedent): thick host-language compiler first, rewrite in a defined Sooth subset
later, verify by fixpoint.

Dropping LLVM and Z3 removes the constraints that previously forced the host
language, so the choice is now preference, not necessity. **Rust remains the
sensible default** (ADT + pattern-matching-heavy compiler workload, and `no_std`
for the runtime/intrinsics library in the same language), but nothing now *requires*
it. Keep the bootstrap clean and un-clever so it translates to the self-hosting
subset rather than idiomatic host-language code with no analog.

The self-hosting subset is smaller than before precisely because the language is
smaller: concrete monomorphic types + ADTs + pattern matching, growable collections
+ strings, words + modules, errors as values, and a modest C FFI (now only for the
OS/hosted layer, not for a solver or LLVM). No inference, no refinements, no effect
rows, no borrow analysis needed to write the compiler in it.

## Decided

- Scope: craft language, not a product. Optimise for legibility, hold-in-head size,
  and the joy of building and writing it.
- Signature idea: affine by default, `dup` is the explicit copy, drop is a
  statically-known destructor point.
- Surface: concatenative, Forth-lineage, checked stack effects, `| named locals |`.
- Control flow: `if/else/then` for boolean branching (becomes an ordinary combinator,
  `cond [ then ] [ else ] if`, once quotations land in Phase 4); `match` for exhaustive
  structural dispatch on ADTs; a `cond` combinator (library word) for multi-way
  branching. No clause-based definitions (bad fit for a stack language). No loop
  keywords.
- Iteration: quotations (`[ ]` + `call`) are the sole primitive; lowers to an internal
  loop primitive for constant stack; combinators (`each`/`while`/`fold`/`times`/`map`)
  are library words built on quotations and inlined at call sites. Raw recursion is
  legal but not the idiom.
- Type system: small. Concrete types + ADTs + minimal row polymorphism + a `Copy`
  marker. No full HM inference, no refinement/SMT, no effect rows, no dependent
  types.
- Memory: ownership + affine, deterministic drop, no GC, RC opt-in; second-class
  refs (Hylo-style), no borrow checker; non-null pointers; hidden/checked return
  stack.
- Codegen: compile-time virtual stack to native; words as functions.
- Backend: QBE (small, legible, multi-arch native + C ABI for free); no LLVM and no
  hand-written native backend, because the joy is the language, not codegen. WASM is
  a sibling lowering off the neutral IR via binaryen, not routed through QBE. IR keeps
  `Ptr[T]` abstract so both lowerings concretise it. No in-process JIT: comptime/
  immediate words run in an interpreter; REPL batch-compiles or dlopens.
- Errors as values, no THROW/CATCH, no unwinding.
- Concurrency: library, not core. Only atomics + spawn are intrinsics; data-race
  freedom is free from affine + non-escaping refs.
- Real-time: soft-RT out of the box; hard-RT by discipline (fixed layer + static
  topology), not by enforced guarantee.
- `no_std` core with core / fixed / alloc / hosted layering; allocator interface in
  core; seams fixed day one.
- Bootstrap: host-language compiler then self-host a small subset, fixpoint-verify.
  Host language now free choice; Rust the sensible default.

## Open / deferred

- Exact surface syntax for quotations/closures and their captures (illustrative
  above, not settled).
- Whether to add optional HM inference later (kept possible by the `(value, type)`
  slot representation, not planned).
- Immediate-word / defining-word facility (typed, on the comptime interpreter),
  replacing `CREATE`/`DOES>`. Deferred to implementation planning.
- Tail-call handling. With loops dropped and iteration provided by combinators over an
  internal loop primitive, constant-stack iteration no longer *depends* on TCO (the
  loop primitive gives it). TCO is demoted to a pure optimisation for user-written
  recursive words in tail position: compile a *self*-tail-call as a jump to the entry
  block. General/mutual TCO needs a trampoline or backend support QBE lacks, and can be
  deferred indefinitely. Interacts with deterministic drop: the transform must run the
  outgoing frame's destructors before the jump, so affine ownership and TCO have to be
  co-designed.
