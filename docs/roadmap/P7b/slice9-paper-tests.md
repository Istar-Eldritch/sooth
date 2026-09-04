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
**build-time** effect (Rust's `HashMap` reseed happens once per `sooth build`
process), not runtime. One built binary is 100% deterministic on repeat execution;
only rebuilding flips the outcome. All "before" run counts below are rebuild+run
cycles.

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
`mk sized`; otherwise identical to G1 (`/tmp/p7bs9-probes/mkvar/`).

**Command:** `build_and_run(main.sth)`
**Before (validated, 10 rebuild+run cycles):** **nondeterministic** — `1\n1` 8/10,
`2\n2` 2/10 (this measurement; the probe round's own run measured 6/10 vs 4/10 —
both confirm the flip, the exact ratio is not stable and must not be asserted in
committed code). Root cause (`nm` evidence, VERDICT.md §2): `sized`'s
monomorphization key is the grounded type's *rendered name* (`"Widget_i64_"`),
identical for a's and b's distinct `StructId`s, so both callers share one compiled
`sized` body whose single internal `size` call is wired through a
`HashMap<Span, String>` keyed on one fixed span — last-writer-wins, reseeded per
compiler process.
**The flakiness itself is the pre-fix evidence** — note it (e.g. run 10x pre-fix
in a scratch check) rather than asserting a ratio.
**After (expected):** deterministic `1\n2`.
**Witnesses:** roadmap exit item 2 (corrects the roadmap's assumed "silent `1 1`"
to "nondeterministic `1 1`/`2 2`"); H1/H5 — one finding, `sized`'s monomorphization
identity must key on `(StructId, module)`, not the rendered name alone.
**Defers:** monomorphization-key widening vs. obligation-routing key widening
(grounding-keyed, not span-keyed) vs. both.

---

## G2r — `cross_module_same_shaped_impls_eager_minter_wins_regardless_of_caller` (new)

Provenance mirror of G1: `a` gets the `mk` (eager `Widget[i64]`), `b`'s consumer
stays bare (`Widget sized`, no annotation) — isolates "first eager mint wins"
without G2's `HashMap`-reseed noise.

```sth
// a.sth: type: Widget['T] v 'T ; impl: Sized for Widget : size drop 1 ; ;
//        : mk ( i64 -- Widget[i64] ) Widget ; : run ( i64 -- i64 ) mk sized ; export: run ;
// b.sth: type: Widget['T] v 'T ; impl: Sized for Widget : size drop 2 ; ;
//        : run ( i64 -- i64 ) Widget sized ; export: run ;
// main.sth: identical shape to G1
```

**Command:** `build_and_run(main.sth)`
**Before (validated, 7 rebuild+run cycles, identical):** `1\n1`, deterministic.
**After (expected):** `1\n2`.
**Witnesses:** H2, directly and deterministically. **Recommend as the primary
regression pin for the provenance fix**; G1 as secondary (same bug, opposite
eager side).
**Defers:** same as G1.

---

## G3 — `duplicate_blanket_impl_across_modules_is_a_declared_error`

Investigated per the probe round's open item (P5b found concrete-target
ambiguity apparently unconstructible). Two *independent* modules (no import
edge between them) each declare `impl: Sized for 'T`, pulled into one program
via a third module.

```sth
// a.sth: impl: Sized for 'T : size drop 1 ; ; : run ( i64 -- i64 ) sized ; export: run ;
// b2.sth: impl: Sized for 'T : size drop 2 ; ; : run2 ( i64 -- i64 ) sized ; export: run2 ;
// main2.sth: imports f, a, b2; : main ( -- ) 5 a::run . 5 b2::run2 . ;
```

**Command:** `build_error(main2.sth)`
**Before (validated):**

