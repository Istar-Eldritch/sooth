## What shipped

**R1 — a header binds one or more type variables; every bound variable must be used.**
`type: Box 'T | val 'T ;` and `type: Pair 'A 'B a 'A b 'B ;` parse. A field naming an
unbound variable is a located error; so is a *phantom* variable (bound, referenced by no
field, e.g. `type: Phantom 'T x i64 ;`). Phantom rejection is what makes R5's dispatch
claim unconditional: two instantiations of a phantom parameter have constructors with
identical inputs, differing only in output type, which operand-type overload resolution
cannot disambiguate. A `'`-prefixed word is also rejected at every field-*name* position,
so the header/body reading of `'x` is consistent (`type: Foo 'bar i64 ;` would otherwise
read as generic while `type: Foo x i64 'y i64 ;` took `'y` as a field name).

**R2/D6 — application syntax is bracketed: `Box[i64]`, `Pair[i64 bool]`.** Type arguments
are always explicit; there is no inference. Applications parse at a field-type position
and at a word-signature slot (`: unwrap ( Box[i64] -- i64 ) ;`). The juxtaposed form the
original spec preferred was falsified by R3: in a slot list, `( Box i64 bool -- )` reads
identically as over-applied `Box` and as `Box i64` beside a `bool` slot, so
over-application is undecidable there; greedy argument parsing would instead make that
slot list unwritable. Brackets match ROADMAP's own spelling (`Option['T]`) and the type
sublanguage's existing delimiter. The *header* stays juxtaposed (`type: Result 'T 'E`),
where `'`-prefixing keeps the variable list unambiguous.

**R3 — wrong argument count is a located error** naming the generic type, the declared
variable count, and the supplied count, raised at the same parse/resolve site as the
application syntax.

**R4 — distinct applications mint distinct `StructId`/`EnumId`s; the same one dedupes.**
Dedup is keyed on `(generic decl index, instantiating module, Vec<Type> args)` compared by
`Type`'s own `Eq` — *not* on the rendered instantiation name. The name is built from
`Type::name()`, which is module-blind, so two distinct structs sharing a bare name across
modules render identically; deduping on the string would collapse them into one `StructId`
with the wrong layout.

**R5/D7 — an instantiation behaves like a hand-written concrete `type:`.** Layout,
destructor synthesis, and every existing struct/enum check apply unchanged. Two changes
were needed, not the one the spec first named:

1. `struct_generated_sigs`/`enum_generated_sigs` key generated words on the **surface**
   name (`generic_surface_name`, strips the `[...]` suffix: `Box`, `Ok`), while
   `Overload::symbol` keeps the mangled per-instantiation spelling so QBE symbols stay
   distinct. Without this, env keys were `Box[i64]>val` — unspellable, since `[` is a lexer
   delimiter.
2. Registration changed from `env.insert(name, vec![Overload{..}])` (overwrite) to
   `env.entry(name).or_default().push(..)`, matching the user-word path. On its own this is
   unobservable (under mangled names no two instantiations share a key); it only becomes
   load-bearing given (1).

Three further checker/lowering sites resolved a struct or variant by name and had to go
through `generic_surface_name` too: `src/ir/layout.rs`, `src/check/word_families.rs`, and
`check_struct_peek_word` (the `S|>fi` path, missed by phase 3's first exit golden and
pinned afterwards by `generic_instantiation_peek_word_dispatches_by_surface_name`).

**R6/D1 — no open nesting.** A concrete application inside an ordinary struct's field
(`type: Wrap x Box[i64] ;`) works. `type: Outer 'T x Box['T] ;` is out of scope.

## Non-functional outcomes

- **No new `Type` variant.** Variable-bearing field lists live in a separate
  `GenericStructDecl`/`GenericEnumDecl` pair holding `ty_var_names: Vec<String>` and
  `PolyType`-shaped fields; the concrete registries keep exactly their existing shape.
- **Deterministic instantiation names, spelled structurally** (`type_instantiation_name`,
  `Box[i64]`), the way `ArrayDecl` names a shape `[i64 4]` — not
  `instantiation_symbol`'s `sooth_mono_{name}__t{id}_{ty}`. Reasons: a struct's QBE symbols
  come from its `StructId` (`struct_drop_symbol`); the one QBE-facing use of the name (the
  aggregate `type :Name`) is sanitized injectively at the emission site, whereas
  `instantiation_symbol`'s sanitize is lossy enough for two argument lists to collide; and
  this name renders in every diagnostic naming the type. A struct/enum argument whose bare
  name is shared by more than one declaration gets an id suffix (`P.3`); wrapped arguments
  (`&P`, `^P`, `[P 4]`) are rebuilt from their registry entries so the tie-break lands at
  the leaf where the ids live. Order-independence holds because the pre-pass registries are
  fixed before any body parses.
