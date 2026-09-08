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

**P7b.S5 — Member-word routing and env-dispatch residuals.** Shipped for its scoped
mechanism, see [slice5-spec](./P7b/slice5-spec.md) for the full rulings; the residual
trait-impl-dispatch collision below is carved out to **P7b.S9**.
The nested-receiver member diagnostic interpolates the trait header's actual variable
(`declarations.rs` `nested_receiver_member_error`), never a hardcoded `'F`. Same-shaped
cross-module generic **ctor construction** dispatches correctly per a 3-tier
caller-module/visibility policy in `select_overload` (`builtins.rs`): a caller's own
module wins first, then a single visible candidate, then an accepted ambiguity error for
2+ visible candidates from neither the caller's module. This governs which module's
`Widget[i64]` *struct symbol* a construction picks — it does not reach trait-impl
dispatch (see P7b.S9). Golden #10
(`same_named_ctors_in_two_modules_dispatch_distinct_impls`,
`tests/phase7b_slice2.rs:644`) **keeps** its `i64`/`str` payload split as a regression
witness — its inputs never collide, so the tier policy never fires for it, and the
split was never removed. `mono_member_unroutable_error`'s non-inline call site
(`poly.rs`, `resolve_mono_member_call`'s generic-impl branch) is a dead guard: every
word `poly_env` is whole-program, built once over the fully assembled module, so a
dispatched impl's member word is always present in it; a `debug_assert!` replaces the
error return, and cross-module collisions are caught upstream instead by
`mono_member_no_dispatch_error`. The guard's remaining call site (the inline
re-entry path, reached when a `declares_inline` trait member recurses through another
while already on the active splice stack) is recorded inconclusive: four distinct
fixture shapes were each intercepted by a different, earlier guard, and no known
fixture reaches it. Cross-module member-word *routing* for a colliding mono caller
(dispatching per the operand's constructor across modules) remains out of scope,
blocked behind the standing cross-module generic-instantiation limit (P7b.S4).

**P7b.S6 — Container traits: Functor/Bifunctor/Foldable over the real lib types, plus linear
merge.**
A compiler slice with a library payload riding on top, not a library slice: the exit
criterion's shared-bound dispatch is a polymorphic body, and three compiler fixes gate it
— a diagnostic-rendering panic and a cross-representation unifier gap on a quotation-taking
member called from a poly body, and a twinned `unreachable!` on `List['T]`'s own
self-reference. `core::option`/`core::result` take ctor-keyed impls; `Bifunctor['F: * -> * -> *]` with `bimap ( 'F['A 'B] [ 'A -- 'C ] [ 'B -- 'D ] -- 'F['C 'D] )` unifies
`map`/`map_err`/`swap` on Result; `Foldable['F:* -> *]` with
`fold ( 'F['T] 'A [ 'A 'T -- 'A ] -- 'A )` is the most concatenative abstraction in the
ladder, and the linear spine is what makes it stronger than its Haskell cousin: every
element moves into the fold quotation exactly once, and forgetting one (never consuming it,
never dropping it) is caught by the same general arm-parity/arity machinery every
quotation-eliminator body already has — explicitly dropping a payload is legal, not a
linearity violation. `Monoid['T]` carries *linear merge* — `combine ( 'T 'T -- 'T )` totally
consumes both operands, `empty ( -- 'T )` grounds only from an explicit instantiation
(`empty[i64]`; bare `empty` is a located error, no consuming-context inference), and
`mconcat` takes the merge quotation as a bound parameter
(`( 'F['T] [ 'T 'T -- 'T ] -- 'T ) empty swap fold`) rather than materializing a written
literal at a poly call site. `List['T]` is promoted into `core`, takes a non-inline trait
impl, and its fused iterative destructor drops linear payloads per instantiation. `Monoid`
for `i64` and `mconcat` over `Option`/`List` are measured and grounded; `Monoid for
List['T]` (a real append) and `Functor for List` both hit a **recorded, unfixed wall**
(closed by P7b.S8b, below — both dropped goldens land there):
any trait-member body over `List['T]` that *reconstructs* a `Cons` (as opposed to only
destructuring one, which `Foldable.fold` does and works) panics in
`poly_bind_construction_arg` on a bare `PolyType::Generic` field the existing `OwnedCell`
arm does not cover — the wall is unrelated to recursion; a single non-recursive
construction inside a trait member reproduces it identically. Arrays do not become
Functor/Foldable instances in this slice: `impl: ... for array['T 'N]` has no constructor
representation to dissolve the application into (`array` is a built-in `Type::Array`, never
wrapped in `Type::CtorImage`), and the impl-target parser discards per-variable kinds
regardless — a widening carved out to its own follow-on slice, **P7b.S6d** (the entry
below; the originally-pointed name S6b was consumed by P7's explicit-length-arguments
slice, and S6c by generic-length array indexing). The carve-out is
measured, not assumed: `GenericId`'s `(is_enum, idx, module)` triple is a binary switch
into two header-indexed registries (`GenericStructDecl`/`GenericEnumDecl`, walked by
`instantiate_struct`/`instantiate_enum`), while `array`'s own registry (`ArrayDecl`) is
content-addressed by `(element, count)` with no header to index — a third `GenericId`
case would need a new registry and instantiation pair bridging the two shapes, not a
widened match arm. See [slice6-spec](./P7b/slice6-spec.md)'s Phase 6 section for the
full inventory and occurrence counts this ruling rests on.
**Exit:** a program `map`s and folds over `Option`, `Result`, and `List` through shared
bounds with impls on the real lib types (`List` folds but does not map, per the
construction wall above); `combine`/`empty`/`mconcat` goldens for `i64` and
`Option`/`List`; core gains `List['T]` with a constant-stack destructor that drops linear
payloads per instantiation; the array-as-constructor widening and the `List`
construction wall are recorded rulings, not landed capabilities (the wall itself is
fixed by P7b.S8b, below).

**P7b.S7 — Quotation effects over type constructors (the `call` extension).**
`Monad.bind ( 'F['T] [ 'T -- 'F['U] ] -- 'F['U] )` declares and dispatches over
Option/Result through two already-shipped mechanisms, not new grounding code:
`ground_member_poly`'s existing `App` arm dissolves the row-nested `'F['U]` into a
plain `Generic` at parse time (callee side), and `unify_member_operand`/
`render_member_decl` — already exercised by `Functor.map` — ground the caller side.
A single `bind` call over Option/Result splices to the same IR a hand-written
inline call would produce, no call frame. `Applicative.ap
( 'F[ [ 'A -- 'B ] ] 'F['A] -- 'F['B] )` is deferred: its quotation sits inside `'F`'s
own argument list, a different shape from `bind`'s direct member parameter, and is
rejected by `audit_poly_input_quotation`'s `Generic` arm
(`src/check/audits.rs:431`), which calls into `reject_poly_quotation_anywhere`'s
own `Quotation` arm (`src/check/audits.rs:484`) the moment an `impl:` is declared,
independently of the parser's argument-quotation fence. Grounding it needs new logic
in that audit; gated on whichever slice takes up
`reject_poly_quotation_anywhere`'s constructor-of-quotation case.
**Exit:** `bind` through a shared `Monad` bound type-checks, dispatches per
constructor, and splices to the same IR a hand-written inline call would produce;
goldens for Option and Result dispatch and short-circuit correctly. See
[slice7-spec](./P7b/slice7-spec.md) with its [brief](./P7b/slice7-brief.md) for the
full mechanism verification and the `ap` deferral's citation trail.

**P7b.S8 — Linear iterators (HKT as the associated-type substitute).**
Associated types stay out of scope and are not needed: make the iterator itself the type
constructor — `trait: Iterator['It: * -> *] : next ( 'It['T] -- Step['T 'It['T]] ) ;`,
where `type: Step['T 'Rest] | Done | More 'T 'Rest ;` is the protocol row. The
"associated type" is just the constructor's own parameter, and linearity *is* the
protocol: `next` consumes the iterator and yields element-plus-remainder, so no borrows,
no lifetimes, no `&!` exclusivity puzzles. The Option-shaped alternative
(`next ( 'It['T] -- Option['T] 'It['T] )`) was considered and rejected (probe round P8,
260905): arm parity would force the dead iterator out to every caller and the final
drop to every `None` path, instead of the Step shape's single canonical site — the
final drop lives inside `next`'s own `Done` arm, and `Done` carries nothing. One real
impl each for List (generic target) and a count-up `Range[i64]` (the S2-6
concrete-impl-target lift, Delta B); `for_each`/`fold` over the Iterator bound, written
once against neither impl; whether chained splices actually fuse in the IR is recorded
as evidence, and ruled deferred 260907 (the fusion ruling below).
**Exit:** `for_each`/`fold` through the Iterator bound over List and Range goldens (no
per-impl copy of either consumer); `next` over `Range[i64]` dispatches at a plain mono
call site; the exhausted-case ruling and the fusion evidence below are written down.
See [slice8-spec](./P7b/slice8-spec.md) with its [brief](./P7b/slice8-brief.md) and
[probes](./P7b/slice8-probes.md) for the full mechanism verification.
Rulings of 260905 (probe round P8, [slice8-probes](./P7b/slice8-probes.md)): the
protocol row is the **Step shape** (`Step['T 'It['T]]`, `Done` carries nothing, the
final drop inside `next`'s `Done` arm; the Option row above is the recorded rejected
alternative); the **S2-6 concrete-target lift lands in S8** (`impl: Iterator for
Range[i64]`, members grounded as instantiations, not a poly word); the exit's consumers
are **`for_each`/`fold`** — `map` is not in S8 (a bound-generic body cannot produce
`'It['U]` and cannot `dup` the abstract iterator). **P7b.S8b** (carved out, same
date): the S6 construction-wall fix (`poly_bind_construction_arg`'s bare-`Generic`
`^Self['T]` self-reference-field arm, refined by the P8 round) plus per-impl
traitful `List` members (`map`, `append`) over the `Functor`/`Monoid` protocols S6
left as a recorded wall (see below). A second
follow-up, **P7b.S8c** (260906): the per-site binding fix for member signatures with
free input type variables — the `src/ir/driver.rs:579` unification-expect ICE
surfaced by the integrated review (pre-existing; reproducible at the S8 base).
Fusion evidence (REQ-11, recorded facts only — the automated pin asserts no verdict;
the measured ruling follows): a consuming loop's
monomorphized `for_each` over `Range[i64]` lowers to **one emitted function** whose
self-call in the `More` arm is a backward `jmp` to its own loop header (the P7.S3g
self-tail transform, `src/ir/driver.rs:1093`, unchanged by this slice); `next` over
`Range[i64]` remains its own, separate, real monomorphized function, called (not
spliced) from inside that loop — and per element the loop body also calls the
consumer's own quotation, its own frame; the one-frame claim excludes the caller's
quotation. "One frame" is true of the loop, not of loop-plus-`next`
together; a two-consumer chain (e.g. two dispatches through the bound in sequence) is
two dedicated frames, not one fused one. **Ruled 260907 (P8-F fusion probe, measured on
the amd64_sysv QBE emission, 10M-iteration min-of-3): fusion is deferred.** The direct
`next` call fusion would remove costs ~1.3ns/element — a zero-call back-edge loop runs
0.8ns/element, +one direct call 2.1, the protocol `for_each` drain 8.2 vs a hand-rolled
drain 1.2 — i.e. ~16% of the protocol's per-element cost. The dominant ~62% is the
Step-value protocol itself (pack/destructure/drop per element), which call-level fusion
does not touch. On embedded the trade runs backwards regardless: splicing duplicates
`next`'s ~150-byte body into every consuming loop (flash size) for a small cycle win,
and the direct call is the WCET-friendly element (static target) — unlike the indirect
quotation call, which fusion cannot remove. The high-leverage future lever is
**streaming fusion** (eliding Step values in consuming loops — recovers most of the
7ns abstraction tax), a design-bearing slice to reopen only on measured need. For
calibration: the same fold over the same half-open range in Rust (100M elements, rustc
-O, min-of-3) measures ~0.6ns/element in BOTH the iterator form and the hand-written
loop — LLVM collapses the former into the latter, so Rust's protocol tax is ~1× and
Sooth's is ~13× today. That gap is the streaming-fusion-plus-inlining lever, not the
call lever. Hot-loop
escape hatch, documented: hand-roll the loop (the P8-F f2c shape, 6.8× faster than the
protocol drain, within ~2× of Rust's hand-written loop).
(`tests/phase7b_slice8.rs`'s
`consuming_loop_over_range_is_one_frame_with_a_back_edge_and_next_is_a_real_frame` is
the automated pin, captured via `driver::emit_ssa_with_manifest` rather than any
`src/ir/` change, which stays diff-empty.)
Known follow-up (not a change made in S8): `core::iterator`'s `fold` and
`core::combinators`' array `fold` share the bare name `fold`; a consumer that
wildcard-imports both gets a fails-closed duplicate-binding error at the import site,
not a silent shadow. Left as a naming collision for whichever future slice wants a
disambiguation convention (an alias import, or a rename), not addressed here.
Growth-structure re-check (CLAUDE.md, at this phase's exit) over every file this slice
touched — `src/parser.rs`, `src/ast.rs`, `src/check/poly.rs`, `lib/core/iterator.sth`,
`lib/core/range.sth`, and `tests/phase7b_slice8.rs`: `lib/core/iterator.sth` (78 lines) and `lib/core/range.sth` (38 lines) are each
one cohesive thing — the protocol vehicle plus its List impl and consumers, and the
mono `Range[i64]` impl beside its own type, respectively (`range.sth`'s separation from
the original single-protocol-module plan is itself the split this convention asks for,
recorded in the spec's Codebase Map) — neither shows the X+Y+Z or import-divergence
signal. `src/check/poly.rs`'s and `src/parser.rs`'s S7+S8-accumulated edits (the
member-row gate lift, the impl-target/member-body continuations) sit beside their
existing neighbors as more pure-predicate/parse-continuation functions of the same
kind already there (recursive shape predicates beside `member_shape_is_supported`;
impl-target grounding beside `parse_impl_target`/`parse_impl_member_body`) — no forced
circularity, no functions added that never call their neighbors. Both files are large
overall (parser.rs and poly.rs are each one compiler-stage module, per CLAUDE.md's
"group by responsibility" convention), which is a standing size fact about the parse
and check stages generally, not a signal this slice's own edits introduced — no split
is warranted from this phase's diff alone.

**P7b.S8b — The construction wall, a pre-existing two-defect fix, and the traitful
`List` surface.** Closes S6's recorded construction wall (`poly_bind_construction_arg`
panicking on a bare `PolyType::Generic` self-reference field, e.g. `^List['T]`) with
one new arm (`src/check/poly.rs:6256`, inserted before the catch-all): bind the field
positionally against a same-identity `Generic` operand, recursing over field args vs
operand args; a differently-headed or non-`Generic` operand is a located
`poly_rendered_type_mismatch_error`, never a panic. A `Generic` field carrying a length
**variable** (a self-referential length-carrying header is spellable, e.g.
`Ring['T 'N: Len] head 'T next ^Ring['T 'N]`) is a separate, dedicated located error
(`poly_generic_field_len_unbound_error`, `src/check/poly.rs:6110`) naming the header
and its unbindable length variable — a sharper message than a rendered mismatch, whose
expected side can only show a placeholder for it: a field's length-variable id lives in
its own header's space, which the caller's tables cannot name. A **concrete** length is
not fenced (the round-1 review found the original any-non-empty fence rejected
`Ring['T 3]` while naming a variable it does not have): it grounds when both sides
agree, and is an ordinary rendered mismatch when they do not. Routing that case to the
renderer exposed a **pre-existing panic class** the old fence had been hiding
(round-3 review, P0): `poly_type_str` indexed the caller's variable tables raw, so any
id from another declaration space was an index-out-of-bounds ICE *on the error path* —
reachable before this slice via a length-variable-carrying operand against a
differently-headed field, and newly reachable for `Holder['T] r Ring['T 3]`, a clean
located error one commit earlier. The renderer is now total: an unnameable id prints
`'?0` / `'?len0` / `'?row0`, in-range spellings byte-identical, so a diagnostic can no
longer be the thing that crashes the build. Lifting the wall exposed a
pre-existing, S6-era two-defect bug the probe round root-caused independently of the
arm (both reproduce at base `c406149` with no self-reference field at all, `type:
Opt['T]` / `impl: Monoid for Opt`): (a) a nullary trait member called with an explicit
type argument over a generic, single-type-variable impl target (`empty[List[i64]]`
over `impl: Monoid for List`) seeded its θ positionally instead of through the
impl-target equation, minting `List[List[i64]]` instead of `List[i64]` — fixed by
seeding through `match_impl_target` on a channel separate from call-site `type_args`
(`src/check/poly.rs:2371-2391`, the nullary rescue branch) — the round-1 review then
closed that channel's own silent drop: for a **multi-variable** target the arity gate
catches only the short spelling (`empty[Pair[i64 str]]`), while the full-length
`empty[Pair[i64 i64] str]` passed it, reached a seed that grounds both variables from
the dispatch type alone, and minted the same monomorph as `empty[Pair[i64 i64] i64]`
with its second argument read by nothing; past position 0 (the dispatch type on this
path) a written argument must now agree with what the target determined, or is a
located conflict; (b) the wrong mint's
variant words then clobbered the lowering-side bare-name last-write-wins variant map
(`src/ir/layout.rs:597-640`), so *every* `Cons`/`Nil` construction program-wide lowered
with wrong field shapes — a silent 40-byte layout corruption, SIGSEGV on any program
mixing a prior construction with an `empty[<inst>]` call — fixed by recording each
checker-resolved enum-construction/destructure site's own resolved mangled symbol
span-keyed into `builtin_overloads` (`is_generated_enum_word`, `src/check/terms.rs`),
so a site lowers with its own instantiation's shape even when a later mint of the same
header overwrites the bare-name map entry — with R5's carve-outs intact: non-generic
enums' sites are deliberately not recorded (their symbol already is the bare name the
bare-key path resolves) and mono-body eliminator routes are out of scope; shipping the
arm without this pair would
have traded a compile-time panic for a silent miscompile. The Opt-shaped repro (no
self-reference field, prior `Some` construction, then `empty[Opt[i64]]`) is pinned as
a golden independent of the List wall (`nullary_member_over_a_generic_impl_target_runs_after_a_prior_construction`,
`tests/phase7b_slice8b.rs`); the concrete-target nullary path (S6's `empty[i64]`
golden) and the S6 `mconcat_over_list_dispatches` golden stay byte-unchanged. Host
trait ruling (PB-5, user-authorized 260907): `append` grounds through `Monoid for
List` (`combine` = append, `empty` = `Nil`), and S8b closes S6's *whole* recorded
wall — both dropped goldens (`Monoid for List`, `Functor for List`) land here as
goldens, per S6's convention (trait declarations, impls, and consumers live in the
golden programs; only the already-shipped `lib/core/list.sth` and the unchanged
`lib/core/sooth.pkg` module list ship — no new lib modules). The S6 wall witness
flips from a panic pin to a positive golden
(`monoid_for_list_append_construction_builds_and_runs_clean`,
`tests/phase7b_slice6.rs:407`, renamed from `..._wall_is_recorded`). `map` through a
shared `Functor` bound grounds a real `List[i64]` end-to-end, including a
shared-bound-dispatched-twice variant; `combine` through a `Monoid` bound (one poly
middleman at a single impl) prints `1 2 3 5 3`; `empty[List[i64]]` grounds
explicitly in a mono main. Residual (R9, recorded not measured): bound-directed
`empty` resolving at the `List` impl itself remains unverified — the S6 golden
exercises `Monoid for i64`'s `empty`, and no Phase 3 golden pins the `List` route.
Linearity teeth hold on the new constructions: an
undropped `map`/`append` result is a located compile error, and `dup` of a
`List['T]` operand stays fenced byte-exact
(`poly_copy_generic_error`). Recorded fence (R12, not fixed): a map producing a
distinct `'U` through a shared bound remains unspellable (inference does not bind
output-only vars; quotation types rejected against plain-var slots; poly→poly
quotation passing fenced) — verified substitutes are `'U := 'T` specialization
(the shipped `map` consumer), composition, and a mono middleman. Two further
pre-existing fences recorded this round, neither fixed: a bare nullary variant ctor
of a generic header in a mono body grounds at the single first candidate regardless
of the expected output — a second-instantiation nullary construction cannot be
spelled, caught only as a located signature mismatch (adjacent to S6 R4's future
consuming-context-grounding slice); and the struct-word twin of the bare-key
last-write-wins class, plus cross-module same-named variant names in the flat
`enums.words` map, has no known miscompile repro but is left for a future slice. One
caveat is new in this slice, not pre-existing: a mono `inline` (combinator) word's
body is checked as an ordinary mono word, which can record a construction span into
`builtin_overloads` even when the word is never spliced anywhere in the program,
benign at one θ per mono body, pinned by
`mono_inline_combinator_variant_construction_builds_and_runs`.
**Exit:** the Opt repro builds and runs exit 0; both P8 shapes (impl-member-body and
plain-generic-word `Cons` construction) build and run; the ctor-mismatch and
`len_args` errors are located, byte-exact, golden-pinned; the S6 wall witness passes
as a positive golden; `map`/`combine`/`empty[List[i64]]` goldens ground per above;
undropped-result and `dup` errors byte-exact; `mconcat_over_list_dispatches` and
every other S6/S8 pre-existing golden byte-unchanged; `git diff c406149..HEAD --
src/ir/` empty modulo one comment-only correction to a doc comment at
`src/ir/func_builder/mod.rs:195-199` (Phase 1's `builtin_overloads` record falsified
its old "empty on every corpus/test path" claim); `lib/` and `lib/core/sooth.pkg`
unchanged; full gate green. See [slice8b-spec](./P7b/slice8b-spec.md) with its
[probes](./P7b/slice8b-probes.md) for the full mechanism verification, the segfault
root-cause bisection, and the spellings log.
Growth-structure re-check (CLAUDE.md, at this phase's exit) over every file this slice
touched — `src/check/poly.rs`, `src/check/terms.rs`, `tests/phase7b_slice6.rs`,
`tests/phase7b_slice8b.rs`: `src/check/poly.rs`'s new arm (with its one new dedicated
error constructor, `poly_generic_field_len_unbound_error`, beside the pre-existing
`poly_rendered_type_mismatch_error` it twin-styles) sits beside its existing neighbors
as more binding-arm/error-rendering
functions of the same kind already there (`poly_bind_construction_arg`'s own
`Var`/`Concrete`/`App`/`OwnedCell` arms), and the θ-seeding change extends the existing
nullary rescue branch
in place rather than adding a parallel path; `src/check/terms.rs`'s new
`is_generated_enum_word` sits beside its existing twin `splice_enum_site` (the
doc comment says as much) and the single-candidate arm's new `else if` mirrors the
multi-candidate arm's existing `builtin_overloads` insert in the same function — no
import divergence, no function added that never calls its neighbors;
`tests/phase7b_slice6.rs`'s diff is three doc-comment corrections plus one test rename/
body-swap in place, no new functions. `tests/phase7b_slice8b.rs` is a new file, but
it is the established one-file-per-slice pattern every prior S6/S7/S8/S10 golden
suite already follows, not a new module needing a split. `poly.rs` and `terms.rs`
remain large overall (each one compiler-stage module, per CLAUDE.md's "group by
responsibility" convention), which is a standing size fact about the check stage
generally, not a signal this slice's own edits introduced — no split is warranted
from this slice's diffs.

**P7b.S9 — Module-aware trait-impl matching.**
Carved out of S5's review (260904): `find_bound_impl` (`poly.rs:8235`) matches a
concrete `Type` against every `impl:` target pattern for a trait, whole-program.
`match_impl_target`/`match_impl_target_rec`'s `Generic` arm compares header identity
`(idx, module)` and resolves per-module correctly, so the matcher and pattern
resolution are sound. Same-shaped cross-module trait-impl dispatch resolves upstream
of the matcher through two mechanisms: a bare, un-annotated ctor call grounds at the
caller's own resolved header — never another module's eagerly-minted instantiation —
and `instantiation_symbol` renders the `StructId`/`EnumId` a grounding already
carries, so distinct groundings of a shared bound word mint distinct symbols and
lowering's dedup keeps both. `pb2`, S5's own motivating fixture (two modules each
declaring their own same-shaped generic struct `type: Widget['T] v 'T ;` with their
own `impl:` for the same trait), pins both mechanisms: it prints `1` then `2`, each
caller's own impl.
**Exit:** `pb2` prints `1` then `2` (each caller's own impl). The constructible
ambiguity shape is covered by the declaration-time duplicate check, not a new
dispatch-time mechanism: a single cross-module blanket impl is already
placement-illegal (must-live-in-declaring-module rule), and two such impls surface as
a duplicate error only because `check_impl_decls`' module-blind duplicate scan runs
before the placement loop — that scan's own coverage is the same-module duplicate
shape, not a deliberate cross-module ambiguity check. A third-module bare caller with
no own header is governed by **P7b.S10** (see [slice9-spec](./P7b/slice9-spec.md)
Phase 4 for the shape's original determination): the dispatch on the single
instantiation minted into the shared whole-program env that S9 left open is closed —
the bare call is a located compile-time error unless an exemption holds, never a
deterministic pick.

**P7b.S10 — Header-level export ambiguity for the third-module bare caller.**
Implemented in parallel with P7b.S6 (both branch from base `a9eca84`; maintainer
ruling 260905). Landing rule: if S6's own probes demand a check-stage grounding
change in `terms.rs`, or the two slices' changes interact at merge time, S10's
checker change lands first and S6 rebases onto it. Closes S9's Residual: a bare
generic-ctor or destructure call in a module declaring no same-named header of its
own, whose single `env` candidate is a foreign eager mint, no longer silently
dispatches on whichever module happened to spell the instantiation. The grounding
fall-through (`foreign_single_candidate_grounding`, `src/check/terms.rs`) raises a
located compile-time error unless an exemption holds: the caller's own header grounds
first (S9's R1.1a); at most one same-named header is reachable **and** the sole
candidate's declaring module is itself reachable, over the caller's own import set —
`imports` ∪ `selective` targets, name-independent, extended through a generic-header
export-origin walk that chases re-exporting hubs; the call reached the multi-candidate
arm (S5's tier policy, untouched); or a *named* selective import — never a `*`
wildcard desugar — hub-resolved to the sole candidate's declaring module.
Reachability is scoped to the caller's own import set, never the whole-program
closure; the check runs after the candidate-identity guard, so an ordinary user word
returning another module's instantiation keeps its own resolution; and the exemptions
compare against the header's *declaring* module (`GenericStructDecl.module`), never
the instantiating one. Two located shapes: the ambiguity error (every reachable
declaring module named by the caller's own import qualifier, lexicographically
sorted; wildcard-bound modules phrased structurally) and the reach-failure error
(sole candidate's declaring module unreachable, named structurally). See
[slice10-spec](./P7b/slice10-spec.md) with its frozen
[brief](./P7b/slice10-brief.md), [paper tests](./P7b/slice10-paper-tests.md), and
[probes](./P7b/slice10-probes.md).
**Exit:** GA-GP goldens (`tests/phase7b_slice10.rs`): the ambiguity and reach-failure
errors byte-exact and deterministic across import order and minter placement; the
existing 2-candidate error and every single-header, hub, and selective-import shape
byte-identical; S9's G4 golden retired (GA/GB are its inverted replacements); 12
units beside the changed `terms.rs` code.

**P7b.S6d — Slices as Iterator targets (the built-in-view lift).**
Corrected 260907 after a live probe (two earlier sketches in this entry's history were
wrong: the S6 carve-out's "third `GenericId` case" framing, and an interim "the value
type is missing" reading — `Slice['T]`/`!Slice['T]` have existed since P7.S3c as
runtime-length views, `Type::Slice` interned per `(element, mutable)` with no count
("the length is a runtime component of the value"), `slice`/`subslice`/`len` words, and
one-instantiation-over-all-lengths semantics — no per-N monomorphs).

Today (probe-verified): `impl: Iterator for Slice[i64]` and `!Slice[i64]` parse as
impl targets but the App-headed member row hits the S2-6 fence
(`member_app_concrete_target_error`) — the P7b.S8 lift admits only fully-applied
ctor-application targets (`Range[i64]`), and a built-in slice has no ctor header for
`is_mono_ctor_app` to recognize. The generic spelling `Slice['T]` fails earlier
("unknown type `'T`": no ctor header to bind the target variable).

Landed scope: admit element-concrete slice targets (`Slice[i64]`, `!Slice[i64]`) to the
P7b.S8 lifted-mono route — the member grounds mono exactly like Range's — then
`impl: Iterator for Slice[i64]` is a library impl over existing words (`len`, `&!>`
element read, O(1) `subslice` remainder, Step construction over plain fields — the S6
wall is not in play). Whether the generic-element spelling (`Slice['T]`) is also worth
admitting is a discovery measurement; the mono route may suffice for the stdlib's
per-element impls. Discovery questions: shared vs exclusive iteration (`Slice` is
multi-use, `!Slice` linear — which takes the impl, or both), the remainder's
mutability, and the linear-discipline story for a view that owns nothing. Size: `S-M`
(checker/target-grammar only; no `src/ir/` change; lib impl + goldens). See
[slice6d-brief](./P7b/slice6d-brief.md) with its [probes](./P7b/slice6d-probes.md):
the blocker is confirmed exactly as above (`member_app_concrete_target_error`,
identical for `Slice[i64]`/`!Slice[i64]`; `Slice['T]` is unreachable as a target
spelling, not merely unadmitted) and the S8 Range lift's mechanism
(`is_mono_ctor_app`/`GenericId`) does not transfer to a built-in `Type::Slice` target.
A follow-up spike (round S6d-2, 260907) answered the fence-lift mechanism (a
sentinel-substitution grounding path works, smaller than S8's own route) but surfaced
a second, independent blocker: `Step`'s `Rest` field cannot hold a reference-shaped
`Slice[i64]`/`!Slice[i64]` under `check_no_stored_references`. A third spike (round
S6d-3, 260907) closed the carve-out option on this: unsound by the rule's own
escape-safety rationale (no lifetime tracker exists to catch it any other way) and
independently unbuildable (a deliberate, tested `layout.rs` refusal for slice-shaped
enum payload fields). A fourth spike (round
S6d-4, 260907) closed the alternate-shape route as well: an Option-row `next`
(remainder as a bare stack value) dies at lowering on the same slice-field layout
refusal, reached through the synthesized ≥2-output return-bundle struct — under any
protocol whose `next` has two outputs, slices-as-Iterator is gated on one missing
capability: slice-shaped fields in aggregates (IR layout) plus a storage-class/taint
rule for reference-bearing aggregates. Ruling direction (maintainer consistency
preference, 260907): no protocol fork, no carve-out — S6d is **deferred behind that
prerequisite capability slice**; once it exists the Step-row protocol works for slices
unchanged.

**P7b.S6d-PREREQ — Reference-bearing aggregates (the capability S6d defers
behind; prerequisite, not an S6d deliverable).** Recorded 260907 from probe
rounds S6d-3/S6d-4 ([slice6d-probes](./P7b/slice6d-probes.md)); evidence-
scoped, no spec written. Today two rules jointly make any reference-shaped
type (`&T`/`&!T`, and `Type::Slice` by design, `builtins.rs:564`) unstorable
and unreturnable: `check_no_stored_references`
(`check/declarations.rs:1074`, the sole escape-safety mechanism — no
lifetime tracker exists in Sooth) bans reference-shaped payloads from
struct/enum fields, and `check_reference_free_signature` bans a non-inline
word from declaring a reference-shaped output. Four walls close every
shortcut past them: a `contains_reference` carve-out for `Type::Slice` is
unsound by the rule's own rationale (a `Step[i64 Slice[i64]]` could
outlive the buffer it views, with no tracker to catch it) *and* ICEs at
`src/ir/layout.rs:271` (enum/struct payload layout has a deliberate, tested
refusal — `scalar_size_align_refuses_a_slice` — for slice-shaped fields: a
slice is two words `{ptr, len}`, not one); an Option-row protocol shape
(remainder as a bare stack value) dodges the user-visible storage rule but
not the IR — `intern_output_bundles` (R8/R10) synthesizes a return-bundle
struct for every ≥2-output word, and that struct hits the identical layout
refusal; a named local captured by a quotation builds a closure-env struct
with a slice field (same storage class); and the poly-body stack checker
loses an App-headed dispatch call's outputs (standing S8-era limitation,
independent). The capability slice's content, when it is taken up: (1)
slice-shaped fields in declared and synthesized aggregates — new enum/struct
payload layout for a two-word slot (tag placement, projection, codegen;
`scalar_size_align` gains a slice arm or slices resolve through a
non-scalar slot path), touching `src/ir/layout.rs`, not just the checker;
(2) the soundness story the layout work unblocks: a storage-class/taint
rule extending the no-stored-reference discipline to *containing* values
— e.g. an aggregate holding a slice is itself reference-bearing, cannot be
returned from a non-inline word, cannot be captured, and cannot outlive
the frame its borrowed storage lives in — which without real lifetime
tracking must remain a conservative ban pattern, not an escape-permitting
one; and (3) the standing poly-body App-dispatch output loss, if generic
Iterator consumers over slices are to type-check. Landing (1) without (2)
opens the exact escape the current rules exist to prevent; that is why
this is a design-bearing slice and why S6d waits for it rather than
shipping a checker relaxation. Size: `M-L` (layout + checker design;
not driven by need until a slice-class target is actually wanted).
See [slice6d-prereq-spec](./P7b/slice6d-prereq-spec.md).

**P7b.S8c — Located fence for member signatures with unbindable free type
variables.**

**P7b.S8c — Per-site binding for member signatures with free input type variables.**
Closes the lowering panic the P7b.S8 integrated review found (260906; pre-existing, not
introduced by S8 — the repro reproduces at the S8 base `86ca5eb`): a trait member whose
signature carries a free INPUT type variable — not the trait's header variable, not bound
by any bound bracket — passed checking and panicked at lowering, reaching
`subst_polytype`'s unification `expect` (`src/ir/driver.rs:579`, "checked: unification
bound every input type variable") from lowering's R9 instantiation loop; the panic was a
**build-time lowering panic**, not a run/IR-time one — the check succeeded, `sooth build`
exited 101, and produced no binary. Shape: a struct type with a `mkbox`-style constructor
word (ctor names resolve under a declared-type expectation), a trait member carrying a
free input variable (`trait: Odd['T] : odd ( 'U &'T -- ) ;` — `'U` is bound by nothing),
an impl, and a bound-dispatched consumer (`: consume ['T: Odd] ( 'U &'T -- ) odd ;`
called as `mkbox | b | 7 &b consume`).

The surface was a dispatch taxonomy keyed in `resolve_user_bound` (`src/check/poly.rs:8981`)
on the obligation's `ty`: the healthy CtorImage route (a CtorImage-headed operand grounds
every member variable per site); the ICE route (the non-CtorImage generic-winner mint arm,
recording the member instantiation without the member row's own variables in θ — the
operand cells that hit it were the plain slot, the `&'T` ref, the quotation-row, and the
array-element shapes); the concrete-winner hole (the non-CtorImage concrete-winner arm was
a *silent* no-site-check hole — `ground_member_type`'s `Var` arm, `src/ast.rs:2241`,
collapses every free member local to the target type, so an operand of any type flowed
through unchecked); and the QuotLit cell, shipped fenced by this slice —
`fence_quotation_literal_slot` (`poly.rs:9480`, formatter
`member_slot_quotation_literal_error`, `poly.rs:9612`) fires in both the mint
and concrete arms, closing a post-fix hazard the re-grounding step would
otherwise newly expose. The direct mono member call (Route D) resolves
outside this taxonomy entirely, through `match_slot` (`src/check.rs:381`),
untouched; the lifted mono ctor-app target (Route E) sits inside the
taxonomy, with two tenancies in `resolve_user_bound`: the CtorImage arm's
`lifted_mono` branch (`src/check/poly.rs:9203-9219`) and the bare-symbol
arm's other tenant (`src/check/poly.rs:9365-9399`) — both untouched and
healthy; D2's `is_concrete()` gate (`poly.rs:9384`, in that second arm)
is what keeps the Route E tenancy out of the site check. On the
impl-target side, three
spellings hit the ICE route the same way: the bare-var-target spelling
(`for 'T`); a bare-CTOR-target spelling (`impl: Odd for Box`, no type
argument) — `for Box` desugars to `for Box['ctor0]` (`src/parser.rs:677-678`),
structurally the same one-variable generic-pattern shape as the bare-var-target
cell; and a mono ctor-app spelling (`impl: Odd for Box[str]`), which rides
this route whenever the member row keeps a free local (its grounded
signature is not var-free at `src/parser.rs:4519`, so it takes the generic
path). All three panicked at the slice's baseline and are grounded by this
fix the same way (the third measured: minted symbol
`sooth_mono_odd_Odd_0_Box_str___m0__t0_i64`); only the bare-var-target cell
carries a dedicated golden, the other two are unwitnessed end-to-end.

Shipped: direction B — per-site binding in the mint arm (`resolve_user_bound`'s
non-CtorImage generic-winner arm, `src/check/poly.rs:9315-9364`; `compose_member_theta`,
`poly.rs:9415-9471`) composes `theta` as `subst.clone()` (the impl-target match
substitution) extended per call site with that site's own re-grounded operands, unified
against the member word's signature (`unify_poly_input`, `poly.rs:10747`), and fails
closed on any signature variable still unbound before minting (`poly.rs:9462`) — the
fence that keeps `driver.rs:579` unreached, never touching it directly (REQ-3). D2's
concrete-arm site-slot check folds in alongside it (`check_concrete_member_site_slots`,
`poly.rs:9506`, gated on `imp.target.is_concrete()`): a plain `Type` equality comparison
between each re-grounded operand and the member word's already-grounded declared input —
stricter than the direct mono call's `match_slot` (`src/check.rs:381`), which admits
exactly one coercion (an `i64` literal to a size type); bound-dispatch slots carry no
literal coercion at all, by design.

The repro twins panicked→**grounded** (they build and run): only three shapes stay
**located** rather than grounded — the mint arm's residual-unbound-variable fail-closed
tail (a shape neither the impl-target match nor the site's own operands determine), the
QuotLit cell, and D2's concrete-arm mismatch. Scope: `src/check/` only, no `src/ir/`
change (`driver.rs:579` unchanged; G2/G4 double as the proof it never fires on the
already-healthy routes). Size: `M`/`S`/`M`/`S` across the slice's four phases (mint-arm
fix / concrete-arm fix / goldens / roadmap-and-gate). See [slice8c-spec](./P7b/slice8c-spec.md)
with its frozen [slice8c-brief](./P7b/slice8c-brief.md), [slice8c-probes](./P7b/slice8c-probes.md),
and [slice8c-paper-tests](./P7b/slice8c-paper-tests.md).
Growth-structure re-check (CLAUDE.md, at this phase's exit) over the files this slice
touched — `src/check/poly.rs` and `tests/phase7b_slice8.rs`: no new signals from
this slice's diff; `poly.rs`'s standing count (still 3/5) is unchanged (the two new
functions, `compose_member_theta` and `check_concrete_member_site_slots`, sit beside
`resolve_user_bound`'s existing arms as more binding/check functions of the same kind
already there), so the split stays deferred. `tests/phase7b_slice8.rs` is an existing
file gaining goldens in place, not a new module.

**P7b.S11 — Per-call-site grounding for bare generic constructors.**
Carved out of the P7b semantics walkthrough (260907) after probe round dp_a–dp_h
traced how bare generic-ctor calls actually ground: there is **no per-call-site
mechanism** — concrete type applications anywhere in a module mint monomorphs into a
whole-module registry at parse time (`parser.rs:7294`), and a bare ctor call scans that
registry (`mint_fallback_candidates`, `terms.rs:2010`): zero mints → a generic
`unknown word` error (`terms.rs:923`); one mint, even an unrelated one → taken
unconditionally (`terms.rs:951`); two or more → silent first-declared-wins
(`builtins.rs:164-183`), so two programs differing only in unrelated declaration order
can construct different runtime types from the same call (dp_g2/dp_g3 — a correctness
gap, not a diagnostics gap). Explicit type args on a bare ctor are category-illegal
today (`poly_call_takes_type_args`, `terms.rs:1300`). The slice builds per-call-site
grounding: literal-driven partial inference, consumer-driven re-grounding in S2-9's
obligation style, an explicit-type-args category for bare ctor/destructure names (full
arity), and a deterministic tie-break — explicit args > single compatible candidate >
located ambiguity error, first-wins retired. Checker-stage only; S5's declared-overload
tier policy, S2-9's member dispatch, and S10's foreign grounding + exemptions stay
byte-unchanged (baseline `probes/dp_baseline.md`). Maintainer ruling: ground at the
call site ("B"), 260907. See [slice11-brief](./P7b/slice11-brief.md) and
[slice11-probes](./P7b/slice11-probes.md).
**Exit:** `1 Ok drop` is a located unbound-parameter error naming the parameter (not
`unknown word`); a wrong sole mint is a located grounding error, never a far-away
operand mismatch; tied candidates are a located ambiguity error byte-identical across
declaration orders; `1 Ok[i64 i64] drop` is legal and grounds; the
`1 Ok [ 1 sub ] map[i64 i64 i64] drop` shape grounds through the consumer's
constraints; genuinely undefined names keep `unknown word`; S10's diagnostics and the
dp_a/dp_b/dp_f behaviors are byte-identical to baseline.

**Dogfood:** S6 — a program that `map`s and folds over `Option`, `Result`, and `List` through
shared bounds, with the impls declared against the real lib types and output matching
hand-written inline equivalents. S7 — `bind` dispatching per constructor over `Option`/`Result`
through a shared `Monad` bound. S8 — a consuming `for_each`/`fold` loop over an `Iterator`, real `List` and `Range`
impls, no per-impl copy of either consumer.

## Out of scope

GATs (generic associated types), associated types, dependent types, polymorphic kind recursion,
kind polymorphism. `Applicative.pure` alone is library work once S6 settles
grounding for return-type-polymorphic words. A general `Default` trait for construction stays
out (S6's `Monoid.empty` is scoped to merge identities, not defaults). Drop-forwarding for
user-declared generic containers (a `Drop` trait) rides with P9's alloc layer where the need
first becomes real; whether drop_graph already covers payload drops for minted generic
instantiations is an S6 probe note (partially answered by the dp probe round, 260907:
grounded non-linear instantiations drop flat and nested — `probes/dp_findings.md`
dp_a/dp_d; resource-owning payloads remain P9.S2's question).
