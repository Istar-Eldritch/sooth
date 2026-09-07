# Spec: P7b.S8b — the construction-wall fix, the pre-existing two-defect pair it exposes, and traitful `List` members (`map`, `append`)

**Status:** Draft
**Created:** 2026-09-07
**Discovery:** [slice8b-brief](./slice8b-brief.md) (the S8 carve-out record, wall delta,
probe questions PB-1..PB-6). Evidence base: the P8b probe round — segfault root-cause log,
spellings log, and the consolidated ledger, all persisted verbatim into
[the slice's probe log](./slice8b-probes.md) (landed in `b667916`, ahead of this spec).
Roadmap entry: [P7b-higher-kinded-types.md](../P7b-higher-kinded-types.md), "P7b.S8b".
**Base:** `p7b-s8b` worktree clean at `b667916` or later on this branch (`445a74e` —
S6/S6b/S6c, S7, S8, S8c, S9, S10 merged — plus the brief and probes-log commits). All
`path:line` anchors below were verified on the clean tree at `c406149`; the probe round's
own line numbers were taken on `c406149 + /tmp/p8b-arm.patch` (+60 lines after
`poly.rs:6190`), so they differ — re-locate by symbol name, not by the probe's numbers.
The `git diff c406149..HEAD -- src/ir/ lib/` exit checks (R13/Phase 4) remain valid at
`b667916`, since the intervening commits are docs-only.

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
- **R3.** The system must reject, as a located error (recorded fence, PB-2), a
  `Generic` field carrying a non-empty `len_args` — the stored `^List['T]` field is
  `len_args: []` today, but the shape is spellable now (a self-referential field with a
  `Len` variable, e.g. `Ring['T 'N: Len] head 'T rest Ring['T 'N]`); the rejection is a
  dedicated message (not `poly_rendered_type_mismatch_error`, which would render
  identical text on both sides for this exact case), naming the field's header and
  saying its length variable cannot be bound by this construction.
- **R4.** A nullary trait member called with an explicit type argument over a **generic,
  single-type-variable** impl target must instantiate the member through the impl-target
  equation — `empty[List[i64]]` over `impl: Monoid for List` seeds the member's element
  variable from matching the impl target `List['E]` against the call-site type (`'E :=
  i64`), minting a monomorph declared and returning `List[i64]` (a 24-byte Nil,
  SSA-verifiable) — never `List[List[i64]]`. The concrete-target nullary path (S6's
  `empty[i64]` golden) stays byte-unchanged. Qualification (review round, P1-4/P2-6): a
  **multi-variable** impl target (e.g. `impl: Monoid for Pair['A 'B]`) is a pre-existing
  fence, not covered by this requirement — `check_poly_call`'s arity gate
  (`poly.rs:7421-7423`) compares the call site's `type_args.len()` against the member
  sig's *full* variable count (target vars + member locals, `build_member_var_union`,
  `src/parser.rs:768`), so `empty[Pair[i64 str]]` hits `instantiation_arity_error` before
  the seed channel ever runs; the seed cannot be reached with an arity-matching call at
  more than one target variable. Separately, the seed channel itself carries types only —
  `Subst.len` is not threaded through it (`poly.rs:7439-7443`) — so a length-carrying
  generic target (a `'N`-style variable) cannot ground a nullary member through this
  route either; this is the same fence family as R3's unspellability note (which covers
  the length-argument *spelling* side, not this seeding side). Asymmetry note: on the
  nullary seed-channel path a written type argument means "the dispatch type" (matched
  against the impl target pattern); on the operand-dispatched generic-call path (S3t) the
  same written type argument still means "variable #0" (bound positionally). Both
  fences are recorded here, not fixed — a future slice that lifts either is a distinct
  unit of work from Phase 1's. Related defensive note: the seed's `Some(empty)` fallback
  arm (`poly.rs:7441-7448`, positional restore when the impl-target equation binds
  nothing) has no spelled fixture today — an all-concrete target pattern is the only shape
  that reaches it, and no test spells one; mutating its discriminator to `is_none()`
  survives the suite. Accepted as defensive coverage, recorded rather than fixture-built.
