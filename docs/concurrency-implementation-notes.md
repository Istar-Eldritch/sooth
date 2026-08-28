# Concurrency implementation notes

Exploration, not decision. Records the implementation shapes the design permits
for the concurrency library, to give a future implementation a starting point and
to surface design questions that would bite during implementation. Nothing here
overrides a decision in DESIGN.md; it fills in mechanism the design leaves open.

Extends the "Concurrency: a library, not a core feature" section of DESIGN.md,
which names spawn + atomics as the two core intrinsics and lists "channels,
mutexes, condvars, pools, futures, and actors (a mailbox + a loop + move-only
messages)" as library words. The actor model discussed here is that library
entry given concrete shape, not a new language feature.

## The scheduling mechanism

### The event loop is the `vm.sth` pattern

`examples/vm.sth`'s `run` word is already an event loop: a self-tail-recursive
word with clause dispatch over an enum (`Op`), state bundled in a struct (`Vm`)
that rides the back-edge, in-place mutation through `&!` references. Replacing
`Op` with an `Event` enum (the `epoll_wait`/`kqueue` output) and `Vm` with a
`LoopState` struct yields an event loop. The back-edge is proven at 1M+
iterations in constant stack (Slice 6).

The syscall FFI is expressible today: `epoll_create1`, `epoll_ctl`, `epoll_wait`
are `extern:` declarations in the same shape as the `open`/`read`/`close` in
`examples/resources.sth`. No language feature is missing for the bounded event
loop; a fixed-size `[Conn N]` array as the connection table works with existing
arrays + `usize`. A growable `Map<fd, Conn>` table waits for Phase 9's `alloc`
layer.

### Coroutines via `ucontext`

Real coroutines need a context switch primitive (save registers, swap the stack
pointer, jump). This is not expressible in pure Sooth: words are native frames on
the real machine stack, the compile-time virtual stack is compile-time only, and
the language gives no handle on a runtime frame. The mechanism is
`ucontext`'s `swapcontext`/`makecontext` (POSIX) or ~100 lines of assembly per
architecture (`libco`-style), reached via `extern:`:

```forth
extern: swapcontext ( &!ucontext &ucontext -- i32 ) "swapcontext" ;
```

Wrapped as ordinary Sooth words (`spawn`, `resume`, `yield`), this gives real
stackful coroutines: a `yield` inside a blocking-style word swaps back to the
scheduler; a `resume` swaps back in exactly where it left off. The user writes
sequential-looking code; the library parks and resumes it.

This is the same mechanism Lua, `libco` (RetroArch), and early Go use. It is a
library, not a language feature, consistent with the design's "concurrency is a
library" stance. The context-switch primitive is arguably a third hosted-layer
intrinsic alongside the spawn and atomics the design already names, or an FFI
to a small C/asm routine — either is consistent.

### The cancellation problem

A suspended coroutine holds live linear values on its stack (`Conn`, `File`,
buffers). Freeing the stack leaks them; the linear spine requires each be
consumed exactly once. So `cancel` means **resume-to-teardown**, not
free-the-stack: a cancelled coroutine is handed a cancellation signal and runs
its own `drop` logic normally before its stack is released.

This is the same shape as Rust's `Drop` running on task cancellation, sharpened
by Sooth's "nothing auto-drops" invariant: in Rust a dropped value's `Drop`
runs automatically, but in Sooth the destructor fires only where the programmer
wrote the disposal, so cancellation must drive the teardown explicitly rather
than relying on implicit drop. Solvable, but it must be designed for: the
scheduler resumes a cancelled coroutine in a cancellation mode and lets it run
to completion. Get this right and the file can't leak and the fd can't
double-close — the same guarantees the spine gives everywhere.

### M:1 vs M:N

**M:1 (single OS thread, cooperative).** The event loop *is* the scheduler: a
run queue of ready coroutines, a current-coroutine pointer, and a `yield` that
enqueues the current one and dequeues the next. Fully doable as a library after
Phase 4 (quotations, for `spawn [ body ]`) and Phase 9 (growable queues, heaps
for stacks). A fixed-capacity pool of fixed-stack coroutines works with what
exists today.

**M:N (multi-core, work-stealing).** A Chase-Lev work-stealing deque over the
atomics intrinsic, one scheduler per OS thread (spawned via the `spawn`
intrinsic), stealing from siblings when the local queue is empty. Expressible
with committed features, but a correct work-stealing scheduler is genuinely
hard to get right in any language, and Sooth has no `loom`-style concurrency
sanitizer — memory ordering bugs are debugged with `printf` and stress tests.
Defer to a tested implementation rather than hand-rolling.

### Fixed-stack limits

No preemption: timer-interrupting a coroutine mid-computation needs `sigaltstack`
and a signal handler that safely swaps context, which is fragile with
`ucontext` (it's the part even Go moved away from, switching to cooperative
safepoints plus a watchdog).

No stack growth: Go's grow-on-overflow stacks need either segmented stacks or a
copying collector that can relocate a stack. Sooth has no GC, by design. So
coroutines are fixed-stack, cooperative-yield-only. Size generously (8–64KB
per coroutine) and use iterative algorithms for deep work. The self-tail-call
guarantee (Slice 6) means the common iteration pattern — the dispatch loop, the
`each`/`map` combinators — is stack-safe and doesn't grow the stack. The risk is
non-tail recursion on deep input (a recursive JSON parser on deeply nested
data); the fix is the idiomatic Sooth shape anyway: iterative with an explicit
worklist.

The concurrency ceiling is memory-bound, not architectural: 300 coroutines at
64KB each is ~19MB, trivial. 10,000 is 640MB, where async state machines use
~100–200 bytes each. For a personal product with bounded concurrency this is
irrelevant; it's the wall that would push toward M:N or async state machines at
production scale.

