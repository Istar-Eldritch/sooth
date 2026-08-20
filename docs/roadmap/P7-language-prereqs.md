[← ROADMAP](./ROADMAP.md)

### Phase 7 — Language prerequisites for the stdlib  `[L]`  `[the features a standard library cannot be written without]`

The language-level mechanisms a standard library needs before any of it can be written:
receiver-directed field projection, static storage with per-word global sets, generic
instantiation inside a polymorphic body, slices as length-carrying views, and
user-declarable trait bounds. None of these is stdlib content; each is a checker or type
system feature whose absence makes a collection, an allocator, or an exported signature
unwritable. They land here rather than alongside their consumers because retrofitting any
of them changes every signature that mentions it.

**Exit:** a generic word can project a field, branch, carry a bound, and take a
length-carrying slice; a module can hold private static state and declare it on an
exported word.
**Dogfood:** a generic `binary_search` over a `Slice['T]` with a user-declared ordering
bound, and a module driving memory-mapped registers through private static state.

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
type system declines. **It looks like a Phase 11 feature and isn't**, because two later
items in this phase need it first:

- The **allocator rework** (P9.S2). A user-supplied allocator has state: a bump
  pointer, a free list. Today that state hides inside libc's `malloc`; the moment the
  allocator is ordinary Sooth code bound as foreign words, it needs somewhere in the
  program to live. Statics are a prerequisite of the explicit-allocator item, not a
  sibling of it.
- The **API description** (P8.S3). A global clause on an exported word is part of that
  word's exported signature. Building the serialisable API format first and adding globals
  to it later means retrofitting the format and re-baselining every diff it has already
  emitted.

Ordered after S1 (no ordering constraint against the accessor item either way) and before
everything downstream that needs it. The target-facing half of the embedded story
(fixed-address MMIO overlays, the volatile aspect, bit-level register layout, ISR symbol
export) stays in Phase 11, where its consumer is. This is what pushes the phase from `[L]`
toward `[XL]`: it is a language feature and a new checker analysis, not a stdlib item.
**Exit:** a module with private static state exports a word whose declared global set the
checker verifies, and an *exported* word's undeclared static access is a located compile
error naming the static (a private word's static access is inferred, not declaration-forced:
inferred everywhere, declared at the export boundary).

**P7.S3a — Generic instantiation over a poly word's own type variable.** `[ done ]` Discovered as a
blocker, not planned: the paper dogfood for S3e (below) found that a polymorphic word
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
This is Phase-5-shaped, not P7.S3e-shaped: it is a type-system extension (a new
`PolyType` variant for a symbolic/deferred generic application, threaded through
unification, `apply_subst`, and resolved to a real monomorphized type only once
`check_poly_call` has a concrete `Subst` — the same shape Phase 4 Slice 6a/7a's
quotation-type variant took), not a checker whitelist extension. Needs its own recon
and brief before implementation; only the parser-side root cause has been traced so
far, not unification, monomorphization, or lowering.
Was a prerequisite of S3e's `Map['K 'V]` consumer and, transitively, of P9.S1's generic
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

