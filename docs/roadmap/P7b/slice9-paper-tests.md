# P7b.S9 paper tests — validated golden designs (recon round)

Designed and executed against the clean tree at HEAD `600bc1b` by the paper
round; fixtures lived under `/tmp/p7bs9-probes/paper-fixtures/` (ephemeral).
Companion docs: [slice9-brief](./slice9-brief.md),
[slice9-probes](./slice9-probes.md). Preserved verbatim below.

## Paper-test designs

Golden designs for `tests/phase7b_slice9.rs`. Every fixture below was built and run
at HEAD `600bc1b` under `/tmp/p7bs9-probes/paper-fixtures/{g1,g2,g2r,g3,g4}/` (not
committed; repo verified clean after every step). Conventions from
`tests/phase7b_slice5.rs`: `Tree::new`/`write_manifest`/`build_and_run`/
`build_error`, `thing_condition_expected` naming.

**Correction to the brief's assumption:** the `mk` variant's nondeterminism is a
**build-time** effect: the checker's own `HashMap<String, CallInst>` of
`module.instantiations` reseeds once per `sooth build` process
(`src/ir/driver.rs:350-373`'s dedup loop iterates it), not runtime. One built
binary is 100% deterministic on repeat execution (round-1 review measured: 5/5
stable reruns on each of two builds); only rebuilding flips the outcome
(measured: 6/8 `1\n1`, 2/8 `2\n2` across 8 rebuilds). All "before" run counts
below are rebuild+run cycles, never same-binary reruns. This is the single
nondeterminism story across every S9 doc; an earlier probe-log reading ("the
same binary flips... with no source change") was a mislabelled series of
rebuild+run cycles — see the errata in [slice9-probes](./slice9-probes.md).

`f.sth` is identical across G1/G2/G2r/G3/G4 unless noted:

```sth
import: intrinsics * ;
trait: Sized['S] : size ( 'S -- i64 ) ; ;
: sized['S: Sized] ( 'S -- i64 ) size ;
export: Sized sized ;
```

---

## G1 — `cross_module_same_shaped_impls_dispatch_each_callers_own_impl`

Verbatim `pb2` (S5's motivating fixture, `/tmp/p7bs9-probes/pb2/`).

```sth
// a.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 1 ; ;
: run ( i64 -- i64 ) Widget sized ;
export: run ;

// b.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 2 ; ;
: usesize ( Widget[i64] -- i64 ) size ;
: run ( i64 -- i64 ) Widget usesize ;
export: run ;

// main.sth
import: intrinsics * ; import: hosted::show | . | ;
import: self::f * ; import: self::a ; import: self::b ;
: main ( -- ) 5 a::run . 5 b::run . ;
```

**Command:** `build_and_run(main.sth)`
**Before (validated, 3 rebuild+run cycles, identical):** `2\n2`, exit 0.
**After (expected):** `1\n2` — each caller's own impl.
**Witnesses:** roadmap exit item 1; H2 (`a::run`'s bare `Widget` ctor call
silently reuses b's eagerly-minted `Widget[i64]`, since only b's `usesize`
spells it explicitly).
**Defers:** the provenance fix site.

---

## G2 — `cross_module_same_shaped_impls_via_named_instantiation_dispatch_each_callers_own_impl`

Both modules add `: mk ( i64 -- Widget[i64] ) Widget ;` and `run` becomes
`mk sized`; otherwise identical to G1. This is the same shape (mk variant, two
modules, impl constants 1/2) as the test already committed at
`tests/phase7b_slice4.rs:427-490`
(`same_named_ctor_mk_ambiguity_resolves_but_impl_dispatch_still_cross_picks`) —
different trait (`Functor` there, `Sized` here) and different text, not the
same fixture. That test is currently hard-pinned to the pre-fix `1\n1` and is
re-pinned in Phase 3 (R5/REQ-5 in the spec); this G2 golden is written fresh in
`tests/phase7b_slice9.rs`. Do not churn one fixture to match the other.

```sth
// f.sth
import: intrinsics * ;
trait: Sized['S] : size ( 'S -- i64 ) ; ;
: sized['S: Sized] ( 'S -- i64 ) size ;
export: Sized sized ;

// a.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 1 ; ;
: mk ( i64 -- Widget[i64] ) Widget ;
: run ( i64 -- i64 ) mk sized ;
export: run ;

// b.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 2 ; ;
: mk ( i64 -- Widget[i64] ) Widget ;
: run ( i64 -- i64 ) mk sized ;
export: run ;

// main.sth
import: intrinsics * ; import: hosted::show | . | ;
import: self::f * ; import: self::a ; import: self::b ;
: main ( -- ) 5 a::run . 6 b::run . ;
```

**Command:** `build_and_run(main.sth)`
**Before — three independent measurements, all confirming the flip, none
stable (never assert a ratio in committed code):**

| Round | Cycles | `1\n1` | `2\n2` |
| --- | --- | --- | --- |
| Probes | 10 | 6 | 4 |
| Paper | 10 | 8 | 2 |
| Round-1 review | 8 | 6 | 2 |

Root cause (`nm` evidence): `sized`'s monomorphization key
(`instantiation_symbol`, `src/ast.rs:2869`) renders the grounded type's
*rendered name* (`"Widget_i64_"`) for its `Type::Struct` fall-through arm
(`:2886`), identical for a's and b's distinct `StructId`s. Both groundings'
`CallInst`s (each with its own correct, separate `trait_calls` map —
`src/ast.rs:2808`) therefore render the identical lowering symbol; lowering's
dedup (`src/ir/driver.rs:350-373`, a `HashSet<String>` keyed on that symbol,
iterating a randomized `HashMap`) keeps only the first `CallInst` reached and
discards the other whole — not a span-keyed last-writer-wins inside one shared
map (an earlier reading of this trace; see the errata in
[slice9-probes](./slice9-probes.md)).
**The flakiness itself is the pre-fix evidence** — note it (e.g. run N× pre-fix
in a scratch check) rather than asserting a ratio.
**After (expected):** deterministic `1\n2`.
**Witnesses:** roadmap exit item 2 (corrects the roadmap's assumed "silent `1 1`"
to "nondeterministic `1 1`/`2 2`"); H1/H5 — one finding, resolved by widening
`instantiation_symbol`'s fall-through arm to render `(StructId, module)`
provenance (spec R2.1), not the rendered name alone.

---

## G2r — `cross_module_same_shaped_impls_eager_minter_wins_regardless_of_caller` (new)

Provenance mirror of G1: `f.sth` identical to G1's; `a` gets the `mk` (eager
`Widget[i64]`), `b`'s consumer stays bare (`Widget sized`, no annotation) —
isolates "first eager mint wins". CORRECTION (measured at implementation,
2026-09-04): this isolation does not hold end-to-end — once the V2 fix mints
the two distinct caller-owned groundings, both of G2r's calls go through the
shared bound word `sized`, whose two groundings collide in
`instantiation_symbol`'s fall-through (the V3 defect), so post-V2-fix G2r is
nondeterministic (`2\n2` 4/6, `1\n1` 2/6). Its golden therefore lands with
Phase 3 (after R2.1), not Phase 2; G1 remains Phase 2's end-to-end pin.

```sth
// a.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 1 ; ;
: mk ( i64 -- Widget[i64] ) Widget ;
: run ( i64 -- i64 ) mk sized ;
export: run ;

// b.sth
import: intrinsics * ; import: self::f * ;
type: Widget['T] v 'T ;
impl: Sized for Widget : size drop 2 ; ;
: run ( i64 -- i64 ) Widget sized ;
export: run ;

// main.sth
import: intrinsics * ; import: hosted::show | . | ;
import: self::f * ; import: self::a ; import: self::b ;
: main ( -- ) 5 a::run . 5 b::run . ;
```

**Command:** `build_and_run(main.sth)`
**Before (validated, 5 rebuild+run cycles, identical):** `1\n1`, deterministic,
exit 0.
**After (expected):** `1\n2`.
**Witnesses:** H2/V2, directly and deterministically. **Recommend as the primary
regression pin for the provenance fix**; G1 as secondary (same bug, opposite
eager side).

---

## G3 — `duplicate_blanket_impl_across_modules_is_a_declared_error`

Investigated per the probe round's open item (P5b found concrete-target
ambiguity apparently unconstructible). Two *independent* modules (no import
edge between them) each declare `impl: Sized for 'T`, pulled into one program
via a third module. **Note:** this fixture's `a.sth` is a *different program*
from G1/G2/G2r's `a.sth` (same filename, unrelated content — the two never
coexist in one build; if implementing goldens in one shared test module, name
this one's files distinctly, e.g. `dup-a.sth`/`dup-b.sth`, to avoid confusion).

```sth
// f.sth
import: intrinsics * ;
trait: Sized['S] : size ( 'S -- i64 ) ; ;
: sized['S: Sized] ( 'S -- i64 ) size ;
export: Sized sized ;

// a.sth (dup-a.sth)
import: intrinsics * ;
import: self::f * ;
impl: Sized for 'T
  : size drop 1 ;
;
: run ( i64 -- i64 ) sized ;
export: run ;

// b2.sth (dup-b.sth)
import: intrinsics * ;
import: self::f * ;
impl: Sized for 'T
  : size drop 2 ;
;
: run2 ( i64 -- i64 ) sized ;
export: run2 ;

// main2.sth
import: intrinsics * ; import: hosted::show | . | ;
import: self::f * ; import: self::a ; import: self::b2 ;
: main ( -- ) 5 a::run . 5 b2::run2 . ;
```

**Command:** `build_error(main2.sth)`
**Before (validated, exact reconstruction, built against this HEAD):**

```text
error: duplicate `impl:` for `'T` (line 3, col 1); first declared at line 3, col 1
```

Exit code **1** (the probe log's verbatim `exit=0` for this and every other
error case is a harness artifact — see the errata in
[slice9-probes](./slice9-probes.md); the compiler exits 1). Line 3, col 1 in
*each* file (`a.sth`'s own `impl:` and `b2.sth`'s own `impl:` are each at their
file's line 3 — the import headers above are load-bearing for this: without
them the `impl:` line shifts and the byte-exact pin breaks). Confirmed
module-blind: `a` and `b2` share no import edge; the error fires purely from
joint assembly.

**Verdict:** roadmap exit item 3 ("a real ambiguity error, not a silent pick")
**is already satisfied**, but not by a new dispatch-time mechanism — it's the
pre-existing declaration-time duplicate check (`check_impl_decls`,
`src/check/declarations.rs:544`, "P7.S4 (R7)"), which compares `target.pattern`
by `PolyType` structural equality, module-blind *by design*: a bare
`PolyType::Var` carries no header identity (unlike `PolyType::Generic`'s
`(idx, module)`), so any two catch-all impls for one trait always collide,
anywhere. This is why `pb2`'s two `impl: Sized for Widget` (distinct
`(idx, module)` headers) don't trigger it. Other shapes tried and ruled
unconstructible (VERDICT.md §3 P5b): impl-in-trait's-module targeting another
module's type forces an import cycle or fails the placement rule
(`declarations.rs:588`); a third module can't even import two same-named
concrete types at once (selective-import collision fires first). **No
concrete-target dispatch-time ambiguity is constructible** once identity-correct
matching is in place — distinct modules' headers have distinct `(idx, module)`
identity and the import system prevents a caller ever holding an ambiguous
operand.
**Witnesses:** roadmap exit item 3, reworded. **Recommend:** keep this golden as
a regression pin for existing behavior, not evidence of new S9 work; reword the
roadmap's implied "add a dispatch-time ambiguity error" framing.

---

## G4 — `third_module_mono_caller_is_not_silently_cross_picked`

P5a shape: a third module wildcard-imports `a` and `b` and makes an unbounded,
un-annotated mono call. Reuses G1's `f.sth`/`a.sth`/`b.sth` verbatim.

```sth
// f.sth, a.sth, b.sth: identical to G1

// c.sth
import: intrinsics * ; import: self::f * ;
import: self::a ; import: self::b ;
: try ( i64 -- i64 ) Widget size ;
export: try ;

// main.sth
import: intrinsics * ; import: hosted::show | . | ;
import: self::c ;
: main ( -- ) 5 c::try . ;
```

**Command:** `build_and_run(main.sth)`
**Before (validated, 3 rebuild+run cycles, identical):** `2`, exit 0, no
diagnostic. Same root cause as G1/H2: one collapsed `Widget[i64]` `StructId`
(b's), so `c`'s bare ctor call has no other candidate.
**After — a Phase-4 determination, not asserted here.** R1.1's own rule
("ground at the caller's own resolved header") is in tension with assuming `c`
automatically gets a 2-candidate collision: `c` declares no `Widget` header at
all, and probes P5a-ii separately measured that a third module naming
`Widget[i64]` explicitly with no header of its own is a hard `unknown type`
error. Phase 4 must build this fixture against the landed Phase-2/3 fix and
observe which actually happens:

- **(a)** `c`'s bare ctor call surfaces a 2-candidate `select_overload`
  collision (e.g. because both mints are visible through `c`'s wildcard
  imports) → per S5's tier policy `c` is neither declaring module →
  ambiguity tier → compile-time error. Post-fix assertion: `build_error`
  pinning a located message naming `Widget`, both candidate modules, and `c`'s
  call site — measured against the fixture, not re-derived.
- **(b)** `c`'s bare ctor grounds against no visible header at all (consistent
  with P5a-ii and R1.1's own-header rule) and errors earlier, with whatever
  message the mechanism actually produces. Post-fix assertion: `build_error`
  pinning that measured text.

Either outcome satisfies the roadmap's "real ambiguity error, not a silent
guess" intent; build first, pin second.
**Witnesses:** exit item 3's other half; the concrete fixture R3's Phase-4
determination attaches to.

---

## G5 — regression pins (no new fixtures)

- **#10** `same_named_ctors_in_two_modules_dispatch_distinct_impls`
  (`tests/phase7b_slice2.rs:655`, assert `:696`). Load-bearing: the only
  existing golden exercising per-operand trait-impl dispatch with **distinct**
  substitutions (`i64`/`str`) across modules — the fix must not regress it
  while repairing the same-substitution (`pb2`) case. Must keep printing
  `1\n2`.
- **s5-tier1** `cross_module_same_shaped_ctor_dispatches_callers_own_impl`
  (`tests/phase7b_slice5.rs:100`, assert `:129`). Load-bearing: pins S5's
  ctor/destructure tier policy (`select_overload`), a different registry from
  S9's (`find_bound_impl`/dispatch identity); must keep printing `15\n25`.
- **`same_named_ctor_mk_ambiguity_resolves_but_impl_dispatch_still_cross_picks`**
  (`tests/phase7b_slice4.rs:427-490`) — **not** a stable regression pin as it
  stands: same shape as G2, a different test file, hard-pinned to the pre-fix
  coin-flip `1\n1`, and reds ~3/8 on rerun. Must be re-pinned (not merely kept
  green) to deterministic `1\n2` in the same phase as G2, and renamed off its
  dead pre-fix criterion.

---

## Unit-test sketches (`thing_condition_expected` naming; fix sites named in the spec)

- `instantiation_symbol_same_rendered_name_different_struct_ids_mints_distinct_symbols`
  — two groundings with the same rendered type name but different
  `(StructId, module)` provenance must not collapse to one specialization.
  (Near `instantiation_symbol`, `src/ast.rs:2869` — this is the mono key
  itself, not a separate site to bring into alignment with it.)
- `bare_ctor_operand_provenance_is_callers_own_header_not_a_borrowed_mint`
  (final name pending Phase 1's R1.1a/R1.1b verdict) — a lazily-minted,
  un-annotated ctor operand's provenance is never borrowed from an unrelated
  eager mint. A bare `Widget` construction with no `Widget[i64]` named
  anywhere in its own module must mint its own instantiation keyed to its own
  header. (V2's fix site; F1/F3/H3 show the matcher and pattern resolution are
  already sound — this unit does not touch them.)
- `check_impl_decls_duplicate_blanket_impl_across_modules_still_errors` —
  regression: two `impl: Trait for 'T` in different modules still produce
  `duplicate_impl_error` after the fix (G3's mechanism must not be touched by
  a fix scoped to `instantiation_symbol`'s `Struct`/`Enum` arm).

---

## Spec decisions this paper round defers (resolved in slice9-spec.md)

1. **Fix site for H2**: ctor construction vs. `find_bound_impl` operand
   normalization vs. both. *Resolved:* R1, gated on a Phase-1 diagnosis.
2. **Fix site for H1/H5**: *Resolved:* one fix, one site — widen
   `instantiation_symbol`'s `Struct`/`Enum` fall-through arm (R2.1).
   `trait_calls`/`builtin_overloads` are never touched: they are already
   correct per-instantiation (F5); re-keying them is both inert (the
   collision is in lowering's symbol-keyed dedup, not in these maps) and out
   of bounds (lowering-consumed — R2.2 withdrawn).
3. **G4's after-column**: *Resolved as a procedure*, not a fixed outcome — R3,
   Phase 4 builds the fixture against the landed fix and determines which of
   (a)/(b) actually happens (R1.1's own-header rule is in tension with
   assuming (a) automatically; probes P5a-ii already measured a
   no-own-header third module hitting `unknown type` earlier).
4. **G3's framing**: *Resolved:* exit item 3 accepted as already satisfied by
   the pre-existing declaration-time check; no new dispatch-time diagnostic.
5. **Roadmap correction** (brief's Q5): *Resolved and widened* — R7 names five
   edit targets, not one (two roadmap sentences, one stale anchor at two
   sites, two falsified `slice5-spec.md` claims).
