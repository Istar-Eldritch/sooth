# P7.S6a -- Length parameters in `type:` headers, and the `Kind` type

**Status:** Planned
**Discovery:** `docs/roadmap/P7/slice6a-brief.md`

## Problem Statement

A user-defined generic type can bind a type variable (`type: Box['T]`, S6's
bracket binding site) but not a length variable. `type: Buffer['T 'N] data
array['T 'N] ;` does not parse, and even if it did the field-substitution and
instantiation paths could not carry the length:

- `parse_header_bracket` (`src/parser.rs:5305`) accepts only `'`-prefixed words
  with no way to mark one as a length rather than a type.
- `GenericStructDecl`/`GenericEnumDecl` (`src/ast.rs:542`, `:556`) carry only
  `ty_var_names: Vec<String>`; there is no length-variable field.
- `substitute_generic_field` (`src/ast.rs:815`) matches only
  `PolyType::Array(elem, Len::Concrete(count))`; a `Len::Var` field would fall
  through to the `other => unreachable!()` catch-all (`src/ast.rs:861`), whose
  own comment records that no header binds a length variable today.
- `GenericTypes::struct_keys`/`enum_keys` (`src/ast.rs:602-603`) dedup on
  `(usize, u32, Vec<Type>)` -- type args only -- so `Buffer[u8 256]` and
  `Buffer[u8 512]` would collide onto one monomorph.
- `type_instantiation_name` (`src/ast.rs:777`) renders only the `Vec<Type>` args.
- The concrete use-site parser `resolve_type_or_apply` -> `parse_type_arguments`
  (`src/parser.rs:5143`, `:5270`) parses `arity` (from `ty_var_names.len()`)
  full type expressions, eagerly monomorphizing at parse time; it has no path
  to read a length literal.
- **Lowering is a fourth consumer, on the run path rather than the check
  path.** `subst_polytype`'s `Generic` arm (`src/ir/driver.rs:614`) grounds a
  `PolyType::Generic` to a concrete `Type` by calling `lookup_struct`/
  `lookup_enum` (`src/ast.rs:936`, `:944`), keyed on `(idx, module, args:
  &[Type])` -- the same type-args-only key `struct_keys`/`enum_keys` carry.
  Once two distinct-length monomorphs of one header exist, this lookup must
  also match on length or it resolves to whichever monomorph happens to sit
  first, which either silently lowers the wrong one or trips the site's own
  `.expect("checked: apply_subst already minted this instantiation")` panic.
  Found by reading the lowering path directly, not named in the brief or the
  roadmap -- the exit criteria's "builds *and runs*" goldens are what make this
  load-bearing rather than academic.
- A third application parser exists at the field level, inside another
  header's own field list: `parse_generic_field_application`
  (`src/parser.rs:5611`) parses a generic type applied to a nested field
  (`type: Pair['T 'N] a Buffer['T 'N] ;`, `Buffer['T 'N]` in `a`'s position).
  It carries its own arity split (from `ty_var_names.len()` alone) and its own
  eager-concrete-collapse calling `instantiate_struct`/`instantiate_enum`
  directly -- the same two defects R6 and R7 exist to fix, at a third,
  previously-unnamed site.

A `Buffer`-shaped struct wrapping a generic-length array (any fixed-capacity
container) is common stdlib material and is currently unwritable.

## What already works (verified against source, not roadmap prose)

A **word** signature already carries length variables of a kind distinct from
type variables. This machinery is unchanged by the slice:

- `VarKind { Ty, Len }` (`src/parser.rs:1312`) drives
  `PolyBuilder::intern_ty_var`/`intern_len_var` (`:1436`, `:1452`), which reject
  a name used as both kinds (`var_kind_conflict_error`).
- `PolySig` carries `ty_var_names`/`len_var_names` separately;
  `PolyType::Array(Box<PolyType>, Len)` carries `Len::Var` for a word's declared
  array-length parameter.
- `unify_poly_input`'s `PolyType::Array` arm (`src/check/poly.rs:7699-7729`,
  inside the function starting `:7655`) binds a `Len::Var` from a concrete
  array's count; `apply_subst` (function starting `:7958`) resolves it from
  `Subst.len`; `len` on a generic-length array folds to `usize` in a poly body.

What is missing is entirely on the **declaration**, **field-construction**,
**use-site-instantiation**, **check-time-binding**, and **lowering** paths for
`type:`/`trait:` headers, which never touch `PolyBuilder` -- they go through
the separate `parse_generic_header` (`src/parser.rs:4872`) / `parse_header_
bracket` path, which has no kind distinction, through the concrete
`parse_type_arguments`/`parse_generic_field_application` application parsers,
through the `struct_instantiation_of`/`enum_instantiation_of`-keyed check-time
machinery, and through `subst_polytype`'s lowering-time grounding.

## Design Rulings

R1--R6 carry the brief's "What changes" items 1--6; R7 carries the use-site
parsing the brief surfaced by reading (absent from the roadmap prose); R8 is the
check-time correction surfaced during this spec's own verification -- a genuine
divergence from the brief's "check-time machinery needs no change" claim (see
**OQ1** for the full analysis). **Ruled in scope**: the exit criterion is a word
declaring `Buffer['T 'N]` unifying against a concrete caller, and R8 is the
mechanism that makes that true rather than an accident of R5's dedup bug.
Deferring it to S6b would ship a header feature whose main signature-position
consumer silently mis-binds; S6b is the orthogonal call-site-length-literal
axis (`sum[i64 4]`), not this mechanism.

**R2a and R2b, added in review round 2.** A fresh-context review of this spec
found two P0s and three P1s the first draft missed -- all closed below, not
deferred: R2a closes the two P0s (the exit fixture's own field could not be
parsed or phantom-checked under R1--R8 as first written); R2b closes the
word-binding-site P1 (the Exit Criteria promised `: Len` at a `:` word site
with no ruling behind it); R3, R7, and R8 were expanded in place (not given
new IDs) to close the remaining three P1s (a dropped `GenericVariant`/
`Operative` length component, an unlengthed `RawTy::Generic` collapse, and
impl-target matching/specificity left length-blind).

**Round 3: a further, exhaustive inventory.** Two more review
rounds each found a widened type's consumer this spec had missed -- twice now
the same shape of gap (a ruling widens `PolyType::Generic` or its supporting
registries; a consumer several call sites away goes unmentioned). This pass
re-derives the full consumer set from scratch (every match/construction site
of `PolyType::Generic`/`GenericVariant`/`Operative::Generic`, every caller of
`instantiate_*`/`lookup_*`/`*_instantiation_of`) rather than patching the
latest two findings in isolation. Closed this round, all folded into existing
rulings rather than given new top-level IDs (R2a widens further; R3, R4, and
R8 each widen in place; see **OQ3** for the index):

- The third field-level application parser (`parse_generic_field_application`)
  -- closed by widening **R2a**.
- `substitute_generic_field`'s own `Generic` arm dropping a nested `len_args`
  -- closed by widening **R4**.
- `ground_member_poly` and three diagnostic renderers silently dropping
  `len_args` -- closed by widening **R3** (mechanical: a clone-forward and a
  cosmetic string addition, no design decision).
- Lowering (`subst_polytype`, `lookup_struct`/`lookup_enum`) left entirely
  unaccounted for -- closed by widening **R8**.
- `poly_mentions_len_var`'s guard going blind to a header-carried length --
  closed by widening **R8**.
- A length-only header (zero type variables) silently breaking four
  `args.is_empty()`-as-"non-generic" convention sites in `src/check/poly.rs`
  -- closed by a new constraint on **R2** (at least one type variable is
  required), ruling the shape out rather than chasing every site.
- Phase 1 depending on phase 4's own work for its own test to build -- closed
  by moving that test's fixture, not the ruling (see the **Phases (JSON)**
  section and R2b's own text).
- The R8 goldens/fixtures using invalid `impl:` syntax (missing `for`), and
  carrying no pinned observable, making two mutations potential placebos --
  closed in **Tests** and **Mutation recipe**.

**Round 4 (this pass): the headline exit golden was still a placebo, and the
mechanical inventory still had gaps -- a fourth, literal grep sweep, not a
fourth prose re-derivation.** Round 3 closed real findings but reintroduced
the placebo shape on its own headline exit criterion, and its "exhaustive"
re-derivation still missed several consumers a literal
`grep -n "PolyType::Generic\|PolyType::GenericVariant"` across every file
turns up directly. This pass ran that grep (and the equivalent for
`instantiate_struct`/`instantiate_enum`), cross-checked every hit against this
spec's rulings, and closed what it found -- see **OQ4** for the index and the
literal grep output:

- **The headline exit golden required field projection out of a
  `PolyType::Generic` receiver, which is rejected in a non-inline generic
  body** (`receiver_is_aggregate_projection`/`poly_unsupported_accessor_error`,
  `src/check/poly.rs:8417-8438`, `:4762-4771`), and the `inline` rescue
  destroys discrimination (a spliced concrete monomorph never consults `'N`'s
  binding). Closed by **redesigning the fixture** (see **Tests**): a second,
  bare-array-typed parameter carrying the same `'N` replaces field
  projection, and a new *negative* golden (mismatched lengths across the two
  `'N` occurrences must be rejected) is the actual mutation-5 discriminator.
- **R5's "contained to `src/ast.rs`" claim was false.** `instantiate_struct`/
  `instantiate_enum`'s signature widening forces `poly_construct_generic`
  and `apply_subst`'s `Generic`/`GenericVariant` arms (all in
  `src/check/poly.rs`) to compile against the new parameter the moment R5
  lands, in phase 3 -- two phases before R8a supplies the real value. Closed
  by ruling exactly what each of the three sites passes in the interim, and
  why each is safe (see **R5**'s revised text).
- **`Operative::Generic`'s *construction* site was never named** (only its
  destructure/consumption site was) -- `src/check/poly.rs:3163-3172` builds
  an `Operative::Generic` from a `PolyType::Generic` scrutinee and must carry
  `len_args` forward too. Closed by widening **R3**.