- **R5.** A checker-resolved enum construction or destructure site must lower with its own
  resolved instantiation's field shapes even when another instantiation of the same header
  is minted later in the program — per-instantiation resolution applies wherever the
  checker resolved an instantiation that differs from the bare-name default. The bare-name
  last-write-wins variant-word map (`src/ir/layout.rs:597-640`, hazard pre-documented at
  `src/ast.rs:1421-1424`) still correctly decides every site with no such resolution,
  including every non-generic enum's construction/destructure sites — for those the
  recorded symbol IS the bare surface name (`enum_generated_sigs` yields `variant.name`,
  `src/check/declarations.rs:1885`; `layout.rs:604-609` keys both spellings to the same
  word). Eliminator call sites are out of scope for this requirement (see Scope &
  Boundaries's residual).
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
  appends two `List[i64]` spines (prints `1 2 3 5 3`); `empty[List[i64]]` grounds
  explicitly in a mono main (verified). The bound-directed route is verified only at the
  `i64` impl — the existing `mconcat_over_list_dispatches` golden
  (`tests/phase7b_slice6.rs:359`) dispatches `Monoid for i64`'s `empty`, not `List`'s, and
  stays byte-identical; bound-directed `empty` resolving at the `List` impl itself is
  **unverified** and is a Phase 3 measure-and-pin item, constrained by the spelling
  fence Phase 3 records (a single `List` instantiation per program — no nested
  `List[List[i64]]`-style helpers).
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
  stays behaviourally diff-empty for the whole slice (see Scope & Boundaries for how this
  survives the lowering-side defect) — a comment-only correction to the doc comment at
  `src/ir/func_builder/mod.rs:195-199`, which Phase 1 falsifies (it claims
  `builtin_overloads` is "Empty on every corpus/test path"), is permitted; no other
  `src/ir/` edit is.
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
- [ ] A `Generic` field with non-empty `len_args` is a located error, both a unit test
      and a golden — the shape is spellable (`Ring['T 'N: Len] head 'T rest Ring['T
      'N]`), so CLAUDE.md's golden-per-exit-criterion convention is satisfied by the
      golden itself, not waived — R3.
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
- [ ] `git diff c406149..HEAD -- src/ir/` is empty modulo the permitted comment-only
      correction at `src/ir/func_builder/mod.rs:195-199`; `lib/` and
      `lib/core/sooth.pkg` are unchanged — R13.
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
- Mono-body eliminator (`Type?`) call sites (e.g. `List?` in `showlist`): these route to
  `check_eliminator_call` ahead of this slice's fix surface (routing documented at
  `src/check.rs:143-147`; dispatched in `src/check/terms.rs` ahead of the
  single-candidate arm Phase 1 touches) and record a resolution only under a splice
  (`src/check.rs:2471-2473`, `if let Some(uid) = prov.splice_uid`) — a mono-body
  eliminator call never reaches the fix. It still resolves through the bare-name map,
  correct whenever all instantiations in the program agree, and a miscompile risk only
  when two different instantiations coexist with an eliminator call in a mono body — same
  pre-existing class as the other recorded fences here, no S8b action.
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
  *superseded* by a designed, unmeasured cheaper fix (checker-side per-site recording
  through the existing `builtin_overloads` channel — see Solution Approach). No probe
  measured this checker-side recording patch; the only measured patch is
  `/tmp/p8b-arm.patch` (the wall arm + witness flip). Mechanical justification for the
  design: mangled per-instantiation symbols come from `enum_generated_sigs`
  (`src/check/declarations.rs:1885`), and `lower_call`'s `builtin_overloads` branch
  already dispatches mangled keys (`src/ir/func_builder/calls.rs:502`,
  `enums.words.get(&sym_name)`) — so `src/ir/` stays diff-empty for all phases. If Phase
  1's implementer finds a checker-invisible construction route that must lower correctly,
  that is a stop-and-record condition, not license to edit `src/ir/`.

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
of the term chooser (`src/check/terms.rs:962-980`). That arm is an `if`/`else if` chain:
it already records under a splice (`splice_enum_words`) and for an always-spliced
combinator name (`builtin_overloads`, `:973`) — but for a non-splice generated enum word
it records nothing today, the gap this phase closes. (The multi-candidate arm at
`:1025-1028` already records `builtin_overloads[span] = chosen.symbol` for its own cases,
and for a minted variant that symbol is the per-instantiation registry spelling —
`Cons[i64]`-style — exactly the mangled key `enums.words` carries,
`src/check/declarations.rs:1876` + `src/ir/layout.rs:601-610`.) A later mint then
overwrites the bare surface key and the unrecorded site lowers from the clobbered entry.
The fix adds the resolved-symbol record to the single-candidate arm as a further `else`
branch (for generated enum words, the `splice_enum_site` machinery's non-splice twin at
`src/check/terms.rs:2060` already knows how to identify them); lowering needs **no
change** — its
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
| `src/check/poly.rs:7329` | `check_poly_call()` | θ seeding owner; gains a way to receive the impl-target-derived seed on a **separate channel** from call-site `type_args` — the existing arity gate (`instantiation_arity_error`, `:7385-7386`, gated on `type_args.len() != sig.ty_var_names.len()`) applies to explicit `type_args` only and stays untouched |
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
| `src/check/terms.rs:962-980` | single-candidate arm | Records nothing today only for the non-splice generated-enum-word case — the gap behind defect (b); gains the per-site record as a further `else` branch |
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
| `tests/phase7b_slice6.rs:410` | `monoid_for_list_append_construction_wall_is_recorded()` | Flips to `monoid_for_list_append_construction_builds_and_runs_clean` (probe patch hunk; renamed from the patch's `_grounds_after_s8b` — a slice label goes stale once S8b is history, per the `thing_condition_expected` naming convention) |
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
      length-carrying `Generic` field is spellable (a self-referential field with a `Len`
      variable, e.g. `Ring`) and is rejected by a dedicated located diagnostic naming the
      unbindable length (R3, corrected during implementation; the probe round's original
      capture rendered tautologically).
- [x] ~~PB-3 — which operand shapes reach the arm?~~ Resolved: only `Generic` observed;
      everything else takes the located mismatch (R2), mirroring the `App` arm.
- [x] ~~PB-5 — `append`'s host trait, and how much of S6's wall closes?~~ Resolved by
      probe + user ruling (260907 decision interview): `Monoid for List` (`combine` =
      append, `empty` = `Nil`); S8b closes S6's *whole* recorded wall — both dropped
      goldens (`Monoid for List`, `Functor for List`) land in S8b. User-authorized.
- [x] ~~PB-6 — linearity teeth?~~ Resolved: undropped results are compile errors; `dup`
      stays fenced (R10/R11).
- [x] ~~Ship surface: S6 convention (golden fixtures) or S8 convention (`core::iterator`
      lib module)?~~ Ruled here: S6 convention (R13) — the surface is trait fixtures, not
      a protocol module.
- [x] ~~Is the two-defect fix in scope for S8b?~~ Resolved by the probe round's verdict:
      required, as its own phase — the wall-lift's purpose is unusable until the pair is
      fixed, and the fallback (keep the wall) would trade a compile-time panic for a
      silent program-global miscompile.
- [x] ~~PB-4 residual: `map` end-to-end (recursive member reconstructing `Cons` through a
      `Functor` bound)~~ Resolved by probe: VERIFIED, not merely composed-in-principle —
      `p8b-map-list-end-to-end.sth` (build OK, run exit 0, `2 3 4`) and
      `p8b-map-shared-bound-twice.sth` (build OK, run exit 0, `3 4 5`, a shared `Functor`
      bound dispatched twice) both ground end-to-end (Probes Part 1a). The stale framing
      ("not yet probe-verified") is superseded; the narrow true residual is distinct-`'U`
      (R12, already fenced with verified substitutes) — not map end-to-end itself.
- [ ] Two Monoid-for-`List` probe rows are uncited failures Phase 3's goldens must route
      around (Probes Part 1, `p8b-monoid-list-mconcat.sth` /
      `p8b-monoid-list-append-visible.sth`). Round-2 review repro corrected both
      attributions. `p8b-monoid-list-mconcat.sth` fails with
      `` error: no overload of `Nil` in `mkempty` (line 26) accepts these operands
      candidate: no operands
      candidate: no operands `` —
      but the trigger is NOT the `mkempty` helper spelling: a minimal repro of `mkempty`
      plus the fixture's second helper, `: mkemptyof ( -- List[List[i64]] ) Nil ;`
      (line 27) — no traits, no `mconcat` — reproduces the identical error, and deleting
      `mkemptyof` builds clean. The real cause is the fixture's SECOND `List`
      instantiation (`List[List[i64]]` via `mkemptyof`), whose bare-name `Nil` collides
      in the unrecorded mono body — the pre-existing second-instantiation
      nullary-construction fence already recorded below. The `mkempty`-helper spelling
      itself is fine: it is exactly what the byte-unchanged S6 golden
      `mconcat_over_list_dispatches` uses (`tests/phase7b_slice6.rs:382`:
      `` : mkempty ( -- List[i64] ) Nil ; ``). `p8b-monoid-list-append-visible.sth` fails
      with `` `empty` in `main` (line 29, col 3) is a trait member of Monoid, but no
      `impl:` in this program dispatches on these operands `` because its call is
      `empty[i64]` while the program's only `impl:` is `Monoid for List` (line 8) — the
      instantiated type has no dispatching impl in this program, and explicit
      instantiation does not rescue a missing impl (reviewer repro:
      `impl: Monoid for i64` + `empty[List[i64]] drop` fails the same way in reverse).
      This is a missing-impl error, not the recorded consuming-context limit (S6 R4's
      future slice, kept intact below and unrelated to this row). Ruling: Phase 3's
      `mconcat`-over-`List`-Monoid goldens must keep the program to a single `List`
      instantiation — no nested `List[List[i64]]`-style helpers, matching the S6 golden's
      spelling; Phase 3's `empty` goldens must call `empty[...]` at a type with a
      dispatching impl of the host trait present in the program.
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
| A Phase 3 golden surfaces an unmeasured grounding gap (map end-to-end is itself now verified — PB-4 residual narrowed to distinct-`'U`, R12) | Low | Expected shape is a located error, not a panic; Phase 3's scope says stop-and-record for any new gap, not only a map-specific one; the two ruled failing rows (Open Questions) already bound the known List-Monoid spelling fences |
| Parallel edits to `src/check/poly.rs` (Phases 1 ∥ 2) conflict at merge | Med | Disjoint functions (dispatch ~2274-2640 vs construction-bind 6074-6193); landing rule: whoever lands second rebases and re-runs the full gate before proceeding |
| Fixture sources cited by golden descriptions live only under `/tmp/p8b-probes/` (the probe logs themselves are already committed — `b667916` landed the full `slice8b-probes.md`, 741 lines, Parts 1-4) | Low | the five golden-model fixture sources are inlined verbatim in Probes Part 1a (durable, byte-checked against `/tmp/p8b-probes/`); no phase depends on `/tmp` surviving |
| Wrong-θ intermediate state (arm landed, Phase 1 not) is suite-green but crash-prone for `empty[…]` programs | Med | Sequencing rule: Phase 3 (which writes `empty[List[i64]]` goldens) starts only after both Phase 1 and Phase 2 land; no in-repo test exercises the intermediate state |
| The new per-instantiation record (R5) also fires when a mono `inline` (combinator) word's body is checked as an *ordinary mono word* with the real maps (`check.rs:1036-1053`), not only inside a splice — so a construction span can land in both `builtin_overloads` (from that ordinary-word walk) and `splice_enum_words` (from the spliced walk), and `calls.rs:480`'s `builtin_overloads` read wins over the splice-keyed one | Low | Deliberate scope stop, not fixed in Phase 1: threading an `is_combinator` flag through `Ctx` to suppress the ordinary-word-walk record is out of this phase's lane. Benign at one θ per mono body (the flat `builtin_overloads` map is not `span.module`-keyed, so a cross-module same-named variant would resolve through the same pre-existing bare-key hazard this map already carries — R5's own doc comment). Pinned by a golden: a mono `inline` word constructing a variant, called from `main`, builds and runs — see `mono_inline_combinator_variant_construction_builds_and_runs` (`tests/phase7b_slice8b.rs`) |

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
    type — instead of the raw positional `type_args`. The seed rides a channel separate
    from call-site `type_args`: a member word is polymorphic over both the impl target's
    variables and its own locals, so an impl-target-derived seed is partial and cannot
    ride the `type_args` channel used for explicit instantiation. The existing arity
    gate (`instantiation_arity_error`, `poly.rs:7385-7386`, gated on
    `type_args.len() != sig.ty_var_names.len()`) applies to explicit `type_args` only and
    is untouched by this change; a genuinely wrong-arity `empty[A B]` still hits that
    gate exactly as today. The positional contract (P7.S3t) stays untouched for every
    other caller. Pin the concrete-target nullary path (`empty[i64]`) byte-unchanged.
  - `src/check/terms.rs:962-980` (single-candidate arm): record the resolved symbol in
    `poly.builtin_overloads` for generated enum words, mirroring the multi-candidate arm
    at `:1025-1028` and using `splice_enum_site`'s identification logic
    (`:2060`) in its non-splice twin. No `src/ir/` change — lowering's existing read at
    `src/ir/func_builder/calls.rs:480` dispatches the recorded mangled keys. The
    single-candidate arm is today an `if`/`else if` chain (the splice case, then the
    always-spliced-combinator case) — the new record must land as a further `else`
    branch, never restructure the existing arms. Two invariants pin this and must keep
    holding: (1) `trait_calls` is documented "disjoint from `builtin_overloads` by
    construction" (`src/ir/func_builder/mod.rs:200-207`); (2)
    `two_splices_at_two_thetas_record_two_different_enum_ids`
    (`src/check/poly.rs:21599`) asserts `!module.builtin_overloads.contains_key(&span)`
    for every `splice_enum_words` span ("a spliced enum site is redirected, not
    double-recorded") — this test must keep passing, and the new record must not bypass
    the per-`(uid, span)` resolution at `calls.rs:908-928` (the two-θ miscompile the
    redirect exists to prevent).
  - Stop-and-record condition (risk-driven): after this fix, a checker-recorded symbol
    that misses `enums.words` takes the `builtin_overloads` branch at
    `calls.rs:480`/`:502` and falls into `lower_resolved_word_call`'s
    `self.env.get(sym_name).expect("checked resolved call exists")` (`calls.rs:246-250`)
    — an ICE panic, with no fall-back to the bare-key path (unlike today's unrecorded
    sites, pinned by `enum_words_miss_falls_through_to_bare_key_lookup`,
    `calls.rs:2547`). Reachable in principle because recorded symbols can come from
    `mint_fallback_candidates` (`terms.rs:2010`), which deliberately serves unflushed
    check-time mints (`mint_fallback_candidates_finds_an_unflushed_constructor`,
    `terms.rs:3072`), while layout's `ewords` is built from the final enum slice. Verify
    by running the two existing lowering-side enum-dispatch tests
    (`calls.rs:2502`/`:2547`) plus a sweep for a symbol that is checker-recorded but
    layout-absent; if one is found, stop and record — the fix is checker-side-only by
    construction, do not patch lowering to add a fall-back.
  - Unit tests beside both changes: `src/check/poly.rs`'s test module (pattern: the
    `poly_bind_construction_arg_owned_cell_*` pair at `:14700`/`:14724`) and
    `src/check/terms.rs`'s test module (pattern: `mint_fallback_candidates_*` at
    `:3072`+).
  - Files to create: `tests/phase7b_slice8b.rs` (created here; harness modeled on
    `tests/phase7b_slice8.rs:39`) holding: the Opt repro golden
    (R6 — build + run exit 0, prints `ok`); an `emit_ssa_with_manifest` pin (pattern:
    `tests/phase7b_slice8.rs:742`) asserting the empty-only mono is declared/returns the
    one-level instantiation; and the two-mint golden (R5's exit criterion — a program
    minting two instantiations of one generic enum header, constructing a payload variant
    by bare name in a mono body before the second mint, running with correct field shapes
    for both).
  - Also: the segfault log (Part 2) is already persisted verbatim in
    `docs/roadmap/P7b/slice8b-probes.md` (landed in `b667916`, ahead of this phase) —
    verify it is intact; no persistence action needed here.
  - Explicitly out of scope for this phase: `poly_bind_construction_arg` (Phase 2's
    site), any `lib/` file, any `src/ir/` file, the struct-word bare-key twin and
    cross-module variant-name collision (recorded, out of scope slice-wide), the bare
    nullary-variant grounding fence (recorded).
