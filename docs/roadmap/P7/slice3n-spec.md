# P7.S3n spec — A generic struct's field cannot wrap its own type variable

Delivery plan. Read `docs/roadmap/P7/slice3n-brief.md` first: it holds the confirmed
root cause, the live-probed diagnostics, and the paper-traced design this spec commits
to. Every `path:line` anchor below was re-verified against live `main` (`a95f74a`);
`parser.rs`/`ast.rs`/`check/poly.rs` drift as other slices land, so line numbers are
anchors to re-confirm at implementation, not contracts.

The brief left exactly one design question open (the structural-growth detection
mechanism for a `^`-wrapped self-reference). It is resolved below in "Resolved:
growing-recursion detection", mirroring how S3k's spec resolved its own analogue. The
brief's two-phase sizing is adopted unchanged; the mint-before-substitute ordering's
borrow consequence is pinned concretely rather than left to the implementer, and a
header-registration-ordering defect the brief's own confirmed root cause implied but did
not fully resolve (a placeholder header can be instantiated against before it is filled)
is fixed as part of R2. The registry-threading container is a **deliberate deviation**
from the brief's explicit ruling (it ruled for plain threaded parameters, not a struct),
approved by the user during spec-writer's review loop -- see "Resolved: the registry
container" for why.

## Problem

`parse_generic_field_type_expr` (`src/parser.rs:4124`) is the only production a generic
`type:`'s field list resolves through, and it has no recursion at all: one `if` asking
whether the token sitting *right here* is a bare `'`-word bound by the header, else a
fall-through to `parse_field_type_expr` (`parser.rs:3502`), the concrete-only field
parser, which knows nothing of the enclosing declaration's variables. A bare `v 'T`
works because the variable occupies exactly that one inspected position. Wrap it in
anything and the variable is one token deeper than the check reaches:

```text
type: Pair 'T items ['T 2] ;        \ error: unknown type `'T`
type: Cell 'T c ^'T ;               \ error: unknown type `'T`
type: Wrap 'K 'V e Ent['K 'V] ;     \ error: unknown type `'K`
type: NestArr 'T grid [['T 2] 2] ;  \ error: unknown type `'T`
type: Box 'T r &'T ;                \ error: unknown type `'T`
```

The word-signature path is *not* the same gap wearing a different hat:
`parse_poly_slot` (`parser.rs:2421`) already descends recursively into `[`, `&`, and a
following generic application (`: id2 ( ['T 2] -- ['T 2] ) ;` builds today). It is a
partial precedent, not a complete one: it has no `^`-led arm either (`^` appears only as
an exclusion guard at `parser.rs:2492`), so `: idc ( ^'T -- ^'T ) ;` also fails.

Three further layers are broken behind the parser, each independently:

1. **Substitution.** `substitute_generic_field` (`ast.rs:683`) handles only
   `Concrete`/`Var` and `unreachable!`s on everything else. A correctly-parsed
   `Array(Var(0), Concrete(2))` panics there today. `apply_subst`
   (`check/poly.rs:4389`) is the exact template for the missing arms.
2. **Header registration ordering.** `parse_generic_typedefs` (`parser.rs:3867`) pushes
   a completed decl onto `self.generics.structs`/`.enums` only *after*
   `parse_generic_typedef`/`parse_generic_enum_typedef` (`parser.rs:3703`, `:3743`) has
   parsed the entire field list, so a self-reference inside that list has no header for
   `find_struct`/`find_enum` (`ast.rs:793`, `:800`) or `poly_generic_header`
   (`parser.rs:4010`) to find. Probe-confirmed independent of type variables: the fully
   concrete `type: L 'T next L[i64] ;` fails identically to `next L['T]`.
3. **Instantiation ordering.** `instantiate_struct` (`ast.rs:817`) substitutes the whole
   field list (`ast.rs:830-833`) *before* minting the `Type::Struct` id and pushing the
   `(idx, module, args)` memo key (`ast.rs:836-838`). Once substitution can recurse into
   the instantiator, a same-argument self-reference recurses without ever seeing a memo
   hit. `instantiate_enum` (`ast.rs:857`) has the same ordering and calls the same
   `substitute_generic_field`.

## Requirements

- **R1** `parse_generic_field_type_expr` gains a real recursive descent, mirroring
  `parse_poly_slot`'s intercept arms, resolving each leaf `'name` against the
  declaration's own `ty_vars: &[(String, Span)]` (marking `used[idx] = true`). The
  covered shapes: array (`['T 2]`, nested to any depth), reference (`&'T`, `&!'T`),
  owned cell (`^'T`), and generic application (`Ent['K 'V]`, including a mixed
  concrete/variable argument list). The fully-concrete fall-through to
  `parse_field_type_expr` is unchanged in behaviour.
- **R2** Header registration is **two-stage**, not a single "push a placeholder then fill
  it" (a mutual cross-header cycle forces this: `parse_generic_typedefs`'s existing single
  loop registers one header immediately before parsing *that header's own* field list, so
  a header declared earlier that self-references one declared later still finds nothing --
  no placeholder exists yet for the later one either). Stage (a), a genuine pre-pass
  extension: scan the whole token stream once, and for every generic `type:` header found,
  call `parse_generic_header_vars` alone (name, `ty_var_names`, span, module -- nothing a
  field/variant list needs is missing) and push a placeholder decl with an empty
  `fields`/`variants`, recording the token position immediately after the header for stage
  (b). Stage (b): revisit each recorded position in order and parse only the field/variant
  list (a small helper split out of `parse_generic_typedef`/`parse_generic_enum_typedef`
  that takes the already-known `name`/`ty_vars` and parses from there), overwriting the
  placeholder's `fields`/`variants` in place once that header's own list finishes parsing.
  This gates every self-referential case, concrete argument or variable argument alike, and
  is required regardless of R1.

  A second problem sits underneath the registration ordering itself: a **placeholder is not
  merely absent, it is actively wrong** if anything instantiates against it before stage (b)
  fills it in. `resolve_type_or_apply`'s existing concrete-application path calls
  `self.generics.instantiate_struct(idx, [i64], ...)` **immediately**, synchronously, the
  moment it parses a fully-concrete self-reference such as `L[i64]` inside `L`'s own field
  list -- which is exactly stage (b) parsing `L`'s fields while `L`'s placeholder (empty
  `fields`) is still the only entry `self.generics.structs[idx]` has. `instantiate_struct`
  today builds its field list from `decl.fields` unconditionally and then *permanently*
  memoizes the result (mints the id, pushes `struct_keys`, `struct_resolved`, and
  `inst_structs` -- all three parallel vectors, see the corrected Open Risks note below) --
  so this mints a permanently-wrong, fieldless struct with no diagnostic, and the memo means
  no later fix-up ever runs. This is independent of R1's recursive descent (a fully concrete
  argument needs none of it) and must be fixed as part of R2, not deferred to R6/phase 2,
  because R2's own success criterion below depends on it.

  **The fix: a `header_pending: Vec<bool>` flag parallel to `self.generics.structs`/`.enums`,
  set when stage (a) pushes a header's placeholder and cleared when stage (b) fills it.**
  `instantiate_struct`/`instantiate_enum` check it before computing fields: if
  `header_pending[idx]` is true, mint the id and push the memo key/`struct_resolved`/
  `inst_structs` placeholder exactly as R6 already does for its own (unrelated, phase-2)
  reason, but with an empty field list, and additionally record `(inst_index, idx, module,
  args)` on a small pending-instantiation list rather than computing fields now. When stage
  (b) fills header `idx`'s real fields, it drains every pending entry recorded against that
  `idx`: recompute fields via `substitute_generic_field` against the now-real `decl.fields`
  and overwrite `inst_structs[inst_index].fields` in place -- the `StructId`/`EnumId` already
  handed out does not change, so nothing that already holds that `Type` needs to change.

  This is a narrower, phase-1-only version of R6's general mint-before-substitute mechanism:
  it fires purely on "is the referenced header still being registered", independent of
  which `PolyType` shape a field is (R2's phase-1 test only exercises `Concrete`/`Var`, the
  two shapes phase 1 already supports) and independent of R4's `Generic`-substitution arm
  (phase 2 only). R6's own mint-before-substitute reordering is a *different* case -- a
  header that is fully registered but whose instantiation at a given `(idx, args)` is
  currently *in the middle of substituting its own fields* re-enters itself -- and stays in
  phase 2, since it is only reachable once R4's `Generic` arm can recursively call
  `instantiate_struct`/`instantiate_enum` at all. The two mechanisms share the same shape
  (mint the shell, fill it later, revisit) at two different layers, which is worth stating
  explicitly rather than conflating.

  Direction chosen, and why: this is option (i) from the two evaluated in delivery ("track
  which header indices are still placeholders ... revisiting every `struct_keys`/`enum_keys`
  entry recorded against it"), not option (ii) (deferring the call into a worklist drained
  before body-level checking begins). Option (ii) would require the field parser to hand
  back something other than a ready `Type` at the moment it parses `L[i64]` -- but a
  `Type::Struct(id, name)` handle is exactly the shape the language's own opaque-handle
  invariant (`Ptr[T]`/`StructId` never assumed to be more than an id) is built for: minting
  the id eagerly and filling its contents in later is the natural fit, not a workaround.
  Option (i) also composes directly with R6's own placeholder-then-fill discipline instead
  of introducing a second, differently-shaped deferral mechanism next to it.
