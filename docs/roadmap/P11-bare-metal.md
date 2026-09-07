[← ROADMAP](./ROADMAP.md)

### Phase 11 — Bare metal  `[M]`  `[the craft milestone: own the vertical to the metal]`

Cross-compile to arm64 (or Cortex-M) bare metal: per-target intrinsics
(memcpy/memset, integer-divide/soft-float helpers), linker script, entry point,
`no_std` core + `fixed` layer on-device, soft-float lint. Soft-real-time works out
of the box; demonstrate hard-RT-by-discipline (fixed layer + static-topology
concurrency, no allocation or spawning on the hot path) if you want it.
**Exit:** a program running on real hardware or QEMU with no OS and no allocator,
blinking an LED or driving a sensor, from your own source language down to the
machine code you emit.

The named-memory declarations below are P11's prerequisite slice pair: everything
else in this phase (per-target intrinsics, the linker script, the entry point) is
built on top of them, and `at` specifically needs the linker script to exist.
Design lineage: `examples/experiments/uart_mmio.sth` is the sketch these slices
realize; the surrounding access discipline it assumes (field projection through
`&!`, bitfield `get-bits`/`set-bits`, the captured-borrow `spin` idiom) already
exists.

## Slices

**P11.S1 — `data:` declarations (the `static:` rename, struct/array-typed data,
`at` placement).**
Renames `static:` to `data:`. The old name imports C/Rust baggage Sooth doesn't
have (linkage, lifetimes), collides with the language's own *static dispatch*, and
the compiler literally emits QBE `data` — the keyword should match the mechanism.
`global:` access clauses are unchanged: `data:` says what storage exists, `global:`
says who touches it. Widens the declaration's type grammar from the four-scalar
allow-list (D1/OQ1) to struct and array types: the parser stays single-pass (admit
a type name, resolve late — `check_static_decls` already re-tests resolved shapes
post-parse), and the emitter writes aggregate `data` per `layout.rs` (`str`
statics already emit descriptors). Adds the `at <addr>` placement clause for
program-owned storage the linker must pin (DMA buffers, tables at fixed RAM
addresses): QBE `data` has no address attribute, so `at` is linker-fragment
emission and lands with this phase's linker script. Settles the drop ruling
struct-typed data forces: whether linear payloads in `data:` declarations drop at
program exit (hosted target) or drops stay explicit-only.
**Exit:** `static:` is gone — `data:` everywhere, examples and living docs
migrated (historical spec prose stays as written), module-privacy semantics
unchanged; struct- and array-typed `data:` emit correct aggregate `data`; an
`at`-pinned declaration compiles to a placed symbol through the emitted linker
fragment; the drop ruling is recorded with a golden.

**P11.S2 — `reg:` declarations and the `Volatile` marker (device memory).**
`reg:` names memory the program never owned:
`reg: UART Uart at 0x40008000 ;` — no storage, no initializer, no drop, and every
access through it is a volatile load/store *by definition*: the declaration is the
trust boundary, so there is no marker to forget and no silent failure mode. `at`
is required (a reg has no placement of its own); single-register declarations
(`reg: FLAGS u32 at ...`) are legitimate — the earlier "wrap it in a struct"
hygiene rule was an artifact of marking types, not of the mechanism. Field
projection through a reg-derived ref lowers volatile per-field accesses; Blit
through a reg-derived aggregate is a located error (memcpy across a register
window is wrong in C too); and volatility must ride ref provenance from the root
`reg:` declaration into captured borrows — the experiment's `spin` polling a
borrowed status field is the pinned idiom. The sibling case — program *owns* the
storage but an external agent mutates it (DMA rings, cross-core mailboxes in
owned RAM) — stays `data:` plus the `Volatile` marker: a reserved, zero-member,
compiler-recognized trait granted by ordinary `impl:` (a *declared* marker mode,
distinct from `Copy`'s structural derivation), queried by the load/store lowering
at monomorphization against P7b.S2's ctor-keyed registry. The hook keys on the
aggregate constructor being projected — a `&!u32` field of a marked block is
volatile, `u32` itself is not — and generic words come free because the query
rides per-instantiation monomorphization. A `data: ... at ...` declaration whose
type lacks the `Volatile` impl warns. Volatile is not a barrier: a separate
`fence` intrinsic word ships here so the two are never conflated. Engineering
check before the brief: whether the QBE build in use accepts volatile
loads/stores, or the shim is a compiler-barrier emission. Prior art note: no
mainstream language makes this split a declaration kind — Ada's
`Import`/`Address`/`Volatile` aspect bundle is the nearest cousin; the novelty is
deliberate and the keyword names the ontology (`reg:`), not the derived property
(`volatile`).
**Exit:** the uart_mmio sketch compiles against real `reg:`/`data:` declarations
and drives a UART on the QEMU target; every reg access lowers to a volatile
load/store in the emitted QBE IL; a `Volatile`-marked DMA ring's accesses are
volatile and unmarked accesses to the same type are not; `fence` orders register
access against DMA; Blit through a reg-derived ref is a located error.
