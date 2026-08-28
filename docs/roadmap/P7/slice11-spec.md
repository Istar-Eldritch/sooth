# P7.S11 -- Generic construction inside an inline combinator's standalone check

**Status:** Planned
**Discovery:** `docs/roadmap/P7/slice11-brief.md`

> Revision note (round 3, 2026-08-29). Round 2's rebase + mint + flush into
> word-scoped `local_enums`/`local_structs` is retained unchanged: three
> independent reviews verified the rebase ordering, the id-to-slice-index
> correspondence and the sig-generation identity against the live source. This
> round changes four things and nothing else:
>
> - **R2's caveat that leaving `arrays`/`cells`/`refs` live is "inert" was
>   false.** It is a lowering-time panic. New **R6** gives the standalone pass
>   its own word-scoped shape registries (and `slices` too -- see R6.4).
> - **R4's stated rationale for the prefix skip was false** ("a fixed number of
>   entries per decl"). The mechanism is right; the reason is *iteration order*.
> - **Two unit tests were unimplementable** (they asserted against function
>   locals and against private `src/ast.rs` fields). Both are re-seated on real
>   seams: a new extracted free function, and the public `lookup_enum`.
> - **The mutation recipe named witnesses that cannot fail.** Fixed, with one
>   mutation gaining a new purpose-built golden and every row re-derived.
>
> Every line citation below was re-read against the working tree at `9b220b8`
> while writing this revision; the round-2 citations for
> `src/check/builtins.rs`, `src/check/captures.rs`, `src/check/drop_graph.rs`
> and `src/check/word_families.rs` still hold, the `src/check/poly.rs` and
> `src/ast.rs` ones were re-confirmed line by line.

## Problem Statement

An **unbounded** `inline` combinator (a quotation-taking generic word with no
`Bound::User`, e.g. `Option`'s `map`/`and_then`, `Result`'s `wrap`) is checked
once, standalone, by `check_poly_combinator_standalone`
(`src/check/poly.rs:407-420`), against `i64` stand-in types rather than per call
site. That function builds its `Ctx` with `generics: None`
(`src/check/poly.rs:426-434`, the `None` literal at `:433`, behind the stale
scope comment at `:422-425`), so `apply_subst` on any declared slot whose shape
is `PolyType::Generic` hits the `ctx.generics()` `None` arm
(`src/check/poly.rs:8055-8062`, and its `GenericVariant` twin at `:8109-8116`)
and returns `poly_generic_not_yet_groundable_error`
(`src/check/poly.rs:7919`). Confirmed live against `main`:

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
output-slot `apply_subst` loop at `src/check/poly.rs:450-456`, before the body
is walked at all.)

`check_module`'s combinator skip comment (`src/check.rs:811-836`) already names
this as the outstanding restriction to lift. This slice lifts it **at the
definition site**, for a combinator whose declared *output* (or a `Generic`
nested inside a declared quotation-input effect, an array element, a referent or
a cell payload) applies a generic header.

## Scope correction discovered while prototyping (read this first)

The prototype (R1+R2+R3+R4 as specified below) makes the **def-site** check
pass. It does **not** make an arbitrary such combinator callable, because of a
*separate, pre-existing* blocker this slice does not fix:

**The word env is frozen before any check-time mint.** `check_module` builds
`env` from `struct_generated_sigs`/`enum_generated_sigs`/
`variant_generated_sigs` over `module.structs`/`module.enums` once, at
`src/check.rs:568-576`, before the word loop at `src/check.rs:908`. A monomorph
minted *during* the word loop (by `apply_subst`'s `Generic` arm, then flushed at
`:873-874` / `:1012-1013`) therefore never gets its constructors into `env`.
When an `inline` combinator's body is spliced into a concrete caller, that
splice is walked by the caller's concrete walk against the same frozen `env`, so
a bare `Ok` in the spliced body is an unknown word unless some **parse-time**
instantiation of the same header already existed.

Measured with the prototype, on the `wrap` program above (minus the def-site
error, which is gone):

| program | result |
| --- | --- |
| `wrap` + a sibling `: mki ( i64 -- Result[i64 i64] ) Ok ;` | builds and runs, prints `8` |
| `wrap` with no sibling monomorph | fails with an unknown-word `Ok` error attributed to **`main`** (line 3) -- the **call site**, not the def site |

Before this slice both programs fail at the def site. So the slice's honest
deliverable is: **the def-site gate is lifted, and a combinator whose header
already has a parse-time monomorph in the program becomes usable end to end.**
The frozen-env gap is recorded in Out of scope as a named follow-up with its own
pinned witness, not silently absorbed.

## What already works (verified against source at `9b220b8`)

- `apply_subst`'s `Generic` arm (`src/check/poly.rs:8047-8088`) mints or finds a
  ground monomorph through the `GenericTypes` cell, keyed by `(header decl
  index, instantiating module, concrete args)` (`src/ast.rs:602-603`, both
  fields private). Grounding every type variable at `i64` turns `Result['U i64]`
  into `Result[i64 i64]`, and dedups against a real monomorph of the same key.
- `GenericTypes::instantiate_enum` mints `EnumId::from_index(self.enum_base +
  self.inst_enums.len())` (`src/ast.rs:1206`); `instantiate_struct` is its twin
  (`:1152`). `rebase` (`src/ast.rs:900-903`) re-points those bases;
  `flush_structs_into`/`flush_enums_into` (`:909-916`) move the batch onto a
  live registry. All three are `pub`.
- Every *other* check-time grounding path brackets its mint with rebase +
  flush: the poly-body pre-pass (`src/check.rs:840-874`, rebase inside
  `check_poly_body`, flush at `src/check.rs:871-875`) and the monomorphic word
  walk (`src/check.rs:992-1013`, rebase at `:992-994`, flush at `:1011-1013`).
- Monomorph constructors reach the concrete env through
  `struct_generated_sigs` (`src/check/declarations.rs:1716`),
  `enum_generated_sigs` (`:1753`), `variant_generated_sigs` (`:1786`), keyed by
  bare surface name with a mangled `Overload::symbol`.

## The three root causes

- **P0-A, no rebase.** The combinator-standalone call site (`src/check.rs:953`)
  has no rebase/flush bracket. A `GenericTypes` value taken from that scope
  still carries the base of whatever the *last* bracket set, so a fresh mint can
  hand out an `EnumId` that an earlier word already flushed into the live
  registry. Verified from `src/ast.rs:1206` plus the absence of any
  `rebase`/`flush` between `src/check.rs:936` and `:962`. **The stale base is
  reachable, not theoretical** -- see R2's "when the base is actually stale".
- **P0-B, no flushed decl and a slice-position sig id.** The concrete body walk
  indexes `enums` unconditionally at `src/check/builtins.rs:235` (`is_copy`) and
  `:309` (`contains_reference`), `src/check/captures.rs:170`,
  `src/check/drop_graph.rs:671`, `src/check/poly.rs:1623` and
  `src/check/word_families.rs:669` (`tag`). A monomorph that is minted but not
  present in the slice handed to `check_word` therefore panics. **Witnessed
  live** with the prototype's flush stubbed out, on

  ```sooth
  import: intrinsics * ;
  type: Result['T 'E] | Ok 'T | Err 'E ;
  : wrapd inline ( 'T ~[ 'T -- Result['T i64] ] -- i64 ) call tag ;
  : main ( -- ) 7 ~[ 1 add Ok ] wrapd . ;
  ```

  ```text
  thread 'main' panicked at src/check/word_families.rs:669:9:
  index out of bounds: the len is 0 but the index is 0
  ```

  With the flush in place the same program produces the correct diagnostic
  (`` `tag` requires an enum whose variants all carry no payload, found
  `Result[i64 i64]` ``). Separately, `enum_generated_sigs`/
  `variant_generated_sigs`/`struct_generated_sigs` all build their output type
  from `EnumId::from_index(idx)` / `StructId::from_index(idx)` where `idx` is
  the **slice position** (`src/check/declarations.rs:1719`, `:1755`, `:1794`),
  so a one-element slice yields id 0 -- an unrelated enum, not the grounded
  output.

- **P0-C, a scratch-minted id leaking into the live shape registries.** New this
  round; round 2 wrongly rated it inert. See R6.

Fixed by the same move throughout: do what every other path does -- rebase,
mint, **flush** -- but flush into *word-scoped extended copies* of the
registries that only this one `check_word` call sees, and intern the body's and
signature's shapes into word-scoped copies too.

## Design Rulings

