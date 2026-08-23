# Spec: P7.S3e user-declarable trait bounds

**Status:** Implemented (4 phases, all landed on `impl/slice3e_spec-2608230301`)
**Discovery:** `docs/roadmap/P7/slice3e-brief.md`

## Problem

Bounds were a closed set of two: `Copy` (via `poly_is_copy`) and `Ord` (a hardcoded
numeric predicate). Neither can express a user-defined relationship, so a struct or enum
key was unwritable. This slice opens `Bound` to `User(TraitId)`, satisfied nominally via
an `impl:` block.

The core problem is a phase split: the *obligation* (which trait, which member) is known
on the abstract body walk; the *symbol* (which concrete word) is known only once a call
site's substitution θ is concrete. Resolution happens at check time, never at lowering.

Forcing consumer: the array form of `sort` (`'T: Copy Order`, `cmp` as `Order`'s member),
Program 2 of `docs/roadmap/P7/slice3-dogfood.md`. `Map['K 'V]` out of scope (P7.S3n).

## Design rulings

1. **`impl:` is a binding, not a body.** `impl: Order for Point  cmp point-cmp ;` maps
   member names to existing, already-declared concrete words. Bare pairs (no `| ... |`,
   which means locals/selective-import elsewhere), mirroring `type:`'s `field Type` and
   `extern:`'s `name ( sig ) "symbol"`. Multi-member traits repeat the pair per member.
   The implementing word may share the member's name; that is ordinary static overload
   resolution. Consequence: impl checking is signature comparison, not body checking, and
   there is no impl-body lowering path to invent.
2. **A member's implementing word must be concrete,** rejected at the `impl:` site, not
   at a later call site. An impl member's own body calling a poly word stays legal
   (ordinary monomorphization).
3. **Trait identity is a flat whole-program index,** mirroring `StructId`/`EnumId`:
   `TraitId` indexes `Module::traits: Vec<TraitDecl>`, each carrying `module: u32`. A
   per-module id cannot name a cross-module trait.
4. **Export/import is symmetric with `type:`/`extern:`,** no implicit-export exception.
   The gate is `exportable_names` (the per-kind name list an `export:` line is validated
   against), not `not_exported_error` alone, which is downstream. The orphan rule (an
   `impl:` lives in the trait's or the type's declaring module) is a `module: u32`
   comparison.
5. **Multi-bound member-name collision is a call-site rejection, not a
   declaration-site one.** `'T: A B` stays legal even when both declare `t1`; the
   ambiguous unqualified *call* is the error. The dispatch branch is already walking the
   bound set, so finding two is the natural error site.
6. **Module-qualified calls disambiguate only across modules.** `o::t1` reuses the
   existing import-alias `::` mechanism (a *module* alias, not a trait namespace).
   Same-module collisions have no escape hatch this slice; no trait-name qualifier
   syntax was added.
7. **Bound-directed dispatch and ordinary `env.get` lookup partition through three
   pre-existing barriers,** not one: `poly_var_to_concrete_error` (non-builtin-named
   call), `poly_delegate_op`'s concrete-suffix truncation (builtin/operator-spelled
   name), and `poly_op_on_variable_error` (a `Ref(Var, _)` operand). All three were hard
   errors before this slice, so no concrete overload was ever reachable from these
   shapes; all three are pinned by test.
8. **Lowering makes no resolution decision.** Impl members lower as ordinary concrete
   words through the existing word path. A check-time-resolved map rides `CallInst`.
9. **A trait member is callable via a bound without importing the implementing word's
   name.** Dispatch reads the trait table (signature) and the whole-program impl registry
   (symbol, keyed by `(TraitId, Type)`), neither scoped by the caller's imports, mirroring
   whole-program type-directed `drop` overload dispatch. The orphan rule keeps this
   coherent. Calling the same word by name still needs an ordinary import.
10. **Obligation recording is order-independent.** `check_poly_body` and
    `check_poly_call` interleave in one source-order loop, so a monomorphic caller
    declared before its poly callee would see an empty obligation list. Closed by
    hoisting `check_poly_body` itself into a pre-pass (not a second parallel walk that
    would have to agree with it forever). The hoist *replaces* the in-loop call, so
    `builtin_overloads`/`slices` are recorded once.
11. **A bound's trait name resolves at parse time,** not via `Resolver::rewrite`:
    `parse_capabilities` bakes `Bound::User(TraitId)` before `rewrite` ever runs, so
    there is no token left to rewrite. It reuses `resolve_type_or_apply`'s
    `self.imports`/`type_is_exported` gate. A qualified *member call* needs no new
    `rewrite` branch (an unrecognized qualified word already falls through to
    `Ok(None)`, left raw for the checker).
12. **A qualified member call colliding with an exported-or-re-exported same-named
    concrete word is a rejection, not a resolution.** `rewrite` mutates the name in place
    before `check::check` runs, so nothing downstream can pre-empt the mangling, and
    nothing downstream knows a trait member was intended. The observable diagnostic is
    the pre-existing `poly_var_to_concrete_error`/`poly_op_on_variable_error`, and only
    when the bound variable itself is an operand: if every operand at the call site is
    `PolyType::Concrete`, the mangled name type-checks and silently calls the unrelated
    word. "Trait wins" would need a new branch inside `rewrite`; not added.
13. **A bound on a poly *combinator*'s own type variable is a located rejection.** A
    combinator is checked via `check_poly_combinator_standalone`; its instantiation
    records go to a scratch map, never `module.instantiations`, so there is no `CallInst`
    for an obligation to live on. Tracked as **P7.S3o**. A combinator *body* calling a
    bounded poly word does resolve, and is tested.