## Actors as the primary model

### Actors are a scheduling policy, not a mechanism

An actor is the `worker` word from DESIGN.md: a mailbox loop that processes one
message per iteration. The scheduler (M:1 event loop or M:N work-stealing) runs
actors; actors don't replace the scheduler, they sit on top of it. Erlang's
actors run on BEAM's preemptive scheduler; Akka's run on a JVM `ForkJoinPool`;
Sooth's would run on the event loop or green-thread scheduler described above.

### `recv` is the natural yield point

A raw coroutine system requires explicit yields, and a forgotten yield in a
long computation blocks every other coroutine. An actor's shape is `recv; handle`
in a loop — `recv` blocks, and blocking is the yield. The actor loop yields on
every message boundary by construction. No forgotten-yield footgun, and the
mechanism underneath is the same coroutine swap.

The `worker` example in DESIGN.md uses `begin ... again` pseudo-syntax (loop
keywords the language deliberately doesn't have). The actual implementation
shape is a self-tail-recursive word (the `vm.sth` `run` pattern) until
quotations land in Phase 4, then a `[ body ] loop`-style combinator.

### Move-based messaging: the linear spine advantage

This is where Sooth actors are not just different from Erlang and Akka but
genuinely better, and the reason is the language's central idea:

- **Erlang** isolates actors by *copying* every message. Safe, but the runtime
  cost is proportional to message size, and it's why Erlang's message passing
  is slower than its reputation suggests.
- **Akka** passes mutable Java object references between actors. The sender can
  mutate the object after sending it; both actors now alias the same mutable
  state. You discipline your way out of it; the language doesn't help.
- **Sooth** *moves* the message. Use-after-`send` is a compile error
  (use-after-move), and the receiver owns it exclusively. No copy, no alias, no
  discipline required.

The actor isolation guarantee — the one Erlang pays copies for and Akka leaves
to convention — is a compile-time property of the linear spine, free.

### Supervision and cancellation

A supervised actor is a linear `Thread` value owned by its supervisor. When the
actor dies (a runtime trap: bounds check, OOM, stack overflow), the supervisor's
`drop` on the dead thread runs teardown — the cancellation discipline from the
coroutine section, now driven by the type system rather than by convention. A
crashed actor's `File` handles and buffers get their destructors; nothing leaks.
Supervision is the natural resolution of the cancellation problem: the same
resume-to-teardown mechanism, triggered by the supervisor disposing the dead
actor's linear handle rather than by an explicit `cancel` call.

### Actors as sole model vs one option

Erlang's bet was actors-only: no shared memory, no mutexes, just processes and
messages. This is a stronger guarantee (no data races because there's no shared
memory, period) but a stricter constraint (can't share read-only data like
configuration without sending it, which means copying or refcounting).

Sooth's linear spine already gives data-race freedom without actors: a linear
value can't be aliased at all, so two threads can't share mutable state by
construction. Actors don't buy safety the spine doesn't already provide — they
buy fault tolerance (supervision, let-it-crash) and location transparency
(actors that could be on another machine). For a personal product, a natural
shape is actors for the connection-handling layer (where supervision and
mailbox semantics pay off) plus raw linear values and split-endpoint channels
for internal coordination (where the overhead of actor wrapping adds nothing).
Both are equally safe.

## What stays deferred

- **M:N work-stealing.** Expressible with committed features (atomics + spawn),
  hard to get right, no concurrency sanitizer. Defer to a tested implementation.
- **Preemption.** Fragile with `ucontext`, not worth the complexity. Cooperative
  yield only.
- **Stack growth.** Needs a GC (ruled out) or segmented stacks. Fixed stacks.
- **Distributed actors / location transparency.** Real but out of scope until a
  concrete need. The move-based messaging model extends naturally (a network
  message is a serialized move), but the serialization, failure detection, and
  location-routing machinery is a project of its own.
- **Worklist-based disposal for branching structures** (Phase 9, per ROADMAP).
  Independent of concurrency but interacts: a work-stealing scheduler's pending
  structure and a branching structure's disposal worklist are the same shape.

## Relationship to existing decisions

- **Extends "Concurrency: a library, not a core feature" (DESIGN.md).** Fills in
  implementation mechanism the design leaves open. The two intrinsics (spawn +
  atomics) are unchanged; everything here is library on top of them.
- **Interacts with "Mutual tail-call elimination (tier 2)" (Open/deferred).** An
  actor's mailbox loop is self-tail-recursive (Slice 6), so it's stack-safe
  today. Mutual recursion between actors (A sends to B, B sends back to A) does
  not need mutual TCO — each actor is an independent self-recursive loop, and
  the message handoff is a channel operation, not a tail call.
- **Interacts with "Late binding for redefinition" (Open/deferred).** No
  redefinition path exists today, so this is moot in practice: a running
  actor's word binding is simply the one compiled into the binary. It becomes
  a live question only if a future `driver::Library`-based reload path
  introduces redefinition, at which point a running actor's handler should
  stay bound to the word it started with rather than pick up a new one.
- **Interacts with "No loop keywords" (DESIGN.md).** The `begin ... again` in
  DESIGN.md's `worker` example is illustrative pseudo-syntax. The real shape is a
  self-tail-recursive word (today) or a quotation-based loop combinator (after
  Phase 4), both of which are already in the design.
- **The context-switch primitive** is a third hosted-layer FFI (or intrinsic)
  alongside spawn and atomics. It is not a language feature and does not require
  any change to the type system or the surface syntax; it is `extern:` plus
  library words, the same shape as `resources.sth`'s FFI to libc.
