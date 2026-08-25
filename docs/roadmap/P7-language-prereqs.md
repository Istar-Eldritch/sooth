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
type's own defining module. `Copy`/`Ord` are pre-seeded predicate-kind trait-table
entries (satisfaction still runs `is_copy`/`is_ord`), so a colliding user `trait: Copy`
fails as an ordinary duplicate declaration. A bare or ref-to-bare bounded variable's
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
body may hand a capture back rather than dispose it. There is no drop pointer, no third layout
slot and no `emit_drop` arm; a plain quotation keeps its two-word `(code, env)` layout and
gains no allocation.
**The body is the sole disposer, which is what the containment rule buys.** An owning
quotation is rejected in every *declared* aggregate position (struct field, variant field,
array and slice element, owned-cell payload, referent, `extern:` slot), so no synthesized glue
can reach one and `field_is_linear`/`layout_field_is_linear` stay untouched. One honest
exception: a multi-output word's synthesized return-bundle struct is interned after these
audits run, so an `owning` output does reach that struct as a field; it stays sound because the
bundle is a destructor-free transient carrier, unpacked at the call site the instant the word
returns, never itself disposed as a container. `drop` on one is a located
rejection in both a monomorphic and a generic body, since releasing the capture means running
code only the closure has. Neither a spliced (`inline`) nor a generic word may declare an
`owning` parameter: the splice route never materializes, and a polymorphic call site
materializes from the declared effect alone, which does not carry the flavour -- so in both
the distinction would be silently unenforced.
**Capture admission.** `classify_capture`'s aggregate arm admits a capture as scalar only when
it is both `Copy` and scalar-represented, so a payload-free enum passes and a pointer-backed
aggregate keeps being rejected however `Copy` it is. At an `owning` boundary the frame-rooted
rejection and the 2+-capture deferral both lift for a *linear* capture, the heap block having
replaced the stack bundle they guarded; the in-frame path is unchanged.
**Two restrictions remain, both waiting on an erased owner:** an owning closure may not be
discarded unexecuted, nor stored in an aggregate. Both are **P7.S3v**, after **P7.S3u**.
Standing hazard, unchanged: a materialized quotation still cannot be linked in the REPL (a
session line building a `(code, env)` value dies on a non-PIC `__quot0` relocation).
**Exit:** a linear value can be moved into an escaping closure's env, the closure returned
from the word that built it, and calling it observes the capture and disposes it exactly once
-- one observation, not zero and not two -- with a forgotten closure, a `drop`ped one and a
capture the body never consumes each a located error rather than a silent miscompile.

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
`[ parked ]` Named at P7.S3e's round-1 review (its spec's own R9/R17), out of scope there.
`reject_user_bound_on_combinator` (`src/check/poly.rs:5919`) is a clean, located rejection, not
a bug. Recon'd and spec'd twice; both designs (a splice-uid resolution key, then a
source-derived `SplicePath` key) were found unsound in review -- see
[slice3o-brief.md](./P7/slice3o-brief.md) for the full recon, the two failure modes, and what
would need to be true before this is worth trying a third time. No spec currently exists for
this slice; revisit only if a concrete program needs it.

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
`&'T` *directly* in some input. It is syntactic: a receiver mentioned only nested inside a
composite input (`sum ( ['T 4] -- i64 )`) is rejected too, since grounding it would need
structural unification through the array type. A nullary member (`fresh ( -- i64 )`) is the
zero-receiver case deferred as **P7.S3t** below.

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

**P7.S3t -- A zero-receiver trait member has no call-site signal to dispatch on.** Named at
P7.S3p's spec, out of scope there. `fresh ( -- i64 )` (a nullary constructor) binds its
trait's type variable in no declared input, so nothing at the call site grounds *which*
concrete type's member is meant -- the language has no type-argument syntax and no context
binding to supply one. P7.S3p's declaration gate (`member_binds_trait_var`) keeps rejecting
any such member rather than shipping a dispatch path with no grounding signal.
**Exit:** a trait member binding its variable in no input may be declared and dispatched,
with some new call-site or context mechanism supplying the concrete type -- mechanism not
yet designed.