- **R3** `PolyType` (`ast.rs:1570`, its `Ref` variant at `:1598`) and `RawTy` (`parser.rs:1175`) each gain an
  owned-cell variant mirroring their `Ref` variant's shape: `OwnedCell(Box<PolyType>)` /
  `OwnedCell(Box<RawTy>)`, with **no id minted** until the payload grounds (the same
  reasoning `Ref` documents: the payload may be a variable, which no registry entry can
  name). `parse_poly_slot` gains a `^`-led arm; `raw_to_poly_type` folds a fully-concrete
  payload to `Concrete(Type::OwnedCell(..))` via `intern_owned_cell_type` (`ast.rs:1000`)
  exactly as it folds `Ref`; `apply_subst` gains the matching arm. Every other exhaustive
  `match` over `PolyType`/`RawTy` is updated — Rust's exhaustiveness check enumerates the
  sites as hard build errors, so they are not hand-listed here.
- **R4** `substitute_generic_field` gains `Array`/`Ref`/`Generic`/`OwnedCell` arms,
  interning each result into the concrete registries and, for `Generic`, recursively
  calling `instantiate_struct`/`instantiate_enum` on its own substituted arguments. Its
  fallback arm stays `unreachable!` and stays *truthful*: R7's parser-side rejection must
  make a `PolyType::Quotation`/`QuotLit` field unconstructible, not merely unexpected.
- **R5** Registry threading does **not** go through `NameRegistries` (`ast.rs:580`),
  which is `Copy` over immutable slices and can intern nothing. See "Resolved: the
  registry container" for the container this spec commits to and the borrow
  restructuring it forces at `parser.rs:3948-4000`.
- **R6** `instantiate_struct`/`instantiate_enum` mint the id, push the memo key
  (`struct_keys`/`enum_keys`) and push a **placeholder** `StructDecl`/`EnumDecl` with an
  empty field/variant list **before** substituting fields, then fill the fields in place.
  A same-argument recursive re-entry therefore hits the memo on first re-entry and
  returns the already-minted id. (Note the structural echo with R2: both are the same
  "register the shell, then fill it" move, at two different layers.)
- **R7** A quotation-typed field naming the declaration's own type variable
  (`type: QF 'T f [ 'T -- 'T ] ;`) is **out of scope**, rejected at the field parser with
  a located, worded error — never a panic — mirroring the existing
  `a ~ quotation cannot appear here` rejection (`parser.rs:3503`). The new `[`-arm must
  replicate `parse_field_type_expr`'s `quotation_type_ahead()` disambiguation
  (`parser.rs:3197`) so a *concrete* quotation field (`f [ i64 -- i64 ]`, legal today)
  is not misparsed as a malformed array.
- **R8** A **growing** self-referential generic application is a **located compile-time
  rejection**, never a hang or a stack overflow. See "Resolved: growing-recursion
  detection".
- **R9** A by-value or array-wrapped self-referential generic struct/enum, **instantiated
  at a concrete type**, is rejected by the **existing** `check_recursion`
  (`check/declarations.rs:1414`, `type_node` at `:1461`) `recursive struct definition
  (infinite size)` diagnostic, now reachable for a generic self-reference for the first
  time. No new diagnostic is invented for it. `check_recursion` only ever walks
  post-instantiation concrete `StructDecl`/`EnumDecl`s; an uninstantiated generic header
  produces no concrete decl at all and so never reaches it -- both golden tests below must
  instantiate the self-referential generic at some concrete type first, then assert the
  diagnostic fires on that instantiation, not on the bare generic declaration.