- **Entry Conditions**: Worktree clean at `b667916` or later on `p7b-s8b` (anchors above
  were verified at `c406149`); the one-time re-baseline recorded (fmt, clippy, full
  `cargo test` green on the untouched tree) — the brief assigns this to the probe round,
  already done; nothing else precedes this phase.
- **Exit Criteria / Verifiable Artifacts**: Opt repro golden passes (was SIGSEGV at
  base); SSA pin shows the one-level mono; a two-mint program (payload variant
  constructed in a mono body before the second mint) runs with correct shapes for both;
  S6's `monoid_for_i64_combine_and_empty_dispatch` and `mconcat_over_list_dispatches`
  byte-identical; full gate green; unit tests beside both changed functions; the segfault
  log's persistence in `docs/roadmap/P7b/slice8b-probes.md` verified intact (already
  landed in `b667916`, ahead of this phase).
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
    `monoid_for_list_append_construction_builds_and_runs_clean` (the spec's name —
    overrides the probe patch's `/tmp/p8b-arm.patch` second hunk, which still uses the
    stale `_grounds_after_s8b` name), asserting build success and a clean run (empty
    stdout), with the pre-S8b panic text preserved in the doc comment.
  - `tests/phase7b_slice8b.rs`, created by Phase 1: holds the P8-2d2 golden — the
    helper-function spelling `p8b-2d2-plain-generic-cons-helper.sth` (Probes Part 1a; a
    plain generic word constructing `Cons` via a helper) — and the ctor-mismatch
    `build_error` pin; pattern: `tests/phase7b_slice6.rs:57` `build_error`.
  - Explicitly out of scope for this phase: `resolve_mono_member_call`/`check_poly_call`
    (Phase 1's surface), any `empty[…]`-involving golden (deliberately Phase 3 — the
    witness fixture contains no `empty` call precisely so this phase is green without
    Phase 1), `lib/`, `src/ir/`, the catch-all's message text for non-`Generic` shapes.
