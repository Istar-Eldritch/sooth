[← ROADMAP](./ROADMAP.md)

### Phase 7 — Stdlib and `no_std` layering  `[L]`  `[where it becomes usable for real programs]`

The four layers from DESIGN.md, with boundaries and the allocator *interface* fixed
now even though hosted is built first: **core** (already accreting), **fixed**
(allocation-free fixed-capacity vec/map/string/ringbuffer), **alloc** (growable
Vec/Map/String, Box, opt-in Rc/Arc, escaping closures, bignum, against core's
allocator interface), **hosted** (files, stdio, time, FFI-to-libc via safe
wrappers). Tag every stdlib word with the layer it needs. Escaping closures appear in that
list as a *layer tag*, not as unbuilt work: the feature itself lands in Phase 4 Slice 7 on
`^`, and what belongs here is only the classification that a closure which escapes its
frame needs an allocator present, so it is unavailable to the `fixed` layer.

**Exit:** real hosted programs using libc via safe wrappers; a usable standard
library; the `fixed` layer works with no allocator present.
**Dogfood:** a genuinely useful small tool (a line-oriented text utility, a small
static-site or markdown thing) written entirely in Sooth.

**P7.S1 — Accessors as receiver-directed projections.** A field access is a
mode-carrying projection word (`&hp` / `&!hp`) resolved against the receiver's type:
`&S -- &A` and `&!S -- &!A` are consuming (chaining, e.g. `u &stats &hp @`), while an
owned `S -- S &A` is non-consuming, leaving the receiver in place. The field name is
part of the projection word and is resolved at check time against the receiver on the
stack, recorded per call site rather than carried as a value, so there is no selector
value to compose or store. `&>`/`&!>` remain array-only, with no struct/array
unification: a struct selector is a name, an array selector is a runtime value. This has
no dependency on static overloading (Phase 4 Slice 8). The per-field generated
`Get`/`Set`/`Peek` words and the fused `Type>field`/`Type<field`/`Type|>field` spelling
are deleted, so the generated-word count stops scaling with field count at all: two
words per type (`Type` and `Type>`) and none per field, with `&f`/`&!f` not env entries
but a checker arm resolved ahead of env lookup, over the pre-existing `@`/`!` builtins.
Two implicit disposals go with them: a non-extracted linear field, which no surviving
operation performs at all now that the whole-value destructure is the only way out and it
extracts every field, and a value overwritten in place, which is the one `!` itself
refuses over a linear referent.
**Ordered first in this phase, before `Vec`/`Map`/`String`**, for exactly the reason modules
were pulled forward in Phase 4: writing the collections against the old accessors and
migrating them afterwards is the waste.
**Exit:** every struct/variant field access in the corpus goes through `&f`/`&!f`; the old
per-field generated accessor words and fused spelling are deleted; `&>` remains
array-only.

**P7.S2 — Static storage and global sets, and they land before the allocator work.**
`[ done ]` Module-level static storage (a *place*, not a value: never owned, moved, or dropped,
reached only through a second-class ref, constant-initialised) plus the per-word
**global set** that keeps it honest, the statics a word touches and in what mode, inferred
within a module and declared on exported words. DESIGN.md's *Embedded* section carries the
full design, including why a static needs its own carve-out from the must-consume rule
beside `Copy`'s, and why this is a closed monomorphic list rather than the effect rows the
type system declines. **It looks like a Phase 9 feature and isn't**, because two later
items in this phase need it first:

- The **allocator rework** (S4 below). A user-supplied allocator has state: a bump
  pointer, a free list. Today that state hides inside libc's `malloc`; the moment the
  allocator is ordinary Sooth code bound as foreign words, it needs somewhere in the
  program to live. Statics are a prerequisite of the explicit-allocator item, not a
  sibling of it.
- The **API description** (S6 below). A global clause on an exported word is part of that
  word's exported signature. Building the serialisable API format first and adding globals
  to it later means retrofitting the format and re-baselining every diff it has already
  emitted.

