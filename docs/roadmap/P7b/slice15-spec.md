# P7b.S15 — Applicative.pure: return-type-polymorphic construction (SOO-30)

**Status:** Draft — rulings await user ratification (see Rulings; each names the
alternative rejected and why). **Slice number 15 is provisional** — the next free
P7b number at writing time (`slice14-spec.md` is the highest existing P7b
document); renumber if the pipeline assigns differently.
**Source:** SOO-30 "P7b — Applicative.pure: return-type-polymorphic construction
(library slice)", recorded out-of-scope in P7b.S6. Companion: `Applicative.ap`
remains gated on the constructor-of-quotation case.
**Discovery ground truth:** [slice15-paper-tests](./slice15-paper-tests.md)
(golden designs G1–G13), [probes/soo30_findings.md](../../probes/soo30_findings.md)
(round 1), [probes/soo30r2_findings.md](../../probes/soo30r2_findings.md)
(round 2, the relaxed-gate world), byte-exact baselines
[probes/soo30_baseline.md](../../probes/soo30_baseline.md) and
[probes/soo30r2_baseline.md](../../probes/soo30r2_baseline.md). All measured at
HEAD `846f952`, worktree `soo-30`. Sibling format models:
[slice13-spec](./slice13-spec.md), [slice14-spec](./slice14-spec.md).

## The premise, corrected — what the probes measured

The ticket's premise is **falsified by the probes**: P7b.S6 did *not* settle
grounding for return-type-polymorphic words. Today, at HEAD `846f952`:

1. `pure ( 'A -- 'F['A] )` is **undeclarable**. The S2-15.a gate
   (`member_binds_trait_var`, `src/check/declarations.rs:408`) refuses any member
   whose nonempty inputs lack a dispatchable trait-var head; the error
   (`nested_receiver_member_error`, `src/check/declarations.rs:452`, text at
   `:461-466`) is byte-pinned by `hkt_member_without_dispatchable_input_is_located_error`
   (`tests/phase7b_slice2.rs:144`) and unit-pinned at
   `src/check/declarations.rs:3932` (r1 P1).
2. Under the round-2 gate relaxation (the exact patch saved at
   `/tmp/soo30_r2_gate.patch`), the shape **declares and its ctor-keyed impls
   check everywhere** — fixture-local Box (r2a) and the real lib Option/Result/List
   including the 2-param Result (r2c). The callee side was never the blocker.
3. **Every mono call route for an output-only-`'F` member is walled** (r2b/r2c):
   bare call → located remedy error whose example `pure[i64]` is unachievable
   (r2b-2); `pure[Box]` bare-ctor arg → parse-refused word-general (r1 p2a);
   `pure[Box[i64]]` applied arg → the arity wall, `instantiation_arity_error`
   (`src/check/poly.rs:5760`, fired at `src/check/poly.rs:4164`) — the dissolved
   member word carries 2 vars, the supply has 1 (r2b-1, generalized r2c);
   `pure[i64 i64]` → impl selection is operand-head-driven, so the `i64` operand
   never names the impl (r2b-1c). The only working mono route carries an
   `'F`-headed operand — the double-wrap idiom (r2b-3b, r2c attempt 4: run
   prints `42` / `5\n5\n5\n`).
4. Plain-word output-only bound vars (the twin class) get a named
   "output variable `'F` that no input binds" error
   (`poly_unbound_output_error`, `src/check/poly.rs:5852`) whose remedy is itself
   unachievable for a `* -> *` var (r2d — parse-refused bare ctor head).
5. `ap` is **parse-fenced** (S1-6, `app_arg_quotation_error`,
   `src/parser.rs:2879`, message at `:2882`) — the shipped trait can declare
   `pure` only (r1 P5).
6. Pre-existing defect, recorded-not-fixed: the zero-input escape hatch fails
   impl check with identical renderings — `check_poly_body`'s residual
   comparison (`if residual_pt != sig.outputs`, `src/check/poly.rs:905`) is
   syntactic `PolyType` `PartialEq` (`#[derive(Debug, Clone, PartialEq, Eq)]`,
   `src/ast.rs:2644`); the body mints `Concrete` (`poly_construct_generic`
   all-concrete path, `src/check/poly/construction.rs:587-613`) vs the declared
   `Generic` (`ground_member_poly`'s App-dissolve splice,
   `src/ast.rs:2460-2470`); true shape escapes because its arg is a `Var` (r2e).

