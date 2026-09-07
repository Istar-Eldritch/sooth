# Spec: P7b.S8b — the construction-wall fix, the pre-existing two-defect pair it exposes, and traitful `List` members (`map`, `append`)

**Status:** Draft
**Created:** 2026-09-07
**Discovery:** [slice8b-brief](./slice8b-brief.md) (the S8 carve-out record, wall delta,
probe questions PB-1..PB-6). Evidence base: the P8b probe round — segfault root-cause log
(`/tmp/sooth_p8b_segfault.md`, 443 lines verbatim), spellings log
(`/tmp/sooth_p8b_spellings.md`), and the consolidated ledger
(`/tmp/sooth-p7b-probe-ledger.md`); all three are to be persisted verbatim into
[the slice's probe log](./slice8b-probes.md) (Phase 1 / Phase 4). Roadmap entry:
[P7b-higher-kinded-types.md](../P7b-higher-kinded-types.md), "P7b.S8b".
**Base:** `p7b-s8b` worktree clean at `c406149` (`445a74e` — S6/S6b/S6c, S7, S8, S8c, S9,
S10 merged — plus the brief commit). All `path:line` anchors below were verified on that
clean tree; the probe round's own line numbers were taken on `c406149 + /tmp/p8b-arm.patch`
(+60 lines after `poly.rs:6190`), so they differ — re-locate by symbol name, not by the
probe's numbers.

## Problem Statement

Two blockers, one shared root, plus a pair of pre-existing defects the first blocker's fix
exposes. S6 recorded a construction wall: any polymorphic body over `List['T]` that
*reconstructs* a `Cons` panics in `poly_bind_construction_arg`
(`src/check/poly.rs:6074`) because the self-reference field `^List['T]` arrives as a bare
`PolyType::Generic` no arm covers — the catch-all at `src/check/poly.rs:6193` fires
`unreachable!`. This wall blocks exactly the members the roadmap wants next: per-impl
traitful `map` (Functor) and `append` (Monoid) over the real `core::list`, S6's two dropped
goldens. The P8b probe round measured the wall's fix (one checker arm; patch verified
green in-suite) — and then found that lifting the wall *exposes* a pre-existing S6-era
two-defect bug: `empty[List[i64]]` over a generic-target impl seeds the member's θ
positionally instead of through the impl-target equation (mono returns `List[List[i64]]`),
and the wrong mint's variant words clobber the lowering-side bare-name last-write-wins
variant map, so **every** `Cons`/`Nil` construction program-wide lowers with wrong field
shapes — a silent 40-byte layout corruption that SIGSEGVs any program containing both a
prior construction and an `empty[<inst>]` call. The crash reproduces at base `c406149`
with no self-reference field at all (`type: Opt['T]`, `impl: Monoid for Opt`), so it is
not caused by the arm — but shipping the arm without the pair-fixed turns a compile-time
panic into a silent program-global miscompile for exactly the programs S8b exists to
enable. S8b therefore delivers three things as one slice: the wall arm, the two-defect
fix (required, not optional), and the traitful `List` surface (`map`, `append`) as goldens.

## Requirements

- **R1.** The system must bind a bare `PolyType::Generic` construction field in
  `poly_bind_construction_arg` positionally against a `Generic` operand carrying the same
  ctor identity (`is_enum`, `idx`, `module`), recursing over field args vs operand args —
  so both P8 shapes ground and run: an impl member body constructing `Cons` (P8-2b) and a
  plain generic word constructing `Cons` (P8-2d2).
