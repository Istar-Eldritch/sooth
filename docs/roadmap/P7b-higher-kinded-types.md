[← ROADMAP](./ROADMAP.md)

### Phase 7b — Higher-kinded types  `[L]`  `[type-class abstraction over type constructors]`

Sooth's traits dispatch on concrete types, monomorphized per instantiation. `Ord` works
because `cmp ( 'T 'T -- Ordering )` is monomorphic within each `impl:` — the trait's type
variable binds to one concrete type. But a `Functor` trait needs to abstract over a *type
constructor* (`Option`, `List`), not a type (`Option[i64]`): its `map` has the signature
`'F['T] ~[ 'T -- 'U ] -- 'F['U]`, where `'F` has kind `* -> *` and the output is a *different
concrete type* from the input. No type variable in Sooth today ranges over anything but a
concrete type (kind `*`), so this signature cannot be expressed.

The motivation is sharing combinator bodies — `map`, `bind`, `filter` — across `Option`,
`Result`, `List`, and future container types without per-type duplication. Each container's
combinators are 3-5 `inline` words today, and the duplication is small (maybe 60 lines across
all containers), but the abstraction is the kind of type-system work the language exists to
explore, and the craft project's principle is that the author's enjoyment is a valid reason
to build something.

**No language at Sooth's scale does this.** No concatenative, statically typed, linear,
systems-targeted language has higher-kinded polymorphism. The languages that do (Haskell)
built the entire type system around it; the closest relative on the linear + systems axis
(Rust) has spent 10+ years on GATs and still cannot express `Functor` cleanly. Sooth is
unusually well placed: whole-program monomorphization means the winning `impl:` is chosen
statically per instantiation with no cross-unit coherence question, and the absence of
lifetimes and associated types removes the two soundness hazards that make specialization
and HKT hard in Rust. That makes the implementation tractable, not the design small.

**Prerequisites:** P7.S4 (polymorphic impl targets — the constructor-keyed dispatch builds on
its machinery for matching a polymorphic target against a concrete type), P7.S3o (bound dispatch on a
poly combinator's own type variable — landed; P7b.S3 extends its per-splice resolution to
constructor applications), and
P7.S6a (length parameters in `type:` headers and the `Kind` type — S6a introduces the
`Kind` enum replacing `VarKind`, the `: Kind` annotation syntax at binding sites, and the
`Len` kind. P7b extends this foundation with `Arrow` for higher kinds; it does not
re-introduce a separate kind mechanism).

**Sequencing:** independent of P8 (packages) and P9 (stdlib layers). May be taken up any time
after P7.S4 lands, by the craft principle of reordering by what you want to play with first.
S6 requires S4 (real lib types taking ctor-keyed impls) and S5 (mono member routing for
colliding member words); S7 is probe-first and needs only S4 — its dogfood composes with S6's
traits; S8 sits on S6's `List`.

## The five pieces

1. **Kinds** — every `Type` and every type variable carries a kind (`*`, `* -> *`,
   `* -> * -> *`, and `Len`). The parser annotates type-variable declarations with kinds
   (inferred from usage context: if `'F` appears in `'F['T]`, its kind is `* -> *`, with
   explicit annotation as fallback), the checker validates kind-correctness at every type
   application site, and the IR's monomorphization resolves constructor applications to
   concrete types. The `Kind` enum and the `: Kind` binding-site annotation are introduced by
   **P7.S6a** with `Star` and `Len` variants; P7b.S1 adds `Arrow` for higher kinds. No new
   annotation syntax — the same `'N: Len` shape extends to `'F: * -> *`.

2. **Type-level application** — a new `Type` variant or resolution step that lets `'F['T]`
   appear in signatures and survive checking. Today `Type` is always concrete after binding
   (`Struct(StructId, &'static str)`, etc.); an abstract application form must survive
   checking and resolve during instantiation when both `'F` and `'T` are concrete. Sooth
   already has type-level application in the form of generic instantiation — `Option[i64]`
   is already an "application" of the Option constructor to i64, resolved eagerly during
   monomorphization. HKT requires the type system to handle the case where the constructor
   itself is a type variable.

