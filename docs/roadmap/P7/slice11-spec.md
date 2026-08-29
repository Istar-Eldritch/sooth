# P7.S11 -- Generic construction inside an inline combinator's standalone check

**Status:** Done
**Discovery:** `docs/roadmap/P7/slice11-brief.md`

Implemented on `main` (base `f4dba7d`, merged at `1abd565`):

- `0ef853f` feat(poly): word-scoped registries for combinator monomorphs (R1/R2/R2.1/R6)
- `d3aeac7` fix(phase-1): witness R6 cells, isolate R3 twin, rename golden 8
- `f5fd01d` feat(phase-1): top-level generic-input slot guard (R3)
- `169c93a` fix(phase-2): resolve freshly minted R4 env signatures
- `c94eec9` fix(phase-2): witness R4's struct half, correct spec mutation table
- `1b0e6c7` fix(phase-2): align mortality rows, golden 10 measurement witness
- `55832d2` docs(phase-3): mutation recipe measured, mark slice done
- `1abd565` docs: fix stale spec/roadmap prose

Test file: `tests/phase7_slice11.rs`; stage unit tests in `src/check/poly.rs`
and `src/check/declarations.rs`. Roadmap entry: `docs/roadmap/P7-language-prereqs.md`
(`[ done ]`). Follow-up carved out here: `P7.S11-follow` (frozen call-site env,
see Out of scope).

## Problem Statement

An **unbounded** `inline` combinator (a quotation-taking generic word with no
`Bound::User`, e.g. `Option`'s `map`/`and_then`, `Result`'s `wrap`) is checked
once, standalone, by `check_poly_combinator_standalone`, against `i64` stand-in
types rather than per call site. That function built its `Ctx` with
`generics: None`, so `apply_subst` on any declared slot whose shape is
`PolyType::Generic` hit the `ctx.generics()` `None` arm and returned
`poly_generic_not_yet_groundable_error`. Confirmed live before the fix:

```sooth
type: Result['T 'E] | Ok 'T | Err 'E ;
: wrap inline ( 'T ~[ 'T -- 'U ] -- Result['U i64] ) call Ok ;
: main ( -- ) 7 ~[ 1 + ] wrap drop ;
```