So **SOO-30 is a compiler slice with a library payload**: the gate relaxation
(R-30.1), an output-side grounding route for mono call sites (R-30.2), a remedy
correction (R-30.3) — plus the `Applicative` trait module in `lib/core` with
co-located impls and goldens. Correcting the roadmap's out-of-scope line
(`docs/roadmap/P7b-higher-kinded-types.md:915` — "`Applicative.pure` alone is
library work once S6 settles grounding for return-type-polymorphic words") is a
deliverable of this slice, because S6 settled no such thing.

## Rulings (awaiting user ratification)

### R-30.1 — the gate admits an output-App-headed member; App-headed only

`member_binds_trait_var` (`src/check/declarations.rs:408`) gains an output arm:
a member whose nonempty inputs don't mention the trait var is admitted **iff at
least one output mentions it as an application head** — `App { head: 0 }`,
under one `Ref` layer, mirroring the input arm's ref-unwrapping courtesy
(`dispatchable_head`, `src/check/declarations.rs:404`). The refusal arm keeps
today's located S2-15.a text and the conditional nested-input note
(`member_inputs_nest_trait_var`, `src/check/declarations.rs:421`) unchanged.

**Resolves the patch-vs-ruling delta:** the r2 patch reuses `dispatchable_head`
verbatim over the outputs, which would also admit a **bare** `Var(0)` output —
a member like `( 'T -- 'F )`. **Rejected:** a bare-`'F` output member is a
different, unneeded beast — its dissolved member word has no App in its output
to key impl selection on (R-30.2's route has nothing to unify against), no
ticket requirement names it, and no probe measured it. The predicate is
App-headed-only, and the refusal of the bare-output shape is pinned by a new
negative golden (G10.4). The r2 patch (`/tmp/soo30_r2_gate.patch`) is the
starting point; its output arm is tightened to `App{head:0, ..}`-only.

The relaxation's blast radius is **measured**: under the exact patch, the full
suite flips exactly the two gate pins, nothing else (r2f).

### R-30.2 — output-side grounding: the single explicit instantiation unifies against the member's output App

At a **mono** call on a member with no dispatchable input whose output row
contains a trait-var-headed App (the R-30.1-admitted shape), a **single**
explicit type argument unifies against that output App:
`5 pure[Option[i64]]` dissolves `'F:=Option`, `'A:=i64`; impl selection keys on
the **dissolved ctor head**; partially-applied ctor heads ride along exactly as
on the operand path (P3b's 2-arity Result receipt). Mechanically this extends
the existing S6/S8b escape hatch (`src/check/poly/ground.rs:1183-1209`), whose
`find_bound_impl(trait, type_args.first())` already reads the first
instantiation argument **as the impl-target/output type** — for an R-30.1 member
that reading *is* the output-App unification's impl selection. What is missing
today and what this ruling adds: the member's residual vars bind from the
output App's arguments (positionally against the instantiation's ctor
arguments), the S8b impl-target seed carries them, and the arity gate
(`src/check/poly.rs:4164`) takes the route's exception. The bare-ctor spelling
`pure[Option]` **stays parse-refused** (p2a/P3/G7 bytes).

**Rejected alternatives.** (a) Consuming-context inference — the S6 Q1
no-inference rule is load-bearing (`bare_nullary_member_without_instantiation_is_located_error`,
`tests/phase7b_slice6.rs:269-290`): the route fires only on an explicit
instantiation, never a lookahead. (b) Generalizing output-keyed impl selection
to **positional** supplies (the ≥2-arg forms) — it would flip G8's measured
diagnostic (`pure[i64 i64]` succeeding) with no ticket requirement behind it,
for a larger delta across the dispatch path; G8/G13 keep their conservative,
measured bytes. (c) Lifting the bare-ctor parse fence so `pure[Option]` names
the head — kind-unaware fence surgery in the parser for no gain; the applied
spelling parses and is golden (G2–G4).

### R-30.3 — the bare-call remedy keeps its shape; its example becomes achievable

The bare-call remedy error (`mono_nullary_member_no_instantiation_error`,
`src/check/poly/ground.rs:1514`) keeps its located two-line shape — line 1
byte-identical to r2b-2's measurement — but the example token must be
**achievable**: a spelling that, substituted at the same call site, builds and
dispatches under the shipped semantics. Under R-30.2 that is the output-App
spelling (`pure[Box[i64]]` for the G6 fixture). **Rejected:** the positional
two-arg form (`pure[i64 i64]`) — under R-30.2's conservative G8 it still fails
impl selection, so following today's-remedy-style advice would reproduce the
failure the correction exists to remove. Exact example bytes are pinned from
the live binary before the golden asserts (measure-then-pin, the slice13
posture); the paper's G6 assertion strategy (line 1 + `e.g. \`` prefix
byte-exact, example asserted separately) is robust to the rendering.

### R-30.4 — plain-word output-only bound vars stay walled (deferred)

The twin class's three walls (r2d: the arity wall, the named
`poly_unbound_output_error`, the unachievable HKT remedy) stay byte-identical.
R-30.2 names member calls only; plain-word output-only grounding is deferred.
Shared-bound consumer goldens use an `'F`-in-input consumer shape (`repure`,
r2d-a + P3b pattern) — callable at mono sites by operand grounding.

### R-30.5 — `ap` stays out; the shipped trait declares pure only

`ap ( 'F[ [ 'A -- 'B ] ] 'F['A] -- 'F['B] )` cannot appear in any trait
declaration today — the S1-6 parse fence fires first (`src/parser.rs:2882`;
r1 P5, G9). The S7 deferral stands, the coverage-vs-audit ordering question
stays moot behind the fence (the impl-side audit arms,
`src/check/audits.rs:431`/`:484`, remain unreachable as spelled), and no phase
touches the fence.