3. **Constructor-keyed dispatch** — `resolve.rs` and `check/poly.rs` key trait lookup on
   `(TraitId, concrete Type)` today. For `impl: Functor for Option`, the registry needs to
   key on `(TraitId, ConstructorId)` and re-instantiate the constructor's type variables at
   the call site. P7.S4's polymorphic-impl-target machinery — matching a parameterized target
   against a concrete type — is the foundation this builds on.

4. **Higher-kinded trait declarations** — the trait syntax needs to express that `'F` has
   kind `* -> *` and that `map`'s output is `'F['U]`. This is the parser/checker surface:
   `trait: Functor 'F` with a member `map ( 'F['T] ~[ 'T -- 'U ] -- 'F['U] )`, and an
   `impl: Functor for Option` whose body eliminates and reconstructs with the mapped type.

5. **Inline + HKT bounds** — if `Functor.map` is to splice (and it should, to avoid the frame
   tax that already costs `cmp.sth`'s comparisons ~2x over inline), P7.S3o's bound-carrying
   splice machinery extends to handle constructor applications in the output type. An inline
   word with an HKT bound splices at the call site with zero frame cost, the same way array
   combinators do today.

## Slices

**P7b.S1 — Kinds and type-level application.**
The type-system foundation. Type variables may carry higher kinds; `'F['T]` is a type-level
application that type-checks and monomorphizes. Kind inference from usage context, with
explicit annotation as fallback. Kind-incorrect application is a located error. Builds on
the `Kind` enum and `: Kind` annotation syntax that **P7.S6a** introduces — S6a plants
`Star` and `Len`; P7b.S1 adds `Arrow` for higher kinds (`* -> *`, `* -> * -> *`).
**Exit:** a type variable may have a higher kind; `'F['T]` type-checks and monomorphizes to
a concrete type; kind-incorrect application is a located error; a signature may mention
`'F['T]` where `'F` is a type variable of kind `* -> *`.
Scoped against the tree 260830 (probe round + paper recon): see
[slice1-brief](./P7b/slice1-brief.md) — design rulings R1-R8, witnesses, and the golden
list — and [slice1-probes](./P7b/slice1-probes.md) for the verbatim compile-probe log.
The spec driving implementation is [slice1-spec](./P7b/slice1-spec.md) for the full
design (revised 260830 after a three-reviewer round; constructor representation
`Type::CtorImage(GenericId)`).
Headline: applied-target impl dispatch already works (`impl: Functor for Option[i64]`),
which narrows S2 to the constructor-abstract target; S1's parse work is the prerequisite
for the whole trait surface.

**P7b.S2 — Constructor-keyed dispatch and higher-kinded trait declarations.**
The trait machinery. The impl registry keys on `(TraitId, Constructor)` for HKT traits. A
trait may declare a type variable with a higher kind and use type-level application in member
signatures. An `impl:` target names a constructor, and the impl is instantiated per call site
with the call's concrete type arguments.
Implemented on branch `p7b-s2` (base `5443a0d`): see [slice2-spec](./P7b/slice2-spec.md) —
now the condensed implemented reference (rulings R1–R11, witnesses, goldens, and the
recorded deviations live in the [brief](./P7b/slice2-brief.md) and the spec's Open
questions).
**Exit:** `trait: Functor 'F` with `map ( 'F['T] ~[ 'T -- 'U ] -- 'F['U] )` type-checks;
`impl: Functor for Option` resolves; a call to `map` on `Option[i64]` with `~[ i64 -- bool ]`
dispatches to the Option impl and produces `Option[bool]`; the same call on `Result[i64 Err]`
dispatches to a separate `impl: Functor for Result` and produces `Result[bool Err]`.

**P7b.S3 — Inline + HKT bounds (the zero-cost splice).**
Extends P7.S3o's bound-carrying splice to handle constructor applications in output types. An
inline word with an HKT bound splices at the call site with zero frame cost, matching the IR
a hand-written inline `map` would produce. P7.S3o has landed (bound dispatch reaches
spliced combinator bodies), so this slice extends a working mechanism.
**Exit:** `Functor.map` called through a bound on an inline word splices to the same IR as a
hand-written inline `map` would produce; no call frame; no runtime dispatch.

**P7b.S4 — Declaring-module identity for generic instantiations.**
Carved out of S2's implementation review (260901). An operand of a lib-declared generic
type is instantiated at the *naming* module (`resolve_type_or_apply` →
`instantiate_enum`/`instantiate_struct`, memo key `(idx, module, args, lens)`), while an
impl-target pattern records the *declaring* module — and both dispatch paths compare the
two for equality. Consequence: `impl: Functor for Option` in a user module never matches
an `Option[i64]` operand named in that module, so real `core::option`/`core::result`
cannot take constructor-keyed impls; S2's W3/W4 goldens run over fixture twins for
exactly this reason (recorded in the spec's Open questions). The fix — a module-blind
instantiation identity, or module-blind matching — is an S1-era convention change with
dedup/monomorph-symbol implications, so it gets its own brief rather than a drive-by.
**Exit:** `impl: Functor for Option` in a user module dispatches for an `Option[i64]`
operand named in that module; S2's W3/W4 goldens migrate from fixture twins to the real
lib types unchanged in behavior; no duplicate monomorphs are introduced by the widened
identity. S3's dogfood (real `Option`/`Result`/`List` through a shared bound) depends on
this slice or on twin workarounds.

**P7b.S5 — Member-word routing and env-dispatch residuals.**
Carved out of S2's golden #10 (260901), two pre-existing conventions the S2 machinery
exposed: (a) a *mono* caller of a member word whose synthesized name collides across
modules (same-named ctors; the `$$N` overload suffixes are lowering symbols, not
`poly_env` keys) gets `mono_member_unroutable_error` — mono member routing should resolve
through the same registry the poly path uses; (b) same-named ctor env dispatch is
module-blind name+input-shape *first-match*, so identically-shaped ctors in two modules
cross-pick each other's words (S2's #10 fixture had to disambiguate ctor payloads); and
(c) the nested-receiver member diagnostic hardcodes the example variable `'F` regardless
of the trait header's actual variable spelling (`declarations.rs`
`nested_receiver_member_error`) — fixing it interpolates the header variable and updates
the two pinned goldens.
**Exit:** a mono caller in a module that can see both impls dispatches a colliding
member word per its operand's constructor; same-named ctors with identical payload
shapes resolve by a pinned rule (module scope or qualified spelling), and the #10
workaround (payload disambiguation) becomes unnecessary.

**P7b.S6 — Container traits: Functor/Bifunctor/Foldable over the real lib types, plus linear
merge.**
The tier-1 library slice, unblocked by S4. `core::option`/`core::result` take ctor-keyed
impls; `Bifunctor['F: * -> * -> *]` with
`bimap ( 'F['A 'B] [ 'A -- 'C ] [ 'B -- 'D ] -- 'F['C 'D] )` unifies `map`/`map_err`/`swap`
on Result; `Foldable['F: * -> *]` with `fold ( 'F['T] 'A [ 'A 'T -- 'A ] -- 'A )` is the most
concatenative abstraction in the ladder, and the linear spine is what makes it stronger than
its Haskell cousin: every element moves into the fold quotation exactly once and forgetting
one is a compile error. Carries `Semigroup`/`Monoid` as *linear merge* —
`combine ( 'T 'T -- 'T )` totally consumes both operands (StrBuf concat, list append,
numeric add), and `empty ( -- 'T )` is return-type polymorphism grounded bottom-up from the
consuming stack or pinned by explicit instantiation; `mconcat` falls out as
Foldable+Monoid. Also promotes a linear cons `List['T]` into core: the monomorphic example
exists, but whether `^List['T]` self-reference grounds under a generic declaration, and
whether the fused iterative destructor dispatches per-instantiation payload drops, are S6
probes. The array-as-`'F: * -> Len -> *` probe (a member-scoped `Len` var in `map`'s
signature) runs here too: either it grounds and arrays become Functor/Foldable instances, or
the kind story gets a ruling.
**Exit:** a program `map`s and folds over `Option`, `Result`, and `List` through shared
bounds with impls on the real lib types; `combine`/`empty`/`mconcat` goldens; core gains
`List['T]` with a constant-stack destructor that drops linear payloads per instantiation;
the array-kind and `^List['T]` probes have recorded rulings.

**P7b.S7 — Quotation effects over type constructors (the `call` extension).**
S2's recorded residual, now its own slice. `bind ( 'F['T] [ 'T -- 'F['U] ] -- 'F['U] )`
needs `call` to ground a quotation whose effect contains a constructor application with an
unbound head; `ap`'s first operand `'F[ [ 'A -- 'B ] ]` asks the mirror question (a
constructor application over an *effect type*). Probe-first per the fragile-slice
convention: one probe round answers both shapes before the brief. Scope is the
quotation-effect grounding paths only — no new trait surface, no new declaration syntax.
Dogfood: `Monad` for Option/Result (`bind`, `and_then`, early-exit chains over
errors-as-values); `Applicative.ap` if shape (b) grounds for free in the same extension.
**Exit:** `bind` through a shared bound type-checks, dispatches per constructor, and
splices to the same IR a hand-written inline `and_then` would produce; `and_then` goldens
for Option and Result; the probe round's rulings on which effect shapes ground are
recorded in the brief.

