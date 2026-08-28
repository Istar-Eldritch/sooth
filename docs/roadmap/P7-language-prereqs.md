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
because a generic struct's field cannot be an array of the struct's own type variable
(**P7.S3n**), and the `sort` consumer needs S3b (a polymorphic body that branches). Both
were established by probing after this slice landed.
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
Elimination is the only quotation consumer this slice gives a generic body: the row-typed
combinators (`if`/`unless`/`times`) need a declared row grounded against an abstract stack,
and land in **P7.S3b-follow**; the `call`/`branch`/`tag` primitives declare no `~[ ]`
parameter to dispatch off, and `call` on a literal was **P7.S3d**'s own exit criterion
(landed) — `branch`/`tag` stay a located rejection naming no slice yet scoped to resolve
them. A quotation may not
be materialised — stored, returned, or left unconsumed at word or arm exit — and every escape
route is its own located error.
One standing limit bounds what can be written against this today: field projection (`&w`) is
rejected in every generic body, so an arm destructures (`Rect>`) rather than projects. A second,
narrower one: the arm join compares each slot's *type* and not the compile-time literal beside
it, so arms disagreeing on an index
literal leave the join carrying the first arm's — `~[ a 0 ] ~[ a 9 ]` joined over a `['T 4]`
satisfies the static bounds check and traps at runtime instead. Memory-safe, and no laxer than
the concrete path, which ICEs on the same program rather than rejecting it.
P6.S3b widens the consumer to generic enums;
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

**P7.S3b-follow — Row-typed quotation consumers in a polymorphic body.** `[ done ]` S3b
shipped one quotation consumer, enum elimination; every row-typed inline combinator
(`if`/`unless`/`times`, and any library or user `inline` word declaring `~[ ]` parameters)
stayed a located rejection naming this slice. A **non-inline** polymorphic word can now
consume one, so it can branch and loop as a **monomorphized function** — one compiled body
per instantiation — instead of forcing every call site to splice its whole body. Scheduled
on code size, not on an unwritable-program witness: every candidate motivating program
turned out writable today with `inline` (a self-*tail*-recursive generic word already lowers
to a loop back-edge, `inline` and all); what a non-spliced body actually saves is that a
generic word's *callers* no longer each carry a full copy of it. Dispatch is driven by the
callee's declared `PolySig`, not by name, so one mechanism covers `if`, `unless`, `times`,
and a user's own row-typed combinator alike; `unless` is the witness it is not name-driven,
since it never reached the old rejection at all (it landed on an unrelated operand-window
error instead). `call`, `branch`, `tag` are deliberately **not** delivered — `call` on a
literal was P7.S3d's own exit criterion (landed), and `branch`/`tag` keep the located
rejection, naming no slice yet scoped to resolve them. An arm operand that is not a
splice-consumed
quotation literal (a value, or one that lost its identity through a local bind) is a located
rejection reusing S3b's materialisation diagnostics, never an inherited backend panic — this
forecloses the pre-existing `while`-over-an-erased-quotation ICE from being reached through
the new path. Type variables and rows stay rigid throughout (S3b's L1/L2, unchanged): no
mid-body `Subst`, and a declared row grounds once, to the caller region beneath the
combinator's fixed inputs, never solved for.
**Exit:** a non-inline generic word can pass a quotation literal to a row-typed inline
combinator's `~[ ]` parameter, in both the non-shape-changing (`times`) and shape-changing
(`if`/`unless`) cases, with the arms' borrow table unioned and their exit rows checked
structurally under rigid type variables — a single-arm combinator's declared row is
pre-seeded as its own baseline (soundness: `times`'s one arm has no sibling to compare
against), and shape-changing sibling arms are cross-checked by output-row id. Landed by
extracting the eliminator's per-arm walk and N-arm join (borrow-union, `Scope::leave`
analogue, `Moves::join`) into a shared `poly_walk_arms`, now called by both
`poly_eliminator_call` and a new `poly_combinator_call`; a `poly_row_combinator` lookup in
`poly_call_term` dispatches ahead of the narrowed `call`/`branch`/`tag` guard
(`docs/roadmap/P7/slice3b-follow-spec.md`, `tests/phase7_slice3b_follow.rs`).

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

**P7.S3d — Rowless quotation-consumer splice.** `[ done ]` S3b's eliminator intercept
admitted a quotation marker only where an enum eliminator collects it; every other
consumer was denied, row-typed or not — probe-verified: `~[ -- ] call` and a fully
concrete `inline ( ~[ -- ] -- )` helper both failed even though neither needs any row
unification (`` `call` on a quotation ... is not yet supported ``,
`` ... is not permitted on a quotation literal ``). The family splits into two tiers by
cost. This slice is the cheap tier: splices a `call` on a quotation *literal* written
directly in a non-inline poly body in place against the live poly stack (C1), and grounds
a body-local literal passed as an argument to a **concrete** callee's declared, ground
`Type::Quotation` parameter (C2) — no `..a`/`..b` in either shape's signature, so no row
unification against an abstract stack. The row-typed combinators
(`if inline ( ..a bool ~[ ..a -- ..b ] ~[ ..a -- ..b ] -- ..b )`) are the expensive tier
and are **P7.S3b-follow**'s, shipped separately by grounding the declared row against the
poly walk's abstract stack. `branch`/`tag` are compiler-known primitives that declare no
`~[ ]` parameter to dispatch off and stay a located rejection, unchanged (this slice's own
scope exclusion), naming no follow-up slice yet.
This was also the second quotation consumer S3b's own exit findings named as the trigger
for re-running `poly.rs`'s deferred split signals; the re-run found 3 of 5 signals still
firing and both previously-rejected splits still wrong, so the split stays deferred.
Ordered after S3c and ahead of S3e, because it is what actually unblocks `sort`'s
comparator call — not branching, which S3b-follow already shipped. **Does not** reach a
comparator declared as a `~[ ]` *parameter* (a non-inline word still cannot declare one,
R6) or an abstract/forwarded quotation crossing the polymorphism boundary — both are
P7.S3f's gap, independent of this slice either way.
**Exit:** a poly body can `call` a quotation literal written in its own body (splicing it
in place) or pass one as an argument to a concrete word's ground `Type::Quotation`
parameter (grounding it against that declared effect); `branch`/`tag` keep a located
rejection naming no follow-up slice yet
(`docs/roadmap/P7/slice3d-spec.md`, `tests/phase7_slice3d.rs`).

**P7.S3e — User-declarable trait bounds.** `[ done ]` `Bound` opens from the closed
`Copy`/`Ord` pair to `Bound::User(TraitId)`. A trait (`trait: Name  member ( sig ) ... ;`)
is satisfied **nominally**: an `impl: Trait for Type  member word ... ;` block maps each
member to an existing, already-declared *concrete* word (impl checking is signature
comparison, never body checking), confined by an orphan rule to the trait's or the
type's own defining module. `Copy` is a pre-seeded predicate-kind trait-table entry
(satisfaction runs `is_copy`), so a colliding user `trait: Copy` fails as an ordinary
duplicate declaration; `Ord` is an ordinary library trait (**P7.S3s**), pre-seeded
nowhere. A bare or ref-to-bare bounded variable's
call to a member its bound set declares is resolved in `poly_call_term` ahead of the
ordinary `env.get` lookup, mints a per-instantiation dispatch record
(`CallInst::trait_calls`) keyed on `(callee, θ)`, and lowering only looks the symbol up —
it never re-resolves, preserving the "lowering never re-runs resolution" invariant. Two
matching bounds on one call is an ambiguity rejection unless a module-qualified name
disambiguates (`o::t1`, the existing import-alias `::`, not a trait-name qualifier — same-
module collisions have no escape hatch). A bound on a poly *combinator*'s own type
variable is a located rejection, tracked separately as **P7.S3o** (a combinator's body
calling a bounded poly word does resolve, and is tested). Consumer: the array form of
`sort` (`'T: Copy Order`), Program 2 of `docs/roadmap/P7/slice3-dogfood.md`, runs at two
distinct concrete instantiations. `Map['K 'V]` stays out of scope: **P7.S3n** landed the
array-field-of-own-type-variable blocker, but `Map` still needs a struct-header length
variable and the `Eq`/`Hash`/`Default`-style bounds this slice's own dogfood already named,
not anything this slice leaves undone.
Out of scope, unchanged: trait objects/runtime dispatch, associated types, default
bodies, blanket/supertraits, generic constants, multi-type-variable traits, a
compiler-known third trait kind for a `Fallible`-style bound (`bool`'s own `.`-overload
registry slot is the closest existing precedent, revisit once S3c's deferred fallible
slice indexing needs it), and a trait-name-based qualifier for same-module collisions.
**Exit:** a user can declare a bound naming required word signatures, a polymorphic word
can declare `'T: TraitName` and call a bounded word inside its body, and
monomorphization rejects an instantiation whose concrete type has no matching word with
a located error naming the missing word and the trait
(`docs/roadmap/P7/slice3e-spec.md`, `tests/phase7_slice3e.rs`).