- **`module: u32` set explicitly** on every minted declaration (D4: the instantiating
  module). Generic headers also participate in duplicate-type-name checking.
- **No regression:** existing `parse_typedef_*`/`check_struct_*`/`check_enum_*` tests and
  all golden `.sth` files pass unchanged.

## Decisions

- **D1 — no generic-in-generic nesting.** Fields are concrete or a bare variable. A
  concrete application inside a non-generic struct is fine.
- **D2 — N type variables from the start, no bounds.** Result needs two; the grammar
  generalizes at the same cost. `Copy`/`Ord` bounds deferred.
- **D3 — no recursive self-reference.** `List`/`Vec`-shaped generics are Phase 6; proving
  them needs the pre-pass sequencing fix (registering a generic name before its own body
  can reference it) that nothing here needs.
- **D4 — single-module only.** `GenericTypes::find_struct`/`find_enum` match on
  `(name, module)`.
- **D5 — minting is parse-time through a `&mut`-threaded side registry** (`GenericTypes`,
  threaded through `parse_bodies` beside `arrays`/`owned_cells`/`refs`), not inside
  `resolve_type` (which holds `&self` over an immutable `&[StructDecl]`) and not at
  check-time like `intern_bundle_struct` (too late for a struct field's type).
  `struct_base`/`enum_base` are the post-pre-pass registry lengths, so an instantiation's
  id is final the moment it is minted.
- **D6 — bracketed application arguments** (see R2).
- **D7 — an instantiation carries both a mangled name and a surface name** (see R5).

## Out of scope

`Result`/`Either`/`Option`, `?` sugar, branch-on-result codegen (Slice 2); open nested
applications (R6); recursive generics; variable bounds; cross-module generic
import/instantiation; the default-allocator parameter (`Vec['T 'A = Global]`, Phase 6).

## Verified by

`tests/phase5_slice1.rs` (declared-but-unused generic struct and enum build clean; two
struct instantiations reach the backend and run; enum instantiation runs; wrong argument
count errors; two instantiations sharing a surface name dispatch correctly; peek-word
dispatch; destructor runs like a concrete type's; application at a word-signature slot),
plus unit tests for dedup and base offsets (`instantiate_struct_dedups_and_counts_from_its_base`
and its enum twin), name determinism and cross-module disambiguation
(`type_instantiation_name_*`, `instantiate_struct_distinct_across_modules_same_bare_name`,
`instantiate_struct_distinct_for_wrapped_cross_module_args`), and the parse-side
unbound/phantom/duplicate-variable errors. Every new test was mutation-tested.

Delivered in three phases: (1) generic header parsing and the variable-scoped field list;
(2) application syntax, the parse-time instantiation table with structural dedup, and the
argument-count error; (3) surface-name keying plus overload-append registration, with the
goldens.

## Prerequisites this slice hands to Slice 2

Neither blocks Slice 1; both must be named in Slice 2's own spec or a small slice ahead of it.

1. **Cross-module generic instantiation.** ROADMAP's Phase 5 exit criterion says
   `Option['T]` must be importable from `core`. Importing a generic *declaration* and
   instantiating it elsewhere cannot ride the concrete-type import path, which only ever
   imports an already-concrete type. D4 keeps Slice 1 single-module; this is not free.
2. **Generic enums can be constructed but not eliminated.** Three mangled-name comparisons
   block a clause-style body over a generic enum's variants (`is_variant_name` in
   `src/parser.rs`, `is_registered_variant` in `src/check.rs`, the clause matcher in
   `src/check/word_entry.rs`), and fixing them is not sufficient: `prepass_type_decls` skips
   a generic header, so the parse-time clause-vs-locals discriminator never learns the
   variant names. A clause body over `Res[i64 bool]`'s `Ok`/`Err` parses as a locals binding
   and fails with a name collision. Slice 2's `Result 'T 'E` is an enum whose whole point is
   elimination.