**P7.S3s -- `Ord` as a library trait, not a compiler-hardcoded bound.** `Bound::Ord`
(`src/ast.rs:1417-1421`) is a reserved, member-less trait-table entry (`seed_predicate_traits`,
`ast.rs:1528-1546`) satisfied by `is_ord` (`src/check/poly.rs:120-122`), a hardcoded
`ty.is_numeric()` check consulted at four discharge sites -- never the whole-program
`(TraitId, Type)` impl registry `Bound::User` (S3e) already dispatches through. `'T: Ord`
therefore categorically excludes a struct or enum, by construction, regardless of any `impl:`
a user writes: `examples/traits.sth` worked around this by inventing a separate `Order`
trait rather than using the language's own `Ord`. Not yet recon'd: whether the fix collapses
`Bound::Ord` into `Bound::User(TraitId)` (seeding a real, member-bearing `trait: Ord` and
replacing the numeric fast path with per-width `impl:` blocks or an implicit numeric
short-circuit ahead of the registry lookup) or adds a second, separately-named nominal trait
alongside the existing numeric-only `Ord` -- a real design choice with a real blast radius
(four call sites, every diagnostic naming `Ord` by variant), not a mechanical migration.
Depends on neither S3k (closed) nor S3p (a binary `cmp`-shaped member's receiver is already
trailing); overlaps P8.S2's planned `lib/cmp.sth` migration, which should sequence after this
slice if the satisfaction mechanism changes, or be written explicitly against whichever `Ord`
shape this slice produces.
**Exit:** `Ord` bounds a struct or enum, satisfied nominally by an `impl:` block, so a
comparison-bounded generic word (`sort`, `bin_search`) can be instantiated over a user type,
not only the numeric tower -- with no per-width boilerplate `impl:` required for the numeric
tower itself, and every existing `'T: Copy Ord` numeric program unaffected.

**P7.S3u -- Trait objects (an erased owner with a reachable destructor).** Traits dispatch
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
engineered from a closure special case. Scope guard: heterogeneous collections (`Vec[dyn T]`)
are a weak motivation on their own, since a closed enum is usually the better design and
already works in `core`; the forcing consumer is disposal through erasure, which is why
**P7.S3v** is sequenced immediately after.
**Exit:** a value of a user trait can be held and used behind an erased owner whose concrete
type the use site does not name, a bounded word can be called through it, and an owning trait
object's destructor runs exactly once through the object with no leak and no double free, with
the `Copy`-versus-linear rule for objects stated and tested.

**P7.S3v -- Dropping and storing a linear-capturing quotation.** **P7.S3h** ships owning
closures under two restrictions that exist only because nothing can invoke a per-value
disposer: an owning closure may not be discarded unexecuted (`drop` on one is a located
rejection, because releasing the capture requires running the body), and it may not be a
struct field, array element, slice element or owned-cell payload (a container's synthesized
glue would have to dispose it, and `emit_drop` (`src/ir/func_builder/quotation.rs:305`) has no
arm it could take: its `_ => {}` fall-through would silently swallow the field and leak both
the capture and the env block). Both restrictions dissolve the moment an erased owner can
carry a destructor, which is what **P7.S3u** supplies. This slice is the consumer that proves
S3u's disposal answer on a real case: give the owning closure value a disposer reachable
without running its body, then lift the `drop` rejection and the aggregate-position gate, so an
owning closure becomes an ordinary linear value that can be stored, forwarded, and discarded.
Depends on both **P7.S3h** (the marker, the containment rule, the heap env) and **P7.S3u** (the
mechanism). Sizing note: the checker work is mostly *deletion* of S3h's gates plus the
`field_is_linear`/`layout_field_is_linear` widening (`src/ir/layout.rs:66`, `:889`) that S3h
deliberately leaves untouched; the new construction is the disposer itself and the `emit_drop`
arm, both of which belong to S3u's mechanism rather than to this slice.
**Exit:** an owning closure can be `drop`ped without being called, disposing its captures and
env exactly once; it can be stored in a struct field and an array element and disposed
transitively through the container exactly once; and every S3h golden that asserted a
rejection has been migrated to assert the new behaviour rather than deleted.
