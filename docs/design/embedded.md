# Sooth — embedded: statics, MMIO, and interrupts

Design detail for embedded/RT features, split from [DESIGN.md](../../DESIGN.md).

## Embedded: statics, MMIO, and interrupts

Four things the language has no answer for, all of them prerequisites for the bare-metal
milestone rather than extras on top of it. Ada is the prior art throughout, because it is
the one language that solves all of this *in the language* rather than with macros (C) or
generated `unsafe` peripheral crates (Rust).

**Static storage is a third category, not a storage class.** The linear spine assumes a
value has one owner that moves it forward and one checked `drop` endpoint. A static has
neither: it exists before `main`, is reachable from arbitrarily many call sites, and is
never consumed, because an embedded device does not shut down. So a static is a *place*,
not a value: never owned, never moved, never dropped, reached only through a second-class
ref. `Copy` already carves one exception into linearity, for cheap duplicable values; this
is a second one, for permanent mutable state, and the must-consume rule needs an explicit
carve-out saying so rather than an accident that happens to typecheck. Initialisers are
compile-time constants (literals, zero) only, which is not a new restriction to argue for:
it falls straight out of "no comptime interpreter". Ada tiers this (`Pure` / `Preelaborate`
/ arbitrary startup code with binder-computed ordering); constants-only is the
`Preelaborate` tier, and the tier above it is available if that ever proves too tight.
Static state is declared at module level where it is nameable, never hidden inside a word
the way C's function-local `static` allows, because the analysis below has to be able to
name it.

**MMIO is a typed overlay with a volatile aspect, not a cast.** Two flavours of static,
and they are not the same feature: storage the compiler allocates (a ring buffer, a
counter), and a *fixed address the hardware defines* with a type asserted onto it. The
second is closer to `extern:` than to a variable, and the declaration site is the trust
boundary exactly as it already is for foreign calls, so there is still no separate `unsafe`
marker. Volatile is a property of the declaration, not a hope: for a register the access's
*existence* is the side effect, so the backend may never elide, coalesce, or reorder it.
**QBE does not model volatile**, so a discarded load, two loads of the same address, and a
store followed by a load of the same address are all fair game for elimination, CSE, and
forwarding respectively — all three matter for real registers (clear-on-read status bits, a
FIFO data register, write-then-verify), so QBE gets patched with a `vol` flag rather than
routing every access through an opaque call. `struct Ins`'s `op:30` field has a spare bit
to give it at zero size cost. Three sites need to honour it: `load.c`'s redundant-load/
store-forwarding elimination, `gvn.c`'s `dedupins` (must never call a volatile op equal to
anything, including itself elsewhere), and the dead-result guard duplicated across each
target's own `isel.c` (amd64/arm64/rv64, no longer one shared file). GCM's block-pinning
already covers in-block ordering for free, since loads and stores are `pinned` in the op
table, so a volatile access spinning in a loop is not hoisted out without any new work.
Copy Ada's declaration-side triple wholesale: an address clause, a `Volatile` aspect, and
record representation clauses that lay a control register out to the bit, so a register is
a typed record with named fields instead of hand-rolled shifts and masks.

**An ISR is an exported symbol, not a called word.** On Cortex-M the hardware stacks the
caller-saved registers on exception entry, so a handler is an ordinary C-ABI `void(void)`
function reached through a vector table. What's missing is only a way to give a word a
fixed symbol name and linker section, which is a linker/attribute mechanism rather than a
language design problem.

**Shared state between an ISR and mainline code is the hard part, and it gets an
analysis.** An interrupt has no call site, so there is no move point at which ownership
could transfer: both sides genuinely touch the same object, which is precisely what the
rest of the language is built to prevent. What makes it tractable is a **global set** per
word, the statics it touches and in what mode, computed bottom-up over the call graph. The
hazard is then a set intersection: any static reachable from both an interrupt handler and
mainline code needs masking or a protected wrapper. Without that set the hazards cannot
even be enumerated, which is why C and embedded Rust both leave this to human discipline.
Ada goes further and makes the mutual exclusion structural (a handler *must* be a protected
procedure, and ceiling locking compiles to interrupt masking); whether the wrapper here is
a library type or a language construct is open, but the analysis is needed either way.

**Inferred everywhere, declared at boundaries.** Global sets are inferred within a module
and *declared* on exported words and ISR-attached words. That line is not a staging plan,
it is where visibility ends: inside a module the compiler sees every body, across a module
boundary a caller cannot. The argument for declaring is not documentation, it is blame
localisation, and it is the same argument that already made stack effects declared rather
than inferred: an inferred contract reports a violation wherever it surfaces, not where the
mistake was made, so an access added three words deep is silently absorbed and detonates at
an ISR boundary the author never read. A declaration is a ratchet, catching the change in
contract at the moment of the change. It also buys SPARK's `Abstract_State` benefit for
free: an exported word declares that it touches module state without publishing which
variables, so refactoring internals doesn't churn callers' contracts. Inferring internally
is what keeps the annotation burden off every intermediate helper, which is the specific
friction that makes real Ada projects skip `Global` contracts.

**This is a global clause, not effect rows, and the difference is the whole budget.**
Effect rows mean row *variables*, polymorphism, unification, and inference threaded through
the type system, which the type system section declines. A global set is a closed
monomorphic list of names with modes, checked by set inclusion; the bottom-up computation
is a fixpoint over the call graph, not HM unification, so "no inference" does not settle it
either way. Higher-order code is normally exactly where that distinction collapses, since
`each`'s effects would have to be whatever its quotation's are, and Sooth escapes it
through a decision already made for other reasons: combinators are **inlined at call
sites**, so `each` never exists as a separate callee needing a polymorphic contract. The
exception is escaping quotations, which cannot be inlined, and those are already
`alloc`-layer and already excluded from the RT subset. The restriction lands exactly on a
boundary that exists anyway, which is the reason to believe the narrow version holds rather
than sliding into the general one.
