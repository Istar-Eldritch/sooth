# Sooth — concurrency and real-time

Design detail for concurrency and real-time, split from [DESIGN.md](../../DESIGN.md).

## Concurrency: a library, not a core feature

Only two things must be core intrinsics; everything else is a library.

**Core intrinsics (cannot be synthesised from below):**

- **Atomics + memory ordering** (compare-and-swap, acquire/release). Codegen must
  respect them as barriers. QBE gets patched with real per-target CAS/fence ops (x86
  `LOCK CMPXCHG`, arm64 `LDAXR`/`STLXR` or LSE `CAS`, rv64 `LR.W`/`SC.W` or `A`-extension
  AMOs) rather than FFI to libc, sharing the volatile patch's touch points (the `Ins`
  flag bit, `load.c`/`gvn.c`'s dedup passes, each target's `isel.c` dead-result guard).
  **No single codegen strategy across targets**, and the embedded ones are where a
  uniform one breaks: LL/SC/AMO where the ISA has it (arm64, RISC-V with `A`); a critical
  section by interrupt masking (`cpsid i`/PRIMASK) where it doesn't (ARMv6-M has no
  LDREX/STREX, a RISC-V core without `A` has neither AMO nor LR/SC) — the same technique
  `libatomic` and Rust's `portable-atomic` use on those cores, sufficient against
  anything that can only *preempt* (an ISR against mainline code on the same core); and a
  hardware spinlock where masking can't reach, since masking is per-core (RP2040's SIO
  spinlocks for its two Cortex-M0+ cores). Fences order accesses, they do not exclude a
  concurrent one, so they aren't a fourth option.
- **A spawn primitive.** Can be a thin FFI to `pthread_create` at the hosted layer
  rather than a language feature.

**Free from the linear spine:** data-race freedom. Sending a message *moves* the
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

: worker ( Recv[Job] -- )
  | ch |
  begin
    ch recv            \ ( -- ch Job )  ownership of the Job MOVES to us
    handle
  again ;

: main ( -- )
  chan                          \ ( -- Send[Job] Recv[Job] )  two linear ends
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

**One piece of that moved from discipline to enforcement**: unsynchronised sharing
between an interrupt handler and mainline code is a checked error, via the global-set
analysis below. WCET reasoning stays the programmer's. **Ravenscar is the reference to
check the RT concurrency library against**, not to derive it from scratch: Ada's
restricted tasking profile (static task topology, no termination, no dynamic
priorities, one entry per protected object, mandatory ceiling locking) is the same
shape as "static topology, fixed mailboxes, no escaping captures", except it has
already survived DO-178C level A certification in avionics and space. Deviating from
it is allowed; deviating without noticing is not.