- **Entry Conditions**: Worktree clean at `b667916` or later on `p7b-s8b` (anchors above
  were verified at `c406149`; if landing after Phase 1: rebased onto it, full gate
  green). The probe patch (`/tmp/p8b-arm.patch`) available; if `/tmp` was lost, the arm's
  contract is fully specified by R1-R3 and the `App` arm at `src/check/poly.rs:6124-6170`
  is the pattern.
- **Exit Criteria / Verifiable Artifacts**: P8-2b shape (the flipped witness) builds and
  runs; the P8-2d2 golden — the helper-function spelling
  `p8b-2d2-plain-generic-cons-helper.sth` (Probes Part 1a) — builds and runs (note: the
  bare-`Nil` main spelling, `p8b-2d2-plain-generic-cons.sth`, fails with
  `` error: unknown word `Nil` in `main` (line 4) `` because the program never grounds a
  `List[i64]` instantiation (only `cons2['T]`'s generic signature exists), so no `Nil`
  variant word is minted for the bare name to resolve to — unrelated to the arm, and
  distinct from the recorded nullary-variant fence (which is about a mis-grounded
  candidate, not a missing one) — so the golden must use the helper spelling);
  the ctor-mismatch diagnostic byte-exact (integration pin, `badcons`); the
  `Concrete`-operand diagnostic asserted as a located type-mismatch at unit level
  (`.contains("type mismatch")`, matching the unit-level pattern at `:14724`), with its
  full rendering frozen byte-exact separately by the integration pin (no `Concrete`
  operand is reached live today, so only the ctor-mismatch pin is a real-program
  golden); the `len_args` fence unit-tested AND golden (R3, `mkring`); `mklist`,
  `Range`'s `next`, and S8's 24 tests byte-unchanged; full gate green.
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
    additions): the `Functor for List` map golden, per the inlined sources in Probes Part
    1a (`p8b-map-list-end-to-end.sth`, prints `2 3 4`) and the shared-bound variant
    (`p8b-map-shared-bound-twice.sth`, prints `3 4 5`) — trait decl + impl + consumer per
    the S6 patterns (`tests/phase7b_slice6.rs:105` Functor decl + Option map body,
    `:181` recursive non-inline List member, `:453` shared-bound dogfood; consumer
    spelling `'U := 'T` per the spellings probe); the `Monoid for List`
    combine-through-bound golden, per the inlined source in Probes Part 1a
    (`p8b-sp-4-combine-through-bound.sth`, expects `1 2 3 5 3`); the `empty[List[i64]]`
    mono-main golden (explicit route only — see the spelling constraint below); the
    List-shaped crash repro as a regression golden, per the inlined source in Probes
    Part 1a (`p8b-bisect-v1-drop-then-empty.sth` — prior `Cons` construction +
    `empty[List[i64]]`, expressible only once Phases 1+2 have landed); linearity pins
    (undropped `map`/`append` result → located build error; `dup` of a `List['T]`
    operand → byte-exact `poly_copy_generic_error`); the distinct-`'U` fence byte-exact
    pin plus one verified substitute grounding (`'U := 'T` already exercised by the map
    consumer).
  - Spelling constraint (Probes Part 1 rows for `p8b-monoid-list-mconcat.sth` /
    `p8b-monoid-list-append-visible.sth`): keep `mconcat`-over-`List`-Monoid goldens to a
    single `List` instantiation per program — no nested `List[List[i64]]`-style helpers
    (the pre-existing second-instantiation nullary-construction fence, recorded below;
    `p8b-monoid-list-mconcat.sth`'s failure comes from its second helper,
    `: mkemptyof ( -- List[List[i64]] ) Nil ;`, not from the `mkempty` spelling the S6
    golden already uses byte-unchanged). Every `empty` golden must call `empty[...]` at a
    type with a dispatching `impl:` of the host trait present in that program — an
    explicit type argument does not rescue a missing impl
    (`p8b-monoid-list-append-visible.sth` calls `empty[i64]` with only `impl: Monoid for
    List` in scope, which is why it fails; this is not the recorded consuming-context
    fence, a different, real limit — see Open Questions). A bound-directed `empty`
    resolving at the `List` impl itself (as opposed to `i64`'s, which
    `mconcat_over_list_dispatches` already exercises — R9) is unverified; if
    attempted and it does not ground, that is expected to be a located error
    (diagnostics-as-behaviour) — stop, capture it verbatim, record it, and do not widen
    into a new checker mechanism.
  - No `src/` changes are expected in this phase. The stop-and-record rule above is not
    scoped to `map` alone — it covers any grounding gap surfaced by any of this phase's
    goldens.
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
- **Relative Effort**: S — fixture composition over probe-verified shapes; map end-to-end
  is itself already verified (Probes Part 1a), so the residual risk is narrower: the
  unverified bound-directed-empty-at-List-impl spelling, bounded by the stop-and-record
  rule.
