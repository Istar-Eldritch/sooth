# P7.S3n brief — A generic struct's array field cannot be its own type variable

## Problem, confirmed live against current `main` (`5338c06`)

`parse_generic_field_type_expr` (`src/parser.rs:4124`) is the only production a generic
`type:`'s field list resolves through. It checks *one* thing: is the token sitting right here
a bare `'`-prefixed word matching one of the header's bound variables? If so, `PolyType::Var`.
Otherwise it falls straight through to `parse_field_type_expr` (`parser.rs:3502`), the
*concrete-only* field-type parser — the same one an ordinary, non-generic `type:` uses — which
resolves through `resolve_type_or_apply` (`parser.rs:3948`) and has no notion of the enclosing
struct's bound variables at all. A bare scalar field (`v 'T`) works because the variable sits
in exactly that one checked position. Wrap it in *anything* — an array, a reference, an owned
cell, or a nested generic application — and the variable is one token deeper than the single
position the check inspects, so it falls through and dies as an ordinary unknown name. Five
shapes, all live-probed against `main`:

```
type: Pair 'T items ['T 2] ;        \ error: unknown type `'T` at line 3, col 22
type: Box 'T r &'T ;                \ error: unknown type `'T` at line 3, col 17
type: Cell 'T c ^'T ;               \ error: unknown type `'T` at line 3, col 18
type: Wrap 'K 'V e Ent['K 'V] ;     \ error: unknown type `'K` at line 4, col 24
type: NestArr 'T grid [['T 2] 2] ;  \ error: unknown type `'T` at line 3, col 25
```

The exact `Map` shape named at S3e's brief also fails, confirmed directly:

```
type: Ent 'K 'V k 'K v 'V ;
type: Map 'K 'V slots [Ent['K 'V] 8] ;   \ error: unknown type `'K` at line 4, col 28
```

(This is a stand-in probe for the shape `Map`'s backing storage needs, not the real `Map`
header — see "Honest framing of the `Map` consumer" below for why a literal `8` here is not
what `Map` actually requires.)

**The struct-header case is genuinely narrower than the word-signature case, not the same
gap wearing a different hat.** A word's declared effect resolves every slot through
`parse_poly_slot` (`parser.rs:2421`), which *recursively* descends into `[`, `&`, and a
following generic application at any depth — probe-confirmed working today:
`: id2 ( ['T 2] -- ['T 2] ) ;` builds clean. `parse_generic_field_type_expr` has no recursive
counterpart; it is a single `if`-then-fallthrough with no depth at all. This is the root cause
`docs/roadmap/P7-language-prereqs.md:606-613` flagged as unconfirmed, naming
`resolve_type_or_apply`/`parse_type_arguments` (`src/parser.rs:3129`) as the plausible cause —
partially confirmed here: the field parser never recurses, but `resolve_type_or_apply` itself
is not the resolver at fault. `docs/roadmap/P7/slice3a-brief.md:25` cites the very same
function, `resolve_type_or_apply` (`src/parser.rs:3129-3172` at the time slice3a was written;
line numbers have since drifted, and today's `:3129` happens to sit inside
`parse_array_type_expr` instead — coincidence, not a second citation target), for S3a's
word-signature root cause. So the roadmap's hedge names the *same* function both times, not a
distinct one. This brief's own probes show `resolve_type_or_apply` working correctly for every
field case it touches (a concrete generic-application field resolves through it today,
unaffected — see "Recursive `instantiate_struct`/`instantiate_enum`" below); the actual root
cause sits one layer earlier and does not implicate `resolve_type_or_apply` at all — a generic
struct/enum field list is parsed by `parse_generic_field_type_expr` (`parser.rs:4124`), which
does fall through to `resolve_type_or_apply` for its fully-concrete case (via
`parse_field_type_expr`, `parser.rs:3519`) — so the two paths are not wholly code-disjoint —
but never *recurses* into it for a variable-bearing shape, which is the one thing missing;
"the resolver both paths share is broken" was never on the table, since the shared resolver
works correctly for every case it is actually asked to handle.

**`parse_poly_slot` does not cover `^` either — this is not a fully solved precedent to
mirror, it is a partially solved one.** Probe: `: idc ( ^'T -- ^'T ) ;` (a *word signature*,
not a struct field) fails with `` error: unknown type `'T` `` at column 10. `parse_poly_slot`
(`parser.rs:2421-2511`) has arms for `~[`/`[` (array/quotation), a bare `'`-var, a `&`-led ref,
and a following generic application — no `^` arm. `^` appears only as a negative exclusion
guard in the generic-application arm (`parser.rs:2492`). So `Cell 'T c ^'T` is not "wrap the
struct-field fix around an existing mechanism" the way array/ref/nested-generic are — it needs
a **new capability that does not exist anywhere in the compiler today**, on both the word-
signature and the struct-field paths at once. See "Owned-cell type variables: new work, not a
mirror" below.

## Honest framing of the `Map` consumer

This slice **removes one of several blockers** on `Map['K 'V]`'s backing storage, not "unblocks
`Map`" outright. `docs/roadmap/P7/slice3-dogfood.md` (Program 1) writes the real shape as
`type: Map 'K 'V 'N` / `slots [Entry['K 'V] 'N]` — a **struct-header length variable** `'N`
for capacity, which this slice does not add (a generic struct header today binds only
`'`-prefixed type variables, `parse_generic_header_vars`, `parser.rs:4102`; there is no
length-variable table analogous to a word's `PolySig::len_var_names`). The same dogfood names
two further gaps this slice does not touch:

- Friction #7 (`slice3-dogfood.md:173-174`): "`type:` header support for a length-var parameter
  is unverified" — the exact `'N` gap above.
- Friction #6 (`slice3-dogfood.md:164-169`): the backing array is not constructible without a
  third bound (`Default`-style), on top of the `Eq`/`Hash` bound on `'K` that
  `docs/roadmap/P7-language-prereqs.md:251` already names as S3e's own open gap.

So this slice's exit probe (`Map 'K 'V slots [Ent['K 'V] 8]`, a fixed literal `8`) is a
stand-in shape proving the *field-type* mechanism works, not a claim that `Map` is buildable
once this slice lands. Do not cite this slice as removing the `Map` blocker; cite it as
removing the array-field-of-own-type-variable blocker specifically, one of at least three
`Map` needs.

## Existing precedent (what's already there to build on)

**Most of the target representation already exists and needs no new variant.** `PolyType`
(`ast.rs:1436`) already has `Array(Box<PolyType>, Len)`, `Ref(Box<PolyType>, bool)`, and
`Generic { is_enum, idx, module, args: Vec<PolyType>, name }` — minted today only by
`raw_to_poly_type`'s fold of `RawTy` (`parser.rs:1175-1209`), which `parse_poly_slot` builds,
for word signatures. Nothing new needs inventing at the type level for array/ref/nested-generic;
the gap there is purely that a struct field never gets a chance to produce these shapes, and
nothing consumes them once instantiation is reached. Owned-cell (`^'T`) is the one shape this
does not cover — see below.

**`Len::Var` cannot occur in a struct field and the fix must not add a path for it.** A generic
struct header (`parse_generic_header_vars`, `parser.rs:4102`) binds only `'`-prefixed type
variables; there is no length-variable table analogous to a `PolySig::len_var_names` for a
`GenericStructDecl`/`GenericEnumDecl`. `[Ent['K 'V] 8]`'s `8` is always a concrete literal.
`substitute_generic_field`'s array arm therefore only ever needs `Len::Concrete`, never
`Len::Var` — unlike `apply_subst`'s (`check/poly.rs:4389`) word-signature twin, which handles
both. Do not widen struct headers to accept a length variable as part of this slice; that is
a different, unrequested capability (and the one `Map` itself is separately blocked on, above).

**A second, independent layer is broken even for the shapes that parse correctly.**
`substitute_generic_field` (`ast.rs:683`), called from `instantiate_struct` (`ast.rs:817`) and
`instantiate_enum` (`ast.rs:857`, confirmed — both call the same function, at `ast.rs:832` and
`ast.rs:882` respectively, not separate copies) once per field at instantiation time, is:

```rust
fn substitute_generic_field(pty: &PolyType, args: &[Type]) -> Type {
    match pty {
        PolyType::Concrete(t) => *t,
        PolyType::Var(v) => args[*v as usize],
        other => unreachable!("a generic `type:` field is never {other:?}"),
    }
}
```

Even a correctly-parsed `PolyType::Array(Box::new(PolyType::Var(0)), Len::Concrete(2))` panics
here today (`unreachable!`) — this function has never had to handle anything but the two shapes
the current parser can produce. This is not dead code to delete; it is the second half of the
fix, and `apply_subst` (`check/poly.rs:4389-4397`) is the exact template for the missing arms —
its `Array`, `Ref`, and `Generic` cases already do, for a word's `Subst`, precisely what a fixed
`substitute_generic_field` needs to do for a struct's `args: &[Type]`.

**Registry threading must not go through `NameRegistries` — that type is immutable and cannot
intern anything.** `NameRegistries` (`ast.rs:579-585`) is `#[derive(Clone, Copy)]` over
*immutable* slices (`arrays: &'a [ArrayDecl]`, `refs: &'a [RefDecl]`, `cells: &'a
[OwnedCellDecl]`). `intern_array_type` (`ast.rs:1141`), `intern_ref_type` (`ast.rs:1044`), and
`intern_owned_cell_type` (`ast.rs:1000`) all require `&mut Vec<...>`. `apply_subst` — the exact
template this brief points to above — does **not** thread `NameRegistries` for this: it takes
`arrays: &mut Vec<ArrayDecl>, refs: &mut Vec<RefDecl>` as **separate mutable parameters**
(`check/poly.rs:4389-4397`) and only constructs a throwaway, read-only `regs` at the last moment
for its `Generic` arm. `substitute_generic_field` must do the same — take separate `&mut
Vec<ArrayDecl>`, `&mut Vec<RefDecl>`, `&mut Vec<OwnedCellDecl>` parameters, not a
`NameRegistries`.

This ripples through every call site that currently passes an immutable `NameRegistries` where
a fixed `substitute_generic_field` needs mutable vecs instead:
`instantiate_struct`/`instantiate_enum` themselves (`ast.rs:817`, `ast.rs:857`) and their
callers: `parser.rs:2940`/`2943` (struct-name-collision-avoidance path), `parser.rs:3983`/
`3997` (the ordinary `resolve_type_or_apply` generic-application arm), `check/poly.rs:3092`/
`3094`, and `check/poly.rs:4482`/`4484`. One of these is a live hazard, not just a signature
change: at `parser.rs:3948-4000` (inside `resolve_type_or_apply`), the `regs` value passed to
`instantiate_struct`/`instantiate_enum` today is built by *immutably* borrowing
`self.arrays`/`self.refs`/`self.owned_cells` — the very `Vec`s a fixed call would need to
*mutably* reborrow in the same statement. This needs actual restructuring (e.g. taking the
vecs out via `std::mem::take` before the call and putting them back after, or reordering which
borrow happens first), not just widening a function signature.

**Recursive `instantiate_struct`/`instantiate_enum` is a second design problem, distinct from
registry threading, and is only reachable once nested-`Generic` field substitution exists.**
`instantiate_struct` (`ast.rs:817-849`) builds the substituted field list (`ast.rs:830-833`,
holding a live immutable borrow of `self.structs[idx]`), and only *after* that mints the
`Type::Struct` id and pushes the `(idx, module, args)` memo key (`ast.rs:836-838`). Once
`substitute_generic_field` grows a `Generic` arm that recursively calls
`instantiate_struct`/`instantiate_enum` to ground a nested self-reference (e.g. a field of type
`L['T]` inside `L`'s own declaration), that recursive call happens **before** the memo key for
the current, in-progress instantiation is ever pushed — so a same-argument self-reference
would recurse without ever finding a memo hit, and would additionally need `&mut self` while
`decl.fields`'s borrow is still live (won't compile as written).

**The stated reason this is unreachable today is wrong; the real blocker is a header
registration-ordering bug, confirmed by probe with a fully concrete argument.** `type: L 'T
next L['T] ;` does fail today with a plain `` unknown type `L` ``, but so does the fully
concrete variant `type: L 'T next L[i64] ;` — an argument with no type variable in it at all,
requiring none of this slice's recursive field parser. The self-reference is unresolvable for
any argument, concrete or variable, because `parse_generic_typedefs` (`parser.rs:3867-3884`)
calls `parse_generic_typedef`/`parse_generic_enum_typedef` (`parser.rs:3703`, `3743`) to parse
a header's *entire* field/variant list first, and only pushes the resulting decl onto
`self.generics.structs`/`.enums` (`parser.rs:3882`, `3879`) after that call returns — so a
self-referential field, anywhere inside the declaration's own field list, is parsed before the
declaration it names has a header entry `find_struct`/`find_enum` (`ast.rs:793`, `800`) or
`poly_generic_header` (`parser.rs:4010`) can find. This is a **required phase-1 fix**, not an incidental one: it
gates every self-referential case (2a and 2b alike, `^`-wrapped or `&`-wrapped) regardless of
whether the recursive field parser or `substitute_generic_field` are fixed at all. The
mechanism: register the header — name, `ty_var_names` (already fully parsed by
`parse_generic_header_vars` before field parsing starts), span, and module, everything
`find_struct`/`find_enum`/`poly_generic_header` need — as a placeholder decl with an empty
fields/variants list *before* parsing the field list, then fill in the real fields/variants in
place once parsing completes, rather than constructing and pushing the whole decl in one shot
after the fact. This is the phase-1 fix that makes a self-reference resolvable to a type *at
all*; it is independent of, and a prerequisite for, the phase-2 recursive-instantiation memo-
ordering problem below, which only bites once a self-referential field can be named and reaches
`substitute_generic_field`'s `Generic` arm. See "Self-referential generic structs" below for
the ruling on what happens once that reference resolves.

## Self-referential generic structs — two distinct rules

**(a) A by-value (or array-wrapped) self-reference is unconditionally illegal, at any type
arguments, growing or not — and this rule already exists and already works, once instantiation
can reach it.** `check_recursion` (`src/check/declarations.rs:1645`, DFS documented at `:1619-1624` and
implemented around `:1692-1712` via `type_node`) already walks the field graph of every
concrete `StructDecl`/`EnumDecl`/`ArrayDecl` post-instantiation and rejects a by-value cycle as
`` error: recursive struct definition (infinite size) `` — probe-confirmed still firing today
for the non-generic case (`type: Loop next Loop ;`, `check_recursion_by_value_self_cycle_is_error`,
`declarations.rs:3282`) and for the array-wrapped case
(`check_value_recursion_through_array_element_is_error`, `declarations.rs:3201`, `type: Node
kids [Node 4] ;`). `type_node` treats `Type::Array` as a non-breaking edge (an array's size is
`element_size * N`, so it doesn't break the cycle) and deliberately excludes `Type::OwnedCell`
as the one edge kind that *does* break it (a `^T` field is a heap pointer, not an inline copy).
This is exactly the mechanism decision 2(a) needs — **it is not new work**, it just needs to be
reachable: today it can't fire on a *generic* self-reference at all, because
`instantiate_struct`/`instantiate_enum` would infinitely recurse (or fail to parse, per above)
before ever producing the concrete `StructDecl` this checker walks. Fixing the recursion
hazard below (2b) is what lets this existing, already-correct check see a generic
self-reference for the first time; no new "infinite size" diagnostic needs inventing.

**(b) A `^`-wrapped (owned-cell) self-reference (`^L['T]`, legal indirection, pointer-sized
regardless of payload) may recurse at compile time, and the approved rule is: allowed if
non-growing, rejected if growing — mirroring `docs/roadmap/P7/slice3k-brief.md`'s "Locked
decision" for word-call recursion exactly.** This targets `^`, not `&`: `&T`/`&!T` can never
occupy a struct field at all, generic or concrete — `check_types`'s no-stored-reference rule
(`src/check/declarations.rs:1170`, run inside `check_types` after `check_recursion`) rejects
even `type: Box r &i64 ;`, a fully concrete, non-generic struct with no type variables
anywhere, probe-confirmed live against `main` today (`` error: a reference cannot be stored:
field `r` of type `Box__m0` has type `&i64` ``). So `&'T` was never a candidate indirection for
breaking a self-referential cycle; `^'T` is the only field-storable indirection that exists,
and per "Owned-cell type variables" below it does not exist yet either, on the word-signature
or the struct-field path. Probe-confirmed as the right non-generic precedent: `type: LC next
^LC ;` builds clean today (`check/declarations.rs`'s `check_recursion_cell_cycle_in_struct_field_
is_ok` already pins the non-generic case at `declarations.rs:3323`) — `type_node`
(`declarations.rs:1692-1712`) deliberately excludes `Type::OwnedCell` as the one edge kind that
breaks a by-value cycle (a `^T` field is a heap pointer, not an inline copy), and this is the
mechanism 2(b) needs once `^'T` can be named as a struct field at all. The mechanism: fix
`instantiate_struct`/`instantiate_enum`'s ordering so the memo key (`struct_keys`/`enum_keys`)
and the minted `Type::Struct`/`Type::Enum` id are recorded **before** field substitution runs
(not after, as today), so a same-argument recursive call — reached only through a `^`, since a
by-value recursive call is caught by 2(a) regardless — hits the memo on its first re-entry and
terminates immediately, returning the already-minted (not-yet-fully-populated) id; the field
list is filled in and the `StructDecl` pushed only once the (now non-recursive, since the memo
already broke the cycle) substitution completes. A **growing** self-reference (each recursive
hop constructs a structurally larger argument — e.g. `L` containing `^L[Box['T]]`, so every
instantiation targets a distinct, ever-larger concrete type and the memo never hits) must be a
**located compile-time rejection**, not a hang or a stack overflow. As with S3k, the precise
structural-growth detection mechanism (a call-stack membership check comparing each new
argument against the ones already in flight, vs. a recursion-depth cap as a fallback) is
**left as an open question for the spec to resolve, not designed here** — S3n's paper design
only establishes that the case must be caught, not how.

**Consequence: owned-cell support is not a separable bonus capability for this slice — it is
the only mechanism that makes any generic self-referential struct/enum possible at all.**
Without `^'T`, a generic linked-list or tree node has no legal way to refer to itself: `&'T`
cannot occupy the field (the no-stored-reference rule, unconditional and pre-existing), and
wrapping in an array does not break the cycle either (`type_node` treats `Type::Array` as a
non-breaking edge, same as today). So 2(b), and by extension any self-referential generic
struct/enum exit criterion, is gated on "Owned-cell type variables" below landing in the same
slice, not as an independent stretch goal.

## Owned-cell type variables: new work, not a mirror

Unlike array/ref/nested-generic, `^'T` requires genuinely new machinery on **both** the
word-signature and struct-field paths, confirmed by probe (`: idc ( ^'T -- ^'T ) ;` fails
today):

- `PolyType` (`ast.rs:1436-1497`) needs a new variant, mirroring `Ref(Box<PolyType>, bool)`'s
  shape (no id minted until the payload grounds to a concrete `Type` — the same reasoning
  `Ref` documents: the payload may be a variable, which no registry entry can name yet).
- `RawTy` (`parser.rs:1175-1209`) needs the matching variant.
- `parse_poly_slot` (`parser.rs:2421-2511`) needs a genuinely new `^`-led arm — there is nothing
  to copy here, since no such arm exists today on the word-signature path either. Adding it
  fixes `: idc ( ^'T -- ^'T ) ;` as a side effect, which this brief notes honestly as a bonus
  discovered during recon, not as evidence the mechanism already works.
- `apply_subst` (`check/poly.rs:4389-4460`) needs a new arm mirroring its `Ref` arm.
- `substitute_generic_field` needs a matching arm for the struct-field side.
- **Every other exhaustive `match` over `PolyType` or `RawTy` needs a new arm or an explicit
  wildcard, and Rust's own exhaustiveness check will find every one once the variant is added**
  — do not hand-enumerate them speculatively. Confirmed by grep: `PolyType::` is referenced in
  `src/ast.rs`, `src/check.rs`, `src/check/audits.rs`, `src/check/combinators.rs`,
  `src/check/declarations.rs`, `src/check/engine.rs`, `src/check/poly.rs`,
  `src/check/word_entry.rs`, `src/ir/driver.rs`, `src/parser.rs`, and `src/repl.rs` — eleven
  files (401 total
  references, most of them constructors rather than matches); `RawTy::` only in `src/parser.rs`.
  The compiler will not silently miss a site — a new variant makes every non-wildcard match a
  hard build error until handled — but this means the real per-file audit is implementation
  work, not brief work, and Sizing below counts it as its own line item rather than folding it
  into the array/ref/generic work.

Given this, `Cell 'T c ^'T` is honestly scoped as **new capability**, not "wrap the struct-field
fix around an existing mechanism" the way array/ref/nested-generic are.

## Quotation-typed generic fields: explicitly out of scope, with a located rejection

A concrete quotation-typed struct field is **legal today**: `type: Q f [ i64 -- i64 ] ;` builds
clean, via `parse_field_type_expr`'s `quotation_type_ahead()` disambiguation
(`parser.rs:3502-3511`) ahead of `parse_array_type_expr`. `type: QF 'T f [ 'T -- 'T ] ;` today
gives a clean located `` error: unknown type `'T` `` at the effect's input slot. Once the
recursive field parser intercepts a leading `[`, it must replicate the same
`quotation_type_ahead()` disambiguation the concrete path already uses, or it will misparse a
concrete quotation field (`[ i64 -- i64 ]`) as a malformed array. **Ruling (made here, to keep
this slice bounded): a quotation-typed field naming the struct's own type variable (`f [ 'T --
'T ]`) is out of scope for this slice**, rejected with a located, worded error — mirroring how
`~[` is already rejected with `a ~ quotation cannot appear here` (`parser.rs:3503`) — not the
`unreachable!` panic `substitute_generic_field` would otherwise hit if a `PolyType::Quotation`
ever reached it from a field. This is a phase-1 deliverable (see Sizing): the recursive field
parser must special-case this to a location rejection, with two pinned tests — today's concrete
quotation field still builds after the fix, and the variable-quotation field is a clean located
error, never a panic.

## Out of scope, confirmed by probe as pre-existing and unrelated

A generic variant's *attributeless* (positional) array field is rejected at parse time even
when fully concrete: `type: Foo | Some [i64 2] | None ;` (no type variables anywhere) fails
with `` parse error: expected a word, found LBracket `` — through the **non-generic** variant-
field parser (this concrete repro has no generic header at all, so it never reaches
`parse_generic_variant_fields`; the equivalent named-field position in the **generic** variant
parser, `parse_generic_variant_fields`, `parser.rs:3795-3841`, has the same positional-field
limitation for its own attributeless arm). This is a distinct, pre-existing parse limitation on
attributeless array fields in general (nothing to do with `'T`), not part of this slice's exit
criteria. Do not fold it in; a *named* field (`Some xs ['T 2]`) is fully in scope and is the
shape `Map`-style structures use anyway.

## Paper-traced design (validated, not yet spec'd)

1. **Give `parse_generic_field_type_expr` a real recursive descent**, mirroring
   `parse_poly_slot`'s intercept arms (array via `[`, `&`-led ref, a genuinely new `^`-led
   owned-cell arm per above, `Name[` generic application) but resolving each leaf `'name`
   against the struct/enum's own `ty_vars: &[(String, Span)]` table (marking `used`) instead of
   building a `RawTy` against a `PolyBuilder`. The concrete fallthrough (`parse_field_type_expr`)
   stays exactly as it is for the fully-concrete case, including its `quotation_type_ahead()`
   disambiguation, which the new `[`-arm must replicate to avoid misparsing a concrete
   quotation field as an array (see above) and to reach the located rejection for a
   variable-quotation field. A `Generic` leaf whose arguments mix concrete and variable pieces
   (`Ent['K i64]`) must produce `PolyType::Generic { args: vec![Var(0), Concrete(i64)], .. }` —
   `apply_subst`'s existing `Generic` arm already handles a mixed `args` list, so this is a
   parser-side construction detail, not a new substitution case.
2. **Extend `substitute_generic_field` with `Array`/`Ref`/`Generic`/owned-cell arms**, taking
   separate `&mut Vec<ArrayDecl>`/`&mut Vec<RefDecl>`/`&mut Vec<OwnedCellDecl>` parameters (not
   a `NameRegistries`, see above) so the array/ref/cell arms can intern their result and the
   `Generic` arm can recursively call `instantiate_struct`/`instantiate_enum` on its own
   substituted `args`. `Quotation`/`QuotLit` reject with the located error from the previous
   section rather than reaching `substitute_generic_field` at all — confirm the field parser's
   rejection actually prevents a `PolyType::Quotation` from ever being constructed for a field,
   so `substitute_generic_field`'s fallback arm can stay `unreachable!` truthfully.
3. **Fix `instantiate_struct`/`instantiate_enum`'s memo ordering** (mint the id and push the
   memo key before substituting fields, not after) and add the growing-recursion rejection —
   both per "Self-referential generic structs" above. This is required before step 2's `Generic`
   arm can safely recurse into `instantiate_struct`/`instantiate_enum` at all.
4. **No change needed to `check_no_phantom_ty_var`** (`parser.rs:1621`): it only reads the
   `used` bitmap, and the new recursive descent marks `used[idx] = true` at every leaf `Var` it
   constructs regardless of nesting depth, so a variable used only inside an array/ref/generic
   argument is already correctly counted as used.
5. **No change needed to `PolyType`'s array/ref/generic variants, `apply_subst`, or the
   word-signature path for those three shapes** — only the owned-cell variant (per above) is
   new at the type level, and it touches the word-signature path too (as a side effect, not a
   scope increase to justify separately).

## Exit criteria (widened from the roadmap's array-only wording, labelled honestly as a widening, not a "sharpening")

`docs/roadmap/P7-language-prereqs.md:598-616`'s own exit line names only the array-field case.
This brief widens that to ref/owned-cell/nested-generic-application/the enum twin because one
fix mechanism (the recursive field-type parser) naturally covers all of them at once — that is
a real widening of what the roadmap literally asked for, not something the roadmap already
implied, and it is called out as such here rather than folded in silently:

- A generic struct (or enum, sharing the fix) may declare a named field whose type is:
  - an array wrapping the enclosing declaration's own type variable (`Pair 'T items ['T 2]`),
  - a nested array-of-arrays (`NestArr 'T grid [['T 2] 2]`),
  - an owned cell (`Cell 'T c ^'T`, new capability, see above),
  - or a nested generic application over it (`Wrap 'K 'V e Ent['K 'V]`),

  each resolved correctly per concrete instantiation.
- A reference field (`Box 'T r &'T`) is **not** a "must build" criterion — it never can build,
  by the existing, unconditional, unrelated no-stored-reference rule
  (`src/check/declarations.rs:1170`), which rejects even a fully concrete, non-generic
  `type: Box r &i64 ;` today. This is instead a **diagnostic-quality** exit criterion: once the
  recursive field-type parser correctly resolves `&'T` to `PolyType::Ref(Box::new(Var(0)),
  false)`, `Box 'T r &'T` at a concrete instantiation is rejected with the *correct*, located
  `` a reference cannot be stored `` error, not today's misleading `` unknown type `'T` ``. A
  test asserts the error text changed from the unknown-type message to the stored-reference
  message, not that the program builds.
- The `Map`-shaped backing-storage stand-in (`Map 'K 'V slots [Ent['K 'V] 8]`) builds, and a
  regression test instantiates it at two different concrete `('K, 'V)` pairs and asserts on the
  resulting `StructDecl.fields` types and on the two instantiations minting **distinct
  `StructId`s** — mirroring the assertion style of `instantiate_struct_distinct_across_modules_
  same_bare_name` (`driver.rs:1066`) and `instantiate_struct_distinct_for_wrapped_cross_module_
  args` (`driver.rs:1128`) — not merely "it builds".
- A by-value or array-wrapped self-referential generic struct/enum is rejected with the
  existing `` recursive struct/enum definition (infinite size) `` diagnostic
  (`check_recursion`), now reachable for a generic self-reference for the first time; a
  `^`-wrapped, non-growing self-reference is allowed and terminates; a `^`-wrapped, growing
  self-reference is a located compile-time rejection, not a hang.
- A struct-header length variable is *not* introduced; `Map`'s real header shape
  (`type: Map 'K 'V 'N`) stays out of scope — this slice removes one of at least three blockers
  on `Map`, not all of them (see "Honest framing of the `Map` consumer" above).
- The attributeless/positional variant array-field parse gap (`Some [i64 2]`, concrete or
  variable) is named as pre-existing and explicitly left unfixed, with its own probe-verified
  test asserting today's rejection is unchanged.
- A quotation-typed generic field (`f [ 'T -- 'T ]`) is explicitly out of scope, rejected with
  a located, worded error (never a panic); a concrete quotation field (`f [ i64 -- i64 ]`)
  still builds after the fix.

## Sizing

Two phases, not one — the registry-threading and recursion-safety work in phase 2 is
substantial enough, and different enough in kind from the parser work, that shipping them
together risks an intermediate broken state. Phase 1 alone cannot trigger the memo-ordering
recursion hazard: a struct field naming a nested `Generic` application with variable arguments
only reaches `instantiate_struct`/`instantiate_enum` once a *concrete* instantiation is
requested, which is phase 2's `substitute_generic_field::Generic` arm — phase 1 only builds the
`PolyType` tree for a variable-containing field and never calls the instantiator for it. The
ordinary concrete-generic-application call into `instantiate_struct`/`instantiate_enum`
through `resolve_type_or_apply` (`parser.rs:3948-4000`) is a **separate, pre-existing** path
(any concrete `Name[i64]` field already goes through it today, generics-unrelated to this
slice) and is unaffected by either phase — phase 1 introduces no new call into it, so this is
not an intermediate-state risk worth naming beyond confirming it here. So the split does not
ship a hang as an intermediate state.

**Phase 1 — the recursive struct/enum field-type parser, plus the two prerequisites it needs
to have anything to parse against.**

- **Required prerequisite: register a generic struct/enum's own header (name, `ty_var_names`,
  span, module) before parsing its field/variant list, not after.** Confirmed root cause (see
  "Recursive `instantiate_struct`/`instantiate_enum`" above): `parse_generic_typedefs`
  (`parser.rs:3867-3884`) pushes the completed decl onto `self.generics.structs`/`.enums` only
  after `parse_generic_typedef`/`parse_generic_enum_typedef` returns, so a self-reference
  anywhere in the field list — even a fully concrete one, `L['T]`'s `next L[i64]` twin — is
  `unknown type` today. Mechanism: push a placeholder decl with an empty `fields`/`variants`
  list immediately after `parse_generic_header_vars` returns, then overwrite it in place with
  the real list once parsing completes. Independently testable: a parser-level unit test that
  a self-referential concrete field (`L[i64]` inside `L`'s own declaration) resolves to a
  `Type`/`PolyType::Generic` rather than erroring, with no dependency on the recursive
  field-type parser below.
- **Required prerequisite: the new `PolyType`/`RawTy` owned-cell variant and its exhaustive-
  match audit.** Introducing the variant is a hard Rust compile error until every existing
  `match` over `PolyType`/`RawTy` is updated, so the variant and its audit fallout across the
  eleven files grep-confirmed to reference `PolyType::` (`src/ast.rs`, `src/check.rs`,
  `src/check/audits.rs`, `src/check/combinators.rs`, `src/check/declarations.rs`,
  `src/check/engine.rs`, `src/check/poly.rs`, `src/check/word_entry.rs`, `src/ir/driver.rs`,
  `src/parser.rs`, `src/repl.rs`; 401 references total, most constructors rather than matches)
  and `RawTy::` (`src/parser.rs` only) must ship in the same commit that introduces the
  variant, not deferred to phase 2 — the crate would not compile in between. Not itemized
  line-by-line since Rust's own exhaustiveness check enumerates every site as a hard build
  error, not a silent miss, but sized here as its own line item.
- `parse_generic_field_type_expr`'s rewrite: array, ref, nested-generic-application arms
  (mirroring `parse_poly_slot`, each needs its own unit test), plus the mixed-concrete/variable
  `Generic` case.
- The owned-cell arm, plus the new `parse_poly_slot` `^`-arm (word-signature side).
- The quotation-field boundary: replicate `quotation_type_ahead()`'s disambiguation, then a
  located rejection for a variable-quotation field, with both pinned tests (concrete still
  builds, variable is a clean error).
- Independently testable via parser-level unit tests asserting the produced `PolyType` tree,
  without ever reaching instantiation (except the header-registration prerequisite's own test
  above, which resolves a concrete self-reference to a `Type` by design). The `&'T`-in-a-
  struct-field diagnostic-quality criterion (below) needs phase 2's `Ref` arm before it can
  run, so it is not a phase-1 deliverable despite touching phase 1's parser change.

**Phase 2 — substitution, registry threading, and recursion safety.** Depends on phase 1's
header self-registration (a self-reference must already resolve to a type before substitution
can run over it) and its new `PolyType` variant (already exhaustively matched everywhere by
phase 1's audit, so phase 2 adds arms to already-total matches rather than reopening the
audit).

- `substitute_generic_field`'s new `Array`/`Ref`/`Generic`/owned-cell arms.
- The `&'T`-in-a-struct-field diagnostic-quality fix: once phase 1's field parser resolves
  `&'T` to `PolyType::Ref` and this phase's `Ref` arm substitutes it, confirm `Box 'T r &'T`
  at a concrete instantiation now hits the located stored-reference error
  (`check/declarations.rs:1170`) rather than `unknown type` — this needs the `Ref` arm above,
  it is not reachable from phase 1's parser change alone (a bare `PolyType::Ref` field would
  panic in `substitute_generic_field`'s `unreachable!` until this arm lands).
- The positional/attributeless variant array-field parse gap's regression test (`Some [i64 2]`,
  pinning today's rejection unchanged) — grouped here since it is a concrete-only fixture with
  no dependency on phase 1's poly-var work, so it can land whenever is convenient in this phase.
- The registry-threading fix: separate `&mut Vec<ArrayDecl>`/`&mut Vec<RefDecl>`/`&mut
  Vec<OwnedCellDecl>` parameters through `substitute_generic_field`,
  `instantiate_struct`/`instantiate_enum`, and their ~8 call sites (`parser.rs:2940`, `2943`,
  `3983`, `3997`; `check/poly.rs:3092`, `3094`, `4482`, `4484`), including the borrow-conflict
  restructuring at `parser.rs:3948-4000`.
- `instantiate_struct`/`instantiate_enum`'s memo-ordering fix (retarget to a `^`-wrapped
  self-reference, per 2(b) above) and the growing-recursion rejection (the structural-growth
  detection mechanism itself is an open spec question, not designed here).
- The `Map`-shaped two-instantiation regression test (`StructId`/field-type assertions, per exit
  criteria).

## Ready to spec: yes, with one instruction for spec-writer

Verify every citation above against live `main` before writing (`parser.rs`, `ast.rs`, and
`check/poly.rs` line numbers move as other in-flight slices land). Treat the growing-vs-
non-growing structural-detection mechanism (see "Self-referential generic structs" above) as an
open design question to resolve with a concrete mechanism, not a restated requirement — it
hasn't been designed yet, only bounded, exactly as S3k's brief left its own analogous question
open for its spec.