**P7.S3b — Quotations in a polymorphic body.** `[ done ]` A quotation literal may appear in a
non-inline polymorphic body and be consumed there by a generated eliminator, so a polymorphic
word can eliminate an enum: the arms *are* quotation literals (`examples/eliminator.sth`), and
that is what the `unwrap_or`/`map_or`/`Result`-combinator family is built out of. The inline
path is unaffected, because splicing into a concrete caller exposes that caller's full `Slot`
stack.
A quotation's identity rides its stack slot, so `dup`/`swap`/`drop` reorder arms with no
special handling, while a *tagged* arm must still reach its eliminator by written adjacency —
the same rule the concrete path applies, so a generic body is not the laxer of the two.
Elimination is the only quotation consumer a generic body has: `call`/`branch`/`if`/`times`/
`tag` each take a quotation as a row-typed parameter, which needs row unification against an
abstract stack, and each is a located rejection naming **P7.S3b-follow**. A quotation may not
be materialised — stored, returned, or left unconsumed at word or arm exit — and every escape
route is its own located error.
Two standing limits bound what can be written against this today: field projection (`&w`) is
rejected in every generic body, so an arm destructures (`Rect>`) rather than projects; and a
generic word cannot call another generic word (`unknown word g__m0`), so a combinator written
here composes concrete and builtin callees only. P6.S3b widens the consumer to generic enums;
P6.S4, by deleting `WordBody::Clauses`, makes this the only route to elimination that exists.
**Exit:** a polymorphic word can eliminate an enum — quotation-literal arms in its body,
dispatched to the generated eliminator — with the quotation's identity surviving a shuffle,
arms merged over an abstract stack with type variables rigid and the borrow table unioned,
and a materialised (non-spliced) quotation rejected with a located error. Landed as a
`PolySlot { pt, int_val, quot }` stack replacing the poly walk's `Vec<PolyType>` and the
parallel `lits` vector beside it, a `QuotRef` index over a per-body literal interner on
`PolyScope`, an `eliminator_registry` intercept in `poly_call_term` ahead of env dispatch
(`poly_eliminator_call`), and an abstract N-arm join with a poly analogue of `Scope::leave`
run per arm, over zero lowering change (`docs/roadmap/P7/slice3b-spec.md`,
`tests/phase7_slice3b.rs`).

**P7.S3c — Slicing a buffer into a view.** `[ done ]` DESIGN.md lists slices among
`core`'s concrete types but defers the mechanism ("Slicing a buffer into a view is
deferred"); `str` is already the pattern's one instance, a pointer plus a runtime
length. A general `Slice[T]` view over an array carries its length at runtime, which is
what makes it the right answer to a problem the alternatives handle badly: a word over a
slice needs no length variable in its signature, so it never asks the checker to prove an
index against an abstract `'N`. Indexing a slice keeps the existing runtime
out-of-bounds trap, the same one array indexing already uses — a fallible,
`Option`/`Result`-returning accessor is deferred to P7.S3e, which is what will give a
user a declarable failure carrier to report through; until then, an index is not proven
in range at compile time, and the runtime trap is the consequence of that. **Length
arithmetic is explicitly not the answer here and is not in scope**: `'N` is a
length variable usable only as an array count, with no arithmetic (`src/parser.rs:2253`
admits a decimal literal or a bare `'N`), and relating lengths in a signature
(`['T 'N+'M]`) would mean unifying arithmetic terms and owning a decision procedure, the
Dependent-ML tax. Where a later slice genuinely needs a relation, the cheap form is a
constraint checked at monomorphization against concrete literals, not arithmetic in the
type language. Ordered after S3b, ahead of S3d, and before S3e, whose consumers want
slice-shaped signatures rather than fixed-length ones. **It does not depend on S3d**:
every exit criterion below is exercised by a *concrete* consumer — an array-reference
parameter, a runtime `usize` index, a bounds guard, a runtime trap on miss, and a
recursive divide-and-conquer consumer, none of which needs a row or a quotation splice.
Only a comparator-taking consumer (`sort`/`bin_search`) needs S3d's quotation splice.
**Exit:** a buffer can be sliced into a view; a word takes `Slice[T]` over a *concrete*
element type without naming a length variable (a generic element -- `Slice` over the word's
own type variable -- is a locked non-goal, blocked on
generic-instantiation-over-own-variable and rejected by name); indexing traps at runtime on
an out-of-range index, with no `Option`/`Result` accessor (deferred to P7.S3e). Landed as
`Type::Slice(SliceId, bool)` / `IrType::Slice`, a second-class, input-only, non-owning
view interned per `(element, mutable)` and lowered as a 16-byte `{ptr, len}` aggregate, with
`slice`/`subslice` constructing one from a `&[T N]`/`&![T N]` array reference and
`&>`/`&!>`/`len` dispatching on it alongside their existing array arms
(`docs/roadmap/P7/slice3c-spec.md`, `tests/phase7_slice3c.rs`).