- **Difficulty**: `standard` — golden authorship against existing mechanisms; no
  compiler-internal work planned.
- **Open Questions / Blockers**: None identified (map end-to-end is verified; see Risks
  for the narrower residual).

### Phase 4: Records, growth re-check, final gate

- **Goal**: The slice's written record exists and the merged tree passes the canonical
  gate with every convention satisfied — a reader of the roadmap entry can reconstruct
  what landed and why from the repo alone.
- **Requirements Covered**: R13, R14.
- **Scope**:
  - The spellings log (Part 3) and the ledger (Part 4) are already persisted verbatim
    in `docs/roadmap/P7b/slice8b-probes.md` (landed in `b667916`, alongside the segfault
    log Phase 1 verified) — verify both are intact; no persistence action needed here.
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
  - The doc comment at `src/ir/func_builder/mod.rs:195-199` ("Empty on every corpus/test
    path (the checker records nothing there), so their lowering is byte-for-byte") is
    falsified by Phase 1 (`builtin_overloads` is now non-empty on the fixed paths) —
    correct the comment's wording (comment-only, no behavioural change; permitted by
    R13).
  - Final verification: `cargo fmt --check`; `cargo clippy -- -D warnings`; full
    `cargo test`; `git diff c406149..HEAD -- src/ir/ lib/` empty *modulo* the permitted
    comment correction above (R13/R14).
  - Explicitly out of scope for this phase: any BEHAVIOURAL `src/` change — the
    comment-only correction assigned above is the sole permitted `src/` edit;
    re-litigating settled rulings.
- **Entry Conditions**: Phases 1-3 landed and individually gate-green.
- **Exit Criteria / Verifiable Artifacts**: The roadmap entry and probes log exist and
  cite the landed evidence; the growth re-check verdict is written down; the full gate is
  green on the final tree; the `src/ir/` + `lib/` diff contains only the permitted
  comment-only correction (`src/ir/func_builder/mod.rs:195-199`); `lib/` is empty.
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