### R1. Thread the live `GenericTypes` cell into the standalone check

`check_poly_combinator_standalone` (`src/check/poly.rs:407-420`) takes a new
`generics: Option<&RefCell<GenericTypes>>` parameter. `check_module`'s call site
(`src/check.rs:953-966`) passes `Some(&generics_cell)` (already alive in that
scope, bound at `src/check.rs:725`). The stale scope comment at
`src/check/poly.rs:422-425` is replaced by a comment stating R2's and R6's
rules.

The parameter is an `Option` only to match the shared shape of the other paths
(`check_poly_body`'s `generics`); the sole production caller passes `Some`.
Under `None` the function behaves exactly as it does today.

### R2. Rebase into a scratch registry, mint, flush into word-scoped extended slices

Inside `check_poly_combinator_standalone`, in this order:

1. **Scratch clone + rebase.** `let mut g = generics.borrow().clone();
   g.rebase(structs.len(), enums.len()); let scratch = RefCell::new(g);` The
   rebase is P0-A's fix and is the same call `check_poly_body`/`check_word`'s
   brackets make (`src/check.rs:992-994`). Build `ctx` with `Some(&scratch)`
   instead of the hard-coded `None`. `ctx`'s `structs`/`enums` stay the **live,
   unextended** slices: the mint has not happened yet when `ctx` is built, and
   `GenericTypes::enum_decl` (`src/ast.rs:924-928`) is exactly the read path for
   an id in the not-yet-flushed range, as `apply_subst`'s `GenericVariant` arm
   already documents at `src/check/poly.rs:8090-8100`.
2. **Ground.** The existing input and output `apply_subst` loops
   (`src/check/poly.rs:443-456`) are unchanged in *shape*; they now see
   `ctx.generics() == Some(&scratch)` and mint into it. Their `arrays`/`cells`/
   `refs` arguments change per R6.
3. **Flush into word-scoped copies.** `flush_structs_into(&mut local.structs)`
   and `flush_enums_into(&mut local.enums)` over copies cloned from the live
   slices. Because of step 1's rebase, every minted id equals its index in the
   extended vector. This is P0-B's fix.
4. **Hand the extended slices to the body walk.** The `check_word` call
   (`src/check/poly.rs:485-500`) receives `&local.enums` / `&local.structs`
   rather than `enums` / `structs`. When nothing was minted the extended copies
   are equal to the originals. The implementation must **not** branch on
   "nothing was minted" and pass the originals through: R6 makes the copies
   unconditional anyway, and a branch is one more thing a mutation can silently
   satisfy.
5. **Discard.** `scratch` and every `local.*` vector drop when the function
   returns, on the error path as well as the success path. The live
   `generics_cell` (which becomes `module.generics`, `src/check.rs:1088`) and
   `module.structs`/`module.enums` are untouched.

**Steps 1, 3 and 5 are extracted into a free function** so they are testable
without the surrounding machinery -- see R2.1.

**When the base is actually stale (P0-A is live, not hypothetical).** The word
loop at `src/check.rs:908` is a *single* loop: the combinator-standalone branch
is at `:911-966` and the monomorphic branch's rebase/flush bracket at
`:992-1013`. So any monomorphic word **earlier in source order** that grounds a
generic for the first time mints, flushes at `:1012-1013`, and leaves
`enum_base` pointing at the pre-flush length -- stale by exactly that batch's
size when the combinator's turn comes. Measured that such a mint really happens
in that loop:

```sooth
import: intrinsics * ;
type: Box['A] | Empty | Full 'A ;
: boxit ( 'T -- Box['T] ) Full ;
: seed ( -- ) 1 boxit drop ;
: main ( -- ) seed 0 . ;
```

builds and runs clean against `main` today, and the program spells `Box[i64]`
nowhere, so `Box[i64]` can only have been minted during the word loop. Golden 9
turns this into the mutation-4 witness.

**Derives this needs, named exactly.** `#[derive(Clone)]` must be added to:

| type | current derive | line |
| --- | --- | --- |
| `GenericTypes` | `Debug` | `src/ast.rs:589-590` |
| `EnumDecl` | `Debug` | `src/ast.rs:496-497` |
| `VariantDecl` | `Debug` | `src/ast.rs:510-511` |
| `ArrayDecl` | `Debug` | `src/ast.rs:1288-1289` (R6) |
| `OwnedCellDecl` | `Debug` | `src/ast.rs:1299-1300` (R6) |
| `RefDecl` | `Debug` | `src/ast.rs:1344-1345` (R6) |
| `SliceDecl` | `Debug` | `src/ast.rs:1394-1395` (R6.4) |

`StructDecl` (`src/ast.rs:443-444`), `GenericStructDecl` (`:541-542`) and
`GenericEnumDecl` (`:555-556`) are already `Clone`. The first three were
measured sufficient for R2 alone (`cargo build` clean); the last four are new
this round and are mechanical (every field is `Clone`: `String`, `u32`, `Type`,
`bool`, `&'static str`, `Vec<(String, Type)>`).

**Why a scratch clone and not the live cell.** Using the live cell with a
rebase would leave its (private) `enum_keys`/`enum_resolved` dedup entry
pointing at an `EnumId` that is never flushed into `module.enums` -- a dangling
id a later real call site would dedup straight onto. The clone is what makes the
discard safe.

### R2.1. The testable seam: `ground_into_word_scoped_registries`

R2's steps 1/3/5 plus R6's copies are extracted verbatim into one free function
in `src/check/poly.rs`, taking the grounding work as a closure. This exists
**for testability**: without it, `local.enums` is a function-local with no
observable, and P0-A's guard has no unit-level witness at all.

```rust
/// The word-scoped registry set a standalone combinator check runs against:
/// copies of the live registries, extended with whatever the signature
/// grounding minted. Dropped when the check returns (R2.5/R6).
pub(super) struct WordScopedRegistries {
    pub structs: Vec<StructDecl>,
    pub enums: Vec<EnumDecl>,
    pub arrays: Vec<ArrayDecl>,
    pub cells: Vec<OwnedCellDecl>,
    pub refs: Vec<RefDecl>,
    pub slices: Vec<SliceDecl>,
}

/// Rebase a scratch clone of `generics` onto the live registries' current
/// lengths, run `ground` against word-scoped copies of the shape registries,
/// then flush the batch `ground` minted into word-scoped copies of
/// `structs`/`enums`. Returns `ground`'s value and the extended registries;
/// on `Err`, everything is dropped and nothing reaches a live registry.
#[allow(clippy::too_many_arguments)]
pub(super) fn ground_into_word_scoped_registries<T>(
    generics: Option<&RefCell<GenericTypes>>,
    structs: &[StructDecl],
    enums: &[EnumDecl],
    arrays: &[ArrayDecl],
    cells: &[OwnedCellDecl],
    refs: &[RefDecl],
    slices: &[SliceDecl],
    ground: impl FnOnce(
        Option<&RefCell<GenericTypes>>,
        &mut Vec<ArrayDecl>,
        &mut Vec<OwnedCellDecl>,
        &mut Vec<RefDecl>,
    ) -> Result<T, String>,
) -> Result<(T, WordScopedRegistries), String>
```

Body, exactly:

1. `let scratch = generics.map(|c| { let mut g = c.borrow().clone();
   g.rebase(structs.len(), enums.len()); RefCell::new(g) });`
2. `let mut local = WordScopedRegistries { structs: structs.to_vec(), enums:
   enums.to_vec(), arrays: arrays.to_vec(), cells: cells.to_vec(), refs:
   refs.to_vec(), slices: slices.to_vec() };`
3. `let value = ground(scratch.as_ref(), &mut local.arrays, &mut local.cells,
   &mut local.refs)?;`
4. `if let Some(s) = &scratch { let mut g = s.borrow_mut();
   g.flush_structs_into(&mut local.structs); g.flush_enums_into(&mut
   local.enums); }`
5. `Ok((value, local))`

`check_poly_combinator_standalone` calls it once, with a closure that builds
`ctx` (over `structs`/`enums`, the *live* slices, plus the scratch cell) and
runs the two `apply_subst` loops, returning `(inputs, outputs)`. It then passes
`&local.enums`, `&local.structs`, `&mut local.arrays`, `&mut local.cells`,
`&mut local.refs`, `&mut local.slices` to `check_word` at
`src/check/poly.rs:485-500`, and `local.enums`/`local.structs` to R4.

The closure argument is deliberate: the rebase must happen *before* the mint and
the flush *after* it, and a function that returns "the copies" without owning
that ordering cannot enforce it.

### R3. New guard: a declared *top-level* generic input slot stays rejected

This is **new work**, not an existing guard. Today the input loop
(`src/check/poly.rs:443-449`) just calls `apply_subst`, and the rejection comes
only from the `ctx.generics()` `None` arm that R1 removes.

**Exact scope, no ambiguity.** Before the input loop, walk `sig.inputs` and, for
any element that is *itself* `PolyType::Generic { .. }` or
`PolyType::GenericVariant { .. }` at the **top level of the slot**, return
`poly_generic_not_yet_groundable_error(&ctx, span, &word.name,
&poly_type_str(pty, sig))` -- the same construction the `None` arm uses
(`src/check/poly.rs:8055-8062`), so the message is byte-identical. A
`Generic`/`GenericVariant` **nested** inside a slot (a quotation-input effect's
rows, an `Array` element, a `Ref`/`OwnedCell` referent) is **not** rejected and
grounds normally. Declared **outputs** are never checked by this guard.

R6 does not change R3's scope. R6 makes *nesting* safe to admit at lowering; R3
still rejects the top level.

*Why nested must be allowed:* the phase-1 acceptance fixture (`relay`, below)
carries `~[ 'T -- Result['T i64] ]` as an input slot and must ground.
*Why top-level must be rejected:* S12's standing decision that a combinator over
a generic-enum slot is unsupported
(`a_combinator_over_a_generic_enum_slot_is_rejected_before_r15_can_fire`,
`tests/phase7_slice12.rs:632`). **Measured:** with R1+R2 in and R3 absent, the
full suite is green except exactly that one test -- `31 passed; 1 failed` in
`tests/phase7_slice12.rs`, everything else green -- and the fixture advances to
an unrelated `` `drop` is an intrinsic and is not imported in `probe` ``
failure. So R3 is load-bearing and its blast radius is exactly one test.

### R4. Register the grounded monomorphs' generated sigs into a word-scoped env

After R2's flush, if anything was minted (`local.enums.len() > enums.len()` or
`local.structs.len() > structs.len()`), build `local_env = env.clone()` and
append the generated sigs for the **newly flushed tail only**:

- run `struct_generated_sigs(&local.structs)` and skip the first
  `struct_generated_sigs(structs).len()` entries;
- likewise `enum_generated_sigs` and `variant_generated_sigs` over
  `&local.enums`, skipping the first `enum_generated_sigs(enums).len()` /
  `variant_generated_sigs(enums).len()` entries.

Push each `(name, symbol, sig)` with `local_env.entry(name).or_default().push(
Overload { sig, symbol })`, matching `src/check.rs:568-576`'s append-not-insert
rule. Skipping the prefix is required: the base decls' sigs are already in
`env`, and re-appending them would double every constructor overload.

**Why the prefix skip is index-exact (corrected rationale).** Round 2 said "a
fixed number of entries per decl". That is **false** for
`enum_generated_sigs` (`src/check/declarations.rs:1753-1770`) and
`variant_generated_sigs` (`:1786-1808`), which push **one entry per variant**,
not per decl -- a two-variant enum and a five-variant enum contribute different
counts. (It happens to be true of `struct_generated_sigs`, which pushes exactly
two per decl at `:1723-1738`.) The real reason is **iteration order**: all three
helpers are a single `for (idx, decl) in slice.iter().enumerate()` loop that
only ever `push`es. `local.*` is `base.*` with a suffix appended, so the
extended slice's first `base.len()` iterations visit the *same decls at the same
`idx`* in the same order and push the *same* entries. The base's own sigs are
therefore exactly the leading `helper(base_slice).len()` entries of
`helper(extended_slice)`, whatever the per-decl count happens to be. The
implementation must compute that length by calling the helper on the base slice,
never by arithmetic on decl counts.

**Why the ids are right.** The helpers derive the output type from the slice
position (`src/check/declarations.rs:1719`, `:1755`, `:1794`), and after R2's
rebase-and-flush the slice position *is* the minted id. This is what the
previous design's one-element slice got wrong.

Pass `&local_env` to `check_word` in place of `env`.

**What R4 buys, measured.** With R1+R2+R3 but R4 stubbed out, the no-sibling
`wrap` program fails with `` unknown word `Ok` in `wrap` `` (the **def site**);
with R4 it fails with `` unknown word `Ok` in `main` `` (the **call site**, the
frozen-env gap that is out of scope). That word-name difference is the
discriminating assertion for R4.

**Reach of R4, stated precisely.** R4 inserts one env entry per variant (and,
for a struct, constructor + destructure) of every monomorph the *signature
grounding* minted. Any body term naming one of those variants resolves --
including an **intermediate** construction of the same monomorph the body does
not return. A construction over a *different* generic header, which the
signature never grounds, gets no env entry and still fails as an unknown word
(absent a parse-time sibling monomorph). Out of scope restates this; golden 7
pins it.

### R5. `poly_generic_not_yet_groundable_error` keeps both its jobs

R3's new guard reuses the message verbatim (same `op` = word name), so
`tests/phase7_slice12.rs:632`'s assertions are unchanged. The `apply_subst`
`None`-arm call sites (`src/check/poly.rs:8056`, `:8110`) stay: they remain the
shared-parameter default for any caller passing `None`.

### R6. Word-scoped `arrays`/`cells`/`refs`/`slices` too (P0-C)

**Round 2's caveat was wrong and this is the correction.** Round 2 wrote that
`apply_subst`'s `Array`/`Ref`/`OwnedCell` arms interning into the live
registries is "true today, before this slice" and "inert". Both halves are
false, and the second is a lowering panic.

#### R6.1 The defect, verified

*Not true today.* Against `main` at `9b220b8`:

```sooth
import: intrinsics * ;
type: Result['T 'E] | Ok 'T | Err 'E ;
: relay inline ( 'T ~[ 'T -- Result['T i64] ] -- array[Result['T i64] 4] ) call drop 0 ;
: main ( -- ) 0 . ;
```