```text
error: `wrap` in `wrap` (line 2) names the generic type `Result['U i64]`, which
cannot yet be instantiated at a variable-bearing application
  grounding a generic over its own type variable is not yet implemented
```

(op is the word name, not `Ok`: the rejection fires in the standalone
output-slot `apply_subst` loop before the body is walked at all.)

This slice lifts the restriction **at the definition site**, for a combinator
whose declared *output* (or a `Generic` nested inside a declared quotation-input
effect, an array element, a referent or a cell payload) applies a generic header.

### Honest deliverable (a separate, pre-existing blocker not fixed here)

Lifting the def-site check does **not** make an arbitrary such combinator
callable, because the **word env is frozen before any check-time mint**.
`check_module` builds `env` from the generated sigs over `module.structs`/
`module.enums` once, before the word loop. A monomorph minted *during* the word
loop never gets its constructors into `env`, so when the `inline` body is
spliced into a concrete caller, a bare `Ok` in the body is an unknown word
unless a **parse-time** instantiation of the same header already existed.

So the slice's deliverable is: **the def-site gate is lifted, and a combinator
whose header already has a parse-time monomorph in the program becomes usable
end to end.** The frozen-env gap is a named follow-up (`P7.S11-follow`), not
silently absorbed.

## The three root causes

- **P0-A, no rebase.** The standalone call site had no rebase/flush bracket, so
  a `GenericTypes` value taken from that scope carried the base of whatever the
  last bracket set, and a fresh mint could hand out an `EnumId` an earlier word
  already flushed. (Live, not theoretical: the word loop is a *single* loop, so
  any earlier monomorphic word that grounds a generic leaves `enum_base` stale.)
- **P0-B, no flushed decl and a slice-position sig id.** The concrete body walk
  indexes `enums` unconditionally (`is_copy`, `contains_reference`, `tag`,
  captures, drop-graph); a minted-but-absent monomorph panics. Separately, the
  generated-sig helpers derive the output type from the **slice position**, so a
  one-element slice yields id 0, an unrelated enum.
- **P0-C, a scratch-minted id leaking into the live shape registries.** See R6.

Fixed by one move throughout: do what every other check-time grounding path does
(rebase, mint, **flush**), but flush into *word-scoped extended copies* of the
registries that only this one `check_word` call sees, and intern the body's and
signature's shapes into word-scoped copies too.

## Design Rulings

### R1. Thread the live `GenericTypes` cell into the standalone check

`check_poly_combinator_standalone` takes a new
`generics: Option<&RefCell<GenericTypes>>`; `check_module` passes
`Some(&generics_cell)`. The `Option` only matches the shared shape of the other
paths (`check_poly_body`); the sole production caller passes `Some`, and under
`None` the function behaves as before.

### R2. Rebase into a scratch registry, mint, flush into word-scoped extended slices

Inside the standalone check, in order:

1. **Scratch clone + rebase** of the `GenericTypes` cell onto the live
   registries' current lengths (P0-A's fix). Build `ctx` with `Some(&scratch)`;
   `ctx`'s `structs`/`enums` stay the live, unextended slices (the read path for
   a not-yet-flushed id is `GenericTypes::enum_decl`).
2. **Ground.** The existing input/output `apply_subst` loops now mint into the
   scratch cell.
3. **Flush into word-scoped copies** cloned from the live slices. After the
   rebase every minted id equals its index in the extended vector (P0-B's fix).
4. **Hand the extended slices to the body walk** (`check_word`). Unconditional:
   no "nothing was minted" branch (one more thing a mutation could satisfy).
5. **Discard.** The scratch and every word-scoped vector drop on return, error
   path included. The live `generics_cell` and `module.structs`/`module.enums`
   are untouched.

A scratch *clone* (not the live cell) is required: rebasing the live cell would
leave its private dedup entry pointing at an `EnumId` never flushed into
`module.enums`, a dangling id a later real call site would dedup onto.

`#[derive(Clone)]` is added to `GenericTypes`, `EnumDecl`, `VariantDecl`,
`ArrayDecl`, `OwnedCellDecl`, `RefDecl`, `SliceDecl`.

### R2.1. The testable seam: `ground_into_word_scoped_registries`

R2's steps 1/3/5 plus R6's copies are extracted into one `pub(super)` free
function in `src/check/poly.rs` that takes the grounding work as a closure and
returns `(value, WordScopedRegistries)`. It exists **for testability**: without
it, the word-scoped registries are function-locals with no observable, and
P0-A's guard has no unit-level witness. The closure argument enforces ordering:
rebase before the mint, flush after.

`WordScopedRegistries` holds word-scoped copies of `structs`, `enums`, `arrays`,
`cells`, `refs`, `slices`.

### R3. New guard: a declared *top-level* generic input slot stays rejected

New work. Before the input loop, walk `sig.inputs`; for any element that is
*itself* `PolyType::Generic`/`GenericVariant` at the **top level of the slot**,
return `poly_generic_not_yet_groundable_error` with the byte-identical message.
A `Generic`/`GenericVariant` **nested** inside a slot (a quotation-input
effect's rows, an array element, a ref/cell referent) is **not** rejected and
grounds normally. Declared **outputs** are never checked by this guard.

*Why nested must be allowed:* the acceptance fixture `relay` carries
`~[ 'T -- Result['T i64] ]` as an input slot and must ground. *Why top-level
must be rejected:* S12's standing decision that a combinator over a generic-enum
slot is unsupported (`tests/phase7_slice12.rs`
`a_combinator_over_a_generic_enum_slot_is_rejected_before_r15_can_fire`). R6
makes *nesting* safe at lowering; R3 still rejects the top level.

### R4. Register the grounded monomorphs' generated sigs into a word-scoped env

After R2's flush, if anything was minted, build `local_env = env.clone()` and
append the generated sigs (`struct_generated_sigs`, `enum_generated_sigs`,
`variant_generated_sigs`) for the **newly flushed tail only**, skipping the
first `helper(base_slice).len()` entries (append-not-insert, matching
`check_module`). Pass `&local_env` to `check_word`.

**The prefix skip is index-exact by iteration order, not per-decl count.** The
helpers push **one entry per variant** (not per decl), so a fixed per-decl count
is false; but each is a single in-order `push`-only loop, so the extended
slice's first `base.len()` iterations visit the same decls at the same `idx` and
push the same entries. The base's sigs are exactly the leading
`helper(base_slice).len()` entries. Compute that length by calling the helper on
the base slice, never by arithmetic on decl counts.

**Ids are right** because after R2's rebase-and-flush the slice position *is* the
minted id.

**Reach:** R4 inserts one env entry per variant (and, for a struct, both a constructor and a
destructure) of every monomorph the *signature* grounding minted. A body term
naming one of those variants resolves, including an intermediate construction of
the same monomorph. A construction over a *different* header, which the
signature never grounds, gets no entry and still fails as an unknown word absent
a parse-time sibling.

### R5. `poly_generic_not_yet_groundable_error` keeps both its jobs

R3's guard reuses the message verbatim (same `op` = word name), so S12's
assertions are unchanged. The `apply_subst` `None`-arm call sites stay: they
remain the shared-parameter default for any caller passing `None`.

### R6. Word-scoped `arrays`/`cells`/`refs`/`slices` too (P0-C)

Leaving these live is **not** inert; it is a lowering panic. Once R1 threads the
cell in, `apply_subst`'s `Array` arm interns an `ArrayDecl` whose `element` is a
scratch enum id into `module.arrays`; R2 discards the scratch, so the id is
never in `module.enums`. Lowering walks every array unconditionally
(`build_registries_ww` -> `ensure_array` -> `size_align`'s `Type::Enum` arm ->
`ensure_enum` indexing `enum_memo`): dangling id -> index out of bounds, on a
program that type-checked cleanly. `Ref`/`OwnedCell` are lower risk (their
layout consumers go through `ir_type_of`, a pure map that indexes no registry)
but still leak a forgeable id a later `intern_*_type` structural scan can dedup
onto; fixed for the same three lines.

**No rebase and no flush for these.** `ArrayDecl`/`RefDecl`/`OwnedCellDecl`/
`SliceDecl` are plain `Vec` entries interned by a linear structural scan with
the id fixed at `vec.len()`; they have no base. Cloning the live vector and
interning into the clone already yields ids equal to their index. The
word-scoped copies are handed to the grounding closure and `check_word` as
`&mut` and simply dropped.

**Ruling:** the standalone check uses `WordScopedRegistries`' `arrays`/`cells`/
`refs`/`slices` for both signature grounding and the body walk. The live params
become `&[..]` reads.

**`slices` is included** though `apply_subst` never interns a slice: a body that
builds an array of the monomorph and takes a view interns a `SliceDecl` whose
element is the scratch enum id, and `build_slices` matches
`IrType::Enum(id) => &enums.layouts[id.index()]`, the same out-of-bounds shape.

**Ordinary (non-grounding) body shape construction is harmless.** A combinator
mints no `IrFunc` and its body is spliced into each concrete caller, where the
concrete walk re-interns any shape the program needs into the live registries. A
shape only the standalone pass ever interned is dead. Ids are carried by value,
so nothing computed inside the standalone check outlives its local vector. The
one observable difference is registry *ordering/count* in `module.arrays`; this
materialized as seven QBE baselines each losing an unreferenced, declaration-only
`type :arr_N` line (verified benign by grep). The pass's doc comments
(`src/check/poly.rs`, `src/check.rs`) state it records nothing that survives it.

**Asymmetry to note:** for `enums`/`structs` the danger is a decl *missing* from
the slice the body walk sees (P0-B, immediate check-time panic); for
`arrays`/`cells`/`refs`/`slices` the danger is a decl *present* in the live
registry that should not be (P0-C, lowering-time panic).

## Out of scope

- **The frozen call-site env (`P7.S11-follow`, discovered here).** A monomorph
  minted at check time never enters `env` (built once before the word loop). An
  `inline` combinator constructing a generic output is only callable if a
  **parse-time** monomorph of the same header exists in the program; the
  `Option`-shaped-`map`-in-a-library-file case still does not build. Fixing it
  needs an env extensible mid-word-walk, since the mint and the lookup happen
  inside one `check_word` call; a separate slice.
- **An output-only type variable** (`'U` only in the outputs). Blocked by an
  unrelated inference gap, unrepairable by explicit instantiation (a combinator
  call rejects type arguments).
- **A top-level generic *input* slot** (`Option['T]` parameter). Deferred (R3);
  S12's rejection stays live.
- **Bounded (`['T: Foo]`) combinators constructing a generic enum.** They take
  the poly-body pre-pass, not the standalone path, and are rejected by S12's
  `poly_combinator_generic_enum_construction_error`.
- **Eliminating a generic enum inside a combinator body**, field projection, and
  `dup`/`over`/`unify_poly_input`'s consuming side over a generic-typed value.
- **Intermediate construction over a header the signature never grounds.** Still
  fails as an unknown word absent a parse-time sibling; intermediate
  construction of the *same* monomorph the signature grounds **is** admitted.
- **A `Refs`/`Cells` consumer that indexes by id at lowering.** None exists
  today (R6), so R6's fix for those two ships with unit coverage and no golden.
- `docs/book/` (uncompiled) and `tree-sitter-sooth/grammar.js`.

## Open Questions

All resolved during implementation:

- *Small threading fix or distinct machinery?* Distinct machinery: rebase, mint, then
  flush into word-scoped extended registries (R2/R2.1/R6) plus a word-scoped
  env (R4). Threading alone panics or reports an unknown constructor.
- *Per-splice re-check sufficient, or must the standalone pass ground itself?* It
  must ground itself (R1/R2/R4). Because it records no monomorph and never
  lowers, R1.5's `Span`-keyed recording concern does not apply. The per-splice
  recheck *is* the authority over ordinary shape construction, which is why R6's
  discard is safe.
- *Do the shape registries need a rebase?* No. They intern by linear structural
  scan with the id fixed at `vec.len()` and have no base (R6). Cloning suffices.

## Exit criteria

Each is a golden or a named unit test (in parentheses).

- A `Result`-constructing unbounded `inline` combinator with a parse-time
  sibling monomorph builds and runs, stdout `8\n` (golden 1).
- A constructor-free unbounded `inline` combinator whose declared output and
  quotation-input effect both apply a generic header builds and runs, stdout
  `7\n` (golden 2).
- A top-level generic *input* slot still fails with `cannot yet be instantiated
  at a variable-bearing application`, and `tests/phase7_slice12.rs` stays green
  (golden 3).
- A standalone body that type-directs on its grounded output reports a
  diagnostic and does not panic (golden 4).
- A combinator whose declared output is an **array of** its grounded monomorph
  builds and does not panic at lowering (golden 8, compile-only); the live
  `arrays`/`cells`/`refs` are unchanged by the check (stage unit test).
- A standalone mint that follows an earlier *check-time* mint lands at the right
  id (golden 9, and
  `standalone_grounded_output_id_matches_its_extended_slice_position`).
- The discovered boundaries are pinned, not hidden: output-only `'U` still
  uncallable (golden 5), a check-time monomorph's constructors still absent from
  the call-site env with the failure naming `main` (golden 6), a different
  header's constructor still unknown (golden 7). R4's struct half is witnessed
  (golden 10).
- The live `GenericTypes` cell and `module.enums`/`module.structs` are unchanged
  by a standalone check, asserted through the public `lookup_enum`
  (`standalone_stand_in_monomorph_does_not_enter_the_live_registry`).
- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green.