- **`poly_destructure_generic`'s own `enum_sites` push** (`src/check/poly.rs
  :4406-4412`) rebuilds a `PolyType::Generic` from a `PolyType::GenericVariant`
  and drops `len_args` the same way. Closed by widening **R3**.
- **`generic_field_type_str` (`src/parser.rs:1866`) has a *reachable panic*,
  not just a cosmetic gap**: its own `Array` arm's `Len::Var(_) =>
  unreachable!()` fires the moment a nested array field carries a length
  variable in element position (`array[array['T 'N] 'M]`), because
  `parse_generic_field_array` (`src/parser.rs:5593`) calls it unconditionally
  to build a display string, not only on error. Closed by widening **R2a**.
- **`generic_args_of` (`src/check/poly.rs:7332`) has no length twin**, so
  `collect_paired_positions`'s Struct/Enum arms (`:7429-7460`),
  `collect_positions`'s `Generic` arm (`:7084-7112`), and
  `collect_concrete_positions`'s Struct/Enum arms (`:7165-7202`) have no
  source of per-side length args at all -- not compile-forced, so nothing
  catches its absence but the golden it feeds. Closed by widening **R8b**
  with a new `generic_len_args_of` sibling.
- Confirmed, not touched, three genuine dead ends found by the same sweep:
  `poly_type_mentions_caller_var` (`:2565`, a different variable namespace
  entirely -- type-variable growth-tracking never asks about lengths),
  `substitute_generic_variant_field` (`src/ast.rs:2115`, whose own doc
  comment proves a generic enum *variant* field is always `Var`/`Concrete`,
  never array-shaped, independent of this slice), and
  `reject_growing_generic_argument` (`src/parser.rs`, a length is a scalar,
  never itself a growing compound).

- **R1. The `Kind` enum.** `VarKind { Ty, Len }` (`src/parser.rs:1312`,
  parser-private, word-signature-local) becomes a `Kind` enum with variants
  `Star` (replacing the implicit "no kind" of `ty_var_names` and `VarKind::Ty`)
  and `Len`. Whether this renames `VarKind` in place or introduces a shared
  `Kind` that both the word-signature path and the new header path consume is an
  implementation-phase call; the roadmap's framing ("`VarKind { Ty, Len }`
  becomes a `Kind` enum") reads as the former. `Kind` stays `{ Star, Len }`: no
  `Const` (non-length const kinds) and no `Arrow` (higher-kinded), which
  DESIGN.md's "dependent types: never" and P7b respectively keep out (see **Out
  of scope**).

- **R2. Header bracket syntax.** `parse_header_bracket` (`src/parser.rs:5305`)
  gains a `: Len` annotation path: `'N: Len` (colon glued or spaced, mirroring a
  word bound's `'T: Copy`) interns a length variable; a bare `'T` interns a type
  variable (kind `Star`, the unannotated common case). `type: Buffer['T 'N: Len]`
  declares one of each. An annotation naming anything but `Len` is a located
  error (no other kind is spellable this slice). The existing per-bracket
  diagnostics (`duplicate_generic_ty_var_error`, `empty_header_bracket_error`,
  `header_bracket_non_var_error`) stay; a name used as both a type and a length
  variable in one header is a located error, the header-path twin of
  `var_kind_conflict_error`.

  **R2.1 -- at least one type variable is required (added in review round 3).**
  A header binding only length variables (`type: Buf['N: Len] data array[i64
  'N] ;`, zero type variables) is rejected: `empty_header_bracket_error`'s
  existing "the whole bracket is empty" check is joined by a sibling check
  requiring the *type*-kind subset specifically to be non-empty whenever the
  bracket is non-empty at all. **Verified and ruled, not left ambiguous**: a
  length-only generic struct/enum would carry an empty `ty_args`/`args: Vec<Type>`
  the moment it is instantiated, and four sites in `src/check/poly.rs` --
  `collect_concrete_positions`'s `Struct`/`Enum` arms (`:7167-7181`,
  `:7188-7202`) and `collect_paired_positions`'s `Struct`/`Enum` arms
  (`:7429-7432`, `:7452-7455`) -- treat an empty type-args list as the
  signal for "not actually generic here" (`if !args.is_empty() { recurse }
  else { push a concrete/no-op position }`), a convention that is correct
  today only because a generic header with zero *type* variables was
  previously unconstructible. Ruling the shape out (rather than widening all
  four sites to also check length-args non-emptiness) keeps R8's already-large
  consumer set from growing a fifth axis for a shape this slice's own
  motivating example (`Buffer['T 'N]`, which always has a type variable)
  never needs. A future slice can lift this restriction together with fixing
  the four sites, if a length-only header turns out to be wanted.

  **R2.2 -- `Len` is a reserved name (added in review round 3).**
  `reject_reserved_name` (`src/parser.rs:244`) gains a `Len` case for `kind ==
  "trait"` (mirroring the existing `Slice`/`array` reservations for `kind ==
  "type"`), so a user cannot declare `trait: Len ... ;`. Trivial and closed,
  not left as an open item: without it, R2b's bracket intercepts the bare
  word `Len` ahead of `parse_capabilities` unconditionally, so a user-declared
  trait named `Len` would be silently unreachable from any bound bracket --
  the same reasoning `reject_reserved_name`'s own doc comment already gives for
  `Slice`/`array`. **Verified (round 4): no new call site is needed.**
  `parse_trait_decl` (`src/parser.rs:2481`) already calls
  `reject_reserved_name("trait", &name, name_span)` at `:2484` -- it is a
  no-op today only because no existing check inside `reject_reserved_name`
  matches on `kind == "trait"`. The fix is entirely inside
  `reject_reserved_name`'s own body, not a new caller.