```text
error: duplicate `impl:` for `'T` (line 3, col 1); first declared at line 3, col 1
```

Confirmed module-blind: `a` and `b2` share no import edge; the error fires purely
from joint assembly.

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
**Defers:** accept exit item 3 as satisfied as-is (recommended), or specify an
additional narrower diagnostic for some other shape (none found).

---

## G4 — `third_module_mono_caller_sees_the_wrong_impl_silently`

P5a shape: a third module wildcard-imports `a` and `b` and makes an unbounded,
un-annotated mono call.

```sth
// c.sth
import: intrinsics * ; import: self::f * ; import: self::a ; import: self::b ;
: try ( i64 -- i64 ) Widget size ;
export: try ;
// main.sth: import: self::c ; : main ( -- ) 5 c::try . ;
```

**Command:** `build_and_run(main.sth)`
**Before (validated, 3 rebuild+run cycles, identical):** `2`, exit 0, no
diagnostic. Same root cause as G1/H2: one collapsed `Widget[i64]` `StructId`
(b's), so `c`'s bare ctor call has no other candidate.
**After — spec decision, not asserted here.** Two options:

- **(a, recommended)** the provenance fix makes `c`'s bare ctor mint its own
  instantiation (neither `a`'s nor `b`'s header is privileged from `c`'s
  vantage) → a new 2-candidate collision at `c`'s own ctor call; per S5's tier
  policy `c` is neither module → ambiguity tier → compile-time error.
- **(b)** the fix instead resolves deterministically some other way (e.g. first
  visible import) → silent-but-deterministic pick, no error.
- (a) is consistent with S5's tiers and this slice's exit wording
  ("case-appropriate visibility... a real ambiguity error"); post-fix assertion
  would be `build_error` pinning a located message naming `Widget`, both
  candidate modules, and `c`'s call site.
**Witnesses:** exit item 3's other half; brief Q3 (visibility)/Q4 (tier
interplay) — this is the concrete fixture those questions attach to.
**Defers:** entire after-column (spec must rule (a)/(b)/other).

---

## G5 — regression pins (no new fixtures)

- **#10** `same_named_ctors_in_two_modules_dispatch_distinct_impls`
  (`tests/phase7b_slice2.rs:649`). Load-bearing: the only existing golden
  exercising per-operand trait-impl dispatch with **distinct** substitutions
  (`i64`/`str`) across modules — the fix must not regress it while repairing
  the same-substitution (`pb2`) case. Must keep printing `1\n2`.
- **s5-tier1** `cross_module_same_shaped_ctor_dispatches_callers_own_impl`
  (`tests/phase7b_slice5.rs:90`). Load-bearing: pins S5's ctor/destructure tier
  policy (`select_overload`), a different registry from S9's
  (`find_bound_impl`/dispatch identity); must keep printing `15\n25`.

---

## Unit-test sketches (behaviour-level; fix sites are the spec's decision)

- **Monomorphization identity distinguishes same-rendered-name, different-
  `StructId` groundings.** Two groundings with the same rendered type name but
  different `(StructId, module)` provenance must not collapse to one
  specialization. (Near `instantiation_symbol`, `src/ast.rs:2869`.)
- **Obligation/dispatch routing is keyed per grounding, not per source span.**
  A member call inside a shared bound word, called from two distinct
  groundings, must record two independent decisions, not one last-writer-wins
  entry. (`HashMap<Span, String>` shapes at `src/check/poly.rs:929/1046/1116/
  2396`.)
- **A lazily-minted, un-annotated ctor operand's provenance is not borrowed
  from an unrelated eager mint.** A bare `Widget` construction with no
  `Widget[i64]` named anywhere in its own module must mint its own
  instantiation keyed to its own header, or the checker must not silently
  substitute another module's. (H2's fix site — likely the ctor construction
  path feeding `find_bound_impl`'s operand, not the matcher itself; F1/F3/H3
  show the matcher and pattern resolution are already sound.)
- **`check_impl_decls`'s blanket-impl duplicate check is unaffected.** A
  regression unit test confirming two `impl: Trait for 'T` in different
  modules still produce `duplicate_impl_error` after the fix (G3's mechanism
  must not be touched by a fix scoped to `Generic`-pattern dispatch).

---

## Spec decisions this paper round defers

1. **Fix site for H2**: ctor construction vs. `find_bound_impl` operand
   normalization vs. both.
2. **Fix site for H1/H5**: widen `sized`'s specialization key, widen the
   `trait_calls`/`builtin_overloads` key from `Span` to grounding-aware, or
   both — one fix or two.
3. **G4's after-column**: rule (a) [ambiguity error, recommended] vs (b)
   [deterministic pick] vs a third option.
4. **G3's framing**: accept exit item 3 as already satisfied (recommended),
   or specify an additional dispatch-time diagnostic for some other shape
   (none found constructible).
5. **Roadmap correction** (brief's Q5): once 1–2 are ruled, rewrite the S9
   roadmap entry's mechanism sentence (operand provenance + monomorphization
   identity, not "no module-identity check in `match_impl_target`").