- **R10** A `&'T` field is **not** made to build. Once R1 resolves it to
  `PolyType::Ref(Var(0), false)` and R4 substitutes it, a concrete instantiation of
  `type: Box 'T r &'T ;` is rejected by the existing, unconditional no-stored-reference
  rule (`check/declarations.rs:1170`) with `a reference cannot be stored`, replacing
  today's misleading `unknown type 'T`. This is a **diagnostic-quality** requirement.

## Non-functional requirements

- **N1** No panic on any program in scope, legal or illegal: every rejection in
  R7/R8/R9/R10 is a located diagnostic. In particular no field shape admitted by R1 may
  reach `substitute_generic_field`'s `unreachable!`.
- **N2** Compilation terminates on every program the checker admits. Termination is a
  consequence of R8 restricting the reachable `(header, args)` set to a finite closure
  (see the rationale below), not of a depth cap.
- **N3** No struct-header **length** variable is introduced. `parse_generic_header_vars`
  (`parser.rs:4102`) binds only `'`-prefixed type variables and continues to; there is no
  `GenericStructDecl` analogue of `PolySig::len_var_names`. Consequently
  `substitute_generic_field`'s array arm only ever sees `Len::Concrete` and must **not**
  grow a `Len::Var` path (unlike `apply_subst`'s word-signature twin, which handles both).
- **N4** Existing behaviour is unchanged for every shape that works today: a concrete
  quotation field, a concrete generic-application field through
  `resolve_type_or_apply` (`parser.rs:3948`), and a bare `'T` field all keep their
  current results. `check_no_phantom_ty_var` (`parser.rs:1621`) needs no change — it
  reads only the `used` bitmap, which R1's descent sets at every leaf `Var` regardless of
  depth.
- **N5** Craft-scope discipline (CLAUDE.md): no abstraction beyond what these two phases
  need, no pre-staged plumbing for a future length-variable or `Map` slice.

## Success criteria (observable)

Phase attribution in brackets.

- `[P1]` `type: Pair 'T items ['T 2] ;` parses to
  `PolyType::Array(Box::new(Var(0)), Len::Concrete(2))`.
- `[P1]` `type: NestArr 'T grid [['T 2] 2] ;` parses to the nested `Array(Array(Var(0)))`.
- `[P1]` `type: Cell 'T c ^'T ;` parses to the new owned-cell variant over `Var(0)`.
- `[P1]` `type: Wrap 'K 'V e Ent['K 'V] ;` parses to
  `Generic { args: [Var(0), Var(1)], .. }`; `Ent['K i64]` to
  `Generic { args: [Var(0), Concrete(i64)], .. }`.
- `[P1]` `type: Box 'T r &'T ;` parses to `Ref(Box::new(Var(0)), false)` (it does not yet
  build; see R10 / `[P2]` below).
- `[P1]` `: idc ( ^'T -- ^'T ) ;` builds — a word-signature side effect of R3, recorded
  honestly as a bonus, not as evidence the mechanism pre-existed.
- `[P1]` `type: L 'T next L[i64] ;` resolves its self-reference to a `Type` instead of
  `unknown type 'L'` (R2, independently testable with no dependence on R1's descent).
- `[P1]` `type: QF 'T f [ 'T -- 'T ] ;` is a located, worded rejection (R7);
  `type: Q f [ i64 -- i64 ] ;` still builds.
- `[P2]` The `Map`-shaped stand-in `type: Ent 'K 'V k 'K v 'V ; type: Map 'K 'V slots [Ent['K 'V] 8] ;`
  builds, instantiated at two different concrete `('K, 'V)` pairs, asserting on the
  resulting `StructDecl.fields` types **and** on the two instantiations minting distinct
  `StructId`s — mirroring `instantiate_struct_distinct_across_modules_same_bare_name`
  (`src/driver.rs:1066` -- **not** `src/ir/driver.rs`, which has unrelated code at that
  line number; re-verified) and `instantiate_struct_distinct_for_wrapped_cross_module_args`
  (`src/driver.rs:1128`). Not merely "it builds".
- `[P2]` `type: Box 'T r &'T ;` at a concrete instantiation errors with
  `a reference cannot be stored`, asserted against that text, not against `unknown type`
  (R10).
- `[P2]` A by-value (`type: L 'T next L['T] ;`) and an array-wrapped
  (`type: L 'T kids [L['T] 4] ;`) generic self-reference, **each instantiated at a
  concrete type**, are rejected with `recursive struct definition (infinite size)` (R9).
  Neither test may assert against the bare generic declaration: `check_recursion` only
  walks post-instantiation concrete decls, so the diagnostic only fires once something
  instantiates the self-referential generic.
- `[P2]` A `^`-wrapped non-growing generic self-reference (`type: L 'T next ^L['T] ;`)
  builds and terminates, at a concrete instantiation.
- `[P1]` A `^`-wrapped growing self-reference (`type: L 'T next ^L[^'T] ;` -- each hop
  wraps `'T` in another owned cell, with no `Generic`-in-`Generic` nesting anywhere in
  sight) is a located compile-time rejection at parse time (R8), not a hang. This is `[P1]`,
  not `[P2]`: R8 fires structurally on the `args` tree R1's descent has just built, with no
  instantiation involved, matching Phase 1's own Scope/Exit Criteria, which already place
  R8 there (`L[Ent['T i64]]`, R8's original witness, is *not* used here because it is
  already rejected today by the pre-existing, unrelated `generic_nesting_depth_error` (D5,
  `parser.rs:1720`) before R8 would ever fire -- see "Resolved: growing-recursion
  detection" for the reconciliation).
- `[P2]` The pre-existing attributeless/positional variant array-field parse gap
  (`type: Foo | Some [i64 2] | None ;` → `parse error: expected a word, found LBracket`)
  is pinned unchanged.

## Scope and boundaries

**In scope:** R1–R10.

**Explicitly out of scope, each with a pinned test rather than silence:**

- A struct-header **length variable** (`type: Map 'K 'V 'N`). This slice removes **one of
  at least three** blockers on `Map['K 'V]`: friction #7 (`slice3-dogfood.md:173-174`,
  the `'N` gap) and friction #6 (`:164-169`, a `Default`-style bound for constructing the
  backing array, on top of `Eq`/`Hash` on `'K` per
  `docs/roadmap/P7-language-prereqs.md:251`) both remain. Do not cite this slice as
  unblocking `Map`; cite it as removing the array-field-of-own-type-variable blocker.
- A **quotation-typed** generic field (R7), rejected with a located error.
- The **attributeless/positional** variant array-field parse gap, pre-existing and
  unrelated to type variables (the concrete repro has no generic header at all). Pinned
  unchanged. A *named* generic variant field (`Some xs ['T 2]`) is fully in scope.
- **Untouched:** `resolve_type_or_apply`'s concrete-generic-application path except for
  R5's borrow restructuring; `check_no_phantom_ty_var`; `check_recursion`'s own logic
  (R9 makes it reachable, it does not change it); the no-stored-reference rule (R10 makes
  its diagnostic reachable, it does not change it).

**Exit-criteria widening, stated as such.** `docs/roadmap/P7-language-prereqs.md:598-616`
names only the array-field case. This spec covers ref / owned-cell /
nested-generic-application / the enum twin as well, because one mechanism (the recursive
field-type parser) covers them at once. That is a real widening of what the roadmap asked
for, not something it already implied.

## Resolved: growing-recursion detection (the brief's one open question)

The brief bounded the requirement (a growing `^`-wrapped self-reference must be a located
rejection) but explicitly left the **mechanism** open: a call-stack membership check over
in-flight instantiation arguments, versus a recursion-depth cap. This spec resolves it in
favour of neither, and instead of a third option it argues is strictly better: a
**declaration-time structural rule**, transposing S3k's resolution of its own analogue.

**The rule (R8).** R8 must be read as walking **every `PolyType::Generic` node anywhere in
a field's type tree** -- under an `Array`, a `Ref`, an `OwnedCell`, or nested inside
another `Generic`'s own `args` -- not only a field whose own top-level type is `Generic`.
A literal top-level reading ("a field of the form `PolyType::Generic { args, .. }`") would
miss exactly the shapes this rule exists to catch: `^L[Ent['T i64]]` is
`OwnedCell(Generic{..})` at the top level, and the `Map` shape (`[Ent['K 'V] 8]`) is
`Array(Generic{..})` -- the `Generic` node each rule must inspect is nested, not at the
field's own top level. At every `Generic` node the walk finds, each element of that node's
`args` must be **either fully concrete (however deep) or a bare `PolyType::Var(_)`**;
reject a **compound argument that mentions one of the declaration's own type variables**
-- `L[['T 2]]`, `L[^'T]`, `L[&'T]`. Rejection is at the field's span, at parse time, with a
dedicated diagnostic (`growing_generic_self_reference_error`, new).

Read it carefully: it is **not** "`Concrete` or bare `Var`" read shallowly. A
fully-concrete argument at any depth (`L[[i64 2]]`, `L[Ent[i64 str]]`) is allowed; the
reject predicate is exactly "the argument is compound **and** some leaf of it is a `Var`
naming an enclosing header variable".

**Interaction with D5 (`generic_nesting_depth_error`, `parser.rs:1720`) -- a pre-existing,
unrelated rule R8 must not duplicate as its own test.** `raw_to_poly_type`'s fold (the
word-signature path) already rejects any generic application whose own argument is itself
a `PolyType::Generic` -- nesting depth greater than one -- for a depth reason having
nothing to do with growth (probe-confirmed live: `: f ( L[Ent['T i64]] -- L[Ent['T i64]] )
;`, with `L`/`Ent` declared generic structs, fails today with `` `L[...]` at ... names `Ent[...]` as a type argument, but a generic
applied to another generic (nesting depth > 1) is not yet supported ``). `L[Ent['T i64]]` -- R8's original brief-inherited witness --
is a `Generic` nested inside a `Generic`'s own argument, so it is already rejected by D5
before R8's own check would ever run, on the word-signature path.