**P7.S3d — Rowless quotation-consumer splice.** S3b's eliminator intercept admits a
quotation marker only where an enum eliminator collects it; every other consumer is
denied, row-typed or not — probe-verified: `~[ -- ] call` and a fully concrete
`inline ( ~[ -- ] -- )` helper both fail today even though neither needs any row
unification (`` `call` on a quotation ... is not yet supported ``,
`` ... is not permitted on a quotation literal ``). The family splits into two tiers by
cost. This slice is the cheap tier: splice a second, fully concrete quotation consumer
(no `..a`/`..b` in its signature, e.g. a comparator `~[ &'T &'T -- Ordering ]`) through the
poly walk, no row unification against an abstract stack required. `if`/`branch`/`times`/
`tag` stay deferred to **P7.S3b-follow**, the expensive tier: those are row-typed
(`if inline ( ..a bool ~[ ..a -- ..b ] ~[ ..a -- ..b ] -- ..b )`) and need unifying a
declared row against the poly walk's abstract stack, machinery this slice does not add.
This is also the second quotation consumer S3b's own exit findings named as the trigger
for re-running `poly.rs`'s deferred split signals. Ordered after S3c and ahead of S3e,
because it is what actually unblocks `sort`'s comparator call — not branching, which S3b
already shipped.
**Exit:** a poly body can call a fully concrete (rowless) quotation parameter or literal;
`if`/`branch`/`times`/`tag` remain a located rejection naming P7.S3b-follow.

**P7.S3e — User-declarable trait bounds.** `Bound` (Phase 4 Slice 1) is a closed
two-variant enum (`Copy`, `Ord`) satisfied by a hardcoded predicate
(`is_copy`/`is_numeric`); the comment on it says "Kitten-style, with no trait objects,"
and this slice is the intended next step it left open, not a new idea. Forced here,
before P9.S1 and P9.S2, because both need it and retrofitting is the exact mistake P9.S1 already
names for allocators: `Map['K 'V]` needs an equality (or ordering) bound on `'K` that
today's `Ord` cannot express (`is_ord` is `is_numeric` and nothing else), so a map keyed
on a string or a struct is unwritable with the bounds that exist, and a sortable `Vec['T]`
hits the same wall. Adding a bound to a collection's signature after it ships changes
every signature that mentions it — P9.S1's own words, about allocators, apply verbatim to
bounds: "retrofitting it onto collections specified without it is the mistake Rust's
`allocator_api` is still paying for, and this is the only moment it is cheap." S1 made
the same argument about itself, for the same reason: writing the collections against the
old mechanism and migrating afterwards is the waste. It is also a hard dependency of P8.S3
(the API description), for the reason P8.S3 already gives about globals: a trait bound on
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
**Depends on S3a (landed), S3b (landed), and S3d — and the consumer scope is not settled,
because probing falsified one candidate and corrected another.** `Map['K 'V]` is *not*
writable: a generic struct whose field is an array of its own type variable (`keys ['K 8]`,
or `slots [Ent['K 'V] 8]`) fails at the declaration with `` error: unknown type 'K ``, a
third gap distinct from S3a's (which fixed generic-applied-to-own-var in poly *word*
signatures, not in generic *struct fields*). The array form of `sort` was originally
thought to need branching (hence `inline`, hence no monomorph symbol for a per-instantiation
dispatch record to key on) — probe-verified false: S3b's eliminator branching already mints
a monomorph symbol for a non-inline poly word. The real remaining wall is S3d's (calling the
comparator quotation itself from inside the poly body). **This slice must not be specced
until it has a consumer that compiles**, which means after S3d, with the
generic-struct-array-field gap resolved or routed around for `Map`.
**Design decisions settled in the brief** (`docs/roadmap/P7/slice3e-brief.md`):
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
let fallible slice indexing (deferred from S3c), a failing allocator (P9.S2), and P9.S4's fallible push share
one desugaring. **Test before designing it as a trait:** if there is only ever one carrier
type and users cannot add their own, this wants to be a lang *type* like `bool`, not a
lang trait — a trait earns its keep only with two or more carriers.
**Exit:** a user can declare a bound naming required word signatures, a polymorphic word
can declare `'T: TraitName` and call a bounded word inside its body, and
monomorphization rejects an instantiation whose concrete type has no matching word with
a located error naming the missing word and the trait.