```text
error: error: `relay` in `relay` (line 3) names the generic type `Result['T i64]`,
which cannot yet be instantiated at a variable-bearing application
  grounding a generic over its own type variable is not yet implemented
```

The `Generic` arm's `None` rejection (`src/check/poly.rs:8055-8062`) fires while
substituting the array's *element*, **before** `apply_subst`'s `Array` arm
reaches its `intern_array_type` call (`src/check/poly.rs:7997`). So this shape
never reaches the interning arm today. R1 is what first makes it reachable.

*Not inert.* Once R1 threads the cell in, and with `arrays` still the live
`&mut Vec<ArrayDecl>` from `src/check.rs:958`, the `Array` arm interns an
`ArrayDecl` whose `element` is `Type::Enum(scratch_id, ..)` into
`module.arrays`. R2 then discards the scratch, so `scratch_id` is never in
`module.enums`. Lowering walks **every** array unconditionally:
`build_registries_ww` runs `for i in 0..arrays.len() { lb.ensure_array(i); }`
(`src/ir/layout.rs:481-483`), `ensure_array` calls `self.size_align(element)`
(`:832-835`), and `size_align`'s `Type::Enum` arm (`:636-640`) calls
`self.ensure_enum(id.index())`, which indexes `self.enum_memo[idx]` on entry
(`:719`) -- before the arm's own `:638` index is ever reached -- where
`enum_memo` was sized `vec![None; enums.len()]`
(`:472`). Dangling id -> index out of bounds, at lowering, for a program that
type-checked cleanly.

*`Ref`/`OwnedCell` are lower risk, and here is exactly why.* Their layout-side
consumers are `cells.iter().map(|d| ir_type_of(d.payload))` and
`refs.iter().map(|d| ir_type_of(d.referent))` (`src/ir/layout.rs:574-575`), and
`ir_type_of` (`src/ir/types.rs:291-320`) is a pure `Type -> IrType` map that
indexes **no** registry -- `Type::Enum(id, _) => IrType::Enum(id)`. So a
dangling enum id inside a `RefDecl`/`OwnedCellDecl` produces an
`IrType::Enum(dangling)` and does not panic at registry-build time. It is still
a real leaked, forgeable id sitting in `module.refs`/`module.owned_cells`, which
a later `intern_ref_type` (`src/ast.rs:1369-1385`) or `intern_owned_cell_type`
(`:1325-1341`) can *dedup onto* -- both scan structurally for an existing
matching decl and hand back its id -- and which any consumer that later resolves
that `IrType::Enum` against `Enums::layouts` would index out of bounds. Fixing
them costs the same three lines as fixing arrays, so they are fixed.

#### R6.2 How these registries dedup (source proof, and the rebase question)

**They have no base and no rebase concept.** `ArrayDecl`, `RefDecl`,
`OwnedCellDecl` and `SliceDecl` are plain `Vec` entries interned by a **linear
structural scan**, not by a `GenericTypes`-style keyed batch with a base:

- `intern_array_type` (`src/ast.rs:1466-1483`): `arrays.iter().position(|d|
  d.element == element && d.count == count)`, else `ArrayId::from_index(
  arrays.len())` and push.
- `intern_owned_cell_type` (`src/ast.rs:1325-1341`): `position(|d| d.payload ==
  payload)`, else `OwnedCellId::from_index(cells.len())` and push.
- `intern_ref_type` (`src/ast.rs:1369-1385`): `position(|d| d.referent ==
  referent && d.mutable == mutable)`, else `RefId::from_index(refs.len())`.
- `intern_slice_type` (`src/ast.rs:1421-1437`): the `(element, mutable)` twin.

Consequences, stated explicitly because they differ from enums/structs:

1. **No rebase call. R2's rebase ruling does not extend to these.** The id is
   `vec.len()` at intern time, so cloning the live vector and interning into the
   clone already yields ids that equal their index in the clone -- there is
   nothing to re-point. An implementer must **not** invent a rebase for them.
2. **No flush call either.** There is no staging vector; the intern *is* the
   append. `WordScopedRegistries`' `arrays`/`cells`/`refs`/`slices` are handed
   to the grounding closure and to `check_word` as `&mut`, and simply dropped.
3. **The dedup key is `Type`-structural, and `Type::Enum` compares by id**
   (`src/ast.rs:589-603`'s note that `Type` is `Eq` over the real id an argument
   carries). This is precisely why a scratch enum id inside an `ArrayDecl` is
   poison rather than a harmless duplicate: nothing about it is content-addressed
   back to a real decl.

#### R6.3 The ruling

`check_poly_combinator_standalone` uses `WordScopedRegistries`' `arrays`,
`cells`, `refs` and `slices` for **both** the signature grounding (R2 step 2)
and the body walk (R2 step 4). The live `arrays`/`cells`/`refs`/`slices`
parameters at `src/check/poly.rs:412-415` become `&[..]` reads (they are only
cloned from) rather than `&mut Vec<..>`; `src/check.rs:958-961` passes them
unchanged apart from the borrow.

#### R6.4 `slices` is included, and why (an extension beyond the reported finding)

`apply_subst` has no `slices` parameter at all (`src/check/poly.rs:8059-8069`)
and no `PolyType::Slice` arm, so *signature grounding* never interns a slice.
But `check_word` does take `slices: &mut Vec<SliceDecl>` (passed at
`src/check/poly.rs:492`), and a body that builds an
`array[Result[i64 i64] 4]` and takes a view of it interns a `SliceDecl` whose
`element` is the scratch enum id. `build_slices` (`src/ir/layout.rs:327-345`)
matches `IrType::Enum(id) => &enums.layouts[id.index()]` -- the same
out-of-bounds shape as the array case. This hole exists in round 2's design as
shipped (it needs only the enum-side scratch id, not R6's array work), costs the
same one line to close, and leaving it open would mean shipping a known panic.
Closed here. A reviewer who considers this out of scope should strike R6.4 and
open it as a named follow-up rather than leave it undecided.

#### R6.5 Downstream-consumer audit (mirroring R2's `enums[id.index()]` audit)

Sites that index these vectors by id and would see an *unflushed scratch-local
entry* if the standalone pass leaked one. All re-read this round.

| registry | site | effect of a dangling *enum id inside* the decl | effect of a *missing* decl |
| --- | --- | --- | --- |
| `arrays` | `src/ir/layout.rs:481-483` + `:832-835` + `:636-640` | **panic**, index out of bounds on `enum_memo` | n/a: local entries never reach lowering |
| `arrays` | `src/check/builtins.rs:240` (`is_copy`) | recurses to `enums[id.index()]` at `:235` -> panic | n/a |
| `arrays` | `src/check/builtins.rs:315` (`contains_reference`) | recurses to `:309` -> panic | n/a |
| `arrays` | `src/check/drop_graph.rs:677` | descends into `enums[id.index()]` at `:671` -> panic | n/a |
| `arrays` | `src/check/word_families.rs:97`, `:800` | reads element/count only; a dangling *enum* id passes through into a `Type` that a later enum-indexing site panics on | n/a |
| `arrays` | `src/check/declarations.rs:1633`, `:1654` | element/name only, no enum index | n/a |
| `cells` / `refs` | `src/ir/layout.rs:574-575` via `ir_type_of` | no registry index; produces `IrType::Enum(dangling)`, deferred panic at any layout lookup | n/a |
| `slices` | `src/ir/layout.rs:327-345` | **panic** at `enums.layouts[id.index()]` | n/a |
| all four | `intern_*_type` structural scan (`src/ast.rs:1325`/`1369`/`1421`/`1466`) | a later real call site dedups onto the poisoned entry and adopts its dangling element type | n/a |

Note the asymmetry with enums/structs: for `enums`, the danger is a decl that is
**missing** from the slice the body walk sees (P0-B, an immediate check-time
panic). For `arrays`/`cells`/`refs`/`slices`, the danger is a decl that is
**present** in the live registry and should not be (P0-C, a lowering-time
panic). The two fixes are the same shape but guard opposite failure modes; the
tests must reflect that (golden 8 for P0-C, golden 4 for P0-B).

#### R6.6 Ruling on *ordinary* (non-grounding) shape construction in the body

**Explicit ruling: harmless, for three reasons, none of which is an assumption.**

Today, a combinator body's ordinary `array[i64 4]` construction during the
standalone check interns into `module.arrays`. Under R6 it interns into
`local.arrays` and is discarded. What changes:

1. **The shape is not lost to the program.** A combinator mints no `IrFunc`
   (`src/check.rs:926-928`) and its body is spliced into each concrete caller,
   where the ordinary concrete walk re-interns the same shape into the live
   registries with the live `&mut`. So any shape the *program* actually needs
   is interned by the splice site that needs it. The standalone pass is not the
   authority over the construction; the per-splice recheck is, and that is the
   same premise R2's discard already rests on.
2. **A shape only the standalone pass ever interned is dead.** If no splice site
   interns it, no lowered code refers to it, and today's live entry is an
   unreferenced `ArrayDecl` that `build_registries_ww` computes a layout for and
   nothing consumes. Removing it removes work, not meaning.
3. **Ids are assigned at intern time and carried by value.** Every `Type::Array`
   /`Type::Ref`/`Type::OwnedCell`/`Type::Slice` embeds its own id, so no id
   computed inside the standalone check outlives the local vector that defines
   it. Structural dedup means a splice site re-interning the same shape gets
   whatever the live registry's id for that shape is (existing or freshly
   minted), which is exactly what it would get if the standalone pass had never
   run.

The one *observable* difference is registry **ordering/count** in
`module.arrays`: a shape that only the standalone pass interns no longer
appears, and a shape both intern may land at a different index than before. No
id is spelled in source and none is asserted by any test that does not build the
registry itself, but this is the concrete risk this ruling accepts. **Phase 1
must record the full-suite result before and after making the shape registries
local**; any test that moves is a real finding about registry-order coupling and
must be reported, not adjusted away.

**Doc comments to amend.** `src/check/poly.rs:392-405` and `src/check.rs:797-800`
must be amended to say the pass "records nothing that survives it: no generic
monomorph, no instantiation record, and no interned array/cell/ref/slice shape
-- every registry it writes to is a word-scoped copy discarded when the check
returns." Round 2's proposed wording (which conceded shared shape interning) is
now wrong and must not be used.

## Tests

`thing_condition_expected` naming (CLAUDE.md). Diagnostics are behaviour: every
deferred case is tested for the *right* message.

### Stage unit tests (`src/check/poly.rs` `#[cfg(test)]`)

- `standalone_combinator_grounds_a_generic_output_without_a_constructor` --
  the R1/R2 floor, deliberately **constructor-free** so it is satisfiable in
  phase 1 and discriminates R1/R2 from R4:

  ```sooth
  import: intrinsics * ;
  type: Result['T 'E] | Ok 'T | Err 'E ;
  : relay inline ( 'T ~[ 'T -- Result['T i64] ] -- Result['T i64] ) call ;
  ```

  checks clean standalone. Fails against a `None` ctx with
  `poly_generic_not_yet_groundable_error`.
- `standalone_combinator_generic_input_slot_is_still_rejected` -- a top-level
  `Option['T]` input slot returns `poly_generic_not_yet_groundable_error`
  naming the slot (R3).
- `standalone_generic_nested_in_a_quotation_input_is_not_rejected` -- the R3
  scope twin: `~[ 'T -- Result['T i64] ]` as an input slot grounds. Fails if
  R3's guard recurses into slots instead of testing the top level.

- `standalone_stand_in_monomorph_does_not_enter_the_live_registry` --
  **re-seated on a public route (round-3 fix).** Round 2 specified assertions
  against `GenericTypes`' `enum_keys`/`enum_resolved`; both are private to
  `src/ast.rs` (`src/ast.rs:602-603`, `:612`, no `pub`) and unreachable from a
  `src/check/poly.rs` test. Re-verified this round: still private.

  The public route is `GenericTypes::lookup_enum(&self, idx: usize, module: u32,
  args: &[Type]) -> Option<Type>` (`src/ast.rs:944`, `pub`) -- the very function
  `instantiate_enum` consults for dedup (`:1198-1200`), so it observes exactly
  the state the private fields hold. The test:

  1. parse a module containing `type: Result['T 'E] ...` and the `relay`
     combinator, with **no** parse-time `Result[i64 i64]`;
  2. take `module.generics` into a cell, note `module.enums.len()`;
  3. assert `cell.borrow().lookup_enum(result_idx, header_module,
     &[Type::I64, Type::I64]).is_none()` **before** the check;
  4. run `check_poly_combinator_standalone` with `Some(&cell)`;
  5. assert the same `lookup_enum` call still returns `None`, that
     `cell.borrow().inst_enums.is_empty()` (that field *is* `pub`,
     `src/ast.rs:594`), and that `module.enums.len()` is unchanged.

  Step 3 is not decoration: without it, step 5 passes vacuously if the header
  index or module id is wrong. This is R2's discard guard and mutation 6's
  killer.

- `standalone_grounded_output_id_matches_its_extended_slice_position` --
  **re-seated on the R2.1 seam (round-3 fix).** Round 2 specified assertions
  against `local_enums`, a function-local of
  `check_poly_combinator_standalone` with no return value or accessor: as
  written the test could not compile, and it was the *only* named killer for
  mutation 4, so that fix would have shipped with no witness at all.

  The test now calls `ground_into_word_scoped_registries` **directly**:

  1. parse a module declaring `type: Result['T 'E] | Ok 'T | Err 'E ;` plus two
     ordinary concrete enums, so `module.enums` is non-trivial;
  2. take `module.generics`, and call `g.rebase(module.structs.len(),
     module.enums.len() - 1)` to force a **stale** `enum_base` -- exactly the
     state `src/check.rs:1012-1013`'s flush leaves behind (see R2, "when the
     base is actually stale"). Both `rebase` and `Module::generics` are `pub`
     (`src/ast.rs:900`, `:158`);
  3. call `ground_into_word_scoped_registries(Some(&cell), &module.structs,
     &module.enums, &module.arrays, &module.owned_cells, &module.refs,
     &module.slices, |scratch, arrays, cells, refs| { ... })` where the closure
     calls `scratch.unwrap().borrow_mut().instantiate_enum(result_idx,
     &[Type::I64, Type::I64], header_module, MutRegistries { structs:
     &module.structs, enums: &module.enums, arrays, cells, refs })` and returns
     that `Type`. (`instantiate_enum` and `MutRegistries` with all-`pub` fields:
     `src/ast.rs:1193`, `:670-676`.)
  4. assert, on the returned `(ty, local)`:
     - `let Type::Enum(id, name) = ty` and `id.index() == module.enums.len()`
       -- **kills mutation 4**: with the rebase dropped, `enum_base` is
       `module.enums.len() - 1` and the id is one short;
     - `local.enums.len() == module.enums.len() + 1` -- **kills mutation 5**;
     - `local.enums[id.index()].name_static == name` -- the id/index/name
       correspondence, compared against the `Type`'s own leaked name, not a
       hand-written string;
     - `module.enums.len()` is unchanged (the live registry is untouched).

  Step 2's forced staleness is what makes mutation 4 observable at all; without
  it the base already equals `enums.len()` and dropping the rebase is an
  identity edit (a `workflow_span_corruption_mutation_may_be_identity`-shaped
  trap).

- `standalone_grounding_an_array_of_a_monomorph_leaves_the_live_arrays_untouched`
  -- R6's unit guard. Check `relay` with the
  `-- array[Result['T i64] 4]` header (golden 8's source, **no** `mki`)
  **directly through `ground_into_word_scoped_registries`**, not through
  `check_poly_combinator_standalone` -- the standalone entry point returns
  only `Result<(), String>` (`src/check/poly.rs:420`) and has no route to
  observe the local registries it builds and discards, so the `local.arrays`
  assertion below is unobservable through that call and must go through the
  free function directly, the same route
  `standalone_grounded_output_id_matches_its_extended_slice_position` uses.
  Then assert `module.arrays.len()` is unchanged, and that the returned
  `local.arrays` has exactly one more entry whose `element` is
  `Type::Enum(id, _)` with `id.index() == module.enums.len()` (i.e. the array
  points at the scratch monomorph, inside the *local* enum registry). Kills
  mutation 9's `local.arrays` half; the `module.arrays.len()`-unchanged half is
  additionally asserted through an actual `check_poly_combinator_standalone`
  call on the same source, so both call routes are covered.

### Declarations unit test (`src/check/declarations.rs` `#[cfg(test)]`)

- `enum_generated_sigs_over_an_extended_slice_carries_the_monomorphs_own_id` --
  build a base `enums` vector, append a grounded `Result[i64 i64]` decl at index
  `base`, call `enum_generated_sigs` over the **whole** slice, and assert the
  `Ok` entry's `Sig::outputs[0]` **is equal to** `Type::Enum(EnumId::from_index(
  base), ..)` -- the same `Type` the grounded declared output carries, compared
  by value, not by constructor name. A name-only assertion passes under the
  exact P0-B bug and is forbidden here. Also assert the one-element-slice
  failure mode explicitly: the same decl alone in a slice yields
  `EnumId::from_index(0)`, which is why R4 generates over the extended slice.
- `enum_generated_sigs_prefix_is_stable_under_extension` -- R4's corrected
  order-based rationale, tested rather than asserted in prose. Build a base
  `enums` with a **one-variant** enum and a **three-variant** enum (so no fixed
  per-decl count exists), then an extended slice with one more enum appended.
  Assert `enum_generated_sigs(extended)[..enum_generated_sigs(base).len()] ==
  enum_generated_sigs(base)` and the same for `variant_generated_sigs`. Fails if
  a helper ever stops being a prefix-stable in-order push, which is the sole
  property R4's skip depends on.

### Integration goldens (`tests/phase7_slice11.rs`)

Goldens 1-7 were run against the round-2 prototype and their stated outcome is
**measured**. Goldens 8 and 9 are new this round and their outcomes are
**derived** from the source reading in R6/R2, not measured -- the prototype was
discarded and no source change is staged. The implementing phase must record the
measured outcome for each and treat any divergence as a finding.

1. `unbounded_combinator_constructing_its_generic_output_builds_and_runs` --
   the end-to-end exit dogfood (needs a parse-time sibling monomorph, per the
   scope correction):

   ```sooth
   import: intrinsics * ;
   type: Result['T 'E] | Ok 'T | Err 'E ;
   : mki ( i64 -- Result[i64 i64] ) Ok ;
   : wrap inline ( 'T ~[ 'T -- 'T ] -- Result['T i64] ) call Ok ;
   : main ( -- ) 7 ~[ 1 add ] wrap ~[ ( Ok ) Ok> . ] ~[ ( Err ) Err> . ] Result? ;
   ```

   exits 0, stdout `8\n`. (Measured. `+` is not a bare intrinsic here; `add` is.
   `'U` is deliberately collapsed to `'T`: see golden 5.)

   **Golden 1 witnesses R1 and R3 only.** Because `mki` grounds `Result[i64
   i64]` at parse time, the standalone check's `apply_subst` hits
   `lookup_enum`'s dedup (`src/ast.rs:1200-1201`) and **nothing is minted**:
   `local.enums == enums` and R4's `len >` test is false, so R4's body never
   runs. This is the same measurement Open Questions already recorded. Golden 1
   must therefore **not** be claimed as a killer for any R2-mint, R4 or
   sig-generation mutation. See the recipe.

2. `constructor_free_combinator_grounds_its_generic_output_and_runs` --
   the R1/R2-only end-to-end twin, whose body is just `call`:

   ```sooth
   import: intrinsics * ;
   type: Result['T 'E] | Ok 'T | Err 'E ;
   : mki ( i64 -- Result[i64 i64] ) Ok ;
   : relay inline ( 'T ~[ 'T -- Result['T i64] ] -- Result['T i64] ) call ;
   : main ( -- ) 7 ~[ Ok ] relay ~[ ( Ok ) Ok> . ] ~[ ( Err ) Err> . ] Result? ;
   ```

   exits 0, stdout `7\n`. (Measured.) This is the R1-vs-R4 discriminator: it
   dies under "revert R1" and survives "stub R4". Like golden 1 it carries
   `mki`, so it also mints nothing and is likewise not a mint-machinery witness.

3. `combinator_over_a_generic_input_slot_still_rejected` -- **verbatim** the S12
   fixture (including the `mki` line that grounds the header elsewhere, and the
   two `type:` lines of `tests/phase7_slice12.rs:313-314`'s `OPTION`), so it
   cannot pass for an unrelated reason:

   ```sooth
   type: Option['T] | None | Some 'T ;
   type: Pt x i64 y i64 ;
   : probe inline ( Option['T] ~[ -- i64 ] -- i64 )
     | f |
     ~[ ( Some ) drop f call ]
     ~[ ( None ) drop 0 ]
     Option? ;
   : mki ( i64 -- Option[i64] ) Some ;
   : main ( -- ) 7 mki ~[ 5 ] probe . ;
   ```

   must fail with a message containing both `names the generic type` +
   `Option['T]` and `cannot yet be instantiated at a variable-bearing
   application` (R3). Note the fixture has no `import: intrinsics * ;`: with R3
   absent it advances instead to the unrelated "`drop` is an intrinsic and is
   not imported in `probe`" error, which is exactly how the measured R3-removal
   run failed. That is acceptable -- the assertion is on the R3
   message -- and is *why* the fixture must not be "helpfully" given an import.

4. `a_body_indexing_its_grounded_output_decl_reports_not_panics` -- the P0-B
   witness, promoted to a golden. **No sibling monomorph**, so this fixture
   really does mint:

   ```sooth
   import: intrinsics * ;
   type: Result['T 'E] | Ok 'T | Err 'E ;
   : wrapd inline ( 'T ~[ 'T -- Result['T i64] ] -- i64 ) call tag ;
   : main ( -- ) 7 ~[ 1 add Ok ] wrapd . ;
   ```

   must fail with `` `tag` requires an enum whose variants all carry no payload,
   found `Result[i64 i64]` `` -- a *diagnostic*, and the build must not panic.
   (Measured both ways: with the flush, this message; without it, `index out of
   bounds: the len is 0 but the index is 0` at
   `src/check/word_families.rs:669`.)

5. `an_output_only_type_variable_is_still_uncallable` -- pins the discovered
   boundary that made golden 1 collapse `'U` to `'T`. With the original
   `( 'T ~[ 'T -- 'U ] -- Result['U i64] )` shape the def-site error is gone but
   the call site now reports `` has output variable `'U` that no input binds ``
   (measured), and the remedy the note suggests does not apply: `wrap[i64 i64]`
   is rejected with `` takes no type arguments; only a call to a polymorphic
   word may be explicitly instantiated `` (measured). Assert both strings. This
   golden is what stops the next slice from believing the `'U` shape works.

6. `a_check_time_monomorphs_constructors_are_absent_from_the_call_site_env` --
   pins the out-of-scope frozen-env gap so it cannot regress silently or be
   quietly claimed as fixed. Golden 1's source **minus** the `mki` line must
   fail with `` unknown word `Ok` in `main` `` -- naming `main`, not `wrap`.
   (Measured.) The word name is the whole assertion: `wrap` would mean R4 is
   broken, `main` means R4 works and only the frozen env remains. Having no
   sibling monomorph, this fixture **does** mint, and is therefore the real
   witness for R4's mint-and-register path.

7. `an_intermediate_construction_over_a_different_header_is_still_unknown` --
   R4's reach boundary. A combinator whose declared output grounds `Result` but
   whose body also constructs a `Pair['T]` the signature never mentions, in a
   file with no parse-time `Pair` monomorph, must fail with `` unknown word
   `One` in `wrap` ``:

   ```sooth
   import: intrinsics * ;
   type: Result['T 'E] | Ok 'T | Err 'E ;
   type: Pair['A] | Nil | One 'A ;
   : mki ( i64 -- Result[i64 i64] ) Ok ;
   : wrap inline ( 'T ~[ 'T -- 'T ] -- Result['T i64] ) call dup One drop Ok ;
   : main ( -- ) 7 ~[ 1 add ] wrap ~[ ( Ok ) Ok> . ] ~[ ( Err ) Err> . ] Result? ;
   ```

   If the implementer finds this fixture fails on an earlier, unrelated error
   (e.g. `dup` over a non-`Copy` value), they must reshape the body until the
   failure is the `One` unknown-word one, keeping the property "the body
   constructs a header the signature does not mention". Do **not** weaken the
   assertion instead.

8. `a_combinator_returning_an_array_of_its_grounded_monomorph_builds_and_runs`
   -- **new, R6's end-to-end witness (P0-C). Round-3 fix: no sibling
   monomorph.** Round 2's draft of this fixture carried a sibling
   `: mki ( i64 -- Result[i64 i64] ) Ok ;`, which grounds the same
   instantiation key at parse time. Per golden 1's own measurement, that makes
   `apply_subst` hit `lookup_enum`'s dedup and **mint nothing** -- the fixture
   would exercise a live, already-flushed id, not a scratch one, and could not
   witness R6 or mutation 9 at all. This shape has no `mki` and the combinator
   is **never called**, so it is a compile-only witness (property (ii) below is
   dropped deliberately, not by implementer's licence): the full path is ground
   the element -> mint into the scratch `GenericTypes` -> intern the array into
   the *local* `arrays` -> flush the enum into the *local* `enums`, then reach
   lowering without panicking.

   ```sooth
   import: intrinsics * ;
   type: Result['T 'E] | Ok 'T | Err 'E ;
   : hold inline ( 'T ~[ 'T -- Result['T i64] ] -- array[Result['T i64] 4] )
     call dup dup dup arr4 ;
   : main ( -- ) 0 . ;
   ```

   Assertion: **the build exits 0 and does not panic**; `hold` is never called,
   so there is no stdout property to assert. Measured pre-fix on this exact
   source against HEAD: rejected at the def site with `` `hold` in `hold`
   (line 3) names the generic type `Result['T i64]`, which cannot yet be
   instantiated at a variable-bearing application ``, matching R6.1's `relay`
   measurement. Under R1 without R6 it would type-check and then panic at
   `src/ir/layout.rs:719` (`ensure_enum`, reached from `size_align`'s
   `Type::Enum` arm at `:637`) from `ensure_array`, since the live `arrays`
   registry would carry a decl whose element is a scratch-only, never-flushed
   `EnumId`.

   **Implementer's licence, bounded.** The array-literal spelling (`arr4`) is
   *not* a ruling of this spec -- use whatever the current array vocabulary
   actually is. The one property that may not be traded away: the declared
   output is an `array[...]` whose element is a `Generic` the signature
   grounds, with **no parse-time sibling monomorph of the same instantiation
   key anywhere in the file**, so `apply_subst`'s `Array` arm is provably
   interning a decl around a scratch-minted (not live) enum id.

   **Do `Ref`/`OwnedCell` get their own goldens? No -- stated reason.** Per
   R6.1, their layout-side consumers (`src/ir/layout.rs:574-575` via
   `ir_type_of`) index no registry, so no `&Result['T i64]` or
   `^Result['T i64]` output produces an *observable* failure at lowering: the
   leak is a latent forgeable id with no reachable panic today. A golden with no
   failing mode is a placebo. They are covered instead by
   `standalone_grounding_an_array_of_a_monomorph_leaves_the_live_arrays_untouched`'s
   sibling assertion: that same unit test also asserts `module.owned_cells.len()`
   and `module.refs.len()` are unchanged after checking a combinator whose header
   is `( 'T ~[ 'T -- Result['T i64] ] -- &Result['T i64] )`-shaped (or `^`, if
   the reference form's own rules reject it at the def site for an unrelated
   reason -- in which case use the cell form and record why). This is honest
   coverage of a latent defect; an integration golden is owed only when a
   consumer that indexes `Refs`/`Cells` by id exists.

9. `a_standalone_mint_after_an_earlier_check_time_mint_lands_at_the_right_id`
   -- **new, the mutation-4 witness at integration level.** An earlier
   monomorphic word grounds a *different* generic header at check time and
   flushes it (`src/check.rs:1012-1013`), leaving `enum_base` stale; the
   combinator is then checked standalone and must still land its own monomorph
   at the correct id.

   ```sooth
   import: intrinsics * ;
   type: Box['A] | Empty | Full 'A ;
   type: Result['T 'E] | Ok 'T | Err 'E ;
   : boxit ( 'T -- Box['T] ) Full ;
   : seed ( -- ) 1 boxit drop ;
   : wrap inline ( 'T ~[ 'T -- 'T ] -- Result['T i64] ) call Ok ;
   : main ( -- ) seed 7 ~[ 1 add ] wrap drop ;
   ```

   Word order matters and is load-bearing: `seed` precedes `wrap` in the single
   word loop at `src/check.rs:908`, so its flush at `:1012-1013` happens first.
   Nothing in the program spells `Box[i64]`, so that monomorph can only be a
   check-time mint -- **measured** on the three-line reduction in R2 ("when the
   base is actually stale"), which builds and runs clean today.

   `wrap` has no sibling `Result[i64 i64]`, so per golden 6 the expected outcome
   is a failure naming **`main`** (`` unknown word `Ok` in `main` ``), which is
   the assertion. Under mutation 4 (rebase dropped) the scratch mint takes the
   id `Box[i64]` already occupies, the flush appends `Result[i64 i64]` one slot
   further along, and R4 registers `Ok` with output `Type::Enum(that later
   index)` while the declared output slot carries the colliding earlier id --
   so the failure moves to a **type mismatch inside `wrap`**, at the def site.
   Assert the message names `main` and does **not** name `wrap`, so either
   divergence is caught.

   *Derived, not measured* (round-2's prototype is discarded). Phase 1 must
   record the measured message; if it names something else, the correct response
   is to re-derive the fixture until mutation 4 is observably killed, not to
   relax the assertion.

## Mutation recipe (planned, each must fail a named test)

Classify on a named `test result: FAILED`. Commit first; copy the worktree with
`examples/` included (never `cp -r` the live tree); confirm the mutated binary
actually rebuilt before classifying.

**Two fixtures are disqualified as witnesses for any mint-path mutation.**
Goldens 1 and 2 both carry `: mki ( i64 -- Result[i64 i64] ) Ok ;`, so
`instantiate_enum`'s dedup returns the existing parse-time id and **nothing is
minted** (`src/ast.rs:1200-1201`; measured, and already recorded in Open
Questions). R2's mint/flush and all of R4 are dead code on those fixtures. The
minting fixtures are goldens 4, 6, 8 and 9.

| # | mutation | must fail | must stay green |
| --- | --- | --- | --- |
| 1 | Revert R1 (pass `None` to `word_ctx`) | `standalone_combinator_grounds_a_generic_output_without_a_constructor`, `constructor_free_combinator_grounds_its_generic_output_and_runs`, goldens 1, 4, 5, 6, 7, 8, 9 | golden 3 (R3's own guard raises the same message) |
| 2 | Drop R3's input-slot guard | `standalone_combinator_generic_input_slot_is_still_rejected`, golden 3, **and** `tests/phase7_slice12.rs:632 a_combinator_over_a_generic_enum_slot_is_rejected_before_r15_can_fire` (measured: exactly these; the rest of the suite stayed green) | goldens 1, 2 |
| 3 | Make R3's guard recurse into nested positions | `standalone_generic_nested_in_a_quotation_input_is_not_rejected`, goldens 2, 4, 8 | golden 3 |
| 4 | Drop R2's `rebase` call in `ground_into_word_scoped_registries` | `standalone_grounded_output_id_matches_its_extended_slice_position` (its step 2 forces a stale base, so the id is one short), golden 9 (error moves from `main` to a `wrap` type mismatch) | goldens 1, 2 (nothing is minted there) |
| 5 | Drop R2's `flush_enums_into(&mut local.enums)` | `standalone_grounded_output_id_matches_its_extended_slice_position` (`local.enums.len()` unchanged), and **every minting golden**: 4, 6, 8, 9 | goldens 1, 2 |
| 6 | Point R2 at the live cell instead of the clone | `standalone_stand_in_monomorph_does_not_enter_the_live_registry` | goldens 1, 2 |
| 7 | Stub R4 (thread + flush, but no `local_env`) | golden 6 (its error moves back to `wrap`), golden 7 | golden 8 stays green too -- `hold`'s own body never names `Ok` (only `call` on the quotation parameter, plus `dup`/`arr4`), so a missing `local_env` registration is invisible to it; it is an R6/array witness, not an R4 one. **golden 2 and `standalone_combinator_grounds_a_generic_output_without_a_constructor` stay green** -- the pair proving slot grounding and body-constructor resolution are distinct halves (measured: under a stubbed R4, `relay` still runs and prints `7`). Golden 1 also stays green, but for the uninteresting reason that it mints nothing |
| 8 | Feed R4's sig generation a one-element slice instead of the extended one, **at the call site** | `golden 6` -- with no sibling monomorph, `Ok` is registered with output `Type::Enum(EnumId::from_index(0))` (`src/check/declarations.rs:1755`), i.e. the program's *first* enum, while `wrap`'s declared output is the grounded monomorph's id, so `wrap`'s body fails with a type mismatch **at the def site** instead of the expected `` unknown word `Ok` in `main` `` | goldens 1, 2, 8 (`hold`'s body never names `Ok`, so a wrong constructor registration is unobservable there) |
| 9 | Revert R6: pass the live `arrays`/`cells`/`refs`/`slices` to the grounding and body walk | `standalone_grounding_an_array_of_a_monomorph_leaves_the_live_arrays_untouched`, `standalone_grounding_a_cell_through_the_body_leaves_the_live_cells_untouched`, `standalone_grounding_a_borrow_and_a_slice_through_the_body_leaves_the_live_registries_untouched`, `corpus_qbe_stays_byte_identical_to_baseline`, golden 8 (panics at `src/ir/layout.rs:719` during lowering instead of exiting 0 -- golden 8 never calls `hold`, so "stays green" here means "the build completes", not "the program runs") | goldens 1-7 |
| 10 | Drop R4's prefix skip (append the whole `helper(local)` output) | `enum_generated_sigs_prefix_is_stable_under_extension` stays green (it tests the helper, not the skip); the killer is golden 6 -- every base constructor is doubled in `local_env`, so a body call to any of them becomes ambiguous. Golden 8 is **not** a witness here: `hold`'s own body never names `Ok` (that call lives in the caller's quotation argument, and golden 8's `main` no longer supplies one), so a doubled `local_env` is invisible to it. **Accepted risk:** if the overload resolver silently picks the first identical candidate, this mutation survives. Phase 3 must record the measured outcome and, if it survives, record it as a coverage gap with the resolver behaviour as the reason -- not paper over it | goldens 1, 2 (nothing is minted, so R4's body never runs) |

**Note on mutation 8's seam.** The unit test
`enum_generated_sigs_over_an_extended_slice_carries_the_monomorphs_own_id`
calls `enum_generated_sigs` *directly*, so mutating R4's **call site** (which
slice it is fed) does not change what that test observes. That is why golden 6
is named as mutation 8's witness: it is the only fixture that both mints and
routes through the real call site. Round 2 named golden 1, which mints nothing
and would have stayed green.

**Accepted survivor, recorded rather than hidden.** None. Round 2's mutation 4
had no valid witness under any then-specified fixture (its only named killer was
uncompilable, and every fixture's sibling monomorph was parse-time, so
`enum_base` was never stale). Both halves are fixed above: the unit test now
forces staleness through the public `rebase`, and golden 9 constructs a real
earlier check-time mint. Mutation 10 is the one row carrying a named risk of
surviving; see its cell.

## Out of scope

- **The frozen call-site env (`P7.S11-follow`, discovered here).** A monomorph
  minted at check time never enters `env`, which is built once at
  `src/check.rs:568-576`, before the word loop at `:908`. Consequences, all
  measured: an `inline` combinator constructing a generic output is only
  callable if a **parse-time** monomorph of the same header exists somewhere in
  the program; and the `Option`-shaped-`map`-in-a-library-file case (a file
  whose only `Option` instantiation is `map`'s own construction) still does
  **not** build. Golden 6 pins the current behaviour. Fixing it needs an env
  that can be extended mid-word-walk, since the mint and the lookup happen
  inside one `check_word` call; that is a separate slice.
- **An output-only type variable on a combinator** (`'U` appearing only in the
  outputs). Blocked by an unrelated inference gap, and unrepairable by explicit
  instantiation because a combinator call rejects type arguments. Golden 5 pins
  it.
- **A top-level generic *input* slot on a combinator** (`Option['T]` parameter).
  Deferred (R3); its own probe is owed before scoping in. S12's
  `a_combinator_over_a_generic_enum_slot_is_rejected_before_r15_can_fire` stays
  the live rejection, and golden 3 mirrors it on the S11 side.
- **Bounded (`['T: Foo]`) combinators constructing a generic enum.** They take
  the poly-body pre-pass, not the standalone path, and are rejected by S12's
  R1.5 `poly_combinator_generic_enum_construction_error`
  (`combinator_constructing_ungrounded_generic_enum_is_rejected`,
  `tests/phase7_slice12.rs:220`), whose rationale (a `Span`-keyed `enum_words`
  cannot hold per-splice resolutions on the *recorded* poly-walk path) still
  applies. This slice touches only `check_poly_combinator_standalone`.
- **Eliminating a generic enum inside a combinator body**, field projection, and
  `dup`/`over` / `unify_poly_input`'s consuming side over a generic-typed value
  in a combinator body (brief bullet 3).
- **Intermediate construction over a header the signature never grounds.** R4
  registers only the monomorphs the signature grounding minted, so such a
  construction still fails as an unknown word absent a parse-time sibling. An
  intermediate construction of the *same* monomorph the signature grounds **is**
  admitted -- one env entry serves every mention. Golden 7 pins the boundary.
- **A `Refs`/`Cells` consumer that indexes by id at lowering.** None exists
  today (R6.1), so R6's fix for those two registries ships with unit coverage
  and no integration golden. If such a consumer is ever added, a golden is owed
  then.
- `docs/book/` (uncompiled, separately tracked) and
  `tree-sitter-sooth/grammar.js`.

## Open Questions

None outstanding.

- *"Small threading fix or distinct machinery?"* Distinct machinery: rebase +
  mint + flush into word-scoped extended registries (R2/R2.1/R6) plus a
  word-scoped env (R4). Threading alone panics or reports an unknown
  constructor.
- *"Is the per-splice re-check sufficient cover, or must the standalone pass
  ground itself?"* It must ground itself (R1/R2/R4). Because it records no
  monomorph and never lowers, R1.5's `Span`-keyed recording concern does not
  apply to it. The per-splice recheck *is* the authority over ordinary shape
  construction, which is why R6.6's discard is safe.
- *"Keyed on what module and dedup identity?"* Identically to a real monomorph
  (header module, `i64` args, `GenericTypes`' existing dedup), but inside the
  scratch clone, so it dedups cleanly against a parse-time monomorph of the same
  key (measured: with a sibling `mki` present, nothing is minted at all and the
  extended slices equal the originals -- which is exactly why goldens 1 and 2
  are disqualified as mint-path witnesses) and leaks nothing when it is new.
- *"Do the shape registries need a rebase too?"* No. They intern by linear
  structural scan with the id fixed at `vec.len()`, and have no base
  (R6.2). Cloning is sufficient and a rebase would be meaningless.

## Exit criteria

Each is a golden or a named unit test; the parenthesis names it.

- A `Result`-constructing unbounded `inline` combinator with a parse-time
  sibling monomorph builds and runs, stdout `8\n` (golden 1).
- A constructor-free unbounded `inline` combinator whose declared output and
  quotation-input effect both apply a generic header builds and runs, stdout
  `7\n` (golden 2).
- A top-level generic *input* slot still fails with `cannot yet be instantiated
  at a variable-bearing application`, and `tests/phase7_slice12.rs:632` and
  `:220` stay green (golden 3).
- A standalone body that type-directs on its grounded output reports a
  diagnostic and does not panic (golden 4).
- A combinator whose declared output is an **array of** its grounded monomorph
  builds and does not panic at lowering (golden 8, compile-only -- the
  combinator is never called); the live `arrays`/`cells`/`refs` are unchanged
  by the check (stage unit test).
- A standalone mint that follows an earlier *check-time* mint lands at the right
  id (golden 9, and
  `standalone_grounded_output_id_matches_its_extended_slice_position`).
- The discovered boundaries are pinned, not hidden: output-only `'U` still
  uncallable (golden 5), a check-time monomorph's constructors still absent from
  the call-site env with the failure naming `main` (golden 6), a different
  header's constructor still unknown (golden 7).
- The live `GenericTypes` cell and `module.enums`/`module.structs` are unchanged
  by a standalone check, asserted through the public `lookup_enum`
  (`standalone_stand_in_monomorph_does_not_enter_the_live_registry`).
- `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green.
  (Measured on the round-2 prototype with R1+R2+R3+R4 in; R6 is new and unmeasured.)
- Doc comments at `src/check/poly.rs:392-405` and `src/check.rs:797-800` amended
  per R6.6's wording (**not** round 2's).
- P7.S11 marked `[ done ]`; `P7.S11-follow` (frozen call-site env) recorded in
  the roadmap; growth-signal re-run recorded for `src/check/poly.rs` at phase
  exit.

## Phases (JSON)

Phase boundaries are drawn so each phase's own listed tests are satisfiable by
that phase's own rulings.

```json
{
  "phases": [
    { "phase": 1, "focus": "R1 + R2 + R2.1 + R3 + R6: Clone derives on GenericTypes/EnumDecl/VariantDecl/ArrayDecl/OwnedCellDecl/RefDecl/SliceDecl; the WordScopedRegistries struct and the ground_into_word_scoped_registries free function (rebase + local copies of structs/enums/arrays/cells/refs/slices + flush); new generics param on check_poly_combinator_standalone and Some(&generics_cell) at src/check.rs:953; arrays/cells/refs/slices params there become read-only borrows; check_word fed the local registries; R3's new top-level generic-input guard; amend the two doc comments per R6.6. Tests satisfiable without R4: standalone_combinator_grounds_a_generic_output_without_a_constructor, standalone_generic_nested_in_a_quotation_input_is_not_rejected, standalone_combinator_generic_input_slot_is_still_rejected, standalone_stand_in_monomorph_does_not_enter_the_live_registry (via the public lookup_enum), standalone_grounded_output_id_matches_its_extended_slice_position (direct call to the free function with a forced-stale base), standalone_grounding_an_array_of_a_monomorph_leaves_the_live_arrays_untouched, standalone_grounding_a_cell_through_the_body_leaves_the_live_cells_untouched and standalone_grounding_a_borrow_and_a_slice_through_the_body_leaves_the_live_registries_untouched (the body route, the only one that can reach cells/refs/slices -- neither `-- ^Result['T i64]` nor `-- &Result['T i64]` parses at a generic header), goldens 2, 3, 4 and 8 (golden 8 needs no R4: `hold`'s own body never names `Ok`, only `call`/`dup`/`arr4`, so it is a pure R1+R2+R6 witness). tests/phase7_slice12.rs must stay green. Record the full-suite result immediately before and after making the shape registries local (R6.6's accepted risk) and report any test that moves.", "effort": "M", "difficulty": "tricky" },
    { "phase": 2, "focus": "R4: word-scoped env clone with the newly flushed monomorphs' struct/enum/variant generated sigs appended tail-only, the prefix length computed by calling each helper on the base slice (never by arithmetic on decl counts), passed to check_word. Tests: enum_generated_sigs_over_an_extended_slice_carries_the_monomorphs_own_id (identity of the output Type, not the name), enum_generated_sigs_prefix_is_stable_under_extension, goldens 1, 6, 7 and 9. Record the measured outcome of golden 9, whose stated result is derived rather than measured (golden 8 has no R4 dependency and is measured in phase 1 instead).", "effort": "M", "difficulty": "tricky" },
    { "phase": 3, "focus": "Golden 5 (output-only type variable boundary); run the ten-mutation recipe and record the measured pass/fail matrix, including whether mutation 10 survives (accepted-survivor row) and whether goldens 1 and 2 really do stay green on every mint-path row as predicted; roadmap bookkeeping including the P7.S11-follow frozen-env entry; growth-signal re-run on src/check/poly.rs.", "effort": "S", "difficulty": "standard" }
  ]
}
```