**Ruling: R1's new struct-field path does *not* enforce D5's depth-1 rule.** D5 is a
word-signature-only restriction (`raw_to_poly_type`'s fold, reached from `parse_poly_slot`);
R1's field parser is a separate production over a separate `PolyType` construction path and
never calls `raw_to_poly_type`, so D5 does not apply to it by default, and this spec does
not add a new call to extend it there. This is a deliberate, stated choice, not an
oversight: `parse_generic_field_concrete_nested_generic_argument_is_ok` (`L[Ent[i64 str]]`
in a field) is *accepted* by design, asymmetric with the word-signature path's rejection of
the same shape. The asymmetry is acceptable because R8 already bounds every field's
reachable-instantiation set to a finite closure regardless of nesting depth (a fully
concrete nested argument, at any depth, is inert -- it never grows across a recursive
self-reference, since it carries no type variable to substitute); D5's word-signature
restriction exists for a different, narrower reason (`slice3a-brief.md`'s v1 scope-limit on
generic-application arguments in a signature) that this slice does not inherit or relax.
A future slice may choose to unify the two paths' strictness; that is out of scope here,
and this paragraph is the explicit ruling a prior review flagged as missing rather than a
reason to redesign R8 -- D5's existing rule is untouched by this slice per the brief's own
scope, but either way a test built around `L[Ent['T i64]]` as
*R8's* witness is a placebo: it would pass even with R8 unimplemented, because the nesting
rejection already fires first. R8's real, non-overlapping job is rejecting a **growing**
argument that is *not* already caught by D5's flat nesting-depth-1 rule -- an argument that
repeatedly wraps a type variable in `Array`/`Ref`/`OwnedCell` without ever going through a
second `Generic` application, e.g. `type: L 'T next ^L[^'T] ;`: each hop needs `L`
instantiated at `^`-of-the-previous-argument, structurally growing forever, with no
`Generic`-in-`Generic` nesting anywhere in sight, so D5 never sees it. R8's golden test and
success-criteria example use this shape, not `L[Ent['T i64]]`; the latter is kept only as
an aside noting it is already covered by D5, not as R8's own test.

**Why this terminates without a depth cap (N2).** Under the rule, every generic
application appearing in a declaration passes each argument through unchanged: either a
type fixed literally in the source, or one component of the arguments the enclosing
header was instantiated at. No constructor is ever applied to an instantiation argument.
So instantiating any header at concrete `args` reaches only headers applied to types
drawn from the finite pool `{concrete types written literally in the program} ∪
{components of the seed instantiations' arguments}`. Headers are finite and each has
fixed arity, so the reachable `(header, module, args)` set is finite; R6's
mint-and-memo-before-substitute makes each member visited at most once, and the recursion
bottoms out. Termination is a theorem about the reachable set, not a bound.

Permutation and constant cycles still work, which is the point of not banning
self-reference outright: `type: A 'K 'V next ^A['V 'K] ;` alternates between two
instantiations and memo-hits; `type: A 'T next ^B[i64] ;` with `B` referring back to `A`
likewise. Mutual growth is covered because the rule applies uniformly to every generic
application in every declaration, not only to self-naming ones — a two-header cycle where
one hop wraps is rejected at that hop.

**Why declaration-time over the brief's two candidates.**

1. **Locatable.** R8 demands a located rejection. The structural rule fires at the field
   that wrote the growing application, naming that field and that declaration. An
   in-flight-args stack fires mid-instantiation, at whichever concrete use site happened
   to trigger it, naming a synthesized instantiation rather than the source of the
   defect; a depth cap fires further away still, naming nothing.
2. **Total, on information already in hand.** The `args: Vec<PolyType>` tree is fully
   built by R1's own descent, one statement before the check. No stack, no bookkeeping,
   no tuning constant, and no dependence on whether any use site instantiates the header
   at all — a growing declaration is rejected even if nothing ever instantiates it.
3. **Cannot false-reject a finite program by accident.** A depth cap rejects a
   legitimately deep-but-finite instantiation chain. The structural rule's rejections are
   exactly characterised (below) and independent of program size.
4. **It is the same rule S3k already ships,** transposed from a call-site image to a
   declaration-site argument. One growth concept in the language, not two.

**Accepted cost (a choice this spec makes, not something the brief mandated).** The rule
also rejects a *non-recursive* wrapping application that would in fact terminate:
`type: Outer 'T f Ent[['T 2] i64] ;`, where `Ent` never refers back to `Outer`. Admitting
it requires knowing whether the application sits on a cycle, i.e. an SCC pass over a
header-level dependency graph built from every generic field. That pass is buildable in
maybe forty lines, and this spec deliberately does not build it, on a measured payoff:
**not one shape in this slice's exit criteria, nor in `slice3-dogfood.md`'s `Map`
program, has a compound generic argument.** `Map` wants `[Entry['K 'V] 'N]` — an array
*of* a generic application, which the array arm handles, with bare-`Var` arguments. The
over-rejection costs nothing that is currently wanted. The diagnostic must name the
restriction explicitly (not just "recursive") so that a future slice with a real consumer
can lift it by adding exactly that SCC pass, and so a user hitting it is not told their
non-recursive type is recursive.

**Interaction with R9 (do not conflate the two rejections).** They are different checks
at different layers, and both must fire on their own witness:

- R8 (parse time, structural): the *shape of the argument* is growing. `^L[^'T]`.
- R9 (`check_recursion`, post-instantiation): the *edge kind* does not break the cycle.
  `L['T]` by value, or `[L['T] 4]` array-wrapped. `type_node` treats `Type::Array` as a
  non-breaking edge and deliberately excludes `Type::OwnedCell` as the one edge kind that
  does break it (a `^T` field is a heap pointer, not an inline copy). Pinned today for
  the non-generic case by `check_recursion_by_value_self_cycle_is_error`
  (`declarations.rs:3025`), `check_value_recursion_through_array_element_is_error`
  (`declarations.rs:2944`), and `check_recursion_cell_cycle_in_struct_field_is_ok`
  (`declarations.rs:3066`).

A by-value non-growing self-reference (`type: L 'T next L['T] ;`) passes R8 and is caught
by R9. A `^`-wrapped growing one passes R9's cell exclusion and is caught by R8. Neither
check subsumes the other; a test that only exercises one of them is not coverage of both.

**Consequence worth stating once: `^` is the only field-storable indirection.** `&T`/`&!T`
can never occupy a struct field (R10's rule, unconditional and pre-existing), and an array
does not break a cycle (R9). So owned-cell support is not a separable bonus in this slice
— it is the *only* mechanism by which any self-referential generic struct or enum can
exist, which is why R3 is a phase-1 prerequisite rather than a stretch goal.

## Resolved: the registry container (a deliberate deviation from the brief, approved)

The brief explicitly ruled on this: thread separate `&mut Vec<ArrayDecl>`/`&mut
Vec<RefDecl>`/`&mut Vec<OwnedCellDecl>` parameters, **not** a registries struct -- it is
not a gap the brief left open. This spec deviates from that ruling anyway, and says so
plainly rather than reframing the deviation as filling in something "left implicit". The
user approved keeping `MutRegistries` during spec-writer's review loop, on the concrete
argument below, and both a soundness review and an implementability review confirmed the
reborrow pattern and disjoint-field-borrow claims hold against live source. The reason the
deviation earns its cost: threading five parameters by hand through a *mutually
recursive* pair (`substitute_generic_field` ↔ `instantiate_struct`/`instantiate_enum`)
plus eight call sites is where the plain-parameter precedent stops paying -- one small
struct with two methods, introduced at its first and only use, is a smaller total surface
than five parameters threaded by hand through a recursive call graph.

`NameRegistries` (`ast.rs:580`) is `Copy` over immutable slices and cannot intern.
`apply_subst`'s precedent (`check/poly.rs:4389`) is separate `arrays: &mut Vec<ArrayDecl>,
refs: &mut Vec<RefDecl>` parameters with a throwaway read-only `NameRegistries` built at
the last moment for its `Generic` arm — and note it builds that one with `cells: &[]`,
`refs: &[]` (`poly.rs:3084`, `:4477`), a degradation this slice must not copy blindly now
that a cell can carry a variable payload.

Threading five registries by hand through a *mutually recursive* pair
(`substitute_generic_field` ↔ `instantiate_struct`/`instantiate_enum`) plus eight call
sites is where the plain-parameter precedent stops paying. This spec commits to a small
container:

```rust
pub struct MutRegistries<'a> {
    pub structs: &'a [StructDecl],
    pub enums: &'a [EnumDecl],
    pub arrays: &'a mut Vec<ArrayDecl>,
    pub refs: &'a mut Vec<RefDecl>,
    pub cells: &'a mut Vec<OwnedCellDecl>,
}
```

with two methods: `names(&self) -> NameRegistries<'_>` (the read-only reborrow
`type_instantiation_name` needs, with the real `cells`/`refs`, not `&[]`) and
`reborrow(&mut self) -> MutRegistries<'_>` (it is not `Copy`, so the recursion needs an
explicit reborrow at each hop). `instantiate_struct`/`instantiate_enum` take
`regs: MutRegistries` in place of today's `NameRegistries`. `MutRegistries` is scoped to
this mutually-recursive pair alone (`substitute_generic_field` and
`instantiate_struct`/`instantiate_enum`); `apply_subst`'s own new owned-cell arm (R3,
phase 1) does **not** take a `MutRegistries` -- see "Phase 1's `apply_subst` cells
threading" below for why it stays a plain added parameter instead.

