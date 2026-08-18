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

**P7.S1 — Accessors as receiver-directed projections.**
`[ done ]` A field access is a
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

- The **allocator rework** (S5 below). A user-supplied allocator has state: a bump
  pointer, a free list. Today that state hides inside libc's `malloc`; the moment the
  allocator is ordinary Sooth code bound as foreign words, it needs somewhere in the
  program to live. Statics are a prerequisite of the explicit-allocator item, not a
  sibling of it.
- The **API description** (S7 below). A global clause on an exported word is part of that
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

**P7.S3a — Generic instantiation over a poly word's own type variable.** `[ done ]` Discovered as a
blocker, not planned: the paper dogfood for S3d (below) found that a polymorphic word
cannot name a generic type applied to its own type variable in its signature —
`: unbox ( Box['T] -- 'T )` and `: or-default ( 'T Option['T] -- 'T )` both fail
`` error: unknown type 'T `` today, confirmed by compiling
(`docs/roadmap/P7/slice3-dogfood.md`, finding #5), while an *array* carrying a type
variable in a poly signature already works. Traced to `resolve_type_or_apply`
(`src/parser.rs:3129`): a generic name's type arguments are resolved through
`parse_type_arguments` → `instantiate_struct`/`instantiate_enum`, both of which
monomorphize *immediately, at parse time*, against a concrete `Type` — there is no
representation for "a generic applied to an argument that is still abstract." Every
existing use of `Result[i64 str]`-style generics is a concrete argument to a
*monomorphic* word; this is the first time anything has needed a generic applied to a
*poly* word's own `'T`.
This is Phase-5-shaped, not P7.S3d-shaped: it is a type-system extension (a new
`PolyType` variant for a symbolic/deferred generic application, threaded through
unification, `apply_subst`, and resolved to a real monomorphized type only once
`check_poly_call` has a concrete `Subst` — the same shape Phase 4 Slice 6a/7a's
quotation-type variant took), not a checker whitelist extension. Needs its own recon
and brief before implementation; only the parser-side root cause has been traced so
far, not unification, monomorphization, or lowering.
Was a prerequisite of S3d's `Map['K 'V]` consumer and, transitively, of S4's generic
collections. It did not turn out to be sufficient for either: `Map` is still unwritable
because a generic struct's field cannot be an array of the struct's own type variable, and
the `sort` consumer needs S3b (a polymorphic body that branches). Both were established by
probing after this slice landed.
**Exit:** a polymorphic word can declare and use a generic type applied to its own
type variable (`Box['T]`, `Option['T]`, `Map['K 'V]`) in its signature and body,
resolved correctly per concrete instantiation. Landed as a new `PolyType::Generic`
variant (`src/ast.rs`) threaded through unification, a live `GenericTypes` registry
carried through check and lowering instead of being dropped after parsing, and
generic-constructor calls (`Ok`/`Err`-style) admitted inside poly bodies, double-backstopped
by exit-time structural unification and instantiation-time reverse-mint unification
(`docs/roadmap/P7/slice3a-spec.md`, `tests/phase7_slice3a.rs`).

**P7.S3b — Quotations in a polymorphic body.** A quotation in a non-inline polymorphic
body is rejected outright: `` error: a quotation in the polymorphic body of `w` is not yet
supported `` (`src/check/poly.rs:505-513`). It fires for *any* quotation — a bare `~[ ]`,
an `if`, a declared comparator parameter — and it is a deliberate structural rejection,
not a stub: `poly_term`'s stack is `Vec<PolyType>` rather than `Vec<Slot>`, so there is
nowhere to hang the quotation marker, and Slice 10a's D1 forbids a `PolyType` variant for
it. The inline path is unaffected, because splicing into a concrete caller exposes that
caller's full `Slot` stack.
**This is what makes every interesting polymorphic word `inline` today**, and `inline`
words mint no monomorph symbol at all — their bodies are spliced, so nothing per-type is
emitted. Probe-verified: a non-inline poly word over an array *does* work and mints
`sooth_mono_swap01__m0__t0_i64`/`_bool`, but only for a straight-line body; adding an `if`
hits this wall. Since `if` is itself a library word over quotations (Slice 10c), any
polymorphic word that *branches* is forced inline, which is why it is a prerequisite of
S3d rather than a nicety: a bounded `sort` needs compare-then-conditionally-swap.
**Exit:** a non-inline polymorphic word's body may contain a quotation (and therefore
`if`), checked abstractly and lowered per instantiation, with the quotation's identity
surviving unification, `Subst`, and mangling.

**P7.S3c — Slicing a buffer into a view.** DESIGN.md lists slices among `core`'s concrete
types but defers the mechanism ("Slicing a buffer into a view is deferred"); `str` is
already the pattern's one instance, a pointer plus a runtime length. A general
`Slice['T]` view over an array carries its length at runtime, which is what makes it the
right answer to a problem the alternatives handle badly: a word over a slice needs no
length variable in its signature, so it never asks the checker to prove an index against
an abstract `'N`. Indexing a slice is *fallible* rather than provable — it reports through
an `Option`/`Result` and the must-consume rule forces the caller to handle the miss — so
the compile-time guarantee is kept without a runtime panic and without index refinement.
**Length arithmetic is explicitly not the answer here and is not in scope**: `'N` is a
length variable usable only as an array count, with no arithmetic (`src/parser.rs:2253`
admits a decimal literal or a bare `'N`), and relating lengths in a signature
(`['T 'N+'M]`) would mean unifying arithmetic terms and owning a decision procedure, the
Dependent-ML tax. Where a later slice genuinely needs a relation, the cheap form is a
constraint checked at monomorphization against concrete literals, not arithmetic in the
type language. Ordered after S3b (a slice-consuming word still needs to branch) and before
S3d, whose consumers want slice-shaped signatures rather than fixed-length ones.
**Exit:** a buffer can be sliced into a view; a word takes `Slice['T]` without naming a
length variable; indexing reports failure through an `Option`/`Result` the caller must
handle, with no runtime panic path.

**P7.S3d — User-declarable trait bounds.** `Bound` (Phase 4 Slice 1) is a closed
two-variant enum (`Copy`, `Ord`) satisfied by a hardcoded predicate
(`is_copy`/`is_numeric`); the comment on it says "Kitten-style, with no trait objects,"
and this slice is the intended next step it left open, not a new idea. Forced here,
before S4 and S5, because both need it and retrofitting is the exact mistake S4 already
names for allocators: `Map['K 'V]` needs an equality (or ordering) bound on `'K` that
today's `Ord` cannot express (`is_ord` is `is_numeric` and nothing else), so a map keyed
on a string or a struct is unwritable with the bounds that exist, and a sortable `Vec['T]`
hits the same wall. Adding a bound to a collection's signature after it ships changes
every signature that mentions it — S4's own words, about allocators, apply verbatim to
bounds: "retrofitting it onto collections specified without it is the mistake Rust's
`allocator_api` is still paying for, and this is the only moment it is cheap." S1 made
the same argument about itself, for the same reason: writing the collections against the
old mechanism and migrating afterwards is the waste. It is also a hard dependency of S7
(the API description), for the reason S7 already gives about globals: a trait bound on
an exported word's signature is part of that exported signature, so building the
diffable API format before bounds exist means re-baselining every diff it has already
emitted.
Stays compile-time-only and out of trait *objects* on purpose: `'T: Show` is satisfied by
an ordinary word (`show`) resolving for the concrete type at monomorphization, the same
static overload resolution Phase 4 Slice 8 already performs, so a satisfied bound needs
no runtime representation, no vtable, no erasure, no allocation. **The lowering/IR
budget is not zero, however** — probe-verified against the built compiler (brief's
"Resolved recon"): `builtin_overloads: HashMap<Span, String>` records one symbol per
call site shared across every monomorphization, which cannot express a trait method
needing a *different* concrete symbol per instantiation, so this slice needs either a
per-instantiation-aware overload record or lowering re-resolving against `Subst` (the
second option touches an explicitly stated "lowering never re-runs resolution"
invariant and needs that invariant's owner to weigh in). The other real work is on the
body side: `poly.rs`'s whitelist of what a bare type variable (or a *reference* to a
bounded type variable — required members take `&'T`, per the dogfood) may be used for
has to grow one case, calling a word required by a variable's declared bound.
**Depends on S3a (landed), S3b, and S3c — and the consumer scope is not settled, because
probing falsified both candidates.** `Map['K 'V]` is *not* writable: a generic struct
whose field is an array of its own type variable (`keys ['K 8]`, or
`slots [Ent['K 'V] 8]`) fails at the declaration with `` error: unknown type 'K ``, a
third gap distinct from S3a's (which fixed generic-applied-to-own-var in poly *word*
signatures, not in generic *struct fields*). The array form of `sort` is not writable
either: it needs branching, so it must be `inline`, and an inline word mints no monomorph
symbol for a per-instantiation dispatch record to key on — hence the S3b dependency. A
non-inline, straight-line, concrete-length poly word *is* a viable consumer shape today,
but nothing in the stdlib wants one. **This slice must not be specced until it has a
consumer that compiles**, which means after S3b (branching) and S3c (slice-shaped
signatures), with the generic-struct-array-field gap resolved or routed around.
**Design decisions settled in the brief** (`docs/roadmap/P7/slice3d-brief.md`):
satisfaction is **nominal**, via an `impl: Trait for Type ;` block confined by an
orphan rule to the trait's or the type's own defining module; `Copy`/`Ord` become
pre-seeded **predicate-kind** entries in the trait table (satisfaction still runs
`is_copy`/`is_ord`) so a colliding user `trait:` fails as an ordinary duplicate
declaration; a member name colliding across one variable's bound set is a located
rejection. The lowering mechanism is settled as a per-instantiation dispatch record
(check mints, lowering only looks up, so "lowering never re-runs resolution" stands);
probing confirmed the check-time and lowering-time monomorph symbols are byte-identical,
so the key is sound — for a leaf word. **Still open:** a bounded poly word calling another
poly word has no coherent key (`module.instantiations` is span-keyed and excludes nested
poly calls), so the slice must either restrict bounded bodies to leaf calls with a located
rejection or specify obligation propagation.
**A third trait kind is likely needed, beyond predicate and member kinds:** a
*compiler-known, library-declared* trait, so intrinsic compiler logic can be written
against a library implementation. `bool` is already exactly this shape — a library-declared
enum the compiler knows by a reserved registry position (`src/ast.rs:779`) with its `.`
overload injected (`:816`). A `Fallible`-style bound satisfied by `Result`/`Option` would
let fallible slice indexing (S3c), a failing allocator (S5), and S8's fallible push share
one desugaring. **Test before designing it as a trait:** if there is only ever one carrier
type and users cannot add their own, this wants to be a lang *type* like `bool`, not a
lang trait — a trait earns its keep only with two or more carriers.
**Exit:** a user can declare a bound naming required word signatures, a polymorphic word
can declare `'T: TraitName` and call a bounded word inside its body, and
monomorphization rejects an instantiation whose concrete type has no matching word with
a located error naming the missing word and the trait.

**P7.S4 — The `fixed` layer.** Allocation-free fixed-capacity vec/map/string/ringbuffer, built against `core`, needing no allocator at all. No dependency on S2 or S5; can be built in parallel with either once S1's accessor migration is out of the way.
**Exit:** the `fixed` layer's collections work with no allocator present, and every stdlib word in it is tagged with the layer it belongs to.

**P7.S5 — The `alloc` layer: allocator rework and generic collections.** Phase 5's generic
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
language whose point is the opposite. Needs S2 (statics, for the allocator's own state),
S3a (generic instantiation, for naming `Map['K 'V]`/`Vec['T]` at all), and S3d (bounds,
for `Map`'s key type).
**Exit:** the compiler-emitted `malloc`/`free` shim is gone, replaced by ordinary Sooth code
bound as foreign words; `Vec`/`Map`/`String` take an explicit, defaulted allocator type
parameter; `Box`, opt-in `Rc`/`Arc`, and bignum are built against it; a nested resource
field's derived disposal correctly threads a non-default allocator down to it.

**P7.S6 — The `hosted` layer.** Files, stdio, time, FFI-to-libc via safe wrappers. Needs
`alloc` (S5) for anything that allocates (buffered I/O, path strings) and benefits from
`fixed` (S4) for anything that doesn't. This is where the phase's dogfood program actually
runs.
**Exit:** real hosted programs use libc via safe wrappers.
**Dogfood:** a genuinely useful small tool (a line-oriented text utility, a small
static-site or markdown thing) written entirely in Sooth.

**P7.S7 — Modules: the serialisable API description.** Phase 4 Slice 5 already pulled the
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
rather than a personal-reuse one, which is why it waited. Needs S2 (statics) and S3d
(bounds), since a
global clause on an exported word is part of that word's exported signature.
**Exit:** a published package's API diff correctly classifies a PATCH/MINOR/MAJOR bump
across a two-file change.
**Dogfood:** `sooth publish --check` on a two-version bump of a small library, one that adds
a word (MINOR) and one that removes one (MAJOR).

**P7.S8 — Worklist-based disposal for branching structures (moved from Phase 3 Slice 4;
optional, no forcing dependency).** A multi-child recursive type's synthesized destructor
loops only its *last* recursive field and recurses the rest, so a left-leaning tree still
disposes in O(depth); a worklist would let every child dispose iteratively instead. Waits
for here because it needs a growable pending-pointer structure to hold onto siblings while
descending, which is exactly `alloc`'s (S5) job, and because a fallible push wants an
optional to report through, which only exists once Phase 5's generic `type:` declarations
land. Building a private version of either inside a Phase 3 destructor would be guessing at
both. If the fixed-size bound turns out to be enough, `fixed`'s (S4) ringbuffer covers it
without waiting for `alloc`. **No dogfood forces this earlier than the rest of the phase**:
the first real pressure is Phase 10's self-hosted AST, a genuinely deep branching
structure, so this slice can slip past the phase's own exit if nothing else needs it yet.
**Exit:** a left-leaning recursive type's synthesized destructor disposes every child in
O(1) auxiliary structures, not O(depth) stack frames.