Ordered after S1 (no ordering constraint against the accessor item either way) and before
everything downstream that needs it. The target-facing half of the embedded story
(fixed-address MMIO overlays, the volatile aspect, bit-level register layout, ISR symbol
export) stays in Phase 9, where its consumer is. This is what pushes the phase from `[L]`
toward `[XL]`: it is a language feature and a new checker analysis, not a stdlib item.
**Exit:** a module with private static state exports a word whose declared global set the
checker verifies, and an *exported* word's undeclared static access is a located compile
error naming the static (a private word's static access is inferred, not declaration-forced:
inferred everywhere, declared at the export boundary).

**P7.S3 — The `fixed` layer.** Allocation-free fixed-capacity vec/map/string/ringbuffer,
built against `core`, needing no allocator at all. No dependency on S2 or S4; can be built
in parallel with either once S1's accessor migration is out of the way.
**Exit:** the `fixed` layer's collections work with no allocator present, and every stdlib
word in it is tagged with the layer it belongs to.

**P7.S4 — The `alloc` layer: allocator rework and generic collections.** Phase 5's generic
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
language whose point is the opposite. Needs S2 (statics, for the allocator's own state).
**Exit:** the compiler-emitted `malloc`/`free` shim is gone, replaced by ordinary Sooth code
bound as foreign words; `Vec`/`Map`/`String` take an explicit, defaulted allocator type
parameter; `Box`, opt-in `Rc`/`Arc`, and bignum are built against it; a nested resource
field's derived disposal correctly threads a non-default allocator down to it.

**P7.S5 — The `hosted` layer.** Files, stdio, time, FFI-to-libc via safe wrappers. Needs
`alloc` (S4) for anything that allocates (buffered I/O, path strings) and benefits from
`fixed` (S3) for anything that doesn't. This is where the phase's dogfood program actually
runs.
**Exit:** real hosted programs use libc via safe wrappers.
**Dogfood:** a genuinely useful small tool (a line-oriented text utility, a small
static-site or markdown thing) written entirely in Sooth.

**P7.S6 — Modules: the serialisable API description.** Phase 4 Slice 5 already pulled the
whole compilation-unit story forward: a file is a compilation unit, and an import brings a
word or a struct/enum declaration across a file boundary by qualified name, landed once
writing a reusable component — usually a type plus its operations — needed somewhere to
live besides copy-pasted into every consumer. `Vec`/`Map`/`String`/`Box`/`Rc`/`Arc` already
have somewhere to live, courtesy of that slice. Encapsulation went with it: default
private, a per-file `export:` list, and the Elm-style split between exporting a type name
and exporting its constructors. So "which words, types, and externs are public" is already
answered, and answered where it had to be, since a type cannot hold an invariant while its
generated setters cross the boundary unchecked.
What's left is one thing, not two: a **serializable API description**, a compiler pass that
walks the checked AST, filters to the exported declarations Slice 5 already distinguishes,
and emits a file listing every exported signature for the API diff to compare between
versions. That is the remaining prerequisite in `docs/dependency-management.md`, and it is a
packaging/publishing concern (letting other people depend on you with enforced semver)
rather than a personal-reuse one, which is why it waited. Needs S2 (statics), since a
global clause on an exported word is part of that word's exported signature.
**Exit:** a published package's API diff correctly classifies a PATCH/MINOR/MAJOR bump
across a two-file change.
**Dogfood:** `sooth publish --check` on a two-version bump of a small library, one that adds
a word (MINOR) and one that removes one (MAJOR).

**P7.S7 — Worklist-based disposal for branching structures (moved from Phase 3 Slice 4;
optional, no forcing dependency).** A multi-child recursive type's synthesized destructor
loops only its *last* recursive field and recurses the rest, so a left-leaning tree still
disposes in O(depth); a worklist would let every child dispose iteratively instead. Waits
for here because it needs a growable pending-pointer structure to hold onto siblings while
descending, which is exactly `alloc`'s (S4) job, and because a fallible push wants an
optional to report through, which only exists once Phase 5's generic `type:` declarations
land. Building a private version of either inside a Phase 3 destructor would be guessing at
both. If the fixed-size bound turns out to be enough, `fixed`'s (S3) ringbuffer covers it
without waiting for `alloc`. **No dogfood forces this earlier than the rest of the phase**:
the first real pressure is Phase 10's self-hosted AST, a genuinely deep branching
structure, so this slice can slip past the phase's own exit if nothing else needs it yet.
**Exit:** a left-leaning recursive type's synthesized destructor disposes every child in
O(1) auxiliary structures, not O(depth) stack frames.
