[← ROADMAP](./ROADMAP.md)

### Phase 9 — The stdlib layers  `[L]`  `[where it becomes usable for real programs]`

The layers from DESIGN.md, each built as a package with its layer declared in its manifest
and the dependency direction checked: **core** (already accreting, restructured into
packages by Phase 8), **fixed** (allocation-free fixed-capacity vec/map/string/ringbuffer),
**alloc** (growable Vec/Map/String, Box, opt-in Rc/Arc, escaping closures, bignum, against
core's allocator interface), **hosted** (files, stdio, time, FFI-to-libc via safe wrappers).
Escaping closures appear in that list as a *layer tag*, not as unbuilt work: the feature
itself lands in Phase 4 Slice 7 on `^`, and what belongs here is only the classification
that a closure which escapes its frame needs an allocator present, so it is unavailable to
the `fixed` layer.

Building these as packages rather than as files is what pressure-tests Phase 8: the layering
claim is verified by the build rather than asserted in prose, and `core` is demonstrably
usable with no allocator because nothing in it may depend on one.

**Exit:** real hosted programs using libc via safe wrappers; a usable standard library; the
`fixed` layer works with no allocator present; every layer is a package whose declared layer
the build checks.
**Dogfood:** a genuinely useful small tool (a line-oriented text utility, a small
static-site or markdown thing) written entirely in Sooth.

**P9.S1 — The `fixed` layer.** Allocation-free fixed-capacity vec/map/string/ringbuffer, built against `core`, needing no allocator at all. No dependency on P9.S2; can be built as soon as Phase 7's prerequisites land.
**Exit:** the `fixed` layer's collections work with no allocator present, and every stdlib word in it is tagged with the layer it belongs to.

**P9.S2 — The `alloc` layer: allocator rework and generic collections.** Phase 5's generic
`type:` declarations give `Vec['T]` and `Map['K 'V]` somewhere to be named; what's left
here is the piece only a growable, allocating collection needs, not the declaration
mechanism itself. **Explicit allocators ride on this item and belong in its brief, not
after it.** A defaulted type parameter (`Vec['T 'A = Global]`, a zero-size handle in the
default case) is what makes an allocator explicit without the parameter appearing at every
use site, and the `core` / `fixed` / `alloc` split bounds where it can appear at all.
Retrofitting it onto collections specified without it is the mistake Rust's `allocator_api`
is still paying for, and this is the only moment it is cheap. Two prerequisites: whether
*derived* disposal can thread an allocator down to a nested resource field at all, which is
open — every disposal word in Phase 4 Slice 8's design is `drop ( 'T -- )`, so nothing
there answers it, and this slice's own brief has to answer it fresh — and reworking the
compiler-emitted `malloc`/`free` shim into ordinary bound foreign words, since a
user-supplied allocator cannot be a backend special case. Ambient context (Odin/Jai-style)
is not on the menu: it makes disposal depend on dynamically-scoped state at the `drop` site
rather than the allocation site, which converts a compile error into a runtime one in the
language whose point is the opposite. Needs P7.S2 (statics, for the allocator's own state),
P7.S3a (generic instantiation, for naming `Map['K 'V]`/`Vec['T]` at all), and P7.S3d
(bounds, for `Map`'s key type).
**Exit:** the compiler-emitted `malloc`/`free` shim is gone, replaced by ordinary Sooth code
bound as foreign words; `Vec`/`Map`/`String` take an explicit, defaulted allocator type
parameter; `Box`, opt-in `Rc`/`Arc`, and bignum are built against it; a nested resource
field's derived disposal correctly threads a non-default allocator down to it.

**P9.S3 — The `hosted` layer.** Files, stdio, time, FFI-to-libc via safe wrappers. Needs
`alloc` (P9.S2) for anything that allocates (buffered I/O, path strings) and benefits from
`fixed` (P9.S1) for anything that doesn't. This is where the phase's dogfood program actually
runs.
**Exit:** real hosted programs use libc via safe wrappers.
**Dogfood:** a genuinely useful small tool (a line-oriented text utility, a small
static-site or markdown thing) written entirely in Sooth.

**P9.S4 — Worklist-based disposal for branching structures (optional, no forcing
dependency).** A multi-child recursive type's synthesized destructor
loops only its *last* recursive field and recurses the rest, so a left-leaning tree still
disposes in O(depth); a worklist would let every child dispose iteratively instead. Waits
for here because it needs a growable pending-pointer structure to hold onto siblings while
descending, which is exactly `alloc`'s (P9.S2) job, and because a fallible push wants an
optional to report through, which only exists once Phase 5's generic `type:` declarations
land. Building a private version of either inside a Phase 3 destructor would be guessing at
both. If the fixed-size bound turns out to be enough, `fixed`'s (P9.S1) ringbuffer covers it
without waiting for `alloc`. **No dogfood forces this earlier than the rest of the phase**:
the first real pressure is Phase 12's self-hosted AST, a genuinely deep branching
structure, so this slice can slip past the phase's own exit if nothing else needs it yet.
**Exit:** a left-leaning recursive type's synthesized destructor disposes every child in
O(1) auxiliary structures, not O(depth) stack frames.