**P7.S3f — Runtime quotation values crossing the polymorphism boundary.** `[ done ]` Discovered
while scoping S3d, not planned: a *non-inline* word cannot declare a `~[ ]`
(`InlineQuotation`) parameter, and that gate is correct, not a gap — `~[ ]` is splice-only
by design (`src/check/word_entry.rs:112-142`), has no runtime representation, and giving it
one would just reinvent plain `[ ]` under a different sigil. But ordinary `[ ]` quotations
(`Type::Quotation`) already have a real runtime representation, landed and marked
`Implemented.` in Phase 4 Slices 7a/7b: a concrete word taking one and `call`ing it works
today, probe-verified. What does not work is that value crossing the **polymorphism**
boundary, on either side: a poly body calling an abstract (non-literal, parameter-bound)
quotation slot is rejected by `poly_call_term`'s `call` handling
(`src/check/poly.rs:953-958`, `` `call` is not permitted on a quotation in `{word}` `` —
re-probed and re-cited this session; the message text has moved on since this entry was
first written, but the gap has not closed); and any caller — concrete or poly — passing a
real quotation *argument* into a poly callee's signature is rejected outright by
`check_poly_call`'s R9p (`src/check/poly.rs:3270-3271`, `reject_quotation_argument`),
regardless of that parameter's position in the signature (re-probed this session at all
three positions after an earlier pass wrongly reported this as position-dependent — see
`slice3f-brief.md`'s recon #2 for the retraction), whose message still reads "a runtime
quotation value is slice 7" as if 7a/7b never shipped.
**Independent of S3d, not a prerequisite either way.** S3d's gap is a quotation *literal*
written inside a poly body (`poly_call_term`'s `QuotLit`-marker path); this slice's gap is
an *abstract parameter* and the *call-site argument boundary* (`check_poly_call`), a
different code path in the same file. Nothing traced so far makes one need the other.
**This, not S3d, is what would let `sort`/`bin_search` become genuinely non-inline** —
S3d's `call`-on-a-literal fix cannot reach a comparator *parameter* at all (a non-inline
word still can't declare `~[ ]`); this slice's `call`-on-a-parameter fix, paired with an
ordinary `[ ]` (not `~[ ]`) comparator type, is the mechanism that actually would. See
`docs/roadmap/P7/slice3f-brief.md` for full recon: R9p's guard fires unconditionally on
any quotation-marked operand, without checking whether the declared input is actually the
unsound bare `'T` case its own comment names, or a harmless ground
`Type::Quotation`-shaped parameter that should be materialized like the concrete twin
already is — the brief's open questions cover whether closing this and Gap 1 turn out to
be one mechanism or two.
**Exit:** a polymorphic word can declare an ordinary (non-`~`) quotation-typed parameter,
call it inside its own body via an indirect call, and receive a real quotation value at the
argument boundary of a **ground** (fully concrete) declared `Type::Quotation` parameter;
the stale "slice 7" wording is retired
(`docs/roadmap/P7/slice3f-spec.md`, `tests/phase7_slice3f.rs`).
**"Any caller" overclaims two cases, named rather than silently absorbed:** an
*overloaded* poly name still rejects a quotation argument outright
(`resolve_poly_overload`'s `saw_quotation` short-circuit, `src/check/poly.rs:3038-3067`)
without ever reaching this slice's per-position check -- a single-candidate poly name
only. And a still-**abstract** `PolyType::Quotation(ins, outs, ..)` parameter (one whose
brackets still mention a type variable, e.g. `[ 'T -- 'T ]`) was refused at both boundaries
at this slice's exit: R9p rejected a literal argument at that position, and a poly body could
not `call` a *bound instantiation* of it. **P7.S3l** below lifted both.

**P7.S3g — Self-recursion in a non-inline generic body.** `[ done ]` A non-inline
polymorphic word may call itself, so a generic word that loops over an inductively-shaped
value no longer has to be `inline` and spliced at each call site to do it.
**A self-call needs no registry lookup, which is what separates it from the general
generic-calls-generic case (P7.S3k).** The general case fetches the callee's own signature out
of `poly_env` and relates its variables to the caller's; a *self*-call resolves against the
walk's own `sig`. `poly_call_term` recognizes the self-name by `ctx.mangled_name()` — the
*mangled* spelling, since `resolve::mangle` rewrites a self-call
body reference alongside the declaration it names — and matches the operand window against
`sig.inputs` **pointwise and structurally**, producing `sig.outputs`: the same comparison
the walk already runs against `sig.outputs` at body exit, run mid-body. No unification, no
`Subst`, no fresh `GenericTypes` mint; the rigid type-variable ids carry through unchanged.
A call to a *different* poly word takes P7.S3k's own arm, below.
**Polymorphic recursion is unreachable through bare self-call syntax, and so needs no
termination guard.** A self-call at *different* type arguments (`'T` recursing at
`['T 2]`) would demand a fresh instantiation per level, and monomorphization would never
terminate. Under the structural match it never reaches the instantiation worklist: an
operand shaped `Array('T, 2)` does not structurally equal `Var('T)`, so it is an ordinary
located operand/signature mismatch at the call site, rejected exactly as any other
declared-type mismatch is. That is a consequence of the checker's own comparison, not of a
separate guard. The underlying exclusion is still Sooth's monomorphizing codegen, not the
type system: an erased or boxed uniform representation compiles polymorphic recursion fine,
which is precisely what DESIGN.md declines (no trait objects, no vtables, no hidden
allocation).
**S3e's traits do not unlock it, and the two are orthogonal.** A bound answers *what
operations `'T` admits*, an abstraction question; polymorphic recursion is excluded by *how
many instantiations must be emitted*, a termination question, and a bound does not reduce
that count -- a polymorphically recursive word still demands a distinct instantiation per
depth whether or not its variable carries a bound. S3e is explicit that a satisfied bound
resolves at monomorphization with "no runtime representation, no vtable, no erasure, no
allocation", which is the opposite of the uniform representation this would need. Only trait
*objects* would change the answer, and S3e declines them on purpose. Bounded-depth
monomorphization (a fixed instantiation limit, as C++ templates effectively have) is
declined too: it is a larger finite bound with a worse error message, not a solution.
On the lowering side a self-call resolves through neither of the two paths an ordinary call
takes: the checker records no `CallInst` for it (the poly-body walk is abstract, with no
concrete θ to record) and the lowering `env` excludes every poly word. Its callee is
whichever instantiation is currently being emitted, so `lower_word_parts` carries the poly
word's own name (`cur_poly_callee`) and dispatches such a call to `cur_word_name`, this
instantiation's symbol, at that instantiation's own concrete arity. Both real entry points
thread it: the native monomorphization loop and the REPL's `lower_instantiation`.
**Exit:** a non-inline generic word can call itself at its own type arguments, compile at two
instantiations and run; and a self-call at different type arguments is a located type mismatch
at the call site, not a hang (`docs/roadmap/P7/slice3g-spec.md`, `tests/phase7_slice3g.rs`).

**P7.S3g-follow — The self-tail loop transform for a polymorphic body.** `[ done ]` A
self-call in tail position inside a non-inline generic body lowers to a loop back-edge, so a
self-recursive generic word runs in constant stack rather than one frame per recursion
level; a non-tail self-call stays an ordinary recursive call into the instantiation being
emitted. The tail predicate is shared rather than duplicated per body kind:
`has_self_tail_call` (`src/check/drop_graph.rs`) is a purely syntactic name-walk that
handles a poly signature, and `check_poly_body` builds its `Ctx` via
`check::engine::word_ctx`, which computes `self_tail_call` from
`has_self_tail_call(word, combs)` directly, so `Ctx::Word.self_tail_call` and
`ctx.is_self_tail_call()` answer for a generic body with no separate tail-detection
machinery. A poly-side back-edge guard (`src/check/poly.rs`) rejects a reference that would
cross the back-edge into a rebound local, mirroring the concrete guard
(`check_reference_across_back_edge`). On the lowering side both call sites that derive a
word's `self_tail` (`src/ir/driver.rs`, `src/repl.rs`'s `lower_instantiation`) pass
`has_self_tail_call(word, &combinator_bodies)`, and the poly self-call arm in
`src/ir/func_builder/calls.rs` dispatches a back-edge keyed on `self.cur_poly_callee` rather
than `self.env.get(name)`, which panics on a poly name.
**Exit:** a self-*tail* call in a non-inline generic body lowers to a loop back-edge, and a
generic countdown over a large counter runs in constant stack.

**P7.S3h — An escaping closure may capture a linear value (closure env disposal).** `[ done ]`
A closure that owns a linear capture says so in its *type*: `owning [ … ]`, a distinct
quotation flavour spelled in type positions only (a parser prefix at every type-position
entry, and a reserved name so `type: owning ;` cannot shadow it). The type carries the
obligation and says nothing about where the env lives. Such a closure is linear, and `call`
is its consuming use -- no new checker rule, since the pre-existing consumed-on-every-path
check already forces a conditional to call it on both arms, and forgetting one is the
ordinary unconsumed-value error.
**The env is heap storage the body owns.** Captures are *moved* into a `sooth_alloc` block
laid out per literal (each capture's own storage, word-aligned), so the frame no longer owns
them and the block outlives the return. The compiled body's prologue copies every capture out
into its own frame, rebinding a borrow capture with its reference record, and *then* frees the
block -- at entry, not at the return, so the body is thereafter an ordinary word body that
consumes its captures exactly as a word consumes a linear parameter, and an `owning [ -- Spy ]`
body may hand a capture back rather than dispose it. A plain quotation gains no allocation, and
the closure value's third slot (the per-site disposer **P7.S3v** added) is null for one.
**The body is a disposer, and was the sole one until P7.S3v.** A multi-output word's
synthesized return-bundle struct is interned after the containment audits run, so an `owning`
output reaches that struct as a field whatever those audits say; it stays sound because the
bundle is a destructor-free transient carrier, unpacked at the call site the instant the word
returns, never itself disposed as a container. Neither a spliced (`inline`) nor a generic word
may declare an `owning` parameter: the splice route never materializes, and a polymorphic call site
materializes from the declared effect alone, which does not carry the flavour -- so in both
the distinction would be silently unenforced.
**Capture admission.** `classify_capture`'s aggregate arm admits a capture as scalar only when
it is both `Copy` and scalar-represented, so a payload-free enum passes and a pointer-backed
aggregate keeps being rejected however `Copy` it is. At an `owning` boundary the frame-rooted
rejection and the 2+-capture deferral both lift for a *linear* capture, the heap block having
replaced the stack bundle they guarded; the in-frame path is unchanged.
**Two restrictions shipped here, both since lifted by P7.S3v's per-construction-site disposer:**
discarding an owning closure unexecuted (`drop`), and storing one in an aggregate (a struct
field, a variant field or an owned cell; an array or slice element waits on **P7.S5**).
Standing hazard, unchanged: a materialized quotation still cannot be linked in the REPL (a
session line building a `(code, env)` value dies on a non-PIC `__quot0` relocation).
**Exit:** a linear value can be moved into an escaping closure's env, the closure returned
from the word that built it, and calling it observes the capture and disposes it exactly once
-- one observation, not zero and not two -- with a forgotten closure and a capture the body
never consumes each a located error rather than a silent miscompile.

**P7.S3i — `bool` as an ordinary enum, not a compiler-injected one.** `[ done ]` The type was
a plain two-variant zero-payload enum already; the only thing making it special was that
every module's registry was injected with it at a fixed slot ahead of any user enum, so
`Type::from_name("bool")` and the `true`/`false` literal spellings resolved with no import,
unlike every other type P8.S2 made import-gated. Once enums became a real, checked,
user-declarable mechanism (Phase 6) with the same zero-payload scalar layout `bool` already
used, no representational reason for the carve-out was left. `core::bool` declares
`type: bool | False | True ;` and the bool `.` print overload as ordinary source beside
`if`/`unless`; the two injection sites and the fixed slot are gone; `true`/`false` are parser
sugar for the `False`/`True` constructors of *the imported* enum, so a file that imports
neither `core::bool` nor a hub carrying its constructors cannot spell a boolean literal at
all. The compiler resolves the boolean type from the registry per read
(`ast::resolve_bool_type`, which also requires the payload-free shape the logical operators
and the `extern:` boundary rest on) rather than from a constant, and user enums start at
registry index 0.
**Import shape, and the one thing a hub cannot carry.** `core::prelude` re-exports the
`true`/`false` constructors, but neither the *type* `bool` nor the `.` overload: a type name
resolves against its declaring module, and an operator overload's candidate lookup spans the
calling module and the one it selectively imported the name from, one hop. So a file that
spells `bool` in an effect, or prints one, names `core::bool` directly. Widening either to
follow a re-export is its own slice, unclaimed.

**P7.S3j — A shape-changing combinator parameter declaring a slot above its row.** `[ done ]`
A row-typed inline combinator's quotation parameter may declare fixed slots *above* its
output row (`~[ ..a -- ..b i64 ]`, as a hand-written `pick` combinator does), and a
non-inline generic body calling it grounds. `poly_combinator_call` strips the declared
suffix off each arm's exit before reading the produced row -- the fixed point
`check_literal_against_declared_effect`'s shape-changing branch applies on the monomorphic
path -- and holds only the *stripped* region to the cross-arm agreement that fixes the exit
row, so the call hands back the combinator's declared row and the backend's row-length
arithmetic (`ir/func_builder/quotation.rs`) never sees the extra slot.
**Two rules the strip carries, both located.** A suffix disagreeing with the declaration in
type *or* in length is rejected at the arm; the length half is not redundant, since a short
exit would otherwise pass a per-slot comparison that truncates to it and reach lowering.
And two parameters sharing one declared output row must declare the *same* suffix, rejected
before any arm walks: they feed one join and one continuation, so differing suffixes
describe an exit no body can satisfy, while stripping each parameter's own suffix off its
own arm would hide the difference from the cross-arm rule and leave a slot the exit row has
no account of. A variable-carrying suffix stays the
`poly_combinator_abstract_signature_error` rejection: the strip needs the declared types
ground.
**Exit:** a row-typed combinator parameter may declare a slot above its output row, and a
generic body calling it grounds and strips that slot the same way the monomorphic path does.

**P7.S3k — A non-inline generic word calling another generic word.** `[ done ]` A non-inline
generic body may call another generic word -- same-module or imported, user-defined or a library
word like `gt`/`lt` -- passing its own rigid type variables through. `poly_call_term` fetches the
callee's signature out of `poly_env` (the registry a monomorphic caller already dispatches
through) and relates its declared inputs to the caller's operand slots **symbolically**, since
neither side has a θ while a generic body is walked: what comes out is a variable-to-variable
mapping (callee variable -> a caller rigid variable, or -> a concrete type), recorded per call
site as a `PolyCrossCall` on `Module::poly_cross_calls`. A bound the callee declares is
discharged there, against the caller's own declared bound set, so an unsatisfied one is a
located call-site error and never a monomorphization-time failure.
**Monomorphization is a check-time fixpoint, not a lowering-time worklist.** A record grounds by
composing its mapping with a concrete θ of the caller, and those θs are exactly the `CallInst`s
the checker already holds -- so composition, `apply_subst`'s registry interning, and
return-bundle interning all run inside `check`, seeded from the recorded instantiations and
iterated to a symbol-deduped fixpoint. Lowering walks the finished graph only:
`CallInst::poly_calls` routes one body span to the composed callee for *this* caller
instantiation (the global `Span`-keyed table structurally cannot, since one span serves every θ
the body is instantiated at), and `Module::transitive_instantiations` holds the flat set, so each
composed `(callee, θ)` mints one `IrFunc`.
**Termination is a property of the mapping rule, not a depth cap.** A callee variable's image
must be either fully concrete or a bare caller variable -- and "fully concrete" does **not**
mean any concrete type folds: a concrete scalar or generic aggregate (`Box[i64]`) folds cleanly,
but a concrete `Ref`/`Array` image (`&i64`, `[i64 4]`) is refused too, since folding one needs a
fresh `RefId`/`ArrayId` and the poly-body walk holds no mutable path to mint one (only
`structs`/`enums` do). A compound image that *mentions* a caller variable -- the caller wrapping
its own `'T` in a `Box['T]` before handing it over -- is refused as growth for the same reason
R6 refuses it anywhere else. Under that rule every composed θ draws its types from the finite
pool the seed instantiations introduced, so the reachable `(word, θ)` set is finite and a mutual
`g <-> h` cycle revisits `(g, θ)` at the *same* θ and stops. The over-rejection is deliberate: a
single, non-recursive wrap would terminate and is refused too, which buys a check-time
structural rule with no cycle detection.
**Residual narrowings, each its own located rejection.** A callee signature carrying a row
(`..s`), a quotation parameter, a length variable, a user trait bound, or a compound *output* is
refused by name: the first three have no image kind to map to, a user bound's recorded
obligations resolve per ground θ and nothing composes them across a cross-call, and a compound
output would need the interning only a ground θ gets. A concrete `Ref`/`Array` operand image is
refused too, for the minting reason above. A cross-call into or out of a *polymorphic overload
set* is refused too -- the records merge under one name while each indexes its own candidate's
variables. A cross-call whose caller is itself an `inline` combinator is refused at the outer
call site if the spliced body calls a further polymorphic word -- splicing composes no θ for the
nested call, so the fixpoint would otherwise reach a callee it cannot ground. The REPL keeps the
old `unknown word`: its lowering resolves an instantiation through a per-generation store nothing
composes a cross-call into, so grounding there would check clean and then mis-lower.
**Exit:** a non-inline generic word may call another generic word -- same-module or imported,
user-defined or a library word like `gt`/`lt` -- passing its own rigid type variables through;
the callee is monomorphized once per concrete instantiation the caller reaches, the same way a
concrete caller's generic callees already are; a callee whose bound the caller's type variable
does not satisfy is a located error, not a hang or a monomorphization-time panic; a mutual
non-growing pair compiles, runs, and terminates compilation; and a growing cross-call is a
located call-site rejection (`docs/roadmap/P7/slice3k-spec.md`, `tests/phase7_slice3k.rs`).

**P7.S3l -- A poly body calling a bound instantiation of an abstract quotation parameter.**
`[ done ]` Named at P7.S3f's exit, out of scope there. A still-abstract declared quotation
parameter (one whose brackets mention a type variable, e.g. `[ 'T -- 'T ]`) is now live at
both boundaries of a non-inline generic word. In the **body**, `poly_call_term`'s `call`
handling dispatches on a `PolyType::Quotation` operand through a dedicated arm parallel to
S3f's R3, consuming the declared inputs and pushing the declared outputs by structural row
comparison; no new representation was needed. At the **call site**, R9p spares such a slot
instead of rejecting the literal outright: `check_poly_call` unifies every non-quotation input
first, then grounds each declared quotation slot through the completed `Subst` and
materializes the operand exactly as a ground slot's is, so the declared parameter order does
not matter. `subst_polytype` mirrors check's `apply_subst` for the same shape, so the
monomorphized body lowers through the ordinary indirect-call path.
**Exit (met):** a poly body may `call` its own declared, still-abstract quotation parameter,
consuming its declared inputs deepest-first and pushing its declared outputs, each row slot
compared structurally (via `PolyType`'s derived `Eq`) against the operand's own `PolyType` --
no `Subst` is built or consulted mid-body, since every type variable stays rigid there
(`docs/roadmap/P7/slice3l-spec.md`, `src/check/poly.rs`, `tests/phase7_slice3f.rs`). A
literal quotation argument reaching a still-abstract declared position at the *call-site*
boundary (not the body) grounds through the caller's own `Subst` and materializes exactly as
a ground declared quotation slot does.

**P7.S3m -- A declared quotation effect with two or more outputs cannot be lowered.** `[ done ]`
Named at P7.S3f's exit (its ">=2-output lowering gap" finding), pre-existing on the concrete
path and confirmed reachable (without being caused) on the poly path once S3f's R3 landed.
`intern_output_bundles` (`src/check.rs:913`) interns an output-tuple bundle only for a
*declared word*'s outputs, walking `module.words`; a quotation effect's own output row is
never interned, so `bundle_of` returns `None` in `lower_indirect_call`
(`src/ir/func_builder/quotation.rs:226`) and a `call` on a `[ ... -- A B ]`-shaped (two or
more outputs) quotation panics in the backend the moment a consumer reads the second output.
Probe: `: call_it ( [ i64 -- i64 i64 ] -- ) 3 swap call add print ;` panics identically whether
`call_it` is concrete or (per S3f's R3) polymorphic. **Exit:** a declared quotation effect
with two or more outputs interns an output bundle the same way a declared word does, and
`call`ing one on either the concrete or polymorphic path pushes all declared outputs rather
than panicking.

**P7.S3n -- A generic struct's field wrapping its own type variable.** `[ done ]` Named at
P7.S3e's brief (the `Map['K 'V]` consumer's own blocker, `docs/roadmap/P7/slice3e-brief.md`),
distinct from S3a, which fixed generic-applied-to-own-variable in poly *word* signatures
rather than in a generic type's own field list. A generic struct or enum field may wrap the
header's own type variables to any depth: an array (`type: Pair 'T items ['T 2] ;`), a
nested array, an owned cell (`^'T`), and a generic application over them
(`slots [Ent['K 'V] 8]`, the shape `Map`'s open-addressed backing storage needs). Each
grounds per concrete instantiation, interning the shape it produces at each level, and two
instantiations of the same header get distinct registry ids. One mechanism covers the
whole family: a recursive field-type parser feeding a substituter that re-enters the
instantiator.
Three shapes are located rejections rather than gaps: a **reference** field
(`&'T`, `&Ent['K i64]`) meets the pre-existing no-stored-reference rule; a **quotation**
field naming a type variable is rejected at the parser; and a *growing* self-reference
(`type: A 'T next ^A[^'T] ;`, whose instantiation would never converge) is rejected at
declaration, while a **permuting** one (`type: A 'K 'V next ^A['V 'K] ;`) is legal.
A self-referential header terminates because the instantiator mints its id, memo key and a
placeholder decl *before* substituting fields; an owned cell is the only indirection that
breaks the size cycle, so a by-value or array-wrapped self-reference is an infinite-size
error.
**Exit:** a generic struct or enum may declare a field wrapping one of its own type
variables under an array, owned cell or generic application, resolved correctly per
concrete instantiation
(`docs/roadmap/P7/slice3n-spec.md`, `tests/phase7_slice3n.rs`). This removes the
array-field-of-own-type-variable blocker; it does **not** unblock `Map['K 'V]`, which
still needs a struct-header length variable (`'N`) and the `Eq`/`Hash`/`Default`-style
bounds named above.

**P7.S3o -- A bound on a poly combinator's own type variable has no dispatch mechanism.**
`[ done ]` Named at P7.S3e's round-1 review (its spec's own R9/R17), out of scope there.
`reject_user_bound_on_combinator` (`src/check/poly.rs:6129`) is a clean, located rejection, not
a bug. Recon'd and spec'd twice; both designs (a splice-uid resolution key, then a
source-derived `SplicePath` key) were found unsound in review. A third recon round (three
parallel probes) corrected the brief's central latency claim, found a more fundamental
blocker both prior rounds missed, and settled one open item -- see
[slice3o-brief.md](./P7/slice3o-brief.md) for the full recon record and the revised open items.
No spec currently exists for this slice. This is a **hot-path optimization**: a combinator
calling a bare trait member (`cmp` directly, not the `gt` wrapper) cannot dispatch today, so
the fallback is a non-inline word paying a real call frame per instantiation (the shape S3s
chose for `mymax`/`mymax3` precisely to avoid this slice). Bare trait member calls in
combinators will be in the hot path of many programs. The round-3 probes found the primary
blocker is an `i64` stand-in collision: `check_poly_combinator_standalone`'s `i64` scratch
instantiations clobber real `i64` instantiations, silently miscompiling the motivating
program when instantiated at both `i64` and another type. Option (b) -- reject bound
dispatch inside materialized quotations rather than prefixing the resolution key -- is
settled as sound (the motivating program's `~[ ]` arms are spliced, never materialized). The
transitive skip is cheap (a `Provenance` flag) but the hard part is injecting
`poly_trait_member_call` into `check_terms_relaxed` (the splice path has zero bound-dispatch
calls today). **P7.S3s supplied the motivating program**: a bounded `inline` comparison over a library
`Ord`. The library's six comparisons are now `inline` (P7.S8), so the dispatch-target-diffing
oracle this slice originally planned to build no longer has a non-inline baseline to diff
against -- P7.S8's own oracle work found that comparison permanently unsatisfiable once the
library inlines and dropped it, keeping a stdout-identity check with `gt`/`lt` swap controls
on both sides instead (`tests/phase7_slice3s_oracle.rs`). This slice's own oracle strategy
needs re-deriving against a real second variant now that `mymax`/`mymax3` are the only
remaining non-inline comparison-adjacent surface; `examples/poly_if.sth`'s `main` calling
both `mymax` and `mymax3` (currently only `mymax3` is called, so `mymax` mints no monomorph)
is still a prerequisite, and still belongs in a new fixture, since `tests/corpus_stdout/
poly_if.txt` must stay byte-identical.

**P7.S3p -- A trait member declaring its bound variable at any input position.** `[ done ]` A member's
receiver may sit anywhere in its declared input list, not only last: `at ( &'T i64 -- i64 )`
(an index/lookup shape) declares and dispatches through a bound.

Dispatch (`src/check/poly.rs::poly_trait_member_call`) is **name-first**. It finds which of the
body's bound traits declares a member of the called name, each candidate carrying its own type
variable, then reads that member's declared input list to locate the receiver in the operand
window. Selection never reads operand shape to *decline* a candidate; the per-input check
against the stack window is the sole place a mismatch is reported, so a wrong operand is a
located error rather than a fall-through to `env.get`. Shape does decide *which* variable a
call means when candidates span several (below), never whether to dispatch at all. Bound
dispatch continues to front every name-based special case in `poly_call_term` (the S3e
ordering invariant): position-finding is internal to candidate selection, not a change to
call order.

**Two candidates, two rules.** Candidates sharing one variable are the ambiguity error: no
operand shape can pick between them without an overload-resolution mechanism the language does
not have, and input position is not a disambiguator either. Candidates spanning *different*
variables are separated by the operands the call consumes, which is what keeps one trait bound
on two variables (`f ( &'T: A &'U: A -- ) ta ta`, two obligations resolved against their own
thetas) callable. The fit must be unique; several fitting candidates is the ambiguity error,
and *no* candidate fitting is its own `no_candidate_fits_operands_error` -- nothing is
ambiguous there, since every candidate's declared operands already disagree with the stack and
no module qualifier would change that.

**The declaration gate** (`member_binds_trait_var`) requires the trait's variable as `'T` or
`&'T` *directly* in some input, or admits an empty input list outright. It is syntactic: a
receiver mentioned only nested inside a composite input (`sum ( ['T 4] -- i64 )`) is still
rejected, since grounding it would need structural unification through the array type. A
nullary member (`fresh ( -- i64 )`) is the zero-receiver case, resolved by **P7.S3t** below.

Because selection is by name alone and so cannot fall through to a builtin on a shape
mismatch, a member name that a builtin arm owns would capture every such call in a bounded
body. `trait:` therefore rejects any member spelled as a name-dispatched builtin, `call`,
`slice`, and `subslice` included (each is its own arm in `check_term`/`poly_call_term` and is
absent from `BUILTIN_WORDS`, so each needs its own gate). The six surface comparisons stay
legal: they are `lib/` words, and a body that imports one receives it mangled, so the
spellings never collide.

**Exit:** a trait member may declare its bound type variable at any input position, not only
last, and a call to it dispatches correctly regardless of position -- the S3e declaration-time
rejection for a non-trailing `'T` is lifted.

**P7.S3q -- An intrinsic gated into a module can be re-exported through a hub.** `[ done ]`
`import: intrinsics | drop | ;` flips a permission bit (`IntrinsicVisibility`) on a module; a
module's *effective* visibility (`driver.rs::effective_intrinsics`) unions that bit with every
intrinsic name transitively reachable through a selectively-imported hub that itself admits
it, computed once per module before `ModuleInfo` is built. `export:` accepts a name the
exporting module effectively admits, with `resolve.rs::resolve_export_origins` resolving its
origin to the exporting module itself (never mangled against a hub, so builtin dispatch still
sees the bare name) -- but only once a real declared or re-exported word of that name is ruled
out first, so a hub that both admits an intrinsic and re-exports a real word of the same name
still resolves to the real word. The caller-side gate
(`check/word_families.rs::intrinsic_is_gated_out`) is unchanged: it consults the calling
module's own (now effective) `intrinsics` field, so a module reaching `drop` only through a
hub calls it bare with no `import: intrinsics` line of its own. A hub's own
`import: intrinsics * ;` does not leak all intrinsics through it -- only names on its own
`export:` list cross. `core::prelude` re-exports `drop`.
**Exit:** a module that imports a hub re-exporting a gated intrinsic (e.g. `drop` through
`core::prelude`) may call it bare, with no direct `import: intrinsics` line of its own.

**P7.S3t -- A zero-receiver trait member has no call-site signal to dispatch on.** `[ done ]`
Named at P7.S3p's spec, out of scope there. `fresh ( -- i64 )` (a nullary constructor) binds
its trait's type variable in no declared input, so nothing at the call site grounds *which*
concrete type's member is meant. Resolved by an explicit call-site type-argument list,
`f[Point]`, parsed by span adjacency on the callee's token spans and seeded into `Subst`
ahead of operand unification (`check_poly_call`); the declaration gate
(`member_binds_trait_var`) now admits an empty input list.
**Exit:** a trait member binding its variable in no input may be declared and dispatched,
reachable from a concrete call site through one bounded generic word (`f[Point]` grounds
`f`'s own `'T`, then bound dispatch reaches `fresh`). The explicit list is only needed when
nothing else grounds the wrapping word's variable: where an ordinary operand does
(`g ( 'T: Default -- )` called as `1 Point g`), the relaxed gate alone makes the nullary
member dispatchable. Chaining through a second bounded
generic word is not expressible: `parse_type_expr` has no production for a type variable, so
a generic word cannot forward its own variable through an explicit instantiation (`f['U]`
inside `g ( 'U: Default -- 'U )` does not parse) -- a stated residual gap, not designed here.

**P7.S3s -- `Ord` as a library trait, not a compiler-hardcoded bound.** `[ done ]` `Ord` is an
ordinary library trait (`lib/cmp.sth`), declaring `cmp ( 'T 'T -- Ordering )` over an
`Ordering` enum, satisfied nominally by an `impl:` block -- one per numeric width in `core`,
built from the raw comparison intrinsics -- exactly as any other trait dispatches through the
whole-program `(TraitId, Type)` impl registry `Bound::User` (S3e) already uses. The six
surface comparisons (`eq`/`lt`/`gt`/`lte`/`gte`/`ne`) are derived from `cmp`. `'T: Ord` bounds
a struct or enum like any other bound, and a user type opts in with its own `impl: Ord for
Point`; there is no compiler-hardcoded notion of "ordered" left to reject it.

The six comparisons are `inline`, so a comparison and its `Ordering?` eliminator fold into
the caller and a library comparison costs no call frame (**P7.S8**, which supplied the uid
rule that makes a spliced `impl:` body lower correctly). The REPL carries no whole-program
trait/`impl:` registry, so a bound naming `Ord` at REPL scope gets a located, REPL-specific
diagnostic pointing at that gap rather than claiming the name is wrong
(`repl_unknown_capability_error`), in first position and after a folded `Copy` alike.
Closing the gap itself -- a session carrying its imported
modules' trait and `impl:` registries, so `'T: Ord` resolves at REPL scope the way it does in
a file -- has no owning slice.
**Exit:** `Ord` bounds a struct or enum, satisfied nominally by an `impl:` block, so a
comparison-bounded generic word (`sort`, `bin_search`) can be instantiated over a user type;
a polymorphic body may call a polymorphic word carrying a forwarded user bound without ICE;
the numeric tower needs no user-written `impl:`; and every existing `'T: Copy Ord` program
still behaves identically.

**P7.S3s-follow -- Trait member declaration syntax, and an `inline` trait member.** `[ planned ]`
`cmp` and the six surface comparisons are `inline` (P7.S8); bound dispatch reaches a
spliced/materialized body, so an `inline` trait member is mechanically live for the library's
own trait. What is missing is the declaration surface for a *user*-written trait. A trait member
today is a bare `name ( sig )` inside `trait: Name 'T ... ;`, with no slot for the `inline`
keyword `parse_worddef` already recognizes between a word's name and its `(` -- and no slot
*could* be added without one, since `impl:` bodies inherit the trait's signature verbatim
(restating it is a parse error) and `parse_impl_member_body` hardcodes `declares_inline:
false` for every member regardless of impl. This slice does two things together, because the
second has nowhere to live without the first: (1) trait members are declared
`: name ( sig ) ;`, matching every other word-shaped declaration in the language instead of
the bespoke bare-`name` grammar, and (2) an optional `inline` keyword in that now-familiar
slot marks a member so every `impl:` body satisfying it is spliced at its call sites, the
same way `eq`/`lt`/`gt`/`lte`/`gte`/`ne` already are over `cmp`. `inline` is a property of the
member's declaration in `trait: ... ;`, not of any one `impl:` block -- a trait's contract
should not let one conforming type opt out of a call-frame cost another pays. Two consumers
exist in tree: `examples/traits.sth` and `lib/cmp.sth`; `docs/book/` needs checking for
whether it teaches the old bare-member grammar (already flagged separately as teaching
rejected `if`/`else`/`end` syntax).
**Exit:** `trait: Ord 'T : cmp inline ( 'T 'T -- Ordering ) ; ;` parses, and each `impl: Ord for
...` block's `cmp` is spliced at every call site reached through a bound `'T: Ord` word.
`cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green.

**P7.S3u -- Trait objects (an erased owner with a reachable destructor).** `[ parked ]` Traits dispatch
statically today: `Bound::User(TraitId)` (`src/ast.rs:1682`) is discharged per concrete
instantiation against a whole-program `(TraitId, Type)` registry, so every call is
monomorphized and every destructor is type-directed and statically resolved. There is no way
to hold a value whose concrete type is unknown at the use site, and therefore no way to reach
its destructor through the erasure. The shape already exists in the compiler: a materialized
quotation is a `(code, env)` pair, which is a one-method vtable plus a payload, and
`lower_indirect_call` (`src/ir/func_builder/quotation.rs:203`) already performs a runtime call
through a code pointer. What is missing is a vtable with more than one slot, a way to name the
erased type in a signature, and the disposal answer. **The disposal answer is the whole
slice.** A trait object owning a linear value needs its destructor reached *through the object*,
which puts dynamic dispatch on the destructor path, exactly where the linear spine bottoms out.
DESIGN.md's memory model is explicit that disposal is deterministic and statically resolved,
so this is a design amendment and not a mechanical extension: it needs a ruling on whether an
owning trait object's vtable carries a `drop` slot (a hand-rolled `dyn Drop`, one indirect call
at disposal, no hidden control flow beyond the call the programmer wrote), and on whether an
object may be `Copy` at all. Sequence after **P7.S3h**, whose spec deliberately avoids the
mechanism so that this slice introduces it once, with a consumer, rather than being reverse
engineered from a closure special case.

**Parked for want of a consumer.** Disposal through erasure was the forcing consumer, and it
no longer forces this slice: **P7.S3v** disposes an owning closure through a
per-construction-site disposer symbol, which needs no vtable, no erased type in a signature,
and no amendment to the statically-resolved disposal rule. Traits could not have supplied that
answer in any case: a trait keys on a type, and `Type::OwningQuotation` (`src/ast.rs:2295`)
carries only the effect, so two closures with identical effects and different capture sets are
the same type and would select the same `impl:`. What remains is heterogeneous collections
(`Vec[dyn T]`), which the scope guard already called a weak motivation on its own, since a
closed enum is usually the better design and already works in `core`. Unpark when a consumer
appears that a closed enum genuinely cannot serve.
**Exit:** a value of a user trait can be held and used behind an erased owner whose concrete
type the use site does not name, a bounded word can be called through it, and an owning trait
object's destructor runs exactly once through the object with no leak and no double free, with
the `Copy`-versus-linear rule for objects stated and tested.

**P7.S3v -- Dropping and storing a linear-capturing quotation.** `[ done ]` A quotation value
carries three words: its code pointer, its env pointer, and a disposer synthesized per
*construction site*, which disposes each capture through that capture's own type and frees the
env block. The disposer is keyed on the construction site rather than on the type because
`Type::OwningQuotation` (`src/ast.rs:2295`) carries only the effect: two closures with identical
effects and different capture sets are the same type, so nothing type-directed (a `Drop` trait,
a blanket or specialized `impl:`, a trait object's vtable) could discriminate them. The capture
types are always known where the symbol is minted (`materialize_quot_value`), so it is always
constructible. `emit_drop` loads the slot, null-checks it and calls it indirectly: one indirect
call on the disposal path, no runtime type info in the env block, no dynamic dispatch on any
user-visible operation.

**Both flavours carry the third word**, and a capture-free literal's is null, mirroring its null
env. `:Q{n}` is keyed on the effect alone (`quot_index`, `src/backend/qbe.rs:57`), so a plain and
an owning quotation of the same effect are one backend symbol and must stay byte-identical;
diverging the width would re-key that symbol on owning-ness across every site that maps the two
together, to save eight bytes on a materialized closure.

**`drop` on an owning closure is a consuming use distinct from `call`.** `call` runs the body,
which may do arbitrary work before disposing the captures itself; `drop` runs only the disposer,
discarding the closure unexecuted. An owning closure may be a struct field, an enum variant
field or an owned-cell payload: `field_is_linear`/`layout_field_is_linear` (`src/ir/layout.rs:66`,
`:895`) see one, so the container synthesizes a destructor and its ordinary field glue reaches
`emit_drop`. Array and slice elements stay rejected -- a linear element is **P7.S5**, whatever its
type -- as do a reference referent and an `extern:` slot, neither of which owns what it names.

Standing hazard: a closure capturing an array or a slice ICEs at `src/backend/qbe.rs:531`
(`an aggregate field is copied by blit, not scalar-stored`), because the env store assumes one
word per capture. That is the env block, not the closure value, and a scalar or linear-struct
capture works end to end.

REPL: the disposer calls aggregate destructors through `emit_drop`, which resolves them by the
aggregate's positional id symbol (`struct_drop_symbol` and its three siblings,
`src/ir/destructors.rs:8-35`) with no plumbing of its own. Unreachable rather than untested: a
disposer exists only for a *materialized* closure, and no session line can link one (the standing
`__quot0` non-PIC relocation, which storing a closure in a field forces exactly as building a bare
one does), so no session reaches the disposer at all. Pinned as a blocked-state tripwire in
`tests/phase7_slice3v.rs`, beside a plain-quotation control showing the limit is not owning's.
**Exit:** an owning closure can be `drop`ped without being called, disposing its captures and
env exactly once; it can be stored in a struct field, a variant field or an owned cell and
disposed transitively through the container exactly once; and every S3h golden that asserted a
rejection has been migrated to assert the new behaviour rather than deleted.

**P7.S4 -- Generic `impl:` targets, with a specificity chain.** An `impl:` target must name one
concrete type today; this slice lets it name type variables and shape constructors over them
(`impl: Show for ['T N]`), with the most specific match winning and an unordered candidate set
being a located error (no tiebreak rule). See [slice4-brief.md](./P7/slice4-brief.md) for the
full brief, the two-layer rejection confirmed live, and the exit criteria.

Out of scope: `drop`, which is not a trait and is not becoming one -- its blanket behaviour is
synthesized field-wise glue, not a writable default body, and an owning closure's disposer keys
on the construction site rather than the type (**P7.S3v**); trait objects (**P7.S3u**, parked);
default member bodies and supertraits, still unforced. Sequence after **P7.S3s**, which is the
first slice to give the impl registry a real multi-`impl:` consumer in `core`.

**P7.S4b -- Bounds on impl variables.** S4 shipped generic `impl:` targets but left
bounds on an impl's own type variables out of scope: a generic impl's member word carries
`PolySig { bounds: vec![], .. }`, so a body that calls a trait member on an impl variable
(e.g. `impl: Show for ['T N]` whose `show` iterates and calls `show` on each element,
requiring `'T: Show`) falls through to ordinary word lookup and fails to resolve. This slice
closes that gap: new grammar for impl-bound declarations, threading bounds into the member
word's `PolySig`, and recursive per-instantiation bound discharge. See
[slice4b-brief.md](./P7/slice4b-brief.md) for the full brief and exit criteria.

Out of scope: `drop` (not a trait); trait objects (**P7.S3u**, parked); default member bodies
and supertraits, still unforced. Sequence after **P7.S4**, which landed the generic target
pattern matching, specificity chain, and polymorphic member word that this slice populates
with bounds.

**P7.S5 -- Linear array elements.** `[ done ]` `[T N]` rejects a linear element for every linear type:
`type: Arr xs [Spy 2] ;` and `type: Arr xs [owning [ -- ] 2] ;` both fail with `linear array
elements are not supported yet` (`src/check.rs:3227`), while the same type as a struct field
(`type: Box s Spy ;`) builds. So a linear struct is storable but a *collection* of them is not,
which is the one gap that keeps the linear spine from reaching arrays. The restriction dates to
P3 (`P3-linear-spine.md:56`, `P3/slice1-spec.md:61`) and has been re-observed from four slices
since (`P3/slice3-brief.md:70`, `P4/slice6b-brief.md:142`, `P7/slice3c-brief.md:242`, and
**P7.S3v**), always as somebody else's blocker, never as a slice.

Three things make it a real slice rather than a predicate flip.

**Construction.** `fill` replicates one value across every slot, which is a copy per slot
and therefore illegal for a linear element — the diagnostic's own wording ("would replicate a
`{elem}` across every slot") names why. The `[Type; Count]` constructor (`parse_array_ctor_term`,
`src/parser.rs:4021`) is a separate path: it takes a *type* (not a value) and zero-initializes
the allocation (`src/ir/func_builder/calls.rs:74`, a byte-granular memset loop), so it cannot
produce a valid linear element either, since zeroed memory is not a constructed value. It is
the redundant third construction path — overlapping with `fill` for copy types, useless for
linear types — and this slice drops it, migrating its usages to `fill` and deleting the
`array_ctor_ahead` term-parser lookahead (`src/parser.rs:3990`) that distinguishes it from a
quotation. The memset-zero path survives as a `fill` lowering optimization (when the seed's bit
pattern is all zeros), not a separate surface form. A linear array needs a construction form that
produces N *distinct* values, never one replicated N times.
The cheapest shape is an intrinsic generation combinator — `tabulate ( usize ~[ -- T ] -- [T N] )`
— whose lowering is `fill`'s loop body with one swap: instead of storing a replicated seed each
iteration, the loop calls the quotation (spliced, so `tabulate` is `inline` like `times`), gets a
fresh `T`, and stores that. The IR pattern is already proven: `fill`'s `alloc_array` → store
loop → `push dst` (`src/ir/func_builder/word_families.rs:394-454`) allocates raw, uninitialized
storage that never surfaces as a type-system value until every slot is written, which is
exactly the boundary a generation combinator needs. No new storage category is required — the
"no uninitialized memory" rule is a type-system boundary, not an IR constraint, and the IR
already crosses it inside `fill`.

A narrower relaxation also reaches some linear arrays without a new word: `fill` could admit a
*nullary-variant seed* (e.g. `None 3 fill`) even when the enum type is linear, because a nullary
variant carries no linear data to replicate — only a discriminant. The gate
(`check_array_element_gate`, `src/check.rs:510`) currently checks `is_copy` on the *type*; it could instead check whether the *seed value* is nullary, which is
safe regardless of the type's linearity. The lowering choice follows from the discriminant: a
discriminant-0 nullary variant (the first-declared, like `None`) can memset to zero (its bit
pattern is all zeros, same as the `[Type; Count]` path); a non-zero nullary variant needs `fill`'s
store loop writing the correct discriminant. This does not require `Option` to be intrinsic
(DESIGN.md is explicit that it is an ordinary generic enum, never a compiler primitive), and it
does not require a `Default` trait (a `Default`-based construction would still be replication
under another name, which is the same contradiction `fill` hits for a truly linear type with a
real destructor obligation). It does not solve the general case — a linear array of *distinct*
values still needs the generation combinator — but it covers the sentinel-initialized backing
array the `fixed`-layer collections (P9.S1) need.

**Disposal.** An array of linear elements needs synthesized element-wise glue with a static trip
count, plus the partially-initialized window during construction, which is the first place in the
language where a value is neither wholly live nor wholly disposed.

**Why not a capacity/length array type.** A `[T N M]`-shaped type baking a runtime length into
the array was considered and rejected: it collapses the storage/view split (DESIGN.md: "length
lives in storage (`[T N]`), carried at runtime by the view (`Slice[T]`)"), breaks `len`'s
constant fold (`src/ir/func_builder/word_families.rs:508`), kills compile-time index checking
(`check_array_index`, `src/check/word_families.rs:1280`), and makes `dup`/`drop` semantics
ambiguous (copy/dispose N slots or M?). The fully-initialized fixed array is a common value type
(lookup tables, coefficient banks, pixel data) that should not pay a runtime-length tax for the
container case. The container with a capacity/length distinction belongs in the `fixed` layer
(P9.S1) as an ordinary library struct wrapping `[Option[T] N]` + `usize`, not as a redesign
of the language's array type.

**The RT reservation question.** The `Option[T]` sentinel approach initializes N `None` slots at
construction and overwrites them as values arrive, which is bounded and predictable but not
free. The IR's `Instr::Alloc` (`src/ir/types.rs:431`) gives raw, unzeroed storage; `fill`'s
lowering already uses it and writes the seed in a loop. A zero-cost reservation (allocate raw,
gate access by a runtime length, never let the type system see an uninitialized slot) is
mechanically available at the IR level but would be a carve-out from the "no uninitialized memory
in the type system" principle, the same shape as the static-storage carve-out from linearity
(`docs/design/embedded.md`). This is deferred to P11 (bare metal), where a concrete RT program
can prove the sentinel init cost bites; the `Option[T]` sentinel version is correct and safe
today, and the intrinsic is the optimization a real program would demand.

Out of scope: a dynamically-sized or growable array (that is a library `Vec` over an allocator,
and needs a struct header length variable that **P7.S3n** named and did not land); a linear
element reached through a `Slice[T]` view, since a view does not own what it points at;
zero-cost reservation without a sentinel (P11, pending a concrete RT consumer).
**Exit:** `[T N]` admits a linear `T`; such an array can be constructed without any element
being copied (via a generation combinator that produces N distinct values, or a nullary-variant
seed that carries no linear data); dropping it disposes every element exactly once; a
partially-constructed array abandoned mid-construction is either rejected with a located error
or disposes exactly the slots already initialized, with the rule stated; and the `linear array
elements are not supported yet` diagnostic is gone rather than reworded. The `[Type; Count]`
constructor is dropped, its usages migrated to `fill`, and the `array_ctor_ahead` term-parser
lookahead is deleted.
Detail: [slice5-linear-arrays-brief](./P7/slice5-linear-arrays-brief.md). Probe-verified
against the current tree (five `litellm/syn-large-text` worktree-isolated read-only probes);
corrected line references: the diagnostic is at `src/check.rs:3227` (not `:3135`),
`parse_array_ctor_term` is at `src/parser.rs:4021` (not `:3719`), `fill`'s lowering is at
`src/ir/func_builder/word_families.rs:394-455`, array drop is `unreachable!` at
`src/ir/func_builder/quotation.rs:412`, and `synthesize_aggregate_destructors`
(`src/ir/destructors.rs:37`) does not handle arrays. The orphaned testing-vocabulary brief
was renamed and split into **P7.S7a-S7d** (see that entry below).

**P7.S6 -- Surface syntax unification.** `[ planned ]` A legibility pass over the polymorphic
surface: the anonymous array type `['T 'N]` becomes `array['T 'N]` (naming the type, as
`Slice[T]` already is, so a bare `[` unambiguously opens a quotation and the
`quotation_type_ahead` lookahead scan is deleted); `type:`/`trait:` binding sites move from
postfix type variables (`type: Box 'T`, `trait: Ord 'T`) to bracketed parameter lists
(`type: Box['T]`, `trait: Ord['T]`), unifying the spelling with generic application
(`Box[i64]`); and a word's bounds move from inside the effect (`: word ( 'T: Ord 'T -- )`)
to a bracket before it (`: word['T: Ord] ( 'T -- )`), separating variable-and-bound
declaration from the stack effect. The `'` prefix on type variables is retained; `^'T` (owning
cell) and `&'T` (reference) stay sigiled. Parser-and-test change only: the AST and every
downstream checker/lowering/IR consumer are unchanged. The bracket binding site this slice
introduces is the foundation for the kind-annotation syntax `: Len` that **S6a** adds for
length parameters, and that **P7b** extends to `: * -> *` for higher-kinded type
variables. Detail: [slice6-brief](./P7/slice6-brief.md).

**P7.S6a -- Length parameters in `type:` headers and the `Kind` type.** `[ planned ]` Sequenced
after S6 (which lands the bracket binding site), this subslice makes a user-defined type
carry a length parameter — opening the `Len::Var` door that the N3 comment at
`src/ast.rs:805` has held shut since Phase 5. A length variable `'N` is not a type: it is a
`u32` for array counts, physically present in `ArrayDecl::count`, and its kind is `Len`, a
different sort from `*` or `* -> *`. No variable is polymorphic over kinds (P7b's out-of-scope
list keeps that); each has a fixed kind. This is the minimal kind system: `Star` (the default,
replacing `VarKind::Ty`) and `Len`, with `Arrow` added later by P7b.

**Syntax.** The bracket that S6 introduces for `type:`/`trait:`/`:` binding sites carries a
kind annotation for length variables: `'N: Len`. A type variable needs no annotation (kind
`*` is the default), so `type: Buffer['T 'N: Len]` declares one type variable and one length
variable. At use sites, context disambiguates and no annotation is needed: `array['T 'N]` in
an effect or field is unambiguous because count position is count position. This is the same
split as bounds: `'T: Copy` lives in the bracket, bare `'T` at use sites.

```forth
type: Buffer['T 'N: Len]
  data  ^array['T 'N]
  len   usize
;

: capacity['T 'N: Len] ( &Buffer['T 'N] -- &Buffer['T 'N] usize )
  | b | b &data &^ len ;
```

**What changes.** `VarKind { Ty, Len }` (`src/parser.rs:1470`) becomes a `Kind` enum with `Star`
and `Len` (P7b adds `Arrow`). `GenericStructDecl`/`GenericEnumDecl` (`src/ast.rs:523`, `:537`)
gain `len_var_names: Vec<String>`, parallel to `ty_var_names`. `PolyType::Generic`
(`src/ast.rs:1992`) gains `len_args: Vec<Len>`, parallel to `args: Vec<PolyType>`. The S6
bracket parser calls `intern_len_var` for a `'`-prefixed name with a `: Len` annotation,
`intern_ty_var` otherwise. `substitute_generic_field` (`src/ast.rs:795`) — the N3
`unreachable!()` arm for `Len::Var` — becomes real, looking up the concrete length from the
instantiation's argument list exactly as `PolyType::Var` looks up the concrete type. The
`GenericTypes` instantiation machinery (`instantiate_struct`/`instantiate_enum`,
`src/ast.rs:1108`) accepts length args alongside type args; the dedup keys (`struct_keys`/
`enum_keys`, `src/ast.rs:583`) include lengths so `Buffer[u8 256]` and `Buffer[u8 512]` mint
distinct monomorphs; `type_instantiation_name` (`src/ast.rs:758`) renders length args in the
mangled symbol.

**What already works and needs no change.** `unify_poly_input` already binds `Len::Var` from
concrete array counts at call sites (`src/check/poly.rs:6562`). `match_impl_target_rec`
already matches `Len::Var` patterns (`src/check/poly.rs:5813`). `apply_subst` already
resolves `Len::Var` from `Subst` (`src/check/poly.rs:6842`). `len` on a generic-length array
already folds to `usize` in a poly body (`src/check/poly.rs:1263`). The infrastructure is
~70% built; the remaining 30% is the `type:` header path and the `Kind` type.

**Out of scope.** Generic-length array indexing — `poly_generic_length_index_error`
(`src/check/poly.rs:7646`) still rejects `&>` on an `array['T 'N]` in a non-inline body
because the checker cannot statically prove `i < 'N`. The workaround is `inline` (the body
splices into the caller where `'N` is concrete, which is how every combinator in
`lib/combinators.sth` already works). Relaxing this requires tracking loop-variable provenance
or inserting a runtime bounds check, a different kind of work than this subslice. Non-length
const kinds (boolean, string) — these are phantom parameters with no physical layout
significance, and generalizing `Len` to a broader `Const` crosses the dependent-types line
DESIGN.md draws ("Dependent types: never"). `Len` stays `Len`: a `u32` for array counts,
nothing more.

**Exit:** a user can write `type: Buffer['T 'N: Len] data array['T 'N] ;`, instantiate it as
`Buffer[u8 256]`, and a word declaring `Buffer['T 'N]` in its signature unifies correctly
against a concrete caller. `cargo fmt --check && cargo clippy -- -D warnings && cargo test`
is green. The `Kind` enum has `Star` and `Len` variants; the `: Len` annotation syntax is
live at `type:`, `trait:`, and `:` binding sites.

**P7.S6b -- Explicit length arguments at call sites.** `[ planned ]` The instantiation-side
companion to S6a. Today, `check_poly_call` seeds `subst.ty` from explicit type arguments
(`sum[i64]`, `src/check/poly.rs:4662`) but has no path to seed `subst.len`. A caller wanting
`sum[i64 4]` has no syntax for the `4`.

**What changes.** `parse_type_arguments` (`src/parser.rs:5006`) currently parses only types
inside `[...]`. It extends to accept a mix: positions `0..ty_arity` are type arguments (as
today), positions `ty_arity..ty_arity+len_arity` are length literals (parsed as `u32`, the
same `1..=u32::MAX` range check `parse_array_count` uses at `src/parser.rs:4295`). The callee's
`PolySig` already carries `ty_var_names` and `len_var_names`, so the arity split is known at
the call site. `check_poly_call` (`src/check/poly.rs:4662`) extends its seeding loop: after
seeding `subst.ty` for type variables, it seeds `subst.len` for length variables. The
`seeded` vector (`src/check/poly.rs:4661`) extends to cover length variables — the
conflict-check logic in `unify_poly_input`'s `Len::Var` arm (`src/check/poly.rs:6563`) is
already identical in shape to the `Var` arm's conflict check.

**What already works.** Inferred length binding (no explicit args) already works:
`unify_poly_input` binds `'N` from the concrete array's count when an `array[i64 4]` fills an
`array['T 'N]` parameter. The `Subst` type already carries `len: Vec<(u32, u32)>`
(`src/ast.rs:2033`). The standalone combinator check already substitutes a concrete length
for each `'N` (`src/check/poly.rs:395`).

**Exit:** a caller can write `sum[i64 4]` to explicitly bind both `'T = i64` and `'N = 4`, and
a conflicting operand produces the same "explicit instantiation conflict" diagnostic a
conflicting type argument already produces. `cargo fmt --check && cargo clippy -- -D warnings
&& cargo test` is green.

**P7.S7 -- A testing vocabulary, and what it exposed about printing's layer.** `[ planned ]`
Split into four subslices during briefing: the vocabulary itself needed a `hosted` home
that didn't exist yet (`.` prints through libc unconditionally, a hosted-layer dependency
the compiler currently hides as a builtin), and reporting more than a bare label on
failure wants a `Show` trait, whose naive one-impl-per-type shape collides with the
existing orphan rule the moment a second target (embedded, UART) wants its own sink.
Sequenced so nothing but the last subslice touches the compiler:

- **S7a** -- `lib/core`/`lib/hosted` package split, `lib/hosted/libc.sth` with `exit`.
  Detail: [slice7a-libc-brief](./P7/slice7a-libc-brief.md).
- **S7b** -- `hosted::testing`'s `expect`/`expect-eq` and the `sooth test` driver, label-only
  output, printing via the still-intrinsic `.`. Detail:
  [slice7b-testing-brief](./P7/slice7b-testing-brief.md).
- **S7c** -- `core::show`'s `Write`/`Show` trait pair (sink-generic: one `Show` impl per type,
  living in `core`; per-target `Write` impls carry the platform dependency), with
  `hosted::libc` supplying the `Stdout` sink. Detail:
  [slice7c-show-brief](./P7/slice7c-show-brief.md).
- **S7d** -- retires the compiler-intrinsic `.` in favor of an ordinary `hosted::show` word
  over S7c's traits; every printing program migrates to an explicit `depends: hosted`/
  `import:`, no compatibility shim. Detail:
  [slice7d-dot-hosted-brief](./P7/slice7d-dot-hosted-brief.md).

**P7.S8 -- Nested inline-combinator splice-uid collision.** `[ done ]` A spliced trait
member body lowers under **that member's own** check-time uid namespace: the member's seed
(`word_idx * INLINE_UID_STRIDE`, the same numbering `src/check.rs` uses) is pushed onto
`splice_uid_stack` and `FuncBuilder::inline_uid` is reset to it for the duration, so a
combinator splice nested inside the body mints the uid the checker minted for it. The
span-keyed `trait_calls` lookup stands aside while such a re-splice is active
(`member_splice_depth`), because a member body can reach one source span under a second
grounding and the recorded answer is then the wrong one. With both rules in place
`lib/cmp.sth`'s six surface comparisons (`eq`/`lt`/`gt`/`lte`/`gte`/`ne`) are `inline`, so
a user `impl: Ord` delegating to a primitive comparison builds and a library comparison
costs no call frame. Detail: [slice8-spec](./P7/slice8-spec.md).

`MEMBER_SPLICE_SUFFIX`'s disjointness from `INLINE_SUFFIX` is load-bearing: a member body is
spliced at the member's seed, and so is the first combinator splice nested *inside* that
body, so a shared suffix would be a silent wrong answer rather than a panic. Witnessed by
`ord_inline_cmp_member_local_colliding_with_a_nested_splices_local_reads_its_own`.

CLAUDE.md's five split signals against `src/ir/func_builder/calls.rs`: 0 of 5 fire (one
`use super::*`, nine functions in a single call chain, no import divergence, no mixed
high/low-level code) -- no split.

`check_no_combinator_cycle` (`src/check/combinators.rs`) matches a call's surface name
against `c.word.name` to detect a combinator recursing through itself; an `impl:` member is
named `cmp;Ord;Point`, so a bare `cmp` call inside the same member's body adds no edge and
the cycle guard never fires. A recursive `impl: Ord` whose `cmp` calls a surface comparison
on its own type therefore splices forever -- a compiler stack overflow (SIGABRT), not a
diagnostic. Pre-existing (reachable at base via a user `inline` combinator calling `cmp`);
this slice moves it onto the shipped library comparisons, so it is reachable from any
recursive-type `impl: Ord`. Not fixed here; no owning slice yet.

Two more follow-ups the slice deliberately did not fix:

- **Unsatisfied-`Ord` attribution.** An unsatisfied `Ord` bound now names `cmp`, the
  spliced trait member, at `lib/cmp.sth`'s own line, rather than the `lt`/`gt` the user
  wrote. The second, useful line (`no ( T T -- Ordering ) found`) is unchanged. Restoring
  the caller's attribution needs a splice-origin span carried through unsatisfied-bound
  reporting: a diagnostics feature with its own design surface, not a uid fix.
- **REPL trait/impl checking.** `src/check.rs`'s two REPL check sites hardcode
  `TraitResolveCtx::scratch()`, whose premise (a session declares no `trait:`) is false the
  moment a session imports `core::cmp`; a comparison call then indexes past the scratch
  trait table and ICEs at `src/check/poly.rs:976`. Ten `#[ignore]`d REPL tests state this
  as their reason. The fix needs a `Session`-level traits/impls accumulation table (Session
  has `structs`/`enums` but no trait analogue) threaded through both sites -- comparable in
  size to the earlier struct/enum REPL work, so it is its own slice.

**P7.S9 -- Remove the REPL.** `[ planned ]` The REPL (`src/repl.rs`, 5.3k lines, plus
the hand-rolled line editor in `src/editor.rs`) is a second, parallel execution path:
it `dlopen`s freshly compiled words into the session process line by line instead of
going through `build`/`run`'s ordinary whole-program compile. That second path has
produced a standing pile of REPL-only defects with no counterpart in `build`/`run`
-- a name-keyed `bool` conflation, a module-check bypass (anything reachable only
through `assemble_module` goes unchecked at the REPL), a hub re-export the REPL
can't follow, non-inline poly words it loses entirely, and a materialized quotation
it can't link -- because each is a gap in reproducing the real compiler's semantics
incrementally rather than a bug in the compiler itself. Deleting the REPL deletes the
second path, not just its bugs: there is no remaining reason to keep a semantics-
replicating shortcut once it is gone.

**Scope.** Delete `src/repl.rs`'s `Session` and read-eval-print loop, and
`src/editor.rs` in full (its whole reason to exist is the interactive line editor);
relocate `Library` out of `repl.rs` before the rest of the file goes (see below).
Remove the `repl` subcommand (`src/main.rs`) and `driver::repl` (`src/driver.rs`).
Strike every REPL-conditional branch and comment in the files that reference it
outside `repl.rs` itself --
`ast.rs`, `check.rs`, `ir.rs`, `lexer.rs`, `lib.rs`, `packages.rs`, `parser.rs`,
`resolve.rs`, `test_support.rs`, `backend/qbe.rs`, and the `check/`/`ir/` submodules
that carry REPL-specific workarounds (`check/audits.rs`, `check/builtins.rs`,
`check/captures.rs`, `check/combinators.rs`, `check/declarations.rs`,
`check/drop_graph.rs`, `check/engine.rs`, `check/operators.rs`, `check/poly.rs`,
`check/terms.rs`, `check/word_families.rs`, `ir/destructors.rs`, `ir/driver.rs`,
`ir/layout.rs`, `ir/test_helpers.rs`, `ir/types.rs`). Each of those workarounds is a
candidate for outright deletion, not just an `if repl` branch removal, once nothing
exercises the incremental-dlopen path it exists for -- confirm per file rather than
assuming, since a few may guard a real non-REPL case too.

**Not everything `repl.rs`-adjacent goes.** `dlopen` has exactly one caller today --
the REPL -- but two of its supporting pieces are generic, not REPL-specific, and are
relocated rather than deleted: `driver::compile_so` (`src/driver.rs:947`, `.so`-with-
no-`main` codegen through `qbe`/`cc`, no REPL state) and `repl.rs`'s `Library`
(`dlopen`/`dlsym` wrapper, ~40 lines, no `Session` state either) both move into
`driver.rs` as the load-bearing primitives for a future library-output build target
and for future incremental compilation -- both explicitly on the roadmap, not this
slice's problem to solve, but not this slice's problem to delete out from under
either. What actually goes is `Session` (the persistent stack buffer, word env, and
per-line incremental-compile state) and the read-eval-print loop built on it: that is
where every REPL-only defect named above actually lives, not in the dlopen shell
itself.

**Docs.** `docs/book/the-interactive-book.md` (planned, unwritten) drops from
`SUMMARY.md` entirely. `docs/book/preface.md`, `getting-started.md`, and `words.md`
lose their REPL sections/examples and are rewritten to teach the same material
against `sooth run` instead -- tracked as follow-up doc work once this slice lands,
not part of its exit criteria.

**Exit:** `sooth repl` does not exist as a subcommand; no source file references the
REPL or its incremental-compile machinery; every workaround named above is deleted,
not merely unreached, confirmed by grepping the corpus for its own review-graph
notes; `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green with
no REPL-only test module skipped or stubbed out
