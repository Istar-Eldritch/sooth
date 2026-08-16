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

**Static storage and global sets, and they land before the allocator work.** Module-level
static storage (a *place*, not a value: never owned, moved, or dropped, reached only
through a second-class ref, constant-initialised) plus the per-word **global set** that
keeps it honest, the statics a word touches and in what mode, inferred within a module and
declared on exported words. DESIGN.md's *Embedded* section carries the full design,
including why a static needs its own carve-out from the must-consume rule beside `Copy`'s,
and why this is a closed monomorphic list rather than the effect rows the type system
declines. **It looks like a Phase 9 feature and isn't**, because two items in this phase
need it first:

- The **allocator rework**. A user-supplied allocator has state: a bump pointer, a free
  list. Today that state hides inside libc's `malloc`; the moment the allocator is ordinary
  Sooth code bound as foreign words, it needs somewhere in the program to live. Statics are
  a prerequisite of the explicit-allocator item below, not a sibling of it.
- The **API description**. A global clause on an exported word is part of that word's
  exported signature. Building the serialisable API format first and adding globals to it
  later means retrofitting the format and re-baselining every diff it has already emitted.

Ordered before both; no ordering constraint against the lens item either way. The
target-facing half of the embedded story (fixed-address MMIO overlays, the volatile aspect,
bit-level register layout, ISR symbol export) stays in Phase 9, where its consumer is.
This is what pushes the phase from `[L]` toward `[XL]`: it is a language feature and a new
checker analysis, not a stdlib item.
**Exit:** a module with private static state exports a word whose declared global set the
checker verifies, and an undeclared static access inside that module is a located compile
error naming the static.

**Modules: what's left after Slice 5.** Phase 4 Slice 5 already pulled the whole
compilation-unit story forward: a file is a compilation unit, and an import brings a word
or a struct/enum declaration across a file boundary by qualified name, landed once writing
a reusable component — usually a type plus its operations — needed somewhere to live
besides copy-pasted into every consumer. `Vec`/`Map`/`String`/`Box`/`Rc`/`Arc` already have
somewhere to live, courtesy of that slice.
Encapsulation went with it: default private, a per-file `export:` list, and the Elm-style
split between exporting a type name and exporting its constructors. So "which words, types,
and externs are public" is already answered, and answered where it had to be, since a type
cannot hold an invariant while its generated setters cross the boundary unchecked.
What's left here is one thing, not two: a **serializable API description**, a compiler pass
that walks the checked AST, filters to the exported declarations Slice 5 already
distinguishes, and emits a file listing every exported signature for the API diff to
compare between versions. That is the remaining prerequisite in
`docs/dependency-management.md`, and it is a packaging/publishing concern (letting other
people depend on you with enforced semver) rather than a personal-reuse one, which is why
it waited.
**Exit:** a published package's API diff correctly classifies a PATCH/MINOR/MAJOR bump
across a two-file change.
**Dogfood:** `sooth publish --check` on a two-version bump of a small library, one that adds
a word (MINOR) and one that removes one (MAJOR).

**Accessors as lenses, and it lands before the stdlib.** Retire the per-field generated
accessor words in favour of separating the *location* from the *operation*: `q buf &>`
instead of `q Queue&>buf`, matching how arrays already read (`l 0 &>`). See DESIGN.md's
Open / deferred for the full case; the short version is that `>` / `<` / `|>` currently
conflate which field with what ownership transfer happens, lenses separate them, and the
generated-word count drops from O(fields x operations) to O(fields + operations), which is
also what makes the module export list stop needing three entries per field.
**Ordered first in this phase, before `Vec`/`Map`/`String`**, for exactly the reason modules
were pulled forward: writing the collections against the old accessors and migrating them
afterwards is the waste. It cannot land earlier than this phase either, since one `&>`
accepting both an array and a struct *is* static overloading (Phase 4 slice 8).
**Not a locked design.** The open question is what a selector *is*: a compile-time-only
marker (the machinery slice 4 built for quotations, cheap and known, but no composition) or
a first-class `Lens['S 'A]` value (composable, expressible once type variables exist, but it
needs unambiguous selector names, which means qualification, which undoes the terseness that
motivated the change). Its brief has to settle that before anything else, and should size
the corpus migration honestly: every struct access in `examples/` and the test suite, which
is 8c-shaped mechanical work on top of a real design decision.

**Generic types, continued: the allocator parameter.** Phase 5's generic `type:`
declarations give `Vec['T]` and `Map['K 'V]` somewhere to be named; what's left here is
the piece only a growable, allocating collection needs, not the declaration mechanism
itself.
**Explicit allocators ride on this item and belong in its brief, not after it.** A defaulted
type parameter (`Vec['T 'A = Global]`, a zero-size handle in the default case) is what makes
an allocator explicit without the parameter appearing at every use site, and the `core` /
`fixed` / `alloc` split bounds where it can appear at all. Retrofitting it onto collections
specified without it is the mistake Rust's `allocator_api` is still paying for, and this is
the only moment it is cheap. Two prerequisites: whether *derived* disposal can thread an
allocator down to a nested resource field at all, which is open — every disposal word in
Phase 4 Slice 8's design is `drop ( 'T -- )`, so nothing there answers it, and this phase's
own brief has to answer it fresh — and Slice 2's parked rework of the compiler-emitted
`malloc`/`free` shim into ordinary bound foreign words, since a user-supplied allocator
cannot be a backend special case. Ambient context (Odin/Jai-style)
is not on the menu: it makes disposal depend on dynamically-scoped state at the `drop` site
rather than the allocation site, which converts a compile error into a runtime one in the
language whose point is the opposite.

**Worklist-based disposal for branching structures (moved from Phase 3 Slice 4).** A
multi-child recursive type's synthesized destructor loops only its *last* recursive field
and recurses the rest, so a left-leaning tree still disposes in O(depth); a worklist would
let every child dispose iteratively instead. Waits for here because it needs a growable
pending-pointer structure to hold onto siblings while descending, which is exactly the
`alloc` layer's job, and because a fallible push wants an
optional to report through, which only exists once Phase 5's generic `type:` declarations
land. Building a private
version of either inside a Phase 3 destructor would be guessing at both. If the fixed-size
bound turns out to be enough, the `fixed` layer's ringbuffer covers it without waiting for
`alloc`. No dogfood forces this earlier: the first real pressure is Phase 10's self-hosted
AST, a genuinely deep branching structure.