- **R2.** The system must reject, with a located `poly_rendered_type_mismatch_error`
  (byte-exact pinned), any operand reaching the new arm that is not a `Generic` of the
  same identity — including a `Concrete` operand (PB-3: only `Generic` operands are
  observed reaching the arm today; everything else mirrors the `App` arm's rejection) —
  never a panic, never a silent bind.
- **R3.** The system must reject, as a located mismatch (recorded fence, PB-2), a
  `Generic` field carrying a non-empty `len_args` — the stored `^List['T]` field is
  `len_args: []` today and the arm's contract must say what a future length-carrying
  field gets.
- **R4.** A nullary trait member called with an explicit type argument over a **generic**
  impl target must instantiate the member through the impl-target equation —
  `empty[List[i64]]` over `impl: Monoid for List` seeds the member's element variable from
  matching the impl target `List['E]` against the call-site type (`'E := i64`), minting a
  monomorph declared and returning `List[i64]` (a 24-byte Nil, SSA-verifiable) — never
  `List[List[i64]]`. The concrete-target nullary path (S6's `empty[i64]` golden) stays
  byte-unchanged.
- **R5.** A checker-resolved enum construction/destructure/eliminator site must lower with
  its own resolved instantiation's field shapes even when another instantiation of the same
  header is minted later in the program — the bare-name last-write-wins variant-word map
  (`src/ir/layout.rs:597-640`, hazard pre-documented at `src/ast.rs:1421-1424`) must no
  longer decide any checker-resolved site.
- **R6.** The Opt-shaped base repro (`type: Opt['T] | None | Some 'T`, `impl: Monoid for
  Opt` with no self-reference field, a prior `Some` construction, then
  `empty[Opt[i64]]`) must build and run exit 0 — it SIGSEGVs at base `c406149` — pinned as
  a golden independent of the List wall.
- **R7.** The S6 recorded-wall witness (`tests/phase7b_slice6.rs:410`,
  `monoid_for_list_append_construction_wall_is_recorded`) must flip to a positive golden:
  `Monoid for List` append builds, runs, and drops clean (ruled: flip, not retire — the
  probe patch's hunk is the measured candidate and preserves the wall's history in its doc
  comment and the probes log).
- **R8.** `impl: Functor for List` with the S6 `Functor` member signature
  (`map ( 'F['T] [ 'T -- 'U ] -- 'F['U] )`) must ground end-to-end: a consumer dispatching
  through a shared `Functor` bound with the `'U := 'T` specialization maps a real
  `List[i64]`, the member lowers as one non-inline real frame (S6 phase-3 convention —
  non-tail self-recursion needs no new mechanism), and dropping the mapped list disposes
  the constructed `Cons` chain per instantiation with constant stack (S6's destructor
  goldens must not regress).
- **R9.** `append` must ground through the ruled host trait `Monoid for List` (PB-5
  ruling: `combine` = append, `empty` = `Nil`): `combine` through a shared `Monoid` bound
  appends two `List[i64]` spines (prints `1 2 3 5 3`); `empty` routes both verified ways —
  explicit `empty[List[i64]]` in a mono main and bound-directed inside a poly body (the
  existing `mconcat_over_list_dispatches` golden, `tests/phase7b_slice6.rs:359`, stays
  byte-identical).
- **R10.** Linearity teeth on the new constructions (PB-6): an undropped `map` or
  `append` result is a compile error — `drop` is the explicit destructor, nothing
  auto-drops.
- **R11.** `dup` of a `List['T]` operand stays fenced byte-exact (the conservative
  linearity fence, `poly_copy_gate` at `src/check/poly.rs:6826`, `Generic`/`App` arms at
  `:6883`/`:6900`).
- **R12.** The spelling fences measured by the spellings probe round stay byte-identical:
  a map producing a distinct `'U` through a shared bound remains unspellable (inference
  does not bind output-only vars; quotation types rejected against plain-var slots;
  poly→poly quotation passing fenced), with the verified substitutes (`'U := 'T`
  specialization, composition, mono middleman) recorded in the roadmap entry — this is a
  recorded fence, not an S8b deliverable.
- **R13.** Ship surface (ruled): the traitful `List` surface lands as golden fixtures
  under `tests/` per S6's convention — trait declarations, impls, and consumers live in
  the golden programs, only `lib/core/list.sth` (already shipped) and the unchanged
  `lib/core/sooth.pkg` module list ship from this slice; no new lib modules. `src/ir/`
  stays diff-empty for the whole slice (see Scope & Boundaries for how this survives the
  lowering-side defect).
- **R14.** Non-functional gate: `cargo fmt --check` clean, `cargo clippy -- -D warnings`
  clean, full `cargo test` green; every pre-existing golden byte-unchanged (`mklist`,
  `Range`'s `next`, S8's 24 slice tests, S6's suite); unit tests beside every changed
  stage function (CLAUDE.md stage convention); diagnostics are behaviour — every new
  error pinned byte-exact.

## Success Criteria

- [ ] The Opt repro program builds, runs exit 0, prints `ok` (was SIGSEGV at base) — R6.
- [ ] An SSA dump of the `empty[Opt[i64]]`-only program shows the mono declared and
      returning the correctly-typed (one-level) instantiation, not `Opt[Opt[i64]]` — R4.
- [ ] A program minting two instantiations of one generic enum and constructing a
      payload variant by bare name in a mono body *before* the second mint runs with
      correct field shapes for both — R5.
- [ ] Both P8 shapes (impl member body constructing `Cons`; plain generic word
      constructing `Cons`) build and run — R1.
- [ ] A ctor-mismatch operand (constructing `Cons` against a differently-headed operand)
      produces the byte-exact located mismatch error, and a `Concrete` operand at the new
      arm is likewise a located error, not a panic — R2.
- [ ] A `Generic` field with non-empty `len_args` is a located error (unit test) — R3.
- [ ] `tests/phase7b_slice6.rs`'s wall witness passes as a positive golden (build + run
      exit 0) — R7.
- [ ] `map` through a `Functor` bound maps a real `List[i64]` (e.g. `1 2 3` → `2 3 4`
      printed) and the mapped list drops clean — R8.
- [ ] `combine` through a `Monoid` bound prints `1 2 3 5 3`; `empty[List[i64]]` in a mono
      main runs green; the S6 `mconcat_over_list_dispatches` golden is byte-identical — R9.
- [ ] An undropped `map`/`append` result fails the build with a located error — R10.
- [ ] `dup` of a `List['T]` operand produces the byte-exact `poly_copy_generic_error`
      fence — R11.
- [ ] The distinct-`'U` fence error is byte-identical to the spellings probe's captures —
      R12.
- [ ] `git diff c406149..HEAD -- src/ir/` is empty; `lib/` and `lib/core/sooth.pkg` are
      unchanged — R13.
- [ ] Full gate green; unit tests exist beside `poly_bind_construction_arg`, the changed
      member-dispatch code, and the changed term-resolution code — R14.

## Scope & Boundaries

**In scope:**

- The wall arm: one `PolyType::Generic` field arm in `poly_bind_construction_arg`
  (`src/check/poly.rs:6074`), inserted before the catch-all at `:6193`; located-error
  fencing; byte-exact pins; unit tests beside the function.
- The pre-existing two-defect fix, as a required phase: (a) θ seeding for the nullary
  generic-target member path through the impl-target equation; (b) per-instantiation
  resolution of checker-resolved enum-word sites. Both root causes are probe-verified at
  `c406149`; the Opt repro golden pins the fix independent of List.
- Per-impl traitful `List` members `map` and `append` over the ruled host traits
  (`Functor for List`, `Monoid for List`), as golden fixtures.
- The witness flip (`tests/phase7b_slice6.rs:410`), linearity-teeth pins, the distinct-`'U`
  fence record, the roadmap S8b entry, and the persisted probe logs.

**Out of scope** (brief's list, plus probe-round additions):

- Iterator adaptors (`zip`/`take`/`rev`/...) and lazy/streaming iteration.
- A bound-generic `map` producing `'It['U]` (recorded impossible, P8-5b/5c).
- Consuming-context grounding for nullary members (S6 R4's future slice) — and its
  construction-side twin observed this round: a bare **nullary variant ctor** of a generic
  header in a mono body grounds at the single first candidate regardless of the expected
  output (verified: the sig check catches it as a located mismatch; the user cannot spell
  a second-instantiation nullary construction). Recorded, not fixed.
- The distinct-`'U` map spelling (R12: recorded fence with verified substitutes).
- The struct-word twin of the bare-key hazard (two same-module instantiations of a generic
  **struct** sharing a bare ctor name in unrecorded mono sites — the same last-write-wins
  class, pre-documented at `src/ast.rs:1421-1424`); likewise cross-module same-named
  variant names in the flat `enums.words` map. Both are pre-existing, have no S8b-blocking
  repro, and are recorded for a future slice.
- The D5 borrow gate; numeric traits; phantom parameters; the QBE backend; new
  trait-declaration syntax; the array-as-`'F` kind story (S6b); S8's residual
  per-`next`-call frame question.
- **`src/ir/` changes — the brief expected the whole slice diff-empty here, and that
  still holds**: the probe's "re-key the lowering-side variant-word map" recommendation is
  *superseded* by the verified cheaper fix (checker-side per-site recording through the
  existing `builtin_overloads` channel — see Solution Approach), so `src/ir/` stays
  diff-empty for all phases. If Phase 1's implementer finds a checker-invisible
  construction route that must lower correctly, that is a stop-and-record condition, not
  license to edit `src/ir/`.

## Solution Approach

Three pieces, one slice. First, the **wall arm**: `poly_bind_construction_arg`'s catch-all
(`src/check/poly.rs:6193`) gains a `PolyType::Generic` field arm — the `App` arm's twin
with the header already concrete. The operand must be a `Generic` naming the same header
(identity is the `GenericId` triple `is_enum`/`idx`/`module`, compared the way
`match_impl_target_rec`'s `Generic` arm compares `(idx, module)`); anything else is the
same located `poly_rendered_type_mismatch_error` the `App` arm raises for non-`Generic`
operands. With identity matched, the bind recurses positionally over the field's args
against the operand's own args — the header itself is already fixed, so only the arguments
bind — and grounding to the concrete instantiation remains `apply_subst`'s job
(`src/check/poly.rs:10651`), as for every sibling arm. The probe round already wrote and
verified this arm (`/tmp/p8b-arm.patch`: the arm plus the flipped witness, suite green
3293/0), so Phase 2 lands a measured patch, not a design.

Second, the **two-defect fix** the wall-lift exposes, both defects pre-existing at base
and probe-root-caused. (a) *θ seeding*: `resolve_mono_member_call`'s nullary branch
(`src/check/poly.rs:2363-2391`) correctly dispatches via `find_bound_impl`
(`:8613`) on the call-site type, but then falls into the shared generic branch and hands
the raw call-site `type_args` to `check_poly_call` (`:2625`), whose P7.S3t seeding
(`:7366-7391`) binds them *positionally* — variable *i* gets argument *i*. For a
generic-target impl the member word's variable #0 is the impl header's element var, so
`empty[List[i64]]` seeds `'E := List[i64]` instead of `'E := i64`. The fix derives the
member-word θ from the impl-target equation — the same substitution
`match_impl_target` (`src/check/poly.rs:9254`) computes when matching the impl target
pattern against the call-site type — and seeds from it, leaving the positional contract
untouched for every other caller. (b) *variant words*: a mono-body construction site
checked while only one instantiation of the header exists takes the single-candidate arm
of the term chooser (`src/check/terms.rs:962-980`), which records nothing (the
multi-candidate arm at `:1025-1028` already records `builtin_overloads[span] =
chosen.symbol`, and for a minted variant that symbol is the per-instantiation registry
spelling — `Cons[i64]`-style — exactly the mangled key `enums.words` carries,
`src/check/declarations.rs:1876` + `src/ir/layout.rs:601-610`). A later mint then
overwrites the bare surface key and the unrecorded site lowers from the clobbered entry.
The fix records the resolved symbol at the single-candidate arm too (for generated enum
words, the `splice_enum_site` machinery's non-splice twin at `src/check/terms.rs:2060`
already knows how to identify them); lowering needs **no change** — its
`builtin_overloads` read (`src/ir/func_builder/calls.rs:480`) already precedes the
struct/enum arms and already dispatches mangled enum keys (the D7/R5 precedent recorded
there). That keeps `src/ir/` diff-empty and retires the documented last-write-wins hazard
for every checker-resolved site, which is all of them after the fix.

Third, the **traitful `List` surface**, as S6-convention goldens rather than lib modules
(ruled; the S6 precedent landed `Monoid`/`Functor`/`Foldable` as golden programs in
`tests/phase7b_slice6.rs` with only `list.sth` shipping, and the spellings probe verified
the exact shapes: `combine` through a `Monoid` bound grounds and prints `1 2 3 5 3`,
`empty` routes both ways, distinct-`'U` is fenced with working substitutes). The host
trait is ruled `Monoid for List` (`combine` = append, `empty` = `Nil`) — every
List-specific spelling fences per the probe ledger, and S6's recorded-wall shape named
exactly this impl; `map` rides `Functor for List` with the S6 member signature. Member
bodies follow the shipped non-inline patterns: `impl: Foldable for List`'s recursive
destructure-only body (`tests/phase7b_slice6.rs:181`) and the witness's
reconstructing append body (`:410`), composed with the S6 `Functor for Option` map body
(`:105`). Sequencing follows the dependency structure: the two-defect fix (Phase 1) and
the wall arm (Phase 2) are independent — the Opt repro expresses the crash without the
arm, and the flipped witness fixture contains no `empty[…]` call, so it greens without
Phase 1 — and the goldens (Phase 3) need both.

## Codebase Map

| Location | Symbol | Role in this work |
|----------|--------|-------------------|
| `src/check/poly.rs:6074` | `poly_bind_construction_arg()` | The wall site: gains the `Generic` field arm before the catch-all |
| `src/check/poly.rs:6193` | catch-all `other => unreachable!` | The panic this slice removes for the `Generic` shape; other shapes' text unchanged |
| `src/check/poly.rs:6124-6170` | `PolyType::App` arm | The pattern the new arm mirrors (identity check, positional recursion, `poly_rendered_type_mismatch_error` for non-`Generic` operands) |
| `src/check/poly.rs:6172-6191` | `PolyType::OwnedCell` arm (S6 phase 3) | The destructure-side twin already handled; insertion neighbor |
| `src/check/poly.rs:2274` | `resolve_mono_member_call()` | Member dispatch; its nullary branch + generic branch carry θ-defect (a) |
| `src/check/poly.rs:2363-2391` | nullary rescue branch (S6 Phase 4 R4) | Dispatches `empty[List[i64]]` via `find_bound_impl`; the fix point for θ seeding |
| `src/check/poly.rs:2625-2630` | generic-branch `check_poly_call` handoff | Passes raw call-site `type_args` — the positional mis-seed's route |
| `src/check/poly.rs:7329` | `check_poly_call()` | θ seeding owner; gains a way to receive the impl-target-derived seed |
| `src/check/poly.rs:7366-7391` | P7.S3t positional seeding loop | The contract that must keep holding for all other callers |
| `src/check/poly.rs:7636-7640` | span-keyed `enum_words` recording | Precedent for span-keyed per-site enum resolution (poly bodies) |
| `src/check/poly.rs:9254` / `:9267` | `match_impl_target()` / `match_impl_target_rec()` | Computes the impl-target equation (pattern `List['E]` vs `List[i64]` ⇒ `'E := i64`) — reused for the seed |
| `src/check/poly.rs:8613` | `find_bound_impl()` | The nullary branch's dispatch; unchanged |
| `src/check/poly.rs:6311` | `poly_construct_generic()` | The poly-body construction walk the arm serves |
| `src/check/poly.rs:6826` / `:6883` / `:6900` | `poly_copy_gate()` + `Generic`/`App` arms | The `dup` fence R11 pins (byte-exact) |
| `src/check/poly.rs:10633` / `:10651` / `:12076` | `poly_rendered_type_mismatch_error()` / `apply_subst()` / `poly_type_str()` | The error, grounding, and rendering primitives the arm uses |
| `src/check/poly.rs:11090` | `poly_copy_generic_error()` | R11's pinned message |
| `src/check/poly.rs:14700` / `:14724` | `poly_bind_construction_arg_owned_cell_*` unit tests | The pattern for the new arm's unit tests |
| `src/check/terms.rs:880-916` | candidate merge (`env` hit / `mint_fallback_candidates`) | Where mono generated-word candidates assemble — one recording point covers both |
| `src/check/terms.rs:962-980` | single-candidate arm | Records nothing today — the gap behind defect (b); gains the per-site record |
| `src/check/terms.rs:1025-1028` | multi-candidate arm's `builtin_overloads` insert | The D7/R5 precedent the single-candidate arm now mirrors |
| `src/check/terms.rs:2010` | `mint_fallback_candidates()` | Re-derives minted generated-word candidates on env miss |
| `src/check/terms.rs:2060` | `splice_enum_site()` | Identifies generated enum words and reads the operative `EnumId` — the non-splice twin is the recording route |
| `src/check/terms.rs:1153-1155` | nullary-variant symbol comment | Documents that minted variant symbols are the per-instantiation registry spelling |
| `src/ir/func_builder/calls.rs:403` | `lower_call()` | Lowering's resolution order — must stay diff-empty |
| `src/ir/func_builder/calls.rs:480` | `builtin_overloads` read | Already precedes struct/enum arms and dispatches mangled enum keys — why the checker-side fix suffices |
| `src/ir/func_builder/calls.rs:888-935` | per-site `enum_words` + bare-key fallback | The last-write-wins path unrecorded sites take today; becomes defense-only after Phase 1 |
| `src/ir/func_builder/calls.rs:2502` / `:2547` | `enum_words_dispatches_construction_per_monomorph` / `enum_words_miss_falls_through_to_bare_key_lookup` | Lowering-side regression pins that must stay green |
| `src/ir/layout.rs:597-640` | `ewords` dual-key insert | The last-write-wins map (hazard site — read, not modified) |
| `src/ast.rs:1418-1424` / `:1425` | `instantiate_enum()` doc + fn | Pre-documents the clobber hazard; mints per-instantiation variant spellings |
| `src/check.rs:844-969` | `word_enum_sites` pre-pass | Where poly-body enum sites are recorded (mono bodies are not covered — by design after Phase 1's fix) |
| `src/check/declarations.rs:1876` | `enum_generated_sigs()` | Yields `(surface, mangled symbol, module, sig)` per variant — the symbol `builtin_overloads` must carry |
| `lib/core/list.sth` | `List['T]`, `Nil`, `Cons` | The real container under test; unchanged |
| `lib/core/iterator.sth` | `impl: Iterator for List` (`next`) | The shipped per-impl non-inline member pattern; unchanged |
| `lib/core/sooth.pkg:6` | module list | Must stay unchanged (R13) |
| `tests/phase7b_slice6.rs:410` | `monoid_for_list_append_construction_wall_is_recorded()` | Flips to `monoid_for_list_append_construction_grounds_after_s8b` (probe patch hunk) |
| `tests/phase7b_slice6.rs:359` / `:302` / `:105` / `:181` / `:453` | mconcat / Monoid-i64 / Functor-Option-map / Foldable-List / dogfood goldens | The fixture patterns Phase 3 composes; all byte-unchanged |
| `tests/phase7b_slice8.rs:39` / `:742` | `single_file_hosted` harness / `emit_ssa_with_manifest` IR pin | The harness and SSA-pin pattern for the new test file |
| `tests/fixtures/sooth.pkg` | shared fixture manifest | The `--manifest` all file-based fixtures build under |
| `tests/phase7b_slice8b.rs` | *(new file)* | S8b's goldens; modeled on `tests/phase7b_slice8.rs` |

Load-bearing constraints: backend is QBE, no LLVM; IR stays backend-neutral (`Ptr[T]`
opaque); the linear spine — `dup` explicit copy, `drop` explicit destructor, nothing
auto-drops (R10/R11 are that convention's teeth, not new rules); `core` is `no_std`;
no in-process JIT. Do not edit `src/ir/` (Phase 1's fix is checker-side by design); do
not change `check_poly_call`'s positional seeding contract for non-nullary callers; do
not widen the arm beyond the `Generic` field shape (the catch-all's other shapes are
S6-pinned behavior).

## Open Questions

- [x] ~~PB-1 — does the arm fix both P8 shapes without regressing mono/plain-field
      constructions?~~ Resolved by probe: yes; suite green 3293/0 with the patch.
- [x] ~~PB-2 — `len_args` handling in the new arm?~~ Resolved: recorded fence — a
      length-carrying `Generic` field is a located mismatch (R3).
- [x] ~~PB-3 — which operand shapes reach the arm?~~ Resolved: only `Generic` observed;
      everything else takes the located mismatch (R2), mirroring the `App` arm.
- [x] ~~PB-5 — `append`'s host trait, and how much of S6's wall closes?~~ Resolved:
      `Monoid for List` (combine-through-bound grounds, `1 2 3 5 3`; List-specific
      spellings fence); S8b closes S6's *whole* recorded wall — both dropped goldens
      (`Monoid for List`, `Functor for List`) land.
- [x] ~~PB-6 — linearity teeth?~~ Resolved: undropped results are compile errors; `dup`
      stays fenced (R10/R11).
- [x] ~~Ship surface: S6 convention (golden fixtures) or S8 convention (`core::iterator`
      lib module)?~~ Ruled here: S6 convention (R13) — the surface is trait fixtures, not
      a protocol module.
- [x] ~~Is the two-defect fix in scope for S8b?~~ Resolved by the probe round's verdict:
      required, as its own phase — the wall-lift's purpose is unusable until the pair is
      fixed, and the fallback (keep the wall) would trade a compile-time panic for a
      silent program-global miscompile.
- [ ] PB-4 residual: `map` end-to-end (recursive member reconstructing `Cons` through a
      `Functor` bound) is the one S8b shape *not* yet probe-verified — `combine` through
      the bound and the recursive non-inline append body are both verified, and `map`
      composes exactly those two verified halves, but the composition is Phase 3's to
      land. If a grounding gap appears, it is expected to be a located error
      (diagnostics-as-behaviour); stop and record rather than widen.
- [ ] Pre-existing fence recorded this round (no S8b action): a bare nullary variant ctor
      of a generic header in a mono body grounds at the single first candidate and is
      caught by the signature check as a located mismatch — a second-instantiation
      nullary construction cannot be spelled. Adjacent to S6 R4's future
      consuming-context-grounding slice; recorded in the roadmap entry.
- [ ] Pre-existing hazard recorded this round (no S8b action): the struct-word twin of
      the bare-key last-write-wins class, and cross-module same-named variant names in
      the flat `enums.words` map. No known miscompile repro; for a future slice.

## Risks & Mitigations

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| θ-seeding fix perturbs the shared member-dispatch path (S8's lifted-mono `Range` route, S6's concrete-target route) | Med | Byte-unchanged pins: S6's `empty[i64]`/`mconcat` goldens, S8's 24 tests; the Opt repro SSA pin proves the new path; keep the positional contract untouched for non-nullary callers |
| Pattern-var-id alignment between the impl-target pattern and the member word's own variables is subtler than the probe's variable-#0 statement | Med | The Opt repro golden pins the *correct* mono symbol and shape; if alignment needs more than the target equation, stop and record — do not hand-roll a second unifier |
| Single-candidate recording changes dispatch for generated words in edge programs (symbol now recorded where the bare key used to be read) | Low | Lowering's `builtin_overloads` read already precedes and correctly dispatches mangled keys (D7/R5 precedent, `calls.rs:480`); the two lowering-side enum-dispatch tests (`calls.rs:2502`, `:2547`) plus the full suite guard it |
| `map` end-to-end surfaces an unmeasured grounding gap (PB-4 residual) | Med | Expected shape is a located error, not a panic; Phase 3's scope says stop-and-record; the S6 Option-map and S7-bind precedents bound the mechanism |
| Parallel edits to `src/check/poly.rs` (Phases 1 ∥ 2) conflict at merge | Med | Disjoint functions (dispatch ~2274-2640 vs construction-bind 6074-6193); landing rule: whoever lands second rebases and re-runs the full gate before proceeding |
| Probe logs only exist under `/tmp` (volatile) | High | Phase 1 persists the segfault log verbatim into `docs/roadmap/P7b/slice8b-probes.md` as part of its landing; Phase 4 persists the spellings log and ledger |
| Wrong-θ intermediate state (arm landed, Phase 1 not) is suite-green but crash-prone for `empty[…]` programs | Med | Sequencing rule: Phase 3 (which writes `empty[List[i64]]` goldens) starts only after both Phase 1 and Phase 2 land; no in-repo test exercises the intermediate state |

## Delivery Plan

### Phase 1: The pre-existing two-defect fix (θ seeding + per-instantiation variant words)

- **Goal**: A program with a generic-target `Monoid` impl (`Opt`-shaped, no
  self-reference field), a prior payload construction, and an `empty[<inst>]` call builds,
  runs exit 0, and mints the correctly-typed monomorph — the base SIGSEGV becomes a
  passing golden.
- **Requirements Covered**: R4, R5, R6.
- **Scope**:
  - `src/check/poly.rs:2363-2391` (`resolve_mono_member_call`'s nullary rescue branch) +
    `:2625-2630` (the generic-branch `check_poly_call` handoff) + `:7366-7391`
    (`check_poly_call`'s seeding block): seed the nullary generic-target member's θ from
    the impl-target equation — computed with `match_impl_target`
    (`src/check/poly.rs:9254`) by matching the impl target pattern against the call-site
    type — instead of the raw positional `type_args`; the positional contract
    (P7.S3t) stays untouched for every other caller. Pin the concrete-target nullary path
    (`empty[i64]`) byte-unchanged.
  - `src/check/terms.rs:962-980` (single-candidate arm): record the resolved symbol in
    `poly.builtin_overloads` for generated enum words, mirroring the multi-candidate arm
    at `:1025-1028` and using `splice_enum_site`'s identification logic
    (`:2060`) in its non-splice twin. No `src/ir/` change — lowering's existing read at
    `src/ir/func_builder/calls.rs:480` dispatches the recorded mangled keys.
  - Unit tests beside both changes: `src/check/poly.rs`'s test module (pattern: the
    `poly_bind_construction_arg_owned_cell_*` pair at `:14700`/`:14724`) and
    `src/check/terms.rs`'s test module (pattern: `mint_fallback_candidates_*` at
    `:3072`+).
  - Files to create: `tests/phase7b_slice8b.rs` (harness modeled on
    `tests/phase7b_slice8.rs:39`) holding the Opt repro golden (R6 — build + run exit 0,
    prints `ok`) and an `emit_ssa_with_manifest` pin (pattern:
    `tests/phase7b_slice8.rs:742`) asserting the empty-only mono is declared/returns the
    one-level instantiation.
  - Also: persist `/tmp/sooth_p8b_segfault.md` verbatim into
    `docs/roadmap/P7b/slice8b-probes.md` (the phase's evidence base; `/tmp` is volatile).
  - Explicitly out of scope for this phase: `poly_bind_construction_arg` (Phase 2's
    site), any `lib/` file, any `src/ir/` file, the struct-word bare-key twin and
    cross-module variant-name collision (recorded, out of scope slice-wide), the bare
    nullary-variant grounding fence (recorded).
- **Entry Conditions**: Worktree clean at `c406149`; the one-time re-baseline recorded
  (fmt, clippy, full `cargo test` green on the untouched tree) — the brief assigns this
  to the probe round, already done; nothing else precedes this phase.
- **Exit Criteria / Verifiable Artifacts**: Opt repro golden passes (was SIGSEGV at
  base); SSA pin shows the one-level mono; a two-mint program (payload variant
  constructed in a mono body before the second mint) runs with correct shapes for both;
  S6's `monoid_for_i64_combine_and_empty_dispatch` and `mconcat_over_list_dispatches`
  byte-identical; full gate green; unit tests beside both changed functions; the segfault
  log persisted under `docs/roadmap/P7b/`.
- **Parallelism**: PARALLEL with Phase 2 — disjoint functions (member dispatch
  ~`poly.rs:2274-2640` vs construction-bind `poly.rs:6074-6193`), no shared symbol
  edited; same-file landing rule applies (whoever lands second rebases + full gate).
- **Relative Effort**: M — the root causes are probe-verified, but the fix spans two
  stages' resolution routes (member dispatch seeding; mono/splice/poly enum-site
  recording) and its regression surface is program-wide, so the sweep and pin work
  dominates.
- **Difficulty**: `hard` — shared control flow in the checker's member-dispatch and
  term-resolution cores, silent-miscompile failure class (the green-but-mistyped path is
  exactly what must not survive), cross-stage contract.
- **Open Questions / Blockers**: Pattern-var-id alignment between the impl-target
  pattern and the member word's variables (see Risks) — the Opt repro golden is the
  arbiter; stop-and-record if more than the target equation is needed. None else.

### Phase 2: The construction-wall `Generic` field arm

- **Goal**: Both P8 wall shapes — an impl member body constructing `Cons` and a plain
  generic word constructing `Cons` — build and run, a ctor-mismatch operand is a
  byte-exact located error, and S6's wall witness passes as a positive golden.
- **Requirements Covered**: R1, R2, R3, R7.
- **Scope**:
  - `src/check/poly.rs:6074` (`poly_bind_construction_arg`): insert the
    `PolyType::Generic` field arm before the catch-all at `:6193` — identity check on
    `(is_enum, idx, module)` against a `Generic` operand, `len_args` fence (R3),
    positional recursive bind over field args vs operand args. `/tmp/p8b-arm.patch`'s
    `poly.rs` hunk is the measured candidate (probe-verified green in-suite); land it,
    re-anchored to the clean tree.
  - Unit tests beside the arm in `src/check/poly.rs`'s test module (patterns:
    `:14700` happy path, `:14724` mismatch): same-identity bind binds the element var;
    differently-headed operand → error containing the rendered mismatch; `Concrete`
    operand → located error (R2, PB-3); non-empty `len_args` → located error (R3).
  - `tests/phase7b_slice6.rs:410`: flip the witness per the probe patch's second hunk —
    `monoid_for_list_append_construction_grounds_after_s8b`, asserting build success and
    a clean run (empty stdout), with the pre-S8b panic text preserved in the doc comment.
  - Files to create: `tests/phase7b_slice8b.rs` may be created here if Phase 1 has not
    (the P8-2d2 golden — a plain generic word constructing `Cons` — and the
    ctor-mismatch `build_error` pin land in it; pattern:
    `tests/phase7b_slice6.rs:57` `build_error`).
  - Explicitly out of scope for this phase: `resolve_mono_member_call`/`check_poly_call`
    (Phase 1's surface), any `empty[…]`-involving golden (deliberately Phase 3 — the
    witness fixture contains no `empty` call precisely so this phase is green without
    Phase 1), `lib/`, `src/ir/`, the catch-all's message text for non-`Generic` shapes.
- **Entry Conditions**: Worktree clean at `c406149` (if landing after Phase 1: rebased
  onto it, full gate green). The probe patch (`/tmp/p8b-arm.patch`) available; if `/tmp`
  was lost, the arm's contract is fully specified by R1-R3 and the `App` arm at
  `src/check/poly.rs:6124-6170` is the pattern.
- **Exit Criteria / Verifiable Artifacts**: P8-2b shape (the flipped witness) builds and
  runs; P8-2d2 golden builds and runs; ctor-mismatch and `Concrete`-operand errors
  byte-exact; `len_args` fence unit-tested; `mklist`, `Range`'s `next`, and S8's 24 tests
  byte-unchanged; full gate green.
- **Parallelism**: PARALLEL with Phase 1 (same reason and landing rule); SEQUENTIAL
  before Phase 3.
- **Relative Effort**: S — the measured patch exists and was suite-verified; the work is
  anchoring, pins, and the witness flip.
- **Difficulty**: `standard` — additive checker arm with a verified reference
  implementation; no shared control flow rewritten.
- **Open Questions / Blockers**: None identified.

### Phase 3: The traitful `List` surface — `map` and `append` goldens

- **Goal**: `map` through a `Functor` bound over the real `List[i64]` maps and drops
  clean; `append` through a `Monoid` bound appends (`1 2 3 5 3`); `empty` routes both
  ways; linearity teeth and the distinct-`'U` fence are pinned.
- **Requirements Covered**: R8, R9, R10, R11, R12, R13.
- **Scope**:
  - `tests/phase7b_slice8b.rs` (exists from Phases 1-2; all changes are fixture
    additions): the `Functor for List` map golden — trait decl + impl + consumer per the
    S6 patterns (`tests/phase7b_slice6.rs:105` Functor decl + Option map body, `:181`
    recursive non-inline List member, `:453` shared-bound dogfood; consumer spelling
    `'U := 'T` per the spellings probe); the `Monoid for List` combine-through-bound
    golden (spellings fixture `p8b-sp-4` shape, expects `1 2 3 5 3`); the
    `empty[List[i64]]` mono-main golden; the List-shaped crash repro as a regression
    golden (prior `Cons` construction + `empty[List[i64]]` — the P8b bisect fixture,
    expressible only once Phases 1+2 have landed); linearity pins (undropped `map`/
    `append` result → located build error; `dup` of a `List['T]` operand → byte-exact
    `poly_copy_generic_error`); the distinct-`'U` fence byte-exact pin plus one verified
    substitute grounding (`'U := 'T` already exercised by the map consumer).
  - No `src/` changes are expected in this phase. If the map body surfaces a grounding
    gap: stop, capture the located error verbatim, record it in the probes log and the
    roadmap entry — do not widen into a new checker mechanism (PB-4 residual).
  - Explicitly out of scope for this phase: any `lib/` module or `lib/core/sooth.pkg`
    change (R13 — the surface is fixtures only), `src/ir/`, new trait-declaration
    syntax, adaptor combinators, distinct-`'U` support.
- **Entry Conditions**: Phase 1 landed (Opt repro golden passing — `empty[…]` calls are
  correctness-critical here) AND Phase 2 landed (the witness flipped green — member
  bodies reconstructing `Cons` compile).
- **Exit Criteria / Verifiable Artifacts**: map golden prints the mapped spine and drops
  clean; combine golden prints `1 2 3 5 3` (twice-run stable); `empty[List[i64]]` golden
  green; the List-shaped crash repro golden green; undropped-result and `dup` errors
  byte-exact; `mconcat_over_list_dispatches` byte-identical; `git diff` shows no `src/`
  change from this phase and no `lib/` change from the slice; full gate green.
- **Parallelism**: SEQUENTIAL after Phases 1 and 2 (its goldens need the θ fix and the
  arm); no parallel peer.
- **Relative Effort**: S — fixture composition over probe-verified shapes; the risk is
  the one unverified composition (map end-to-end), bounded by the stop-and-record rule.
- **Difficulty**: `standard` — golden authorship against existing mechanisms; no
  compiler-internal work planned.
- **Open Questions / Blockers**: PB-4 residual (map end-to-end unmeasured) — see Risks;
  resolution rule stated in Scope (stop-and-record).

### Phase 4: Records, growth re-check, final gate

- **Goal**: The slice's written record exists and the merged tree passes the canonical
  gate with every convention satisfied — a reader of the roadmap entry can reconstruct
  what landed and why from the repo alone.
- **Requirements Covered**: R14.
- **Scope**:
  - Persist `/tmp/sooth_p8b_spellings.md` and the ledger's verdict sections verbatim
    into `docs/roadmap/P7b/slice8b-probes.md` (Phase 1 persisted the segfault log;
    this completes the round's record).
  - `docs/roadmap/P7b-higher-kinded-types.md`: the S8b entry — carve-out closed; the
    wall arm; the two-defect fix with its root-cause citations; the ruled host trait and
    ship surface; the distinct-`'U` fence with its verified substitutes (R12's record);
    the two newly-recorded pre-existing fences (nullary-variant mono grounding;
    struct-word/cross-module bare-key class); the witness flip.
  - Growth-structure re-check per CLAUDE.md over every file the slice touched
    (`src/check/poly.rs`, `src/check/terms.rs`, `tests/phase7b_slice6.rs`,
    `tests/phase7b_slice8b.rs`), consolidating the per-phase exit checks (Phases 1-3
    each re-run the signals on their own diffs at their exit; this phase records the
    consolidated verdict — expected: more same-kind functions beside existing neighbors
    in two large single-stage modules; no split warranted from this slice's diffs).
  - Final verification: `cargo fmt --check`; `cargo clippy -- -D warnings`; full
    `cargo test`; `git diff c406149..HEAD -- src/ir/ lib/` empty (R13/R14).
  - Explicitly out of scope for this phase: any `src/` change; re-litigating settled
    rulings.
- **Entry Conditions**: Phases 1-3 landed and individually gate-green.
- **Exit Criteria / Verifiable Artifacts**: The roadmap entry and probes log exist and
  cite the landed evidence; the growth re-check verdict is written down; the full gate is
  green on the final tree; the `src/ir/`+`lib/` diff-empty check passes.
- **Parallelism**: SEQUENTIAL after Phase 3 (records the final state).
- **Relative Effort**: S — documentation and verification only.
- **Difficulty**: `standard`.
- **Open Questions / Blockers**: None identified.

### Parallelism Summary

- Phase 1 ∥ Phase 2: genuinely independent (disjoint functions; the Opt repro needs no
  arm; the flipped witness needs no θ fix). Same-file landing rule: secondlander rebases
  and re-runs the full gate.
- Phase 3: after both (its goldens call `empty[List[i64]]` and reconstruct `Cons`).
- Phase 4: after Phase 3 (records the final tree).

### Effort Summary

- Phase 1: M · Phase 2: S · Phase 3: S · Phase 4: S — total ≈ 2-3 weeks, dominated by
  Phase 1's two-stage fix and regression sweep.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "pre-existing two-defect fix: theta seeding and per-instantiation variant words", "effort": "M", "difficulty": "hard" },
    { "phase": 2, "focus": "construction-wall Generic field arm and witness flip", "effort": "S", "difficulty": "standard" },
    { "phase": 3, "focus": "traitful List surface goldens: map, append, empty", "effort": "S", "difficulty": "standard" },
    { "phase": 4, "focus": "records, growth re-check, final gate", "effort": "S", "difficulty": "standard" }
  ]
}
```
