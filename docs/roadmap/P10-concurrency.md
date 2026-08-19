[← ROADMAP](./ROADMAP.md)

### Phase 10 — Concurrency (library)  `[M]`

Core intrinsics only: **atomics + memory ordering** and a **spawn** primitive (thin
FFI to `pthread_create` at the hosted layer). Everything else is library:
split-endpoint channels, mutexes, pools, and actors (mailbox + loop + move-only
messages). Data-race freedom is inherited from the linear spine (send = move) and
non-escaping refs, no separate `Send`/`Sync` apparatus. Ship two libraries: the
convenient hosted one and a constrained `no_std`/RT one (static topology, fixed
mailboxes, no escaping captures).

**Atomics are a QBE patch, not FFI.** Real per-target codegen (x86 `LOCK CMPXCHG`, arm64
`LDAXR`/`STLXR` or LSE `CAS`, rv64 `LR.W`/`SC.W` or `A`-extension AMOs), landing together
with the volatile patch since both touch the same handful of files: the spare flag bit
already available on `Ins`, `load.c`/`gvn.c`'s dedup passes (mustn't ever treat an atomic
op as a redundant/foldable load), and the dead-result guard duplicated across
`amd64/isel.c`, `arm64/isel.c`, and `rv64/isel.c` (no longer a single shared `isel.c` in
canonical QBE). `~/code/qbe` tracks canonical upstream (`git://c9x.me/qbe.git`), so the
fork has a real base to patch against.

**Three codegen strategies, chosen per target, not a language-level choice.** LL/SC or AMO
where the ISA has it (arm64, RISC-V with `A`). A critical section by interrupt masking
where it does not: ARMv6-M has no LDREX/STREX at all, and a RISC-V core without `A` has
neither AMO nor LR/SC; this is what `libatomic` and Rust's `portable-atomic` already do on
those cores, and it is sufficient against anything that can only *preempt*, which covers an
ISR against mainline code on the same core. A hardware lock where masking cannot reach,
since masking is per-core: RP2040 is dual Cortex-M0+ and provides SIO spinlocks for exactly
the cross-core case. Fences order accesses and do not exclude a concurrent one, so they are
not a fourth option and cannot stand in for the missing instructions.
**Check the RT library against Ravenscar rather than deriving it from scratch.** Ada's
restricted tasking profile is the same shape already described here (static topology, no
task termination, no dynamic priorities, one entry per protected object, mandatory ceiling
locking) and has been through DO-178C level A in avionics and space. Deviating from it is
fine; deviating without noticing is not.
**Exit:** concurrent programs that are data-race-free by construction; a deliberate
attempt to alias a sent value is a compile error.
**Dogfood:** a small worker-pool or a producer/consumer pipeline.