- **R2a. Field construction of a length-carrying field, and its phantom
  check (added in review round 2; widened in review round 3 to cover a
  third parser).** R2 alone makes a header *bind* a length variable; nothing
  yet makes one *appear in a field*. Two field-construction sites and the
  header's own bookkeeping are `ty_vars`-only and must gain a parallel length
  path:
  - `parse_generic_field_array` (`src/parser.rs:5579`) reads every array count
    through `parse_array_count` (`src/parser.rs:4502`, literal-only) and
    always builds `Len::Concrete`; its own doc comment ("N3: a struct header
    binds no *length* variable") is the reason R4's `Len::Var` substitution
    arm is unreachable today. It gains a `'`-prefixed-token arm ahead of the
    literal read -- mirroring `parse_poly_array`'s existing `'N` arm
    (`src/parser.rs:3668-3672`) -- that resolves the token against the
    header's length-variable list and yields `Len::Var(i)`. Unlike a word
    signature's `intern_len_var` (which *mints* an id on first sight), a
    header field only *resolves*: every length variable was already bound by
    R2's bracket, so an unresolvable `'N` is `unbound_generic_ty_var_error`'s
    length-path twin, not a fresh interning. This sub-case is self-contained:
    it resolves against the *currently-parsing* header's own local length-name
    list (the `GenericHeader` two-list plumbing below), not against any other
    header's completed AST decl, so it needs neither R3's AST field nor R5.
  - **`parse_generic_field_application` (`src/parser.rs:5611`, added in
    review round 3 -- a third, previously-unnamed application parser).** This
    is the field-level twin of R6/R7's use-site application parsers: a field
    naming *another* generic header applied to arguments (`type: Pair['T 'N]
    a Buffer['T 'N] ;`). Its arity is read from the *referenced* header's
    `ty_var_names.len()` alone (`src/parser.rs:5622-5626`) and its
    eager-concrete-collapse (`:5645-5665`) calls `instantiate_struct`/
    `instantiate_enum` directly whenever every argument is `PolyType::Concrete`
    -- both defects R6/R7 exist to fix. The same fix, one level down: split
    the argument list into `0..ty_arity` type arguments and
    `ty_arity..ty_arity+len_arity` length arguments (a `'`-prefixed token
    resolves against the *current* header's own length-variable list, as in
    the array case above; a literal count is `Len::Concrete`), gate the
    eager collapse on all lengths being `Len::Concrete` too (mirroring R7's
    gate), and pass the length list to `instantiate_struct`/`instantiate_enum`
    (R5). Unlike the array sub-case above, this sub-case reads the
    *referenced* header's `len_var_names` (`self.generics.structs[idx].
    len_var_names.len()`, R3's new AST field) to know the referenced header's
    length arity -- so this sub-case depends on R3 and R5, and lands in a
    later phase than the array sub-case (see **Phases (JSON)**).
  - **`generic_field_type_str` (`src/parser.rs:1866`) has a reachable panic,
    not just a cosmetic gap (added in review round 4).** This is the same
    renderer R3 widens for `Generic`-application length-arg printing, but it
    has a *second*, independent problem: its own `Array` arm reads
    `Len::Var(_) => unreachable!("a generic`type:`field has no length
    variable")` -- true before R2a, false after. `parse_generic_field_array`
    (`src/parser.rs:5593`) calls this function **unconditionally** (to build
    a display string fed to `parse_array_count`, not only on a parse error),
    so a nested field whose *element* is itself a length-carrying array
    (`type: Grid['T 'N: Len] rows array[array['T 'N] 'N] ;`) panics at parse time
    the moment R2a makes the inner `Len::Var` real, regardless of whether the
    outer parse succeeds. Fixed by widening `generic_field_type_str` to take
    a second parameter, `len_vars: &[(String, Span)]` (mirroring `ty_vars`),
    and rendering `Len::Var(v) => len_vars[*v as usize].0.clone()` in the
    `Array` arm exactly as the `Var` arm already renders a type variable --
    threaded through all of this function's own recursive calls and its two
    call sites (`src/parser.rs:1877` internal recursion, `:5593` the
    error-string build).
  - `resolve_field_ty_var` (`src/parser.rs:5559`) and its caller
    `parse_generic_typedef_fields` (`src/parser.rs:4791`, `used: Vec<bool>`
    sized to `ty_vars`) are `ty_var_names`-only. They gain a parallel
    `resolve_field_len_var`/`used_len: Vec<bool>` sized to the header's
    length-variable list, threaded alongside `ty_vars`/`used` through the same
    call chain (both field-construction sites above mark an index used
    exactly as `resolve_field_ty_var` does today). `check_no_phantom_ty_var`
    (`src/parser.rs:2012`) gains a length-path sibling check (same shape,
    same call site, after the whole field list is known): a length variable
    bound in the header bracket but never appearing in any field's array
    count (or, per the widened sub-case above, in a nested application's
    length-argument position) is a phantom, reported by a dedicated
    diagnostic naming the variable and the header (the enum twin,
    `parse_generic_enum_typedef_variants` at `src/parser.rs:4826`, gets the
    same treatment for variant fields).
  - The two-pass plumbing that carries a header's variable list from
    registration to field-parsing also widens: `GenericHeader`
    (`src/parser.rs:1259`, today `(String, Vec<(String, Span)>, Span)`) and
    the `parse_generic_typedefs` (`src/parser.rs:4973`) `headers` accumulator
    both gain a second `Vec<(String, Span)>` for the length-variable list,
    alongside the existing type-variable one -- `parse_header_bracket`
    (R2) already produces a kinded per-entry list; this is where it gets
    split into the two lists every downstream field/variant parser consumes.

- **R2b. Word bound-bracket kind annotation (added in review round 2 --
  closes a P1; Exit Criteria's `: Len` at a word `:` site had no ruling).**
  `'N: Len` was reachable nowhere at a word's `:` site: `parse_optional_bound_
  bracket` (`src/parser.rs:2245`) treats every `'`-prefixed bracket entry as a
  type-variable-with-capabilities, and its capability reader,
  `parse_capabilities` (`src/parser.rs:3717`), has no `Len` case -- `Len` is
  not a `Bound` (`src/ast.rs:1720`) and adding it there would misrepresent a
  kind as a capability. **Resolution: extend the bracket (option (a) from the
  round-2 review), not defer the exit criterion (option (b)) -- the fix is
  small.** A word signature's variable ids are already effect-first-mention-
  derived (the existing comment at `src/parser.rs:2232` says so), and a bare
  `'N` in a `Buffer['T 'N]` signature position already auto-interns as a
  length variable through `PolyBuilder::intern_len_var` (`src/parser.rs:1452`,
  already exists) once R7 lands -- exactly as a bare `'N` in `array['T 'N]`
  already does today with no bracket at all. So `'N: Len` in the bracket needs
  no new interning path, only a **validation** arm, the same shape
  `attach_bracket_bounds` (`src/parser.rs:2310`) already has for a capability
  bound.

  **Mechanism (spelled out precisely -- round 3 closes a P2 the bracket's
  entry tuple was left implicit on).** `parse_optional_bound_bracket`'s entry
  type, `Vec<(String, Span, Vec<Bound>)>`, widens to `Vec<(String, Span,
  Vec<Bound>, bool)>` -- the trailing `bool` is `is_len_kind`. When a colon
  (glued or spaced) is immediately followed by the bare word `Len` and
  nothing else in that bound position, the entry is built with `is_len_kind =
  true` and an empty `Vec<Bound>`, *without* calling `parse_capabilities` (so
  `Len` never becomes a fake `Bound`); every other bracket entry keeps
  `is_len_kind = false` exactly as before. `attach_bracket_bounds`
  (`src/parser.rs:2310`) gains a sibling branch: an `is_len_kind` entry looks
  its name up in `sig.len_var_names` (already populated by ordinary effect
  parsing, whatever route filled it) instead of `sig.ty_var_names`; a name
  absent from `sig.len_var_names` is the length-path twin of
  `bracket_var_unused_error` (`src/parser.rs:1832`).

  **Phase-1 self-containment (round 3 closes a P1: the roadmap's own example
  fixture needs R7, which lands two phases later).** R2b's *mechanism* above
  does not care how `sig.len_var_names` was populated -- only that it was, by
  the time `attach_bracket_bounds` runs. `array['T 'N]` already populates
  `len_var_names` today, with no dependency on R7 at all. So R2b's own unit
  tests (`parse_optional_bound_bracket_len_annotation_validates_against_
  signature`, `parse_optional_bound_bracket_len_annotation_unused_is_error`)
  use an `array['T 'N]`-shaped signature, not a `Buffer['T 'N]`-shaped one,
  keeping R2b fully buildable and testable in its own early phase. The
  roadmap's own richer example, `: capacity['T 'N: Len] ( &Buffer['T 'N] --
  usize )`, is unchanged code-wise (R2b's logic is identical either way) --
  it becomes an end-to-end **integration golden** once R7 lands, exercised
  there instead of pulled forward into R2b's own phase.

- **R3. AST fields, and the mechanical ripple they force (widened in review
  round 3 to close two more P2s).** `GenericStructDecl`/`GenericEnumDecl`
  (`src/ast.rs:542`, `:556`) gain `len_var_names: Vec<String>`, parallel to
  `ty_var_names`. `PolyType::Generic` (`src/ast.rs:2040`) gains
  `len_args: Vec<Len>`, parallel to `args: Vec<PolyType>`, so a signature-side
  application (`Buffer['T 'N]`) can carry its length component through the
  poly path. Two siblings that carry their own `args: Vec<PolyType>` copy of
  a `Generic` scrutinee -- added in review round 2, since R3's own "carried
  forward unchanged" design (see `src/ast.rs:2055`'s comment on
  `PolyType::GenericVariant`) already implies this, it was just not spelled
  out -- gain a parallel `len_args` the same way: `PolyType::GenericVariant`
  (`src/ast.rs:2064`, an eliminator arm's narrowed scrutinee type, an
  already-generic *enum* header's own variant) and `Operative::Generic`
  (`src/check/poly.rs:3462`, the eliminator-construction intermediate that
  `generic_variant_type` (`src/ast.rs:2076`) is built from, `args.clone()` at
  `src/check/poly.rs:3324`). Both are pure carry-forwards (R5.4's existing
  rule: "nothing re-unifies, the scrutinee already carries the substitution"),
  so this is mechanical, not new design -- a generic *enum* header with a
  length parameter, eliminated, must not silently drop its length the moment
  an arm narrows it.

  **The compile-forced ripple, contained (round 3).** Rust requires every
  exhaustive construction and non-`..` match of `PolyType::Generic` to name
  the new field once it exists; `cargo build`'s own completeness check is
  what finds them all, not a hand-enumerated list -- but two sites are named
  here because they need a **specific**, non-obvious value rather than an
  arbitrary placeholder:
  - `ground_member_poly` (`src/ast.rs:1828`, `Generic` arm at `:1841-1853`)
    rebuilds a trait member's declared `PolyType` by substituting `Self`
    (`PolyType::Var(_)`) for the impl's target; it never touches a length,
    exactly as its own existing `Array` arm already clones `len` through
    unchanged (`PolyType::Array(elem, len) => PolyType::Array(Box::new(
    ground_member_poly(elem, target)), len.clone())`, `src/ast.rs:1832-1834`).
    The `Generic` arm's fix mirrors that line exactly: `len_args:
    len_args.clone()` (no recursive grounding -- a `Len` is never a `PolyType`
    to ground).
  - Three diagnostic renderers print `args` only, so a diagnostic naming a
    length-carrying type would misleadingly print `Buffer[u8]` for
    `Buffer[u8 256]` -- actively confusing on exactly the diagnostics this
    slice's own distinct-monomorph feature is likely to trigger (an arity or
    overlap error). Fixed, not left as a note, since each is a one-line
    cosmetic addition (append the length list after the type-arg list inside
    the same bracket string, comma- or space-joined consistently with the
    existing type-arg join): `poly_type_shape_str` (`src/parser.rs:437`),
    `generic_field_type_str` (`src/parser.rs:1866`, plus a second,
    non-cosmetic fix -- see **R2a**), and `poly_type_str`
    (`src/check/poly.rs:9167`).
  - **`Operative::Generic`'s construction site (added in review round 4 --
    the destructure/consumption site alone was named, not this one).** The
    match arm at `src/check/poly.rs:3163-3172` builds an `Operative::Generic`
    (`args: args.clone()`) directly from a `PolyType::Generic` scrutinee
    (`is_enum: true` guard, `generic_surface_name(header) == family_name`);
    once both sides carry `len_args`, this arm also destructures and forwards
    it: `len_args: len_args.clone()`, the same clone-forward shape as
    `ground_member_poly`'s fix above. `Operative`'s own definition
    (`src/check/poly.rs:3462`) gains a matching `len_args: Vec<Len>` field,
    and its destructure/consumption site (`:3304-3324`, feeding
    `generic_variant_type`) forwards that field through to
    `generic_variant_type`'s new parameter (already named below).
  - **`poly_destructure_generic`'s own `enum_sites` push (added in review
    round 4).** `src/check/poly.rs:4406-4412` rebuilds a `PolyType::Generic`
    from a `PolyType::GenericVariant` scrutinee's own (now length-carrying)
    `args`/`len_args` -- `let (idx, module, vi, args) = (*idx, *module, *vi,
    args.clone());` widens to also destructure and forward `len_args`, the
    same carry-forward shape as `ground_member_poly`. Without this, an
    eliminator arm that destructures a narrowed, length-carrying generic enum
    variant drops the length the moment lowering re-grounds it through this
    recorded site.
  - **Confirmed dead end, not touched (round 4):**
    `substitute_generic_variant_field` (`src/ast.rs:2115`) is the enum twin
    of `substitute_generic_field`, but its own doc comment records a
    *measured*, pre-existing, parser-enforced restriction independent of
    this slice: a generic enum *variant* field is always exactly `Var` or
    `Concrete` -- `array['A 2]`, `Inner['A]`, `&'A`, `^'A` are already parser
    rejections for a variant field at HEAD. A length-carrying array field
    therefore never reaches this function, in a variant position, regardless
    of this slice; R4 correctly widens `substitute_generic_field` (struct/enum
    *header* field substitution) and correctly leaves this sibling alone.
  - Every other construction/match site (the mechanical majority --
    `check/declarations.rs`, `check/audits.rs`, `check/poly.rs`'s own
    non-diagnostic sites, test-only literals in `ast.rs`/`poly.rs`) either
    already matches with `..` (untouched) or forwards/ignores `len_args` with
    no logic decision (`len_args: vec![]` for a context that provably never
    sees one yet, or a straight clone/forward where one is already in hand).
    `cargo build` is the actual completeness gate for this category, not an
    enumerated list -- a phase is not done until it is clean. Verified
    dead ends in this bucket (round 4's mechanical sweep, not asserted):
    `poly_type_mentions_caller_var` (`src/check/poly.rs:2565`, a
    fundamentally different variable namespace -- type-variable
    growth-tracking never asks about lengths), `poly_construction_fallback`
    (`:4256`, read-only, never returns a length), and
    `reject_growing_generic_argument` (`src/parser.rs`, a length is a scalar,
    never itself a growing compound).

- **R4. Field substitution (widened in review round 3 to close a real,
  non-mechanical gap in its own function).** `substitute_generic_field`'s
  `PolyType::Array` arm (`src/ast.rs:828`) matches `Len::Var` as well as
  `Len::Concrete`. Given the instantiation's length-argument list (threaded
  alongside the existing `args: &[Type]`), a `Len::Var(i)` field looks up its
  concrete `u32` from that list exactly as `PolyType::Var(v)` looks up its
  concrete `Type` from `args`. The `other => unreachable!()` catch-all
  (`:861`) and its comment are updated to reflect that a length-bearing field
  is now reachable.

  **The function's own `Generic` arm also needs the length list, not just the
  `Array` arm (round 3 finding).** `substitute_generic_field`'s `Generic` arm
  (`src/ast.rs:838-852`) recurses into a nested field's own `header_args` and
  calls `instantiate_struct`/`instantiate_enum` with the substituted result --
  this is exactly the shape R2a's new `parse_generic_field_application` field
  produces (a `PolyType::Generic` with a possibly-variable-bearing
  `len_args`, nested inside *another* header's field). The `Generic` arm
  widens to also substitute `header_args`'s own `len_args` (a `Len::Var(i)`
  resolves against the instantiation's length list exactly as the `Array`
  arm's own `Len::Var` does; a `Len::Concrete` passes through unchanged) and
  pass the resulting length list to `instantiate_struct`/`instantiate_enum`
  (R5's widened signature). Not mechanical -- this is the same category of
  work as the `Array` arm's own fix, applied one level up.

- **R5. Instantiation plumbing, with its ripple deliberately contained
  (clarified in review round 3).** `instantiate_struct`/`instantiate_enum`
  (`src/ast.rs:1139`, `:1193`) take a length-argument list alongside the
  type-argument list. `struct_keys`/`enum_keys` (`src/ast.rs:602-603`) widen
  from `(usize, u32, Vec<Type>)` to `(usize, u32, Vec<Type>, Vec<Len>)`, so
  `Buffer[u8 256]` and `Buffer[u8 512]` mint distinct monomorphs.
  `type_instantiation_name` (`src/ast.rs:777`) renders the length args in the
  mangled symbol (e.g. `Buffer[u8 256]`). A zero-length-arg call is
  byte-identical to today, so every existing generic type's symbol is
  unchanged (a required migration-grep step, not an assumption -- re-run the
  `sooth_mono_*` grep the S6 spec's R8a established; see **Tests** for the
  two existing named tests this already pins, not just a manual sweep).

  **Deferred to R8, not this ruling: `struct_instantiation_of`/
  `enum_instantiation_of`'s *public return signature* stays `Option<(usize,
  u32, &[Type])>` in this phase.** Their bodies update mechanically to
  destructure the new fourth tuple element and discard it
  (`let (gi, m, args, _lens) = &self.struct_keys[i];`, `src/ast.rs:966`,
  `:976`), and this is what keeps `unify_poly_input`, `match_impl_target_rec`,
  and the three `collect_*` specificity/overlap functions (all `src/check
  /poly.rs`) plus lowering's `lookup_struct`/`lookup_enum`
  (`src/ir/driver.rs`) genuinely untouched until R8's own phase.

  **R5's ripple is not actually contained to `src/ast.rs` (round 4
  correction -- round 3's containment claim was false, verified against
  every caller of `instantiate_struct`/`instantiate_enum`).** Widening
  `instantiate_struct`/`instantiate_enum`'s own *parameter* list (not their
  callers' six R8 sites above, which read a different function --
  `struct_instantiation_of`/`enum_instantiation_of`) is a Rust-forced
  ripple into `src/check/poly.rs` the moment this ruling lands, in this
  phase, because two functions there call `instantiate_struct`/
  `instantiate_enum` directly and must keep compiling:
  - **`poly_construct_generic` (`src/check/poly.rs:4546,4548`) passes a
    permanent empty length list, not a phase-scoped placeholder.** This is a
    genuine dead end, not deferred work: its own helper,
    `poly_bind_construction_arg` (`src/check/poly.rs:4271-4302`), has an
    `unreachable!("a generic`type:`field is never {other:?}")` catch-all
    that already restricts every field this construction-inference path can
    bind to `Var`/`Concrete` -- an array-shaped field (length-carrying or
    not) already panics here today, pre-existing and unrelated to this
    slice. `poly_construct_generic` therefore never has a length to infer,
    before or after this slice; it passes `&[]` for the length-argument
    parameter permanently, cited here so a future length-inference extension
    to construction calls knows this is where it would have to start.
  - **`apply_subst`'s `Generic` arm (`src/check/poly.rs:8047`) and its
    `GenericVariant` arm (`:8100-8132`, which also calls `instantiate_enum`)
    pass an explicit, documented placeholder (an empty length list) for this
    phase and the next, replaced by R8a's real `subst.len`-resolved value.**
    Ruled safe, not silently gapped: `PolyType::Generic`/`GenericVariant`'s
    own `len_args` is populated in a checker-visible `PolySig` only by R7's
    signature-parsing fold, which lands in phase 4 -- one phase after this
    one. So during this phase, `apply_subst` never actually receives a
    non-empty `len_args` to drop. Once phase 4 lands, a real length *can*
    flow through `apply_subst` before phase 5 fixes it -- but no test added
    in phases 3-4 exercises an end-to-end `apply_subst` call against a
    length-carrying signature: phase 4's own tests are parser-only units
    (see **Tests**), and the integration goldens that would drive `apply_subst`
    end to end are not added until phase 6, by which point phase 5 (R8a) has
    already replaced the placeholder. `cargo test` therefore stays green
    throughout with no test silently validating wrong behavior. Mark both
    sites with a `// PLACEHOLDER, replaced by R8a` code comment so the gap is
    visible in the diff, not just in this spec.

  R8 is the ruling that changes `struct_instantiation_of`/
  `enum_instantiation_of`'s *public signature* (to `Option<(usize, u32,
  &[Type], &[Len])>`) and updates all of *their* consumers together,
  atomically, in its own phase -- a separate ripple from the two
  `instantiate_struct`/`instantiate_enum`-parameter call sites this ruling
  already had to touch above.

- **R6. Use-site parsing (concrete application).** `parse_type_arguments`
  (`src/parser.rs:5270`), or a sibling for the mixed case, splits a header
  application's bracket contents into `0..ty_arity` type expressions and
  `ty_arity..ty_arity+len_arity` length literals. The arity split is known
  statically from the header the moment it is resolved
  (`ty_var_names.len()`/`len_var_names.len()`). A length literal is a `u32`
  under the same `1..=u32::MAX` range check `parse_array_count`
  (`src/parser.rs:4502`) applies to an array count; a non-literal (a `'`-prefixed
  word, a type expression) in a length position, or a type expression in a length
  position and vice versa, is a located error. `resolve_type_or_apply`'s two
  instantiation call sites (`src/parser.rs:5187`, `:5201`) pass both lists.