This is one struct with two methods, introduced at its first and only use in phase 2 --
not a speculative abstraction laid down ahead of need. The alternative, five parameters
threaded through a recursive call graph, is the shape CLAUDE.md's "start small" guidance
is meant to prevent, not produce; a deliberate, stated deviation from the brief's ruling
is the honest way to adopt it, not a claim that the brief left the question open.

**R5's success criteria and named test (previously absent).** R5 was the only requirement
in the traceability matrix with nothing in either the Success Criteria or the golden test
plan -- fixed here rather than left as a silent gap:

- `[P2]` A nested `Generic` field instantiation (the `Map`-shaped stand-in, or any
  `Ent['K 'V]`-typed field) correctly interns through the live `cells`/`refs` vecs
  `MutRegistries::names()` exposes, not `&[]`/`&[]` -- a regression test that would fail if
  `type_instantiation_name`'s rendering ever degraded back to the empty-slice throwaway.
- Golden test: `map_shaped_field_instantiation_renders_through_live_cells_and_refs` --
  asserts the rendered instantiation name (or the field's resolved `Type`) is correct for
  a field that itself contains an owned cell or reference, which the old `cells: &[]`/
  `refs: &[]` throwaway would render wrong or panic on once a cell/ref entry actually
  exists to look up.

**The borrow restructuring this forces (the one live hazard, not a signature widening).**
At `parser.rs:3948-4000`, inside `resolve_type_or_apply`, `regs` is built by *immutably*
borrowing `self.structs`/`self.enums`/`self.arrays`/`self.owned_cells`/`self.refs` and
handed to `self.generics.instantiate_struct(...)` in the same statement. Those are
distinct fields of the parser, so today's immutable borrows coexist fine with the mutable
borrow of `self.generics`; a `MutRegistries` needs `self.arrays`/`self.refs`/
`self.owned_cells` **mutably** in that same statement, which still borrows three distinct
fields and so should also compile — but only if the `regs` construction and the
`instantiate_*` call are written as disjoint field accesses and not routed through a
`&mut self` helper method. Confirm this at implementation; if the borrow checker refuses,
`std::mem::take` the three vecs before the call and restore them after, rather than
cloning. The other seven call sites (`parser.rs:3131`, `:3134`, `:4174`, `:4188`;
`check/poly.rs:3092`, `:3094`, `:4483`, `:4485` -- 8 call sites total, re-verified by grep
against live `main`, not the drifted numbers cited elsewhere) are signature updates, and
the two `poly.rs` pairs additionally stop passing `cells: &[]`/`refs: &[]`. The unit-test
call sites in `ast.rs` (`:3042`, `:3043`, `:3044`, `:3082`, `:3089`, `:3128`, `:3129`,
`:3154`, `:3204` -- 9 call sites) pass `EMPTY_REGS` and need a mutable equivalent.

## Codebase map

- `src/parser.rs:4139` `resolve_type_or_apply`/field parser context; the
  `parse_generic_field_type_expr` field parser R1 rewrites is the single-`if` production
  it currently falls through to. `parse_field_type_expr` (concrete fall-through,
  unchanged), `quotation_type_ahead` (R7's disambiguation), and the
  `a ~ quotation cannot appear here` rejection R7's wording mirrors are all in the same
  region; re-locate exact line numbers at implementation, since this file has drifted
  materially since the brief was written (see re-verified anchors below).
- `src/parser.rs:2421`-area `parse_poly_slot` — the recursive-descent template for R1's
  arms; its `^` exclusion guard is R3's new arm's displacement target.
- `src/parser.rs:4058` `parse_generic_typedefs` — R2's two-stage rewrite target.
  `:3894`/`:3934` `parse_generic_typedef`/`parse_generic_enum_typedef` (split into
  header-only and fields-only halves per R2), `:4102`-area `parse_generic_header_vars`
  (already returns `ty_vars` before field parsing starts, so R2's stage (a) has everything
  it needs at that point), `:1621`-area `check_no_phantom_ty_var` (untouched, N4).
- `src/parser.rs:1175` `RawTy` and `raw_to_poly_type`'s fold — R3's new variant; `:1720`
  `generic_nesting_depth_error` (D5, the pre-existing rule R8 must not duplicate).
- `src/parser.rs:4139` `resolve_type_or_apply` — R5's borrow hazard, at call sites
  `:4174`/`:4188`.
- `src/ast.rs:1570` `PolyType` (its `Ref` variant at `:1598`) — R3's new variant.
- `src/ast.rs:683` `substitute_generic_field` — R4's new arms (and its doc comment, which
  currently asserts the two-shape invariant this slice removes).
- `src/ast.rs:817`/`:857` `instantiate_struct`/`instantiate_enum` — R6's ordering, and R2's
  `header_pending`-aware deferral (mint+memo at `:836-840`/`:888-891`, field
  substitution at `:830-833`/`:875-885`).
- `src/ast.rs:580` `NameRegistries` (immutable, `Copy`), `:1000` `intern_owned_cell_type`,
  `:1044` `intern_ref_type`, `:1141` `intern_array_type`, `:793`/`:800`
  `find_struct`/`find_enum`.
- `src/check/poly.rs:4390` `apply_subst` — the arm-by-arm template for R4, plus R3's new
  arm and its plain `cells: &mut Vec<OwnedCellDecl>` parameter (phase 1, not
  `MutRegistries` -- see "Phase 1's `apply_subst` cells threading"); `:3087`-`:3088` and
  `:4478` the `cells: &[]`/`refs: &[]` throwaway regs its pre-existing `Generic` arm
  builds.
- `src/check/declarations.rs:1414` `check_recursion`, `:1461` `type_node` (R9,
  untouched), `:1170`-area the no-stored-reference message (R10, untouched).
- `src/driver.rs:1066`/`:1128` — **not** `src/ir/driver.rs` (re-verified: that file has
  unrelated code at those line numbers) -- the assertion style the `Map` regression test
  mirrors: `instantiate_struct_distinct_across_modules_same_bare_name` and
  `instantiate_struct_distinct_for_wrapped_cross_module_args`.

## Open risks

- **Exhaustive-match fallout (R3).** `PolyType::` appears on 401 distinct lines across
  eleven files (`src/ast.rs`, `src/check.rs`, `src/check/audits.rs`,
  `src/check/combinators.rs`, `src/check/declarations.rs`, `src/check/engine.rs`,
  `src/check/poly.rs`, `src/check/word_entry.rs`, `src/ir/driver.rs`, `src/parser.rs`,
  `src/repl.rs`; a line count, `grep -c`, re-verified against live `main` -- the raw
  occurrence count, `grep -o | wc -l`, is 432, since some lines carry two or more
  references); most are constructors, but every non-wildcard `match` is a hard build error
  until handled. The risk is not missing a site (the compiler will not let you) but
  **reaching for a wildcard** at a site that should reject the new variant explicitly.
  Audit each site for whether a cell payload is genuinely inert there; prefer an explicit
  arm to `_ =>`.
- **A wildcard arm already swallowing the new variant.** The inverse risk: an existing
  `_ =>` arm silently accepting an owned cell where it should reject. Grep for `_ =>` in
  the `PolyType` matches in `check/` specifically, per the "Type::Variant falls through
  the type predicates" precedent.
- **R2 versus duplicate-header detection.** `parse_generic_typedefs` guards duplicates
  with `generic_header_at_cursor_is_registered(already)`, where `already` is a
  `(structs.len(), enums.len())` snapshot taken at pass entry (`parser.rs:3868`). A
  placeholder pushed *during* the pass therefore sits above the snapshot and should not
  make a genuine second header look pre-registered — but this interaction is exactly the
  kind that fails silently. Pin it with a test that a real duplicate generic header is
  still rejected after R2.
- **Stage (a) reorders cross-declaration diagnostics.** Today a header-level error (a
  duplicate `'`-var, a malformed header) surfaces only once the parser reaches that
  specific `type:` in file order, interleaved with every other declaration's own errors as
  they're each hit in sequence. Under R2's two-stage split, stage (a) scans and validates
  every header up front, so a header-level error two declarations down now surfaces before
  an earlier declaration's own field-level error would have. This is a diagnostic-ordering
  change on multi-error files, not a soundness issue (every error still fires, just not
  necessarily in the same relative order as today) — worth a one-line mention in the PR
  description when this lands, not a test, since no existing test asserts cross-declaration
  error ordering.
- **R6's borrow, both structs and enums.** `instantiate_struct` holds `let decl =
  &self.structs[idx]` live across the field substitution; once substitution needs `&mut
  self`, that borrow must go. Clone the field `PolyType`s before substituting -- **not**
  `std::mem::take` the placeholder's list: that would take the *declaration's* field list
  itself, so a re-entrant instantiation of the same header at a different argument list
  (exactly what `permuting_generic_self_reference_terminates` re-enters) would find an
  empty list on its second call and silently mint a wrong, fieldless struct -- the same
  failure mode P0-1 fixes, reintroduced by the wrong choice of "clone vs. take" here. Do
  not restructure around a `RefCell` either. `instantiate_enum` has the identical hazard,
  unmentioned in an earlier draft of this note: it holds a live borrow of
  `&self.enums[idx]` across a `.map()` closure over `variant`s that also calls
  `type_instantiation_name` per variant, with a non-`Copy` `MutRegistries` and a `&mut
  self` substitution call inside that closure -- it must become an explicit loop over
  cloned variants, exactly mirroring the struct-side fix, not left as an oversight on the
  assumption the struct fix covers it.
- **R6's index stability, all three parallel vectors, not two.** The minted `StructId` is
  `struct_base + inst_structs.len()`. The placeholder must be pushed onto `inst_structs`
  **and** `struct_resolved` at mint time, in lockstep with `struct_keys` -- re-verified
  against live `ast.rs`: `struct_keys`, `struct_resolved`, and `inst_structs` are three
  parallel vectors pushed together in `instantiate_struct` today (`struct_keys.push`,
  `struct_resolved.push`, `inst_structs.push`, in that order), not two. Pushing the memo
  key without *both* the resolved-type entry and the placeholder decl desynchronises all
  three. The enum twin (`enum_keys`/`enum_resolved`/`inst_enums`) carries the identical
  three-vector requirement.
- **`Len` inference in the array arm (N3).** It is tempting to mirror `apply_subst`'s
  array arm wholesale, which handles `Len::Var`. Do not: a struct header binds no length
  variable, so a `Len::Var` in a field is unconstructible and its arm would be untestable
  dead code.
- **R10's assertion is about text, not success.** A test asserting only "this fails" would
  pass unchanged before the fix. It must assert the message changed from
  `unknown type` to `a reference cannot be stored`.

## Phase split

The brief's two-phase split is adopted unchanged. The load-bearing claim behind it,
re-verified with one narrowing: **phase 1 cannot trigger R6's *substitution-recursion*
hazard**, specifically. A field naming a `Generic` application with *variable* arguments
only reaches `instantiate_struct`/`instantiate_enum` once a concrete instantiation is
requested, which is phase 2's `substitute_generic_field` `Generic` arm; phase 1 builds the
`PolyType` tree and never calls the instantiator for that shape. But phase 1 *does* have
to handle a narrower, related hazard: the ordinary concrete-generic-application path
through `resolve_type_or_apply` is pre-existing and reachable from phase 1 (a fully
concrete self-reference like `L[i64]`, R2's own success criterion, goes through exactly
this path today) -- what's new in phase 1 is not the path itself but the fact that R2's
header placeholder makes that pre-existing path's target sometimes still-registering, which
is why R2 above carries its own `header_pending`-aware deferral mechanism, scoped
narrowly enough that it does not need R4's `Generic`-substitution arm to exist. So the
split still does not ship a *hang* as an intermediate state (R6's substitution-recursion
hazard, the one that can hang, genuinely stays phase-2-only), but phase 1 is not
recursion-free in the looser sense a prior draft of this section implied -- it is free of
the *specific* hazard R6 exists to fix, not of every placeholder-related concern R2
introduces.

The one thing that *must not* be split across the two phases is R3's variant plus its
exhaustive-match audit: introducing the variant is a hard build error until every match is
updated, so the crate does not compile in between. Both land in phase 1's commit.

R8's rejection lands in **phase 1**, not phase 2, despite being a recursion concern: it is
a purely structural check on the `args` tree R1's descent has just built, at parse time,
with no instantiation involved. Putting it in phase 1 also means phase 2 never has to
handle a growing shape at all — R6's memo ordering only has to be correct for shapes R8
already admits.

## Phase 1 — Recursive field-type parser, header self-registration, the owned-cell variant

**Scope (modify):**

- `src/parser.rs`: rewrite `parse_generic_field_type_expr` with recursive array / ref /
  owned-cell / generic-application arms resolving leaf `'name`s against `ty_vars`
  (R1); R7's `quotation_type_ahead()` replication and located rejection; R8's structural
  growth rejection over a `Generic` arm's `args` plus its new diagnostic; R2's placeholder
  registration in `parse_generic_typedefs`/`parse_generic_typedef`/
  `parse_generic_enum_typedef`; R3's `RawTy` variant, `raw_to_poly_type` fold, and
  `parse_poly_slot` `^`-arm.
- `src/ast.rs`: R3's `PolyType` owned-cell variant; the `header_pending: Vec<bool>` flag
  and the pending-instantiation worklist `instantiate_struct`/`instantiate_enum` need to
  defer field computation against a still-registering header (R2's second part -- see
  "R2" above). This does not require `MutRegistries`: it reuses today's `NameRegistries`
  signature unchanged, since it only changes *when* `substitute_generic_field` runs, not
  what it is passed.
- Every file the R3 audit reaches (eleven, per Open risks).
- `src/check/poly.rs`: `apply_subst`'s owned-cell arm (part of the audit — the word
  signature path must be total in the same commit).