### R-30.6 — the P2e zero-input defect is recorded, not fixed

The identical-renderings impl mismatch (G12's bytes; root cause r2e) is
pre-existing, independent of the gate relaxation, and unreachable on the true
shape (its output argument is a `Var`). It stays as-is, pinned by G12; fixing
`check_poly_body`'s residual comparison is a separate defect ticket.

### R-30.7 — the trait ships in `lib/core`, impls co-located in the target modules

New `lib/core/applicative.sth` (pattern: `lib/core/iterator.sth` — header
comment, own `import:` lines, `export: Applicative ;`; member names are **not**
exported — synthesized member words are never bare-nameable, the iterator
export note), a pkg module-list entry `applicative` inserted before `option`
(`lib/core/sooth.pkg:6`; the r2c scaffold receipt), and per-ctor impls
co-located in option/result/list — admitted by the orphan rule's target-module
arm (`impl_target_module` and its doc comment, `src/check/declarations.rs:488-508`).
Each impl module gains its own `import:` header for what its body needs (the
applicative import always; `intrinsics | ^ |` for list's cell-boxing body —
`check_owned_cell_word`, `src/check/word_families.rs:1153`, intern at `:1209`).
Consumer goldens import the trait + ctors, never a member name.

## Requirements

- **REQ-30.1.** The declaration gate admits a member whose nonempty inputs lack
  a dispatchable trait-var head iff at least one output mentions the trait var
  as an application head (`'F[...]`, under one `Ref` layer). `pure ( 'A --
  'F['A] )` declares, and its ctor-keyed impl checks. (G1; r2a/r2c receipts.)
- **REQ-30.2.** The gate's refusal faces hold: a member that mentions the trait
  var nowhere (`pick ( 'T -- 'T )`) and a member whose only trait-var mention
  is a *bare* output var (`pick ( 'T -- 'F )`) each still refuse with the
  located S2-15.a text, member-position span, and the nested-input note only
  when the inputs nest the var. (G10.3, G10.4 — new; the G10.4 face is the
  R-30.1 delta pin.)
- **REQ-30.3.** Exactly the two measured gate pins move, as ruled: the unit at
  `src/check/declarations.rs:3932` flips to a success assertion renamed per the
  `thing_condition_expected` convention with its doc comment updated, and the
  slice2 golden `tests/phase7b_slice2.rs:144` retargets in place to the R-30.1
  positive (`build_ok`); no other pin moves. (r2f's measured blast radius.)
- **REQ-30.4.** The zero-input escape-hatch impl mismatch (P2e) stays
  byte-identical — recorded-not-fixed per R-30.6. (G12.)
- **REQ-30.5.** `ap` stays undeclarable at parse; the S1-6 fence bytes are
  unchanged. (G9.)
- **REQ-30.6.** Output-side grounding: at a mono call on an R-30.1-admitted
  member with no dispatchable input, a single explicit type argument unifies
  against the member's trait-var-headed output App — impl selection keys on the
  dissolved ctor head, the ctor's arguments bind the impl target's parameters
  (the S8b seed equation), the App's arguments bind the member's residual
  vars — and the call type-checks with **no arity error**, dispatching
  end-to-end per constructor. Fires only on an explicit instantiation; never
  inferred from consuming context (the S6 Q1 rule). (G14 — new, fixture-local;
  G2, G3 on lib types.)
- **REQ-30.7.** Supplies that are not the single-argument output-App form keep
  today's behavior byte-identically: a 2-argument positional supply whose first
  argument names no impl still fails operand-driven selection with the
  r2b-1c bytes, and the double-wrap operand idiom keeps building and running.
  (G8, G13.)
- **REQ-30.8.** The bare-call remedy error keeps line 1 byte-identical
  (including the `line`/`col` span) and its example becomes achievable — the
  R-30.2 output-App spelling — with exact bytes pinned from the live binary
  before the golden asserts. (G6.)
- **REQ-30.9.** Plain-word output-only bound vars keep all three twin-class
  walls byte-identical (arity wall; the named `poly_unbound_output_error`;
  the parse-refused HKT remedy). (G11.a/b/c.)
- **REQ-30.10.** The route adds **no** new `PolyType`/`Image` variant, no new
  unification machinery, and no IR change: it rides the existing impl-target
  seed channel (`src/check/poly.rs:4111`, `:4186-4214`), the existing
  span-keyed `CallInst` lowering, and the existing subst plumbing. Verifiable
  by inspection plus the full suite staying green. (Load-bearing invariants:
  backend-neutral IR, no comptime interpreter.)
- **REQ-30.11.** `lib/core/applicative.sth` exists: `trait: Applicative['F:
  - -> *]` declaring `pure ( 'A -- 'F['A] )` and nothing else, `export:
  Applicative ;`, iterator.sth export conventions (no member name exported);
  `lib/core/sooth.pkg:6` lists `applicative` before `option`.
- **REQ-30.12.** Co-located per-ctor impls check and dispatch end-to-end:
  option (`: pure Some ;`), result 2-param (`: pure Ok ;`), list (`: pure Nil ^
  Cons ;`), each in its own module with the applicative import header; the
  Option/Result/List construction goldens build and run. (G2, G3, G4.)
- **REQ-30.13.** A shared-bound consumer dispatches two constructors through
  one definition: `repure['F: Applicative 'A] ( 'F['A] 'A -- 'F['A] )` with body
  `swap drop pure` (the paper's correction of the ticket sketch's `swap pure`)
  builds and runs at two mono sites. (G5.)
- **REQ-30.14.** The bare-ctor instantiation fence stays word-general: the
  `pure[Option]` spelling is parse-refused with the p2a bytes on the lib
  spelling. (G7.)
- **REQ-30.15.** The roadmap correction lands: a new S15 section in
  `docs/roadmap/P7b-higher-kinded-types.md` (after the S14 section ending
  `:891`, before `## Out of scope` at `:912`), the out-of-scope sentence at
  `:915` rewritten to record that S6 did **not** settle the grounding and that
  S15 landed it as compiler + library work, and the aggregate P7b row in
  `docs/roadmap/ROADMAP.md:56` amended per project convention.

### Golden → requirement map

| Golden | Requirement | Status of expected bytes |
| --- | --- | --- |
| G1 `true_shape_member_declares_and_ctor_impl_checks` | REQ-30.1 | measured (r2a) |
| G2/G3/G4 lib construction through `pure[Option[i64]]`-class | REQ-30.12 (route: REQ-30.6) | **UNMEASURED-AT-RISK** |
| G5 `repure` two-ctor dispatch | REQ-30.13 | **UNMEASURED-AT-RISK** |
| G6 bare-call remedy, corrected example | REQ-30.8 | line 1 measured (r2b-2); example **UNMEASURED-AT-RISK** |
| G7 bare-ctor fence on the lib spelling | REQ-30.14 | measured (r1 + this round's layout measurement) |
| G8 2-arg positional stays operand-driven | REQ-30.7 | measured (r2b-1c); conservative expectation ruled |
| G9 ap fence | REQ-30.5 | measured (r1 P5) |
| G10 moved pins (unit flip, in-place retarget, negative sibling) | REQ-30.3, REQ-30.2 | flip measured (r2f); negative faces derived + round-3 trivial |
| G11 twin-class walls | REQ-30.9 | measured (r2d) |
| G12 zero-input defect | REQ-30.4 | measured (r1 P2e) |
| G13 double-wrap idiom survives | REQ-30.7 | measured (r2b-3b); survival ruled |
| G14 fixture-local 1-arg route success (**spec-added**, not in the paper) | REQ-30.6 | **UNMEASURED-AT-RISK** — the primary round-3 measurement |

Every UNMEASURED-AT-RISK golden is measured by the round-3 probe (Phase 2's
entry condition) **before** its golden is pinned; a verdict contradicting a
ruling is escalated, not silently retargeted — goldens are the discovery
mechanism (slice13 posture).

## Mechanism — the mono call-site route

Today's route for an R-30.1 member (all measured):

1. `resolve_mono_member_call` (`src/check/poly/ground.rs:1075`) collects
   candidates by name (`:1099-1115`), then dispatches each on its
   dispatchable input's operand (`:1133-1141`) — `dispatchable_input_pos`
   (`ground.rs:295`) returns `None` for pure's shape, so every candidate is
   skipped and `viable` stays empty.
2. The S6/S8b escape hatch (`ground.rs:1183-1209`) runs when `viable` is empty:
   with an explicit type argument it calls `find_bound_impl`
   (`src/check/poly/trait.rs:174`) on `type_args.first()` **as the impl-target
   type**, keeping the impl-target equation (`match_impl_target`: target
   pattern vs the call-site type) as `impl_target_seed` (`ground.rs:1182`).
   For `pure[Box[i64]]` this **already selects the Box impl** and binds
   `'ctor0:=i64` — impl selection keyed on the dissolved ctor head exists.
3. The generic-impl branch calls `check_poly_call` with the seed
   (`ground.rs:1472`), whose arity gate (`src/check/poly.rs:4164`) compares the
   raw argument count (1) against the dissolved word's var count (2) **before**
   the seed branch (`:4186-4214`) — the wall r2b-1/r2c measured
   (`instantiation_arity_error`, `src/check/poly.rs:5760`).
4. Bare call: the remedy branch (`ground.rs:1213-1232`) fires
   `mono_nullary_member_no_instantiation_error` (`ground.rs:1514`) with the
   hardcoded unachievable `pure[i64]` example (r2b-2). 2-arg positional: the
   escape hatch's `find_bound_impl(trait, i64)` fails, `viable` stays empty,
   and `mono_member_no_dispatch_error` (`ground.rs:1492`) fires (r2b-1c).

R-30.2's route, in the same terms: at step 2, when the candidate member's
output row contains a trait-var-headed App and exactly one type argument is
supplied, the argument **is** the output-App instantiation — `find_bound_impl`
on it keys the impl (unchanged), the seed equation binds the target's
parameters, and the route additionally binds the member's residual vars from
the output App's arguments against the instantiation's ctor arguments (Box:
`'A:=i64`; 2-param Result: `'A` binds at the App's argument position, the
leftover target slot `'T1` from the seed — G3's round-3 measurement pins the
exact bytes). The extended seed rides the existing channel
(`src/check/poly.rs:4111`, S8b) into `check_poly_call`, whose arity gate takes
the route's exception (the written count is not compared positionally on this
path; a wrong-arity supply on any non-route shape keeps `:4164`'s bytes). θ is
complete ('target params + member locals — `build_member_var_union`,
`src/parser.rs:768`, orders them), the operand unification finds its variables
pre-bound (a disagreeing operand stays a diagnostic, the P7.S3t seeded-conflict
discipline), and lowering emits through the existing span-keyed `CallInst`
(symbol + θ) — no IR change (REQ-30.10).

The route is reachable **only** through the escape hatch (operand dispatch
failed): operand-driven selection (`viable` non-empty), plain-word
instantiation (G11), the concrete-target member branch, and the parser fence
(G7) are untouched.

## Blast radius

Measured by r2f (`cargo test --no-fail-fast`, all targets, under the exact
R-30.1 patch): **exactly two failures**, both pins of the same
`pick ( 'T -- 'F['T] )` shape —

1. `check_trait_decls_rejects_member_with_no_dispatchable_input`
   (`src/check/declarations.rs:3932`; helper `trait_check_src` at `:3842`):
   the gate's own unit — panics on `unwrap_err()` because the member now
   declares. Moves to a success assertion (REQ-30.3).
2. `hkt_member_without_dispatchable_input_is_located_error`
   (`tests/phase7b_slice2.rs:144`; `build_error` at `:61`): the round-1-pinned
   golden — the fixture builds clean, so the verbatim text assert fails.
   Retargets in place (REQ-30.3).

No impl-check, dispatch, or diagnostics pin moved. The S2-15.a error text keeps
golden-level coverage through the new negative siblings (G10.3/G10.4) plus the
retained unit `check_trait_decls_rejects_a_receiver_nested_in_an_array_input`
(`src/check/declarations.rs:3964`).

## Phase plan

### Phase 1: the gate admits output-App-headed members; pins move and negatives land

- **Goal**: `pure ( 'A -- 'F['A] )` declares and its ctor-keyed impl checks
  (fixture-local Box builds clean, G1), the two moved pins are retargeted as
  ruled, and the gate's refusal faces (nowhere-mentioned; bare-output) are
  pinned at unit and golden level.
- **Requirements covered**: REQ-30.1, REQ-30.2, REQ-30.3, REQ-30.4, REQ-30.5.
- **Scope**:
  - Modify `src/check/declarations.rs:408-418` (`member_binds_trait_var`):
    add the output arm with an App-headed-only predicate —
    `matches!(t, PolyType::App { head: 0, .. })` under one `Ref` unwrap,
    mirroring the input arm's courtesy. Start from `/tmp/soo30_r2_gate.patch`
    and **tighten its output arm** (the patch's verbatim `dispatchable_head`
    reuse admits a bare `Var(0)` output — R-30.1 refuses it). Do not touch
    `dispatchable_head` (`:404`) itself, the error text
    (`nested_receiver_member_error`, `:452`, text `:461-466`), or
    `member_inputs_nest_trait_var` (`:421`).
  - Modify `src/check/declarations.rs:3927-3931` (doc) and `:3932`
    (`check_trait_decls_rejects_member_with_no_dispatchable_input`): flip to
    `.unwrap()` success, rename
    `check_trait_decls_accepts_member_with_only_output_trait_var`, rewrite the
    doc comment to record the output arm (the paper's G10.1 retarget).
  - Add units beside `:3889` (`member_binds_trait_var_accepts_any_receiver_position`
    is the predicate-unit precedent): the output-App shape returns `true`; the
    bare-output shape and the nowhere shape return `false`; plus
    `check_trait_decls_rejects_member_mentioning_the_trait_var_nowhere`
    (fixture `pick ( 'T -- 'T )`, S2-15.a text + `!err.contains("note:")`,
    `:3932-3962` style) and the bare-output refusal twin.
  - Modify `tests/phase7b_slice2.rs:144`: retarget in place to `build_ok`,
    rename `hkt_member_with_only_output_trait_var_declares` (same fixture,
    member `pick ( 'T -- 'F['T] )` under `Functor['F: * -> *]`); add the
    negative golden sibling beside it (G10.3) and the bare-output negative
    (G10.4).
  - Create `tests/phase7b_slice15.rs` (harness helpers copied from
    `tests/phase7b_slice2.rs:42-136`) with G1 (`single_file` + `build_ok`),
    G9 (`single_file` + `build_error`), G12 (`single_file` + `build_error`) —
    all three fully measured today.
  - **Out of bounds**: no parser changes; no `src/check/poly/` changes (the
    call-site route is Phase 2); no `lib/` changes; no new diagnostics.
- **Entry conditions**: none beyond the repo state (HEAD `846f952` class;
  `/tmp/soo30_r2_gate.patch` available as the starting point).
- **Exit criteria / verifiable artifacts**: `cargo fmt --check && cargo clippy
  -- -D warnings && cargo test` green; the r2a fixture
  (`probes/soo30r2_a_decl_impl.sth`) builds clean under `cargo run -- build`;
  the test inventory shows exactly the intended movement (the flipped unit +
  retargeted golden green, the new negatives green, all other suites
  untouched — reproducing r2f's blast radius deliberately).
- **Parallelism**: SEQUENTIAL — the first phase; Phases 2 and 3 both depend on
  the gate admitting the shape.
- **Relative Effort**: S — a small predicate change plus four test-file edits,
  with the blast radius already measured (r2f).
- **Difficulty**: `standard` — no concurrency, no migration, no
  security-sensitive surface; the rule and its blast radius are measured.
- **Open Questions / Blockers**: R-30.1 awaits ratification; the App-headed-only
  tightening (vs the patch's bare-`Var` admission) is the one decision a
  rejection would change (it would delete G10.4 and widen the predicate).

### Phase 2: output-side grounding at mono member calls, remedy correction, round-3 probe

- **Goal**: `42 pure[Box[i64]] showbox` builds and prints `42` (G14) — the
  explicit instantiation grounds the return type at a mono call site with no
  `'F` operand; the bare-call remedy's example is achievable; every
  not-supposed-to-move call-site diagnostic is byte-identical.
- **Requirements covered**: REQ-30.6, REQ-30.7, REQ-30.8, REQ-30.9, REQ-30.10.
- **Scope**:
  - **Entry condition — the round-3 probe runs FIRST** (mirroring the S13
    measure-then-pin posture): under the Phase-1 gate, probe fixtures record
    live bytes for G14, G2–G5 (temporary lib scaffolding, the r2c pattern,
    reverted after), G6's example rendering, G8, and G13 into
    `probes/soo30r3_baseline.md` (plus a findings addendum) **before any
    golden is pinned**. A verdict contradicting R-30.2/R-30.3 is escalated to
    the user, not silently retargeted.
  - Modify `src/check/poly/ground.rs:1183-1209` (the S6/S8b escape hatch in
    `resolve_mono_member_call`): for a candidate whose output row contains a
    trait-var-headed App and a single type argument, bind the member's
    residual vars from the output App's arguments (positionally against the
    instantiation's ctor arguments) and extend `impl_target_seed`
    (`ground.rs:1182`) with them. `find_bound_impl`
    (`src/check/poly/trait.rs:174`) on the argument is unchanged — it already
    keys impl selection on the dissolved ctor head.
  - Modify `src/check/poly.rs:4164` (the arity gate in `check_poly_call`,
    whose seed branch is `:4186-4214`): the output-App route's exception — a
    supply routed through the extended seed is not compared positionally
    against the dissolved word's var count. Wrong-arity supplies on non-route
    shapes keep `:4164`'s bytes (`instantiation_arity_error`, `:5760`).
  - Modify `src/check/poly/ground.rs:1514-1521`
    (`mono_nullary_member_no_instantiation_error`, called from `:1213-1232`):
    render the example as the R-30.2 output-App spelling from the member's
    output App and the trait's impls (the single-impl case is the pinned one);
    line 1 stays byte-identical. Thread whatever the rendering needs through
    the `:1213-1232` call site.
  - Create goldens in `tests/phase7b_slice15.rs`: G6, G8, G11.a/b/c, G13
    (`single_file` + `build_error`/`build_and_run` per the paper's placement
    section), G14 (`single_file` + `build_and_run`).
  - **Out of bounds**: no parser changes; no `lib/` changes (Phase 3); the
    operand-driven selection path (`viable` non-empty), the concrete-target
    member branch, and `poly_unbound_output_error`
    (`src/check/poly.rs:5852`) are untouched — G11's bytes pin the
    non-leakage; no new `PolyType`/`Image` variants; no IR/lowering changes.
- **Entry conditions**: Phase 1 landed (the gate admits the shape — the route
  is unreachable otherwise); the round-3 probe bytes recorded (same phase,
  first step).
- **Exit criteria / verifiable artifacts**: green gate; G14 runs `42\n`; G6's
  line 1 byte-exact with the example asserted achievable separately; G8, G11,
  G13 byte-exact per the r2 measurements; the probe artifacts exist and cite
  the measured bytes behind every pinned expectation.
- **Parallelism**: SEQUENTIAL after Phase 1 (needs the gate). No parallel
  partner — Phase 3's goldens depend on this phase's route semantics being
  fixed by measurement.
- **Relative Effort**: M — the route is a delta on measured machinery, but it
  spans the escape hatch, the seed channel, the arity gate, and a diagnostic,
  and it carries the probe round.
- **Difficulty**: `hard` — call-site grounding in the checker's shared
  dispatch control flow, with unification semantics (residual-var binding,
  2-param targets) that are exactly the ambiguous-integration-point class;
  the at-risk goldens make silent mis_grounding the failure mode to design
  against.
- **Open Questions / Blockers**: R-30.2/R-30.3 await ratification; the exact
  G3 bytes (2-param Result through the 1-arg spelling) and the G6 example
  rendering are measurement outputs, not spec givens — the ruling survives
  either measured outcome only if the user ratifies the "partial heads ride
  along" and "achievable example" intents as written.

### Phase 3: the Applicative library payload and the roadmap correction

- **Goal**: a consumer writes `import: core::applicative | Applicative | ;`,
  calls `5 pure[Option[i64]]` and friends, and per-constructor dispatch
  produces real `Some`/`Ok`/`Cons` values (G2–G4 print `5\n`); one
  shared-bound definition re-pures two constructors (G5 prints `7\n7\n`); the
  roadmap records the corrected story.
- **Requirements covered**: REQ-30.11, REQ-30.12, REQ-30.13, REQ-30.14,
  REQ-30.15.
- **Scope**:
  - Create `lib/core/applicative.sth` modeled on `lib/core/iterator.sth`
    (header comment; `import: intrinsics | ... | ;` as needed;
    `export: Applicative ;`; the trait with `pure` only — no member name in
    any `export:`).
  - Modify `lib/core/sooth.pkg:6` (the `module:` list): insert `applicative`
    before `option` (the r2c receipt).
  - Modify `lib/core/option.sth:1` (prepend the applicative import; append
    `impl: Applicative for Option : pure Some ;`), `lib/core/result.sth:1`
    (same, `: pure Ok ;`), `lib/core/list.sth:1` (applicative import +
    `import: intrinsics | ^ | ;` for the cell-boxing body,
    `src/check/word_families.rs:1153`; `: pure Nil ^ Cons ;`). The r2c
    scaffold is the measured pattern for all three.
  - Create goldens in `tests/phase7b_slice15.rs`: G2, G3, G4, G5, G7 —
    `single_file_hosted` + `build_and_run`/`build_error`; the fixture texts
    drop their own `intrinsics`/`show` import lines (the harness prepends
    them; duplicate imports collide — the paper's placement section).
  - Modify `docs/roadmap/P7b-higher-kinded-types.md`: insert the S15 section
    between the S14 section (ends `:891`) and `## Out of scope` (`:912`);
    rewrite the `:915` sentence per REQ-30.15. Modify
    `docs/roadmap/ROADMAP.md:56` (the P7b row) to record the S15 landing.
  - **Growth-structure re-check at phase exit** (CLAUDE.md): re-run the five
    signals against `src/check/declarations.rs` and
    `src/check/poly/ground.rs` as they now stand, and record the verdict in
    the phase summary (the slice13-spec precedent).
  - **Out of bounds**: no changes to the option/result/list *type*
    declarations or existing exports; no `ap` declaration anywhere; no
    checker changes (compiler work is Phases 1–2); no twin-class changes.
- **Entry conditions**: Phases 1 and 2 landed; the round-3 probe bytes for
  G2–G5 recorded (Phase 2's first step) so the goldens pin measured
  expectations.
- **Exit criteria / verifiable artifacts**: green gate; G2/G3/G4 run `5\n`
  each; G5 runs `7\n7\n`; G7 byte-exact; a `cargo run -- build` of a
  core-importing probe against the real `lib/` (via
  `--manifest tests/fixtures/sooth.pkg`) succeeds; the roadmap documents
  state the corrected story; the growth-signal verdict is recorded.
- **Parallelism**: SEQUENTIAL after Phase 2 (its goldens exercise the route
  and its fixtures import the trait module the route grounds through). Phase 3
  could start its `lib/` scaffolding beside Phase 2, but its goldens cannot
  pin until Phase 2's semantics are measured — keep it sequential.
- **Relative Effort**: S — the landing shape is fully measured (r2c), the
  goldens' bytes are probe outputs, and the docs edits are bounded.
- **Difficulty**: `standard` — mechanical module/impl/golden/docs work against
  a measured scaffold; no new checker logic.
- **Open Questions / Blockers**: none beyond the ratification shared with
  Phase 2 (the lib goldens inherit whatever the round-3 probe measured).

### Parallelism summary

All three phases are sequential: 2 needs 1's gate, 3 needs 2's measured route
semantics. The only concurrency inside the plan is the round-3 probe, which is
Phase 2's first step, not a separate lane.

### Effort summary

S (Phase 1) + M (Phase 2) + S (Phase 3) ≈ two to three weeks end to end,
dominated by Phase 2's grounding route and probe round.

## Non-functional requirements

- **NFR-1 — the empty precedent's no-inference rule is load-bearing.** The
  route fires only on an explicit instantiation
  (`bare_nullary_member_without_instantiation_is_located_error`,
  `tests/phase7b_slice6.rs:269-290`, is the shipped golden of the rule); no
  consuming-context lookahead is added (folded into REQ-30.6's contract).
- **NFR-2 — diagnostics are behaviour.** The error bytes named in G6, G7, G8,
  G9, G11, G12 are requirements: the golden set pins them, and a phase that
  changes any of them without a ruling has regressed.
- **NFR-3 — closed file list.** Compiler: `src/check/declarations.rs`,
  `src/check/poly/ground.rs`, `src/check/poly.rs`. Tests:
  `tests/phase7b_slice2.rs` (moved pins), `tests/phase7b_slice15.rs` (new),
  `src/check/declarations.rs` `mod tests`. Library: `lib/core/applicative.sth`
  (new), `lib/core/sooth.pkg`, `lib/core/option.sth`, `lib/core/result.sth`,
  `lib/core/list.sth`. Docs: `docs/roadmap/P7b-higher-kinded-types.md`,
  `docs/roadmap/ROADMAP.md`. Probe artifacts under `probes/`. Nothing else.
- **NFR-4 — test coverage conventions** (CLAUDE.md): every touched stage
  function keeps happy-path + error-case unit coverage beside it;
  `thing_condition_expected` naming; the goldens are the phase exit criteria.
- **NFR-5 — the green gate.** Every phase's exit is
  `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green.

## Out of scope

- `Applicative.ap` in any form (R-30.5; the S7 deferral and the parse fence
  stand untouched).
- Plain-word output-only bound-var grounding — the twin class stays walled
  (R-30.4; G11 pins the walls).
- The P2e identical-renderings defect fix (R-30.6; G12 pins it as-is; separate
  defect ticket).
- Consuming-context inference for member grounding (NFR-1).
- The recorded diagnostics gaps: the declaration-gate error's missing file path
  when it fires inside an imported lib module (r1 P3min) and G12's
  identical renderings (`poly_type_str`'s Concrete/ Generic collision,
  `src/check/poly.rs:5912`/`:5978-5990`) — both noted, neither fixed here.
- The bare-ctor parse fence's kind-awareness (`pure[Option]` stays refused,
  G7); the Monoid/`empty` nullary route's behavior beyond what the route
  reuses.

## Open questions

- [ ] Do R-30.1–R-30.7 stand as ratified? (The spec proceeds on them; a
      rejection of R-30.2's conservative-G8 face would flip G8/G14
      expectations and widen Phase 2; a rejection of R-30.1's App-headed-only
      face would delete G10.4.)
- [ ] The remedy example's rendering when the trait has **multiple** impls
      (the pinned case is single-impl; multi-impl choice — first impl vs a
      generic shape — is unspecified; round-3 records the single-impl bytes,
      and the multi-impl case stays out of the goldens).
- [x] ~~Bare-`Var(0)` output members under the relaxed gate — resolved by
      R-30.1: App-headed only, pinned negative (G10.4); the r2 patch's
      verbatim `dispatchable_head` reuse is tightened.~~
- [x] ~~Does G8 flip under output-keyed selection? — resolved by ruling:
      conservative (positional supplies keep operand-driven selection);
      confirmed by round-3 measurement before pinning.~~
- [x] ~~`repure`'s body — resolved by the paper's mechanism note: `swap drop
      pure`, not the ticket sketch's `swap pure`.~~

## Risks & mitigations

| Risk | Likelihood | Mitigation |
| ---- | ---------- | ---------- |
| G2–G4/G14 bytes differ from the ruled expectation (residual-var binding wrinkle, esp. 2-param Result) | Med | Round-3 probe runs before any golden is pinned; contradictions escalate to the user (slice13 discovery posture) |
| The route leaks into plain-word instantiation or operand-driven selection | Low | G11.a/b/c and G8/G13 byte-pin the non-leakage; the out-of-bounds lists name the untouched paths |
| The gate relaxation admits an unintended member shape | Low | r2f measured the blast radius as exactly two pins; the new negative units/goldens pin both refusal faces |
| The corrected remedy example is unachievable in multi-impl traits | Med | Single-impl case pinned; multi-impl flagged as an open question, out of the goldens |
| Moved-pin retargets drift from the r2f-measured flip | Low | The retargets are named test-for-test (G10.1/2); the full-suite inventory is an exit criterion |
| P2e's identical renderings mislead a future debugger into "fixing" G12 here | Low | R-30.6 records the decision; G12 pins the bytes; the root cause (r2e) is cited for the follow-up ticket |

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Declaration-gate relaxation (App-headed-only output arm), the two moved pins, unit and golden negatives, G1/G9/G12",
      "effort": "S",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "Output-side grounding route at mono member calls, remedy correction, round-3 probe, call-site goldens G6/G8/G11/G13/G14",
      "effort": "M",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "lib/core/applicative.sth + co-located Option/Result/List impls, lib goldens G2-G5/G7, roadmap correction",
      "effort": "S",
      "difficulty": "standard"
    }
  ]
}
```