**P7b.S8 — Linear iterators (HKT as the associated-type substitute).**
Associated types stay out of scope and are not needed: make the iterator itself the type
constructor — `trait: Iterator['It: * -> *] : next ( 'It['T] -- Option['T] 'It['T] ) ;`.
The "associated type" is just the constructor's own parameter, and linearity *is* the
protocol: `next` consumes the iterator and yields element-plus-remainder, so no borrows,
no lifetimes, no `&!` exclusivity puzzles. The exhausted case's linear shape — `None`
with the iterator dropped inside `next`, versus a
`type: Step['T 'Rest] | Done | More 'T 'Rest ;` instantiated as `Step['T 'It['T]]` — is
an S8 probe (the `Step` shape exercises nested constructor applications as type
arguments). One real impl each for List and a count-up `Range`; `map`/`fold` over an
Iterator through bounds; whether chained splices actually fuse in the IR is recorded as
a question, not an answer.
**Exit:** `next`/`map`/`fold` through an Iterator bound over List and Range goldens; a
consuming loop runs with one frame total; the exhausted-case ruling and the fusion
evidence from a small chain are written down.

**Dogfood:** S6 — a program that `map`s and folds over `Option`, `Result`, and `List` through
shared bounds, with the impls declared against the real lib types and output matching
hand-written inline equivalents. S7 — `and_then` chains over `Option`/`Result` through a shared
`Monad` bound. S8 — a consuming `map`/`fold` loop over an `Iterator` with no frames between
elements.

## Out of scope

GATs (generic associated types), associated types, dependent types, polymorphic kind recursion,
kind polymorphism. S2's brief answered the `Monad.bind` open question by *recording* it:
`call` cannot see through an HKT member (`slice2-spec.md`, "Monad.bind awaits a later slice"),
so `Monad` is **not** free library work — S7 is its checker extension. `Applicative.ap` asks
the same quotation-effect machinery the mirror question (a constructor application over an
*effect type*, `'F[ [ 'A -- 'B ] ]`); `pure` alone is library work once S6 settles
grounding for return-type-polymorphic words. A general `Default` trait for construction stays
out (S6's `Monoid.empty` is scoped to merge identities, not defaults). Drop-forwarding for
user-declared generic containers (a `Drop` trait) rides with P9's alloc layer where the need
first becomes real; whether drop_graph already covers payload drops for minted generic
instantiations is an S6 probe note.