**Phase 1's `apply_subst` cells threading (resolves the phase-1/phase-2 contradiction a
prior draft left in place).** `apply_subst` (`check/poly.rs:4390`) takes `arrays: &mut
Vec<ArrayDecl>, refs: &mut Vec<RefDecl>` today -- no `cells` parameter. R3's new
owned-cell arm, needed in phase 1 to make `: idc ( ^'T -- ^'T ) ;` build (a `[P1]` success
criterion), must call `intern_owned_cell_type`, exactly mirroring the existing `Ref` arm's
`intern_ref_type` call -- which needs a live `&mut Vec<OwnedCellDecl>` in scope, not the
immutable `&[]` its pre-existing `Generic` arm throws away today. This is decided here as
option (a) from the two the delivery brief poses: **thread a plain `cells: &mut
Vec<OwnedCellDecl>` parameter through `apply_subst`**, used directly by its own new arm --
not the phase-2 `MutRegistries` struct (which is reserved for the mutually-recursive
`substitute_generic_field`/`instantiate_struct`/`instantiate_enum` pair and is out of
phase 1's scope). This is not premature plumbing: the parameter is consumed immediately by
the arm phase 1 itself adds, at its first and only use. Every non-test caller of
`apply_subst` needs the new argument threaded through -- the full list, verified against
live source rather than the shorter one a prior draft carried: `check.rs:1669`/`:1673`
(`back_edge_declared_shape`, which itself has no `cells` param today and needs one threaded
to its own two callers in turn, `check/combinators.rs:488` and `check/engine.rs:2057`),
`check/combinators.rs:654`, `check/poly.rs:382`/`:387`/`:3939`, and `apply_subst`'s own five
(not four) recursive self-calls inside `check/poly.rs`. All already thread `arrays`/`refs`
the same way, so this is a mechanical widening of an existing parameter list at every site,
not a new call shape. While `cells` is now a
live parameter in scope, `apply_subst`'s pre-existing `Generic` arm should also stop
passing `cells: &[]`/`refs: &[]` to the `NameRegistries` it builds for its
`instantiate_struct`/`instantiate_enum` call (a one-line correctness fix, natural now that
`cells` is already threaded, and the exact degradation R5's Open Risks note warns against
copying blindly) -- option (b), moving the owned-cell arm itself to phase 2, was rejected
because it would also displace the `idc`-builds exit criterion out of phase 1, which the
rest of this document treats as load-bearing to phase 1's own definition of done.

**Out of bounds:** `substitute_generic_field`'s new arms, the `MutRegistries` container,
the R6 mint-and-memo-before-substitute reordering for the *substitution-recursion* case
(a fully-registered header re-entered mid-substitution), the `Map` regression test, R10's
diagnostic-quality criterion (all phase 2). `substitute_generic_field` keeps its
`unreachable!` through phase 1 — a *concrete instantiation* of a variable-wrapping field
still panics at the end of phase 1, which is why phase 1's tests are parser-level and
assert the `PolyType` tree, not a build. R2's own narrower, header-pending-aware deferral
mechanism (above) is **in** phase 1's scope -- it is a prerequisite for R2's own
success criterion, not a piece of R6's phase-2 work.

**Entry conditions:** green `main`.

**Exit criteria:**

- Each of the five field shapes in "Success criteria `[P1]`" parses to the stated
  `PolyType` tree.
- `type: L 'T next L[i64] ;` resolves its self-reference (R2), and a duplicate generic
  header is still rejected.
- `: idc ( ^'T -- ^'T ) ;` builds (R3, word-signature side).
- A growing generic argument is a located parse-time rejection naming the restriction
  (R8).
- A variable-quotation field is a located rejection; a concrete quotation field still
  builds (R7).
- `cargo build` is clean with no `PolyType`/`RawTy` match reaching a newly-added
  wildcard.

**Golden test plan (parser-level unit tests beside `parse_generic_field_type_expr`, plus
`tests/phase7_slice3n.rs` for the build-level ones):**

- `parse_generic_field_array_of_ty_var_builds_array_polytype`
- `parse_generic_field_nested_array_of_ty_var_builds_nested_polytype`
- `parse_generic_field_owned_cell_of_ty_var_builds_cell_polytype`
- `parse_generic_field_ref_of_ty_var_builds_ref_polytype`
- `parse_generic_field_generic_application_of_ty_vars_builds_generic_polytype`
- `parse_generic_field_generic_application_mixed_args_builds_mixed_polytype`
- `parse_generic_field_unbound_ty_var_inside_array_is_error` — the leaf error still names
  the declaration, not `PolyBuilder`'s wording.
- `parse_generic_field_ty_var_used_only_inside_array_is_not_phantom` — N4's `used` bitmap
  claim, which would otherwise be an untested assertion.
- `parse_generic_enum_variant_named_field_array_of_ty_var_builds_array_polytype` — the
  enum twin, which shares the fix and must not be assumed.
- `parse_generic_typedef_concrete_self_reference_resolves` (R2) and
  `parse_generic_typedef_duplicate_header_still_rejected_after_self_registration`.
- `parse_generic_field_growing_generic_argument_is_error` (R8), plus
  `parse_generic_field_concrete_nested_generic_argument_is_ok` — the accept side of R8's
  "concrete at any depth" clause, without which the rule's shallow misreading passes too.
- `parse_generic_field_variable_quotation_is_error` and
  `parse_generic_field_concrete_quotation_still_parses` (R7).
- `parse_poly_slot_owned_cell_of_ty_var_builds_cell_rawty` and a build-level
  `owned_cell_type_variable_in_word_signature_builds` (R3).

**Mutation-test the R7 and R8 guards specifically.** Both are rejections whose fixtures
could be rejected by an *earlier* production instead (R7's by the array parser choking on
`--`; R8's by the leaf `'name` lookup) — the classic placebo shape, and Sooth has shipped
five. Delete each guard and confirm its test fails with a different message, not that it
still fails.

**Difficulty:** hard (a new recursive production plus a whole-crate variant audit).

## Phase 2 — Substitution, registry threading, instantiation ordering, growth-free recursion

**Scope (modify):**

- `src/ast.rs`: `substitute_generic_field`'s `Array`/`Ref`/`Generic`/owned-cell arms (R4),
  taking `MutRegistries` and `&mut GenericTypes` — i.e. it becomes a method on
  `GenericTypes` rather than a free function, since its `Generic` arm re-enters the
  instantiator; `MutRegistries` itself (R5); `instantiate_struct`/`instantiate_enum`'s
  mint-and-memo-before-substitute reordering with a placeholder decl (R6).
- `src/parser.rs`, `src/check/poly.rs`: the eight call sites and the `parser.rs:3948-4000`
  borrow restructuring (R5).
- Tests only, for R9/R10 and the pinned out-of-scope gap.

**Out of bounds:** any parser change (phase 1); `check_recursion`'s logic; the
no-stored-reference rule; any length-variable work.

**Entry conditions:** phase 1 merged and green.

**Exit criteria:** every `[P2]` line in "Success criteria", plus: no field shape phase 1
admits reaches `substitute_generic_field`'s `unreachable!` (N1).

**Golden test plan (`tests/phase7_slice3n.rs`, plus unit tests beside
`instantiate_struct`):**

- `map_shaped_backing_storage_instantiates_at_two_key_value_pairs` — the headline
  criterion. Asserts `StructDecl.fields` types at each instantiation **and** two distinct
  `StructId`s. Instantiate **asymmetrically** (`('K, 'V) = (i64, str)` and `(str, i64)`),
  per the symmetric-instantiation placebo precedent: a symmetric pair cannot tell a
  correct substitution from one that swaps the arguments.
- `array_of_ty_var_field_instantiates_to_concrete_array`
- `nested_array_of_ty_var_field_instantiates_to_nested_array`
- `owned_cell_of_ty_var_field_instantiates_to_concrete_cell`
- `ref_of_ty_var_field_is_rejected_as_stored_reference` (R10) — asserts the message is
  `a reference cannot be stored` **and** that it is no longer `unknown type`. Cover the
  *composite* referent too (`&Ent['K i64]`, a `Ref` over a `Generic`, which phase 1 admits
  in both its glued and spaced spellings): today it panics at `substitute_generic_field`
  like the bare `&'T`, so R4's `Ref` arm must recurse into its `Generic` arm rather than
  each arm only handling a variable payload. A `&'T`-only fixture cannot witness that.
- `by_value_generic_self_reference_is_infinite_size_error` and
  `array_wrapped_generic_self_reference_is_infinite_size_error` (R9).
- `cell_wrapped_generic_self_reference_builds_and_terminates` — the R6 memo-ordering
  witness. It must be a *build*, run under the ordinary test timeout: a hang here is the
  failure mode, so the test is the termination witness.
- `mutual_cell_wrapped_generic_self_reference_terminates` — `A` → `^B['T]` → `^A['T]`,
  which the single-header test does not cover (the memo key includes `idx`).
- `permuting_generic_self_reference_terminates` — `type: A 'K 'V next ^A['V 'K] ;`, the
  case that distinguishes R8's rule from a blanket self-reference ban.
- `attributeless_variant_array_field_is_still_a_parse_error` — the pinned out-of-scope
  gap, concrete fixture (`type: Foo | Some [i64 2] | None ;`), asserting today's
  `expected a word, found LBracket`.
- Unit test beside `instantiate_struct`:
  `instantiate_struct_pushes_memo_key_before_substituting_fields` — R6's ordering asserted
  directly, so a later refactor that restores the old order fails here and not only via
  the hang.

**Mutation-test R6's ordering and the `Map` test's distinct-`StructId` assertion.**
Reverting the ordering must fail the cell-cycle tests; collapsing the two instantiations
onto one `StructId` must fail the `Map` test. **The failure mode is an abort, not a
hang** -- measured, not assumed (see "Exit findings"): the recursion is a call chain, so
reverting the ordering overflows the stack and the process dies with `fatal runtime
error: stack overflow, aborting`. No timeout machinery is needed, and none would work: a
watchdog thread joined with `recv_timeout` goes down with the process it is watching.
What this *does* cost is classification -- an aborted binary prints no terminal `test
result:` line at all, so the repo's usual `test result: FAILED` classifier scores the
mutation SURVIVED. Classify these two on the abort string or the runner's exit code
instead. Note the mutation-testing hygiene the repo has been bitten by: commit first,
copy `examples/` into the scratch tree, and never `cp -r` the worktree.

**Difficulty:** hard (mutual recursion between substitution and instantiation, a borrow
restructuring on a shared path, and a compiler stack overflow as the failure mode).

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "Parser: recursive generic struct/enum field-type descent (array, ref, owned-cell, generic-application arms over the header's own type variables), header placeholder self-registration before field parsing, the new PolyType/RawTy owned-cell variant with its whole-crate exhaustive-match audit and parse_poly_slot ^-arm, the located variable-quotation-field rejection, and the declaration-time growing-generic-argument rejection; parser-level tests only, no substitution change", "effort": "L", "difficulty": "hard" },
    { "phase": 2, "focus": "substitute_generic_field's Array/Ref/Generic/owned-cell arms as a GenericTypes method (with the instantiate_enum borrow fix mirroring instantiate_struct's), the MutRegistries container threaded through instantiate_struct/instantiate_enum and their eight call sites including the resolve_type_or_apply borrow restructuring, the mint-and-memo-before-substitute ordering fix (both struct and enum) enabling cell-wrapped self-reference, plus the Map-shaped two-instantiation regression test, the R9 infinite-size and R10 stored-reference diagnostic criteria, the pinned attributeless-variant gap, and plain (un-wrapped) termination-witness tests for the cell-cycle goldens -- bumped from the brief's \"M\" to \"L\": mutual recursion between substitution and instantiation, an 8+ call-site borrow restructuring, and the memo/vector reorder across three parallel vectors for both structs and enums", "effort": "L", "difficulty": "hard" }
  ]
}
```

## Exit findings (confirmed at implementation)

- **The termination failure mode is an abort, not a hang.** The spec planned timeout
  machinery around the cell-cycle goldens on the assumption that an ordering regression
  hangs. Measured by reverting R6's ordering: it does not hang, it overflows the
  compiler's own stack and aborts (`fatal runtime error: stack overflow`). The machinery
  was therefore not built, and could not have worked if it had been -- a watchdog thread
  joined with `recv_timeout` dies with the process it watches. The residual cost is
  classification only: an aborted test binary emits no terminal `test result:` line, so a
  `test result: FAILED` classifier scores such a mutation SURVIVED. Both R6 mutations
  (struct and enum halves) were classified on the abort string instead, and both are
  caught.
- **N1 holds, probed rather than assumed.** No field shape phase 1 admits reaches
  `substitute_generic_field`'s `unreachable!`. The one shape that could -- a quotation
  naming a type variable -- is rejected by R7 under every wrapper tried: array, owned
  cell, ref, nested array, generic argument and enum variant.
- **R8's growth rejection is uniform across headers and kinds.** Beyond the self-naming
  case the goldens cover, cross-header growth (`A['T]` whose field names `^B[^'T]`) and
  the enum-side equivalent are both rejected; struct-to-enum and enum-to-struct mutual
  cell cycles terminate in *either* declaration order. No test covers cross-kind cycles;
  they were probed directly.
- **`MutRegistries::names` over the live registries is load-bearing, not defensive.**
  `type_arg_key` (`src/ast.rs:682`) renders a `Struct`/`Enum` argument from the carried
  `name` but *indexes* `refs`/`cells`/`arrays` by id, so the throwaway `cells: &[]` /
  `refs: &[]` view an earlier caller built renders a cell- or ref-payload argument wrong,
  or panics outright, as soon as one exists to look up.
- **`src/ast.rs` split signals, re-run (CLAUDE.md's phase-exit check).** The file is ~3,990
  lines and this phase grew it by ~420. **3 of 5 signals fire.** *Not* firing: import
  divergence -- the file has **zero** top-level `use` statements, every `std` path is
  written fully qualified, so there are no imports to diverge; and there is no would-be
  circular dependency forcing a split (the generics section never names `Module`, while
  `Module` holds `GenericTypes`, so the dependency runs one way only). Firing: three
  responsibilities in one module (AST/type definitions, the concrete interning registries,
  generic substitution/instantiation); high- and low-level code mixed (plain data
  declarations beside a mutually recursive substituter/instantiator carrying memo-ordering
  invariants); and functions that never call each other (`alpha_rename_locals`/
  `rename_terms` and `seed_predicate_traits` never touch the generics machinery).
  **Decision: defer, but for a weaker reason than `poly.rs`'s.** Unlike `poly.rs`, where
  both candidate splits were judged actively wrong, the `ast/generics.rs` split here is
  available and clean: `GenericStructDecl`/`GenericEnumDecl`/`GenericVariantDecl`/
  `GenericTypes`/`PolyType`/`NameRegistries`/`MutRegistries`/`type_arg_key`/
  `type_instantiation_name`/`generic_surface_name` move as one unit and depend only
  downward. It is deferred because it is pure code motion well outside this phase's
  declared scope, not because it is the wrong cut. **Recommended as its own change**,
  ahead of P7.S3e, which will grow `GenericTypes` again.

### Recommendations for later work (not this slice)

- **The roadmap's S3n entry is closed out here**
  (`docs/roadmap/P7-language-prereqs.md`); its "rejects with `unknown type 'T`" claim and
  its "not yet recon'd" paragraph were both falsified by this slice.
- **`growing_generic_self_reference_error` renders a doubled `error:` prefix.** The
  message bakes in `error:` and `main.rs` re-prefixes. Phase 1's message joining a
  pre-existing, unowned set across the crate; a sweep, not a drive-by fix here.