- **R7. Use-site parsing (signature poly path).** The signature-side application
  `parse_poly_generic_application` (`src/parser.rs:3453`) and its
  `RawTy::Generic` -> `PolyType::Generic` fold (`src/parser.rs:3886`) split the
  same way: a bare `'N` in a length position interns a length variable through
  the enclosing `PolyBuilder` (as `parse_poly_array` already does at
  `src/parser.rs:3672`) and lands in the new `len_args: Vec<Len>`, while a
  literal count lands as `Len::Concrete`. This is the path the exit criterion's
  "a word declaring `Buffer['T 'N]` in its signature" exercises -- and,
  because `parse_impl_target` (`src/parser.rs:2749-2764`) already routes an
  `impl:` target's pattern through this same `parse_poly_slot`/`raw_to_poly_
  type` fold, this is also what makes `impl: Show for Buffer['T 4]` parse at
  all (R8's impl-matching widening is purely check-time; parsing needs no
  separate ruling there). Two concrete additions this implies but did not
  name (added in review round 2 -- closes a P1): `RawTy::Generic`
  (`src/parser.rs:1294`) has no length field today and must gain a
  `Vec<RawLen>` (`RawLen`, already defined at `src/parser.rs:1304`, reused
  rather than a new type), parsed by `parse_poly_generic_application` the same
  way `parse_poly_array` reads a count (a `'`-prefixed word interns through
  `builder.intern_len_var`, a literal is `RawLen::Concrete`); and the fold's
  eager-concrete collapse (`src/parser.rs:3906-3924`), which today computes
  `concrete: Option<Vec<Type>>` from the type args alone and instantiates
  immediately whenever every one is `PolyType::Concrete`, must also require
  every length arg to be `Len::Concrete` before collapsing -- otherwise
  `Buffer['T 4]` (a variable type, a concrete length) would wrongly
  instantiate a concrete struct with no way to place `'T`.

- **R8. Check-time binding and lowering of a header length (ruled in scope;
  widened in review rounds 2, 3, and 4).** This is the ruling that turns on
  the public consumer side R5 deliberately left untouched: `struct_
  instantiation_of`/`enum_instantiation_of` (`src/ast.rs:961`, `:971`) widen
  their public return type to `Option<(usize, u32, &[Type], &[Len])>`. **Ten
  call sites, across five functions (corrected in round 4 -- previously
  miscounted as "eight", and misattributed to `apply_subst`/`lookup_*`, which
  call a different pair of functions -- `instantiate_struct`/`instantiate_enum`
  and `lookup_struct`/`lookup_enum` respectively, both covered separately
  above and below):** `unify_poly_input` (`src/check/poly.rs:7881,7886`),
  `match_impl_target_rec` (`:6977,6982`), `collect_positions` (`:7096,7101`),
  `collect_concrete_positions` (`:7167,7188`), and `collect_paired_positions`
  (`:7429,7452`) -- each calling both the struct and enum form. All ten are
  updated together, atomically, in this one phase (split across two phases
  for effort reasons -- see **Phases (JSON)**).

  **8a -- signature unification and lowering.**
  `unify_poly_input`'s `PolyType::Generic` arm (`src/check/poly.rs:7852`,
  inside the function starting `:7655`) and `apply_subst`'s `Generic` arm
  (`src/check/poly.rs:8047`, inside the function starting `:7958`) bind/
  resolve `len_args` the same way their own neighboring `Array` arms already
  bind/resolve a bare array's `Len::Var` (`unify_poly_input`'s `Array` arm:
  `:7699-7729`). Without this, `Buffer['T 'N]` in a signature cannot bind
  `'N` from a concrete `Buffer[u8 256]` operand, so the exit criterion's
  signature-unification clause fails; see **OQ1** for the full divergence
  from the brief this corrects. **`apply_subst`'s `GenericVariant` arm
  (`src/check/poly.rs:8100-8132`, added in review round 4) needs the same fix
  and was previously unnamed**: it also calls `instantiate_enum` (to ground a
  narrowed variant's own header monomorph) and must resolve `GenericVariant`'s
  own `len_args` through `subst.len` identically to the `Generic` arm's fix,
  replacing the phase-3 placeholder R5's revised text establishes.

  **Lowering (round 3 addition to 8a).** `subst_polytype`'s `Generic` arm
  (`src/ir/driver.rs:614-639`) mirrors `apply_subst`'s own new logic:
  substitute `len_args` through `subst.len` before calling `lookup_struct`/
  `lookup_enum` (`src/ast.rs:936`, `:944`), which widen to accept and match a
  `&[Len]` alongside `&[Type]` (the same `(gi, m, a) == (idx, module, args)`
  key comparison, one more component). Without this, a length-carrying
  monomorph either lowers to the wrong instantiation or hits the site's own
  `.expect("checked: apply_subst already minted this instantiation")` panic
  the moment two distinct-length monomorphs of one header coexist -- which is
  precisely what R5 makes possible. This is what makes the exit criteria's
  "builds *and runs*" goldens literally true, not accidentally true through
  R5's dedup bug (the same reasoning **OQ1** already applies to signature
  unification, one layer down the pipeline).

  **The `poly_mentions_len_var` guard (round 3 addition to 8a).**
  `poly_mentions_len_var` (`src/check/poly.rs:2578`, `Generic` arm at
  `:2583`), used by `poly_cross_signature_supported` (`:2531`) to reject a
  length variable in a callee's signature for a poly-body cross-call, scans
  `args` only -- length-blind. Once a length lives in `len_args` instead of a
  bare array, this guard goes blind to it, silently admitting a poly-body
  cross-call to a callee like `capacity['T 'N: Len] ( &Buffer['T 'N] --
  usize )` that the guard exists specifically to reject (poly-body cross-calls
  are out of scope for this slice, per **Out of scope**). **Ruled: widen the
  guard's `Generic` arm to also scan `len_args`** (`args.iter().any(...) ||
  len_args.iter().any(|l| matches!(l, Len::Var(_)))`), keeping the existing
  reject-on-length-variable behavior intact rather than silently regressing
  it the moment this slice ships. `ground_member_poly` was checked for the
  same hazard and found not to apply (R3's note above: it only ever
  clone-forwards a length, never inspects one). **`poly_cross_match`'s
  `Generic` arm (`src/check/poly.rs:2434-2453`, matches on `(is_enum, idx,
  module, args.len())`) stays length-blind and is not itself widened** --
  noted explicitly (added in review round 4) rather than left unmentioned:
  it is safe only *because* this guard's own widening makes a length-carrying
  cross-call unreachable through the path `poly_cross_match` serves; it is
  not an independent correctness claim about `poly_cross_match` itself.

  **Position ordering (round 3 addition, needed for 8b below to be
  well-defined).** Every widened consumer that pushes or compares leaf
  positions for a `Generic`/`Struct`/`Enum` pattern does so in a fixed order:
  **type positions first (declaration order), then length positions
  (declaration order)** -- mirroring the convention the existing `Array` arm
  already establishes one level down (an array's element position(s), then
  its own single length position last, `src/check/poly.rs:6902-6928` (`match_impl_target_rec`'s twin) and its
  `collect_positions`/`collect_paired_positions` twins). This ordering must
  match between a pattern walk (`collect_positions`) and its concrete-target
  walk (`collect_concrete_positions`), and between two patterns walked
  together (`collect_paired_positions`), or specificity/overlap comparison
  silently misaligns a type position against a length position.

  **8b -- impl-target matching and specificity/overlap (round 2's widened
  scope).** `match_impl_target_rec`'s `Generic` arm (`src/check/poly.rs:6965`,
  zips only `args`) is `impl:`-target matching; `collect_positions`'s
  `Generic` arm (`:7084`) and `collect_concrete_positions`'s `Struct`/`Enum`
  arms (`:7165`, `:7186`) and `collect_paired_positions`'s `Struct`/`Enum`
  arms (`:7427`, `:7450`) are impl specificity-ranking and overlap detection.
  This is not a hypothetical future gap: `impl:` targets over a generic
  struct/enum header already work today (P7.S4/S4b), so the moment R2 lands,
  `impl: Show for Buffer['T 4]` is spellable (R7's parsing already covers
  this, per R7's own note above), and length-blind matching would silently
  match it against *any* `Buffer[T N]` regardless of `N`, and length-blind
  specificity/overlap checking would treat `impl: Show for Buffer['T 4]` and
  `impl: Show for Buffer['T 8]` as overlapping when they are not -- a live
  correctness bug the first time someone writes such an `impl:`, not a
  deferred edge case. The `Position` enum (`src/check/poly.rs:7023`) already
  has `LenConcrete`/`LenVar` variants (used today by a bare `array['T 'N]`
  field's own specificity position) -- the fix is the same pattern one level
  up, in the ordering established above: `match_impl_target_rec` zips and
  matches `len_args` alongside `args` (mirroring its own `PolyType::Array`
  arm's `Len::Concrete`/`Len::Var` handling immediately above it, `:6902-
  6928`), and the three `collect_*` functions push a `Position::LenConcrete`
  or `Position::LenVar` for each length arg exactly as they already do for a
  bare array's count -- after the type-position recursion, per the ordering
  rule. R2.1 (at least one type variable required) is what keeps this
  ruling's four `args.is_empty()` sites correct without a fifth check apiece.

  **`generic_args_of` needs a length twin (added in review round 4 -- the
  plumbing the paragraph above assumed already existed did not).**
  `generic_args_of` (`src/check/poly.rs:7332`) is how `collect_paired_positions`
  (and, indirectly, the pattern the `collect_*` family reasons about) recovers
  a per-side `Vec<PolyType>`, either a `Generic` pattern's own `args` or
  `Concrete`-synthesized args from a fully-concrete instantiation's type-arg
  list. It has no length-args counterpart, so nothing recovers a per-side
  length list at all -- not compile-forced (the function's signature does
  not have to change to keep compiling), so nothing catches its absence but
  the golden it feeds. Fixed by adding a sibling, `generic_len_args_of
  (pattern: &PolyType, len_args: &[Len]) -> Vec<Len>`, identical shape
  (`Generic`'s own `len_args`, or `Len::Concrete`-synthesized from the
  instantiation's length list), wired into three sites, each after its
  existing type-arg handling, per the ordering rule:
  - `collect_positions`'s `Generic` arm (`src/check/poly.rs:7084-7112`): zip
    `len_args` against `found`'s new fourth tuple element (R8's widened
    `struct_instantiation_of`/`enum_instantiation_of`), pushing a length
    position per pair after the existing `args.iter().zip(found_args.iter())`
    loop.
  - `collect_concrete_positions`'s `Struct`/`Enum` arms
    (`src/check/poly.rs:7165-7181`, `:7188-7202`): today these arms recurse
    over `args` only and fall back to a bare `Position::TyConcrete` when
    `args.is_empty()`; each gains a matching push of `Position::LenConcrete`
    per length arg (this walk is over `Type`, never a variable, so
    `LenVar` never applies here) after the type-arg loop, mirroring the
    `Type::Array` arm's own count-position push immediately above them in
    the same function.
  - `collect_paired_positions`'s `Struct`/`Enum` arms
    (`src/check/poly.rs:7429-7460`): call `generic_len_args_of` on both `a`
    and `b` (mirroring the existing `generic_args_of` calls immediately
    above), zip, and push `PairedLeaf::Aligned(len_position(la),
    len_position(lb))` per pair after the existing type-arg zip loop --
    exactly the shape the same function's own `Type::Array` arm already uses
    (`:7408-7412`) for a bare array's single length position.

## Tests

Unit tests beside the stage code (`thing_condition_expected`, CLAUDE.md), plus
integration goldens for the exit criteria.

**Parser -- header declaration (`src/parser.rs` `#[cfg(test)]`):**

- `parse_header_bracket_len_annotation_interns_a_length_variable` -- `['T 'N: Len]`
  yields one `Star` and one `Len` name in declaration order.
- `parse_header_bracket_bare_var_defaults_to_star` -- `['T]` interns a type
  variable, no length names (regression floor for the unannotated common case).
- `parse_header_bracket_unknown_kind_annotation_is_error` -- `'N: Foo` is a
  located error naming `Len`.
- `parse_header_bracket_name_as_both_kinds_is_error` -- `['T 'T: Len]` is a
  located error (the header-path `var_kind_conflict_error` twin).
- `parse_header_bracket_length_only_is_error` -- `['N: Len]` (no type
  variable) is a located error (R2.1).
- `parse_generic_type_header_with_length_parameter_parses` -- the exit fixture
  `type: Buffer['T 'N: Len] data array['T 'N] ;` builds a decl with
  `len_var_names == ["'N"]` and a `Len::Var` field (R2a; must fail before R2a's
  `'`-prefixed-token arm exists -- the P0 this closes).
- `parse_generic_field_array_unbound_length_var_is_error` -- `type: Buffer['T]
  data array['T 'N] ;` (`'N` used in a field but never bound by the header
  bracket) is a located error, the field-path twin of
  `unbound_generic_ty_var_error` (R2a).
- `parse_generic_typedef_phantom_length_var_is_error` -- `type: Buffer['T 'N:
  Len] data 'T ;` (`'N` bound but never used in any field's array count) is a
  located phantom-length error (R2a; the second P0 this closes -- must fail
  before `used_len` bookkeeping exists).
- `parse_generic_field_application_splits_type_and_length_args` -- `type:
  Pair['T 'N: Len] a Buffer['T 'N] ;` (a nested generic application inside another
  header's field) resolves `Buffer['T 'N]`'s trailing `'N` as a length
  variable, not an arity error (R2a's widened scope, closes the round-3 P0/P1
  on the third application parser).
- `parse_generic_field_application_concrete_type_variable_length_does_not_
  collapse` -- `type: Pair['T] a Buffer['T 4] ;` (a variable type, a concrete
  length, inside a field) stays `PolyType::Generic`, not `Concrete` (the
  field-application twin of R7's collapse-gate test).
- `parse_generic_field_array_concrete_element_variable_length_does_not_fold_
  to_concrete` -- `type: Buffer['T 'N: Len] data 'T array[i64 'N] ;` (a
  concrete element type, a variable length, plus a `'T` field so R2.1's
  type-variable-required check and `check_no_phantom_ty_var` both clear)
  builds `PolyType::Array(Concrete(i64), Len::Var)`,
  not `PolyType::Concrete(intern_array_type(..))` (added in review round 4 --
  the array parser's concrete-element collapse branch was previously
  untested against a variable-length fixture).
- `reject_reserved_name_rejects_trait_len` -- `trait: Len ... ;` is a located
  reserved-name error (R2.2).
- `parse_optional_bound_bracket_len_annotation_validates_against_signature` --
  a word signature `: capacity['T 'N: Len] ( &array['T 'N] -- usize ) ...`
  parses, with `'N: Len` in the bracket accepted as a validation, not a
  capability (R2b; the fixture is array-based per R2b's phase-1
  self-containment note, not `Buffer`-based).
- `parse_optional_bound_bracket_len_annotation_unused_is_error` -- a bracket
  `['T 'N: Len]` on a word whose effect never mentions `'N` in a length
  position is a located error, the length-path twin of
  `bracket_var_unused_error` (R2b).

**AST -- substitution & instantiation (`src/ast.rs` `#[cfg(test)]`, beside the
existing `substitute_generic_field_array_of_ty_var_interns_concrete_array` at
`src/ast.rs:3762`):**

- `substitute_generic_field_array_of_len_var_interns_concrete_count` -- a
  `Len::Var` field grounds to the instantiation's length arg (the `Array` arm
  R4 makes reachable; must fail against the pre-R4 `other => unreachable!()`).
- `substitute_generic_field_nested_generic_forwards_its_own_len_args` -- a
  `PolyType::Generic` field (from R2a's field-application case) whose own
  `len_args` contains a `Len::Var` grounds correctly when the outer header is
  instantiated (R4's widened `Generic` arm; must fail if the inner length is
  dropped or left unsubstituted).
- `instantiate_struct_distinct_lengths_mint_distinct_monomorphs` -- `Buffer[u8
  256]` and `Buffer[u8 512]` produce two `StructId`s and two `struct_keys`
  entries (the collision R5 fixes; must fail against the `Vec<Type>`-only key).
- `instantiate_struct_same_length_dedups` -- two `Buffer[u8 256]` hit one
  monomorph (the dedup floor, so R5's widening does not over-mint).
- `type_instantiation_name_renders_length_args` -- the mangled symbol contains
  the length (e.g. `Buffer[u8 256]`), and differs from `Buffer[u8 512]`'s
  rendered name (R5; the "differs", not just "contains", clause is what a
  dropped-length-in-the-renderer-only mutation needs to fail against).
- `generic_variant_type_carries_len_args_from_its_scrutinee` -- a
  `PolyType::Generic` with a non-empty `len_args`, narrowed into a
  `PolyType::GenericVariant`, carries the same `len_args` forward unchanged
  (R3's widened scope; must fail if `len_args` is dropped at the
  `Operative`/`GenericVariant` boundary).
- `ground_member_poly_generic_arm_clones_len_args_unchanged` -- a
  `PolyType::Generic` with a non-empty `len_args`, run through
  `ground_member_poly`, keeps the same `len_args` (R3's mechanical
  clone-forward, named per CLAUDE.md's "every stage function gets a happy-path
  test", not left as an unwitnessed mechanical claim).

**Check -- eliminator round-trip (`src/check/poly.rs` `#[cfg(test)]`, added in
review round 4 -- the construction site this covers was previously unnamed):**

- `operative_generic_construction_site_carries_len_args_from_its_scrutinee` --
  a `PolyType::Generic` scrutinee with a non-empty `len_args`, matched into an
  `Operative::Generic` (`src/check/poly.rs:3163-3172`), carries the same
  `len_args` forward (must fail if this construction site, as opposed to its
  already-covered destructure/consumption site, drops it).
- `poly_destructure_generic_enum_sites_push_carries_len_args` -- destructuring
  a narrowed, length-carrying `PolyType::GenericVariant` (`enum_sites.push`,
  `src/check/poly.rs:4406-4412`) records a `PolyType::Generic` whose
  `len_args` matches the source variant's own (must fail if this carry-forward
  is dropped, silently losing the length the moment lowering re-grounds it).

**Parser -- use-site (`src/parser.rs` `#[cfg(test)]`):**

- `parse_type_arguments_splits_type_and_length_args` -- `Buffer[u8 256]` resolves
  to a struct type; the `256` is read as a count, not a type.
- `parse_type_arguments_length_literal_out_of_range_is_error` -- `Buffer[u8 0]`
  is the `parse_array_count` range error.
- `parse_type_arguments_non_literal_in_length_position_is_error` -- `Buffer[u8
  u8]` (a type where a count is due) is a located error whose message names
  the length-position violation specifically, not merely `is_err()` (the
  sibling S3s-follow spec's own "substance-matched, not just `is_err()`"
  convention).
- `parse_poly_generic_application_binds_a_length_variable` -- a signature
  `Buffer['T 'N]` interns `'N` as a length variable in `len_args`, and the
  resulting `PolySig.len_var_names` contains it (R7; the second clause is
  what a mutation dropping the `RawTy::Generic` length field specifically
  needs to fail against, distinct from a mutation that merely fails to
  collapse).
- `raw_to_poly_type_generic_concrete_type_variable_length_does_not_collapse`
  -- `Buffer['T 4]` (a variable type, a concrete length) folds to
  `PolyType::Generic`, not `Concrete` (R7's widened scope; must fail if the
  collapse gates on the type args alone).
- `generic_field_type_str_renders_a_nested_length_variable` -- a nested array
  field whose *element* is itself a length-carrying array
  (`array[array['T 'N] 'M]`) renders through `generic_field_type_str` without
  panicking, naming `'N` by its surface spelling (added in review round 4 --
  this is a crash fix: the pre-fix `Len::Var(_) => unreachable!()` arm is
  reachable unconditionally from `parse_generic_field_array`'s own
  error-string construction, not only on a parse error).

**Check -- poly-body guard (`src/check/poly.rs` `#[cfg(test)]`):**

- `poly_mentions_len_var_generic_arm_sees_a_length_in_len_args` -- a
  `PolyType::Generic` whose `len_args` (not `args`) contains a `Len::Var`
  makes `poly_mentions_len_var` return `true` (R8's guard widening; must fail
  against the pre-fix `args`-only scan).
- `poly_cross_signature_supported_rejects_a_header_carried_length` -- a
  poly-body cross-call to a callee whose signature names `Buffer['T 'N]`
  (a header-carried length, not a bare array) is rejected with the existing
  "a length variable in the callee's signature" diagnostic (the integration
  twin of the unit test above, proving the guard's *caller* still rejects the
  case end to end).
- `generic_len_args_of_recovers_a_length_list_from_both_shapes` -- called on a
  `PolyType::Generic` pattern with its own non-empty `len_args`, and
  separately on a `PolyType::Concrete` pattern against a synthesized
  instantiation length list, both return the expected `Vec<Len>` (added in
  review round 4 -- the sibling `generic_args_of` had this coverage, its new
  length twin previously had none).

**Lowering (`src/ir/driver.rs` `#[cfg(test)]`, beside the existing
`subst_polytype_grounds_a_poly_ref_slot_from_a_monomorphic_caller` at
`src/ir/driver.rs:1044`):**

- `subst_polytype_generic_arm_grounds_a_length_carrying_monomorph` -- a
  `PolyType::Generic` with a `Len::Var` in `len_args`, substituted against a
  `Subst` binding that variable, resolves through `lookup_struct` to the
  correct one of two distinct-length monomorphs (must fail -- either the
  wrong monomorph or the `.expect` panic -- if `len_args` substitution is
  skipped or `lookup_struct`/`lookup_enum` stay type-args-only).

**Integration goldens (`tests/phase7_slice6a.rs`):**

**The `capacity` fixture is redesigned in round 4: field projection out of a
`PolyType::Generic` receiver is rejected in a non-inline generic body**
(`receiver_is_aggregate_projection`/`poly_unsupported_accessor_error`,
`src/check/poly.rs:8417-8438`, `:4762-4771`), so a body that projects into
`Buffer`'s own `data` field to call `len` does not compile, and marking it
`inline` (the obvious rescue) folds `len` off the spliced monomorph's already-
concrete field type, never consulting `'N`'s *binding* -- exactly the placebo
shape round 3 already flagged once. The fixture below instead carries `'N` in
a **second, bare-array-typed parameter**, which exercises the pre-existing,
already-working `len`-on-a-generic-length-array machinery ("What already
works") instead of projecting through the aggregate, and turns the mutation
discriminator into a *rejection* test (a checker accept/reject question,
which needs no runtime print at all):

```sooth
: capacity['T 'N: Len] ( array['T 'N] &Buffer['T 'N] -- usize )
    drop len swap drop
;
```

- `buffer_header_with_length_parameter_builds_and_runs` -- the exit dogfood
  (`type: Buffer['T 'N: Len] ...` plus the `capacity` fixture above, called
  with an `array[u8 256]` and a `Buffer[u8 256]` of matching length) compiles,
  runs, and prints `capacity`'s result -- pinned to the concrete length
  (`256`), not merely "compiles" (this is also where R2b's richer,
  `Buffer`-shaped validation is actually exercised end to end, per R2b's
  phase-1 self-containment note).
- `distinct_buffer_lengths_are_distinct_types` -- a program using both
  `Buffer[u8 256]` and `Buffer[u8 512]` compiles; the two are not
  interchangeable (a word taking one rejects the other, proving R5's distinct
  monomorphs are load-bearing at check time, not just in the symbol name).
- `word_over_buffer_length_unifies_against_concrete_caller` -- calling
  `capacity` with a matching `array[u8 256]`/`Buffer[u8 256]` pair typechecks
  and returns `256` -- the positive case, proving the R7/R8a
  signature-unification exit clause end to end.
- `word_over_buffer_length_rejects_a_mismatched_length_operand` -- **the
  actual mutation-5 discriminator (round 4; replaces a golden round 3 could
  not make discriminating without field projection).** Calling `capacity`
  with an `array[u8 256]` first operand and a `Buffer[u8 512]` second operand
  (the *same* declared `'N`, mismatched concrete lengths) is rejected with
  `poly_len_conflict_error`. This works *because* `unify_poly_input` binds
  `'N` from the first (`Array`) operand before the second (`Generic`) operand
  is checked against it (`subst.len_of` sees a prior binding and conflicts):
  if R8a's `Generic` arm skips binding `len_args` (mutation 5), its own
  operand never contributes to or conflict-checks against the existing
  binding, so nothing conflicts and this call
  wrongly typechecks -- a checker-level accept/reject flip, not a runtime
  value, so it needs no `len`-observable rescue and cannot be defeated by an
  `inline` splice.
- `impl_target_over_distinct_buffer_lengths_does_not_overlap` -- `impl: Show
  for Buffer['T 4]` and `impl: Show for Buffer['T 8]` (**`for` included --
  the original draft's fixture was missing it and would not have parsed**)
  both compile as non-overlapping impls, and **a bound, trait-generic call
  through each** (e.g. a word generic over `'A: Show` calling `show` on its
  operand -- a direct, concretely-typed call would resolve at parse time
  through the ordinary `env` path and never exercise `impl:`-target dispatch
  at all) **prints a distinguishable, hardcoded per-impl constant** (not a
  `len`-derived value, since indexing a generic-length array in a non-inline
  body is out of scope -- see **Out of scope**) -- proving dispatch actually
  reached the matching impl, not merely that both compiled (R8b's widened
  scope; must fail -- either a spurious overlap-conflict diagnostic, or the
  wrong impl's constant printing, or both printing the same constant -- if
  `match_impl_target_rec`/the `collect_*` specificity family stay
  length-blind).

## Mutation recipe (planned, each must fail a named test)

Classify on a named `test result: FAILED`, in an isolated committed copy with
the mutated binary confirmed rebuilt (per the memory notes on stale-binary and
worktree-copy hazards; commit before mutating).

1. In R4's `substitute_generic_field` `Array` arm's `Len::Var` handling,
   ignore the length-argument list and hard-code `Len::Concrete`'s existing
   literal path: `substitute_generic_field_array_of_len_var_interns_concrete_
   count` and `buffer_header_with_length_parameter_builds_and_runs` fail.
2. Drop the length component from R5's `struct_keys` dedup key (revert to
   `Vec<Type>` only): `instantiate_struct_distinct_lengths_mint_distinct_monomorphs`
   and `distinct_buffer_lengths_are_distinct_types` fail, while
   `instantiate_struct_same_length_dedups` stays green (the discriminator is the
   distinct-length test, not the dedup test).
3. In R2, treat a `: Len` annotation as kind `Star` (intern every header var as a
   type variable): `parse_header_bracket_len_annotation_interns_a_length_variable`
   and the header-parse fixture fail.
4. In R6, parse a length position with `parse_type_expr` instead of the count
   reader (accept a type where a count is due):
   `parse_type_arguments_non_literal_in_length_position_is_error` fails (the
   test's own substance check on the error message, not merely `is_err()`, is
   what still fails it if the mutation produces some *other* error for the
   malformed input -- see the test's own description).
5. In R8a's `unify_poly_input` `Generic` arm, skip binding `len_args` (zip only
   the type args): `word_over_buffer_length_rejects_a_mismatched_length_operand`
   fails (it wrongly typechecks, since only the `array['T 'N]` operand ever
   binds `'N`, with no prior binding to conflict against -- round 4: the
   round-3 fixture could not discriminate this without field projection;
   `word_over_buffer_length_unifies_against_concrete_caller`'s positive case
   stays green either way, so it is not this mutation's discriminator), while
   the construct-only tests stay green (isolating the check-time binding
   from the declaration/instantiation machinery).
6. In R2a's `parse_generic_field_array`, keep the `'`-prefixed-token arm but
   skip marking `used_len[idx] = true`: `parse_generic_typedef_phantom_length_var_is_error`
   loses its discriminating power the *other* way (a real use is
   misreported as phantom), so instead assert the direct symptom --
   `parse_generic_type_header_with_length_parameter_parses` fails (a `'N`
   correctly resolved but never marked used trips the header's own phantom
   check on a fixture that must build clean).
7. In R8b's widened scope, skip zipping `len_args` in `match_impl_target_rec`'s
   `Generic` arm (match on type args alone):
   `impl_target_over_distinct_buffer_lengths_does_not_overlap` fails on its
   pinned per-impl constants (either the wrong constant prints, or both print
   the same one, or a spurious overlap diagnostic fires), while
   `word_over_buffer_length_unifies_against_concrete_caller` stays green (the
   discriminator is impl-target matching, not signature unification --
   isolating which of R8's two widened call families broke; note this
   isolates *matching* specifically -- the three `collect_*` specificity
   functions share this same golden as their only discriminator, so a defect
   confined to `collect_*` alone is not separately isolated from a
   `match_impl_target_rec` defect by this recipe; both live in R8b's single
   phase, so this is an acceptable coarseness, not a gap left across a phase
   boundary).
8. In R2b's `attach_bracket_bounds` sibling arm, route an `is_len_kind` entry
   through `sig.ty_var_names` instead of `sig.len_var_names` (or treat every
   bracket entry as a capability, dropping the `is_len_kind` distinction
   entirely): `parse_optional_bound_bracket_len_annotation_validates_against_
   signature` and `parse_optional_bound_bracket_len_annotation_unused_is_error`
   fail (added in review round 3 -- R2b previously had zero mutation
   coverage).
9. In R2a's phantom-length check itself, delete the sibling check outright
   (not just its `used_len` bookkeeping, per mutation 6): `parse_generic_
   typedef_phantom_length_var_is_error` fails and no other test does (added
   in review round 3 -- the diagnostic itself, as opposed to its bookkeeping,
   was previously unmutated; as written it could have been implemented as
   `Ok(())` and the recipe would still have passed).
10. In R3's widened scope, skip carrying `len_args` across the
    `Operative`/`GenericVariant` boundary (drop it, or replace with an empty
    `Vec`): `generic_variant_type_carries_len_args_from_its_scrutinee` fails
    (added in review round 3 -- R3's widened scope was previously unmutated).
11. In R7's `RawTy::Generic` length-field parsing, skip populating the new
    `Vec<RawLen>` (or always leave it empty regardless of what was parsed):
    `parse_poly_generic_application_binds_a_length_variable` fails on its
    `PolySig.len_var_names` clause (added in review round 3 -- R7's core
    binding test was previously unmutated).
12. In R5's `type_instantiation_name`, drop the length-rendering addition
    while leaving the `struct_keys` dedup key (mutation 2's target) intact:
    `type_instantiation_name_renders_length_args`'s differs-from-`Buffer[u8
    512]` clause fails, even though `instantiate_struct_distinct_lengths_
    mint_distinct_monomorphs` stays green (added in review round 3 -- this is
    the exact symbol-collision hazard class this project has hit before: two
    distinct monomorphs, sharing one rendered name).
13. In R3's widened `Operative::Generic` construction site
    (`src/check/poly.rs:3163-3172`), skip forwarding `len_args` (leave it
    empty): `operative_generic_construction_site_carries_len_args_from_its_
    scrutinee` fails (added in review round 4 -- this construction site was
    previously unnamed and therefore unmutated).
14. In R2a's widened `generic_field_type_str`, skip the `len_vars` lookup and
    restore the `Len::Var(_) => unreachable!()` arm:
    `generic_field_type_str_renders_a_nested_length_variable` fails -- as a
    panic, not merely a wrong string, so this is also a crash-safety
    regression, not only a cosmetic one (added in review round 4).
15. In R8b's `generic_len_args_of`, return an empty `Vec<Len>` regardless of
    the pattern's own `len_args` (or skip wiring it into any of the three
    `collect_*` call sites): `impl_target_over_distinct_buffer_lengths_does_
    not_overlap` fails the same way mutation 7 does (a spurious overlap, the
    wrong impl's constant, or both printing the same one) -- added in review
    round 4 to give the previously-unplumbed `generic_len_args_of` its own
    named discriminator, distinct from mutation 7's `match_impl_target_rec`
    target (both still share this one golden as their only witness, per
    mutation 7's own coarseness note).

Not mutation-testable and flagged as plain regression coverage, each verified
to have no possible discriminating mutation of its own (not asserted, checked):
`parse_header_bracket_bare_var_defaults_to_star` and
`instantiate_struct_same_length_dedups` are floors -- a mutation making them
fail is caught by a partner test above, so they carry no unique kill. The same
is true of `parse_generic_field_array_unbound_length_var_is_error` (caught by
any mutation that makes the `'`-prefixed-token arm disappear entirely, which
the fixture-parsing tests above already catch),
`raw_to_poly_type_generic_concrete_type_variable_length_does_not_collapse`
(a floor for R7's gate, not independently mutation-discriminating beyond what
the collapse-gate's own absence already breaks in the fixture tests), and the
zero-length-arg migration-safety claim in the Exit criteria, which rests on
two *already-existing* named tests rather than a fresh mutation:
`type_instantiation_name_unambiguous_struct_arg_stays_bare` (`src/ast.rs:4181`)
and `instantiation_symbol_reproduces_native_spelling_expected` -- both already
pin the zero-length rendering byte-for-byte, so R5's own migration claim is
regression-tested by tests this slice does not need to add.

## Out of scope

- **Generic-length array indexing in a non-inline body.**
  `poly_generic_length_index_error` (`src/check/poly.rs`, raised near `:4700`)
  still rejects `&>` on an `array['T 'N]` in a non-inline body -- the checker
  cannot statically prove `i < 'N`. The workaround stays `inline` (the body
  splices where `'N` is concrete). Relaxing this is a different kind of work
  (loop-variable provenance or a runtime bounds check). This is also why the
  `impl_target_over_distinct_buffer_lengths_does_not_overlap` golden's
  distinguisher must be a hardcoded per-impl constant, not an indexed buffer
  element.
- **A length-only generic header** (zero type variables, e.g. `type:
  Buf['N: Len]`). Ruled out this slice by R2.1, not left ambiguous: it would
  otherwise defeat four `args.is_empty()`-as-"non-generic" convention sites in
  `src/check/poly.rs` (see R2.1's own text). A future slice can lift this
  together with fixing those four sites, if wanted.
- **Non-length const kinds** (boolean, string phantom parameters). `Kind` stays
  `{ Star, Len }`; generalizing `Len` to a broader `Const` crosses DESIGN.md's
  "dependent types: never" line.
- **`Arrow` kind** (higher-kinded type variables, `: * -> *`) -- a later P7b
  addition, named in the roadmap prose, not this slice.
- **S6b** (explicit length arguments at a *word* call site, `sum[i64 4]`, seeding
  `subst.len` in `check_poly_call`). The next slice; independent in principle.
  This slice covers only the `type:`/`trait:` header and its application.
- `tree-sitter-sooth/grammar.js` (a bracket group tokenises generically) and
  `docs/book/` (uncompiled, separately tracked).

## Open Questions

None outstanding.

**OQ1, resolved (round 1).** The brief and roadmap both asserted the
check-time machinery "needs no change" for the signature-unification exit
clause; this spec's own verification found that **false for a generic *header*
carrying a length**:

- The "already works" machinery (`unify_poly_input`'s `PolyType::Array` arm,
  `apply_subst`'s array arm) binds a `Len::Var` only for a *bare* `array['T 'N]`
  parameter. A `Buffer['T 'N]` parameter reaches `unify_poly_input`'s
  `PolyType::Generic` arm (`src/check/poly.rs:7852`) instead, which recovers the
  operand's build args via `struct_instantiation_of` (`src/ast.rs:961`) -- and
  that returns **type args only** (`Vec<Type>`), zipping only `args`. There is no
  length component to bind `'N` from.
- Today the clause appears to pass only because `Buffer[u8 256]` and `Buffer[u8
  512]` *collide* onto one monomorph -- the exact bug R5 removes. Once R5 mints
  distinct monomorphs, the `Generic` arm must recover and bind the length, or
  signature unification over a length-carrying header silently mis-binds or
  fails.

**Ruling:** R8 is in scope (widen `struct_instantiation_of`/
`enum_instantiation_of` to return the length list, and widen the `Generic` arms
of `unify_poly_input` and `apply_subst` to bind/resolve `len_args`). Deferring
signature-side length unification to S6b was rejected: S6b is the call-site
explicit-length-literal axis (`sum[i64 4]`), a different mechanism from binding
a length out of a header type an operand already carries; narrowing this
slice's exit to construction/application only would ship a header feature
whose signature-position use silently mis-binds or fails.

**OQ2, resolved (round 2).** A fresh-context correctness review of this spec
found two P0s and three P1s, all resolved above rather than deferred -- each
ruling's own text carries the rationale, this is the index:

- Two P0s (the exit fixture's own field could not be parsed, and the
  phantom-variable gate was `ty_vars`-only) -- both closed by **R2a**, a new
  ruling. Neither was optional: without R2a, no ruling in the spec as first
  written made `type: Buffer['T 'N: Len] data array['T 'N] ;` -- the spec's
  own exit fixture -- actually parse.
- A P1 (the Exit Criteria's word-`:`-site clause had no ruling behind it) --
  closed by **R2b**, a new ruling. Resolved as "extend", not "defer", because
  the fix is a validation-only addition: a word's variable ids are already
  effect-derived, so the bracket needs no new interning, only a lookup
  against `len_var_names` R7 already populates.
- A P1 (`RawTy::Generic` and its eager-concrete-collapse fold had no length
  awareness) -- closed by widening **R7** in place.
- A P1 (`PolyType::GenericVariant`/`Operative::Generic` drop a `Generic`
  scrutinee's length on the eliminator path) -- closed by widening **R3** in
  place; mechanical, since R3's own "carried forward unchanged" design already
  implied it.
- A P1 (`struct_instantiation_of`/`enum_instantiation_of` have four more
  length-blind call sites beyond `unify_poly_input`, covering `impl:`-target
  matching and specificity/overlap ranking) -- closed by widening **R8** in
  place. Resolved as "in scope", not "out of scope", because it is not a
  hypothetical: `impl:` over a generic header already works today (S4/S4b), so
  a length-blind impl match/overlap check is a live, immediately-reachable
  correctness bug the first time this slice's own header feature is used in
  an `impl:` target, not a deferred edge case.

**OQ3, resolved (round 3).** Two further fresh-context review
rounds each found a widened type's consumer still missed after round 2's
fixes -- the same shape of gap, one layer further out each time. This pass's
resolution, exhaustively re-derived (every match/construction site of
`PolyType::Generic`/`GenericVariant`/`Operative::Generic`, every caller of
`instantiate_*`/`lookup_*`/`*_instantiation_of`), all closed rather than
deferred:

- **Lowering was entirely unaccounted for** (`subst_polytype`,
  `src/ir/driver.rs:614`, and `lookup_struct`/`lookup_enum`, `src/ast.rs:936`/
  `:944`) -- the exit criteria's "builds *and runs*" goldens could not have
  literally passed without this. Closed by widening **R8** (folded into 8a,
  alongside the signature-unification work it mirrors).
- **A third, previously-unnamed application parser**
  (`parse_generic_field_application`, `src/parser.rs:5611`) carries the same
  two defects R6/R7 exist to fix, at the field level inside another header.
  Closed by widening **R2a**.
- **`substitute_generic_field`'s own `Generic` arm** (as opposed to just its
  `Array` arm) drops a nested field's `len_args` the moment R2a's new
  field-application case produces one. Closed by widening **R4**.
- **The `poly_mentions_len_var` guard** (`src/check/poly.rs:2583`) scans
  `args` only, silently admitting a header-carried length past a check that
  exists specifically to reject a length variable in a poly-body cross-call.
  Closed by widening **R8** (folded into 8a): the guard's `Generic` arm now
  also scans `len_args`, preserving the existing reject behavior rather than
  quietly narrowing it.
- **A length-only generic header** (zero type variables) would defeat four
  `args.is_empty()`-as-"non-generic" sites in `src/check/poly.rs`. Resolved by
  **ruling the shape out** (R2.1: at least one type variable required), not by
  widening all four sites -- the motivating use case never needs it, and this
  keeps R8's already-large consumer set from growing a fifth axis.
- **`Len` was not a reserved name anywhere**, letting a user-declared `trait:
  Len` be silently shadowed by R2b's bracket-position intercept. Closed by a
  one-line addition, **R2.2**.
- **Phase 1 could not build its own test**: the word-bound-bracket ruling's
  natural example fixture (`Buffer['T 'N]` in a signature) depends on R7,
  scheduled two phases later. Closed by restating the *test's fixture* (to an
  `array['T 'N]`-shaped signature, which needs no R7), not the ruling itself
  -- R2b's logic is identical either way; see R2b's own "phase-1
  self-containment" note.
- **Two goldens used invalid `impl:` syntax** (`impl: Show Buffer['T 4]`,
  missing the required `for`) **and carried no pinned observable**, making
  their mutations potential placebos (a documented prior incident in this
  project: two monomorphs compiling to identical loads passing a golden that
  never checked they differed). Closed in **Tests** (both goldens now pin a
  length-derived or hardcoded-constant observable, and the `for` is present)
  and **Mutation recipe** (entries 8-12 added; entries 1, 6, 7 clarified).
- **The mutation recipe was incomplete while claiming to be exhaustive**: R2b,
  R3's widened scope, R7's core binding test, and R5's name-renderer each had
  a test with no discriminating mutation. Closed by mutation entries 8-12
  above.

**OQ4, resolved (round 4 -- this pass).** Two further fresh-context review
rounds found round 3's own headline fix still a placebo, and round 3's
"exhaustive" re-derivation still missed several consumers -- so this pass
ran a literal mechanical grep (`grep -n "PolyType::Generic"`/
`"PolyType::GenericVariant"` across every `src/*.rs`/`src/**/*.rs` file, and
the equivalent for `instantiate_struct`/`instantiate_enum`/
`struct_instantiation_of`/`enum_instantiation_of`), cross-checked every hit
against this spec's rulings, and closed what it found rather than patching
the two named findings in isolation:

- **The headline exit golden required field projection out of a
  `PolyType::Generic` receiver**, unsupported in a non-inline generic body
  (`receiver_is_aggregate_projection`, `src/check/poly.rs:8417-8438`), and
  its `inline` rescue destroyed discrimination (a spliced monomorph never
  consults `'N`'s binding). Closed by **redesigning the fixture**: a second,
  bare-array-typed parameter replaces field projection, and a new *negative*
  golden (mismatched lengths across the two `'N` occurrences must be
  rejected) is the real mutation-5 discriminator -- see **Tests**.
- **R5's "contained to `src/ast.rs`" claim was false**: `instantiate_struct`/
  `instantiate_enum`'s own parameter widening forces `poly_construct_generic`
  and `apply_subst`'s `Generic`/`GenericVariant` arms (`src/check/poly.rs`)
  to keep compiling the moment R5 lands, two phases before R8a supplies the
  real value. Closed by ruling exactly what each site passes and why it is
  safe -- see **R5**'s revised text.
- **`Operative::Generic`'s construction site** (`src/check/poly.rs:3163-3172`)
  and **`poly_destructure_generic`'s `enum_sites` push**
  (`src/check/poly.rs:4406-4412`) both drop a scrutinee's `len_args` the
  moment R3 lands -- previously unnamed; only their sibling
  destructure/consumption sites were. Closed by widening **R3**.
- **`generic_field_type_str` (`src/parser.rs:1866`) has a reachable panic**,
  not just a rendering gap: its `Array` arm's `Len::Var(_) => unreachable!()`
  fires unconditionally (not only on error) the moment a nested array field
  carries a length variable in element position. Closed by widening **R2a**
  with a `len_vars` parameter.
- **`generic_args_of` (`src/check/poly.rs:7332`) has no length twin**, so
  three `collect_*` sites have no source of per-side length args at all --
  not compile-forced, so nothing but the golden it feeds would have caught
  its absence. Closed by widening **R8b** with a new `generic_len_args_of`.
- **Confirmed dead ends, not touched**, from the same mechanical sweep:
  `poly_type_mentions_caller_var` (a different variable namespace entirely),
  `substitute_generic_variant_field` (a pre-existing, parser-enforced
  restriction to `Var`/`Concrete` fields, independent of this slice),
  `poly_construction_fallback` (read-only, never returns a length), and
  `reject_growing_generic_argument` (a length is a scalar, never a growing
  compound).

## Exit criteria

- `type: Buffer['T 'N: Len] data array['T 'N] ;` parses; `len_var_names` and a
  `Len::Var` field are recorded on the decl.
- `Buffer[u8 256]` instantiates as a distinct monomorph from `Buffer[u8 512]`
  (distinct `StructId`, distinct `type_instantiation_name`), and two
  `Buffer[u8 256]`s dedup to one.
- A word declaring `Buffer['T 'N]` in its signature (alongside a second,
  bare-array-typed parameter also naming `'N` -- **not** by projecting into
  the struct's own field, which stays unsupported in a non-inline body per
  **Out of scope**) unifies `'N` against a concrete `Buffer[u8 256]` caller,
  a lowering-time run produces `'N`-derived output correctly, and a call
  passing two operands with *mismatched* concrete lengths for the same `'N`
  is rejected (R7/R8a; the mismatch case is what actually proves `'N` is
  bound from the header operand, not only from the bare-array one).
- Two impls over distinct-length instantiations of the same header
  (`impl: Show for Buffer['T 4]`, `impl: Show for Buffer['T 8]`) compile as
  non-overlapping and dispatch to distinguishable per-impl output (R8b).
- A header field can name a bound length variable in its array count
  (`data array['T 'N]`) or in a nested generic application
  (`a Buffer['T 'N]`), and a length variable bound but never used in any
  field is a located phantom error (R2a).
- A header binding zero type variables (only length variables) is a located
  error (R2.1); `trait: Len ... ;` is a located reserved-name error (R2.2).
- The `Kind` enum has `Star` and `Len` variants; the `: Len` annotation is live
  at `type:`, `trait:`, and `:` (word) binding sites -- the word site through
  R2b's bracket validation, not a new interning path.
- Every existing generic type's mangled symbol is unchanged (the zero-length-arg
  path is byte-identical, per the two already-existing named tests R5 cites;
  `sooth_mono_*` grep clean).
- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green.
- P7.S6a marked `[ done ]`; growth-signal re-run recorded for any file this slice
  meaningfully grows (`src/parser.rs`, `src/ast.rs`, `src/check/poly.rs`,
  `src/ir/driver.rs`) at its phase exit.

## Phases (JSON)

Re-derived from scratch (round 3, corrected round 4) against the actual
dependency graph, not force-fit to five: `struct_instantiation_of`/
`enum_instantiation_of`'s *own* public return signature (and therefore
`unify_poly_input`, `match_impl_target_rec`, the three `collect_*` functions,
and lowering's `lookup_struct`/`lookup_enum`) is deliberately left unchanged
until phase 5, per R5's own note. **Correction (round 4): this is not the same
as phases 1-4 touching only `src/parser.rs`/`src/ast.rs`.** `instantiate_struct`/
`instantiate_enum`'s own *parameter* list widens in phase 3, which is a
separate, smaller ripple into `src/check/poly.rs` (three call sites --
`poly_construct_generic`, and `apply_subst`'s `Generic`/`GenericVariant` arms
-- pass a permanent or interim placeholder length list, per **R5**'s revised
text) and into `src/check/poly.rs`'s `Operative::Generic` construction site
(per **R3**'s round-4 addition). No phase requires a *later* phase's work to
build or pass its own tests -- both `check/poly.rs` touches in phase 3 are
placeholders proven safe in **R5**'s own text, not early access to phase 5's
logic. Phase 5 is split into 5a/5b along the line the mutation recipe already
draws (signature/lowering vs. impl-matching/specificity) -- 5b depends on 5a's
signature widening (both read `struct_instantiation_of`'s new fourth
tuple element), so it is sequenced after, not parallel.

```json
{
  "phases": [
    { "phase": 1, "focus": "R1 Kind enum (VarKind -> {Star, Len}); R2 parse_header_bracket ': Len' annotation, R2.1 at-least-one-type-variable constraint, R2.2 Len reserved-name; header-parse unit tests", "effort": "M", "difficulty": "standard" },
    { "phase": 2, "focus": "R2a's array-field sub-case (parse_generic_field_array/resolve_field_ty_var length arms, GenericHeader two-list plumbing, phantom-length check, generic_field_type_str's len_vars widening and its Len::Var panic fix) and R2b (word bound-bracket 'N: Len validation, entry-tuple is_len_kind flag, array-fixture unit tests only)", "effort": "M", "difficulty": "standard" },
    { "phase": 3, "focus": "R3 AST fields (len_var_names; PolyType::Generic/GenericVariant/Operative::Generic.len_args, plus the compile-forced constructor ripple incl. ground_member_poly, the three diagnostic renderers, Operative::Generic's construction site, and poly_destructure_generic's enum_sites push); R4 substitute_generic_field (Array arm's Len::Var handling and the Generic arm's own len_args forwarding); R5 instantiate_struct/enum + struct_keys/enum_keys + type_instantiation_name length plumbing, struct_instantiation_of/enum_instantiation_of's body updated for the new tuple shape with its PUBLIC SIGNATURE UNCHANGED, plus the three check/poly.rs call sites (poly_construct_generic permanent empty length list, apply_subst's Generic/GenericVariant arms' documented interim placeholder) instantiate_struct/enum's own widened parameter forces; R2a's nested-generic-application field sub-case (parse_generic_field_application, now unblocked by R3+R5); substitution/instantiation/renderer unit tests; sooth_mono_* grep", "effort": "L", "difficulty": "standard" },
    { "phase": 4, "focus": "R6 concrete use-site (parse_type_arguments type/length split) and R7 signature poly path (parse_poly_generic_application, RawTy::Generic length field, length-aware eager-collapse gate); use-site unit tests", "effort": "M", "difficulty": "standard" },
    { "phase": 5, "focus": "R8a: struct_instantiation_of/enum_instantiation_of's public signature widened to expose length; unify_poly_input/apply_subst Generic AND GenericVariant arms (signature unification, replacing phase 3's placeholder); subst_polytype + lookup_struct/lookup_enum (lowering); poly_mentions_len_var guard widened; position-ordering convention established; unit + lowering tests", "effort": "L", "difficulty": "tricky" },
    { "phase": 6, "focus": "R8b: match_impl_target_rec Generic arm, the three collect_* specificity/overlap functions, and the new generic_len_args_of helper wired into all three (impl-target matching), depending on phase 5's widened struct_instantiation_of/enum_instantiation_of signature; all five integration goldens (pinned observables, corrected impl syntax, the redesigned signature-unification fixture and its new negative/mismatch golden); full mutation recipe (15 entries) executed; bookkeeping and growth-signal re-run", "effort": "L", "difficulty": "tricky" }
  ]
}
```