## What shipped

**AST (`src/ast.rs`).** `TraitDecl { name, module, kind, span, members }` with
`TraitMember { name, sig }`; `TraitKind::{Predicate(Bound), User}`, where `Copy`/`Ord`
are `Predicate` entries seeded by `seed_predicate_traits()` carrying a sentinel module
that collides with a user `trait: Copy` in any module. `ImplDecl { trait_id, target_ty,
module, span, bindings }`. Flat `Module::traits`/`Module::impls`. `Bound::User(TraitId)`.
`CallInst::trait_calls: HashMap<Span, String>`.

**Parser (`src/parser.rs`).** `parse_trait_decl`, `parse_impl_decl` (bare pairs),
`prepass_trait_decls` run whole-closure before the per-module `parse_bodies` loop
(mirroring `prepass_generic_typedefs`, without which a cross-module bound cannot
resolve). `parse_capabilities` is one trait-table lookup instead of two string compares,
producing `Bound::Copy`/`Bound::Ord`/`Bound::User` from the same path; an unbound
qualifier in a bound gets its own located diagnostic. Bounds still ride a variable's
first (binding) occurrence, unchanged.

**Declarations (`src/check/declarations.rs`).** `colliding_name_kind` generalized off
`StaticDecl` to a kind-tagged name/module/span with a parameterized diagnostic, plus a
new `check_trait_decls` call site scanning words/externs/structs/enums/statics **and
other traits** (pre-mangle, since the struct/enum arm compares `name_static`).
`check_impl_decls` validates signature match, the orphan rule, completeness, duplicate
`(TraitId, Type)`, a polymorphic implementing word, and — beyond the original spec — a
member bound to a `drop` overload. `local_decl_names` and `exportable_names` each gained
a `traits` arm.

**Check (`src/check.rs`, `src/check/poly.rs`).** The R17 pre-pass hoists
`check_poly_body` for non-combinator poly words into `Vec<WordObligations>` before the
main loop; the combinator branch stays in-loop and calls
`reject_user_bound_on_combinator`. `poly_call_term` gained a branch ahead of its
`env.get(name)` lookup: a bare or ref-to-bare bounded variable whose bound set declares
the called member is an obligation, pushing the trait's abstract outputs and recording
`(span, var, trait, member)`; two matching bounds is the ambiguity rejection unless a
qualifier disambiguates. `check_poly_call`'s bound loop gained the `Bound::User` arm:
look up `(trait_id, θ(v))`, emit the located unsatisfied-bound diagnostic naming the
missing member's full signature, else resolve the mangled symbol into the instantiation's
`trait_calls`. An obligation the pre-pass recorded but resolution cannot match is a
located internal-consistency error; an ungrounded variable (the pre-existing `ty_of`
skip) is special-cased so the backstop cannot trip on it. Also landed: member-output
enforcement and bound dedupe. `trait_calls` is a pure function of `(callee, θ)`, never of
the caller, because the symbol-dedup step picks arbitrarily among equal-`(callee, θ)`
call sites.

**Lowering (`src/ir/`).** The compiled-path `for (symbol, inst) in distinct` loop reads
`inst.trait_calls` and threads it into `lower_word_parts`/`FuncBuilder` alongside
`builtin_overloads`; other callers pass `empty_trait_calls()` (`src/ir.rs`), the existing
`empty_builtin_overloads()` convention. `uncalled_operator_overloads` also spares a
symbol appearing in some instantiation's `trait_calls`, byte-matching
`overload_symbols`'s spelling, or a bound-only-reachable member named after a builtin
operator would be silently pruned. No new pass, no new `IrFunc` path. `lower_instantiation`
is REPL-only and deliberately does not populate this field: the REPL gains no
bound-directed dispatch coverage, the same bypass already true of drop/operator overloads.

## Out of scope (unchanged)

Trait objects / runtime dispatch; associated types, default bodies, blanket impls,
supertraits, generic constants; multi-type-variable traits; the compiler-known third
trait kind (`Fallible`/`bool`-shaped); `Map['K 'V]`; a trait-name-based qualifier for
same-module collisions; `'T: Trait` on a combinator's own variable (P7.S3o).

## Invariants to preserve

- Resolution happens exactly once, in `check_poly_call`, with θ in hand. Lowering never
  re-resolves.
- `trait_calls` keys off the callee's own body spans and θ only.
- Do not reuse `poly_calls_poly_word_error` here: the shape it would guard cannot arise
  under impl-as-binding.
- Do not model a bound's trait name as a `Resolver::rewrite`/`NameTables` branch.
- Do not route `trait_calls` through `lower_instantiation`.
- The R17 pre-pass replaces the in-loop `check_poly_body` call; running both
  double-records `builtin_overloads`/`slices`.
- The trait pre-pass runs *before* the per-module `parse_bodies` loop.
- `check_trait_decls` must stay pre-mangle.

## Tests

`tests/phase7_slice3e.rs` (single file, matching every landed P7.S3 slice) plus unit
tests beside each touched function. Pinned behaviours: cross-module export;
local-vs-selective-import collision; cross-kind collision (`trait: Point` alongside
`type: Point`); `'T: Copy Ord` still parsing byte-identically; the pre-pass's
monomorphic-caller-declared-first ordering; generic-struct minting observed exactly once
after the hoist; all three R10 barrier shapes including an operator-spelled member; the
qualified-collision rejection asserted against the real diagnostic with the bound
variable as an operand; the combinator-bound rejection; a bounded call inside a
combinator body resolving; the R15 dead-code golden in a compiled build; and the `sort`
consumer at two distinct instantiations, each golden distinguishing which impl's `cmp`
ran.
