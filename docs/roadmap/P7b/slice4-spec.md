# P7b.S4 spec — declaring-module identity for generic instantiations

Technical specification for compiler slice P7b.S4. Scope input is the recon
[brief](./slice4-brief.md) (findings F1–F8, machinery map, open questions Q1–Q5) and the
verbatim [probe log](./slice4-probes.md); exit criteria are from the
[phase doc](../P7b-higher-kinded-types.md). All anchors were re-verified against HEAD
`ad136f3` while writing this spec (see [Correction to the brief](#correction-to-the-brief)),
and the design is the probe round's measured winner, not a fresh adjudication: candidate C
(m5's nine-item battery, reference shape `/tmp/p7bs4-probes/candidate-c.diff`).

Diagnostic texts here pin **shape**, not wording; exact strings freeze in the goldens
(S12 precedent, as in S1/S2/S3). The probe round's texts are drafts.

**Probe artifacts are disposable.** `/tmp/p7bs4-probes/` may not survive to the
implementation phases. Everything an implementer needs — the three-hunk change, the
fixture shapes, the output pins, the symbol expectations — is reproduced in this
document. Treat the /tmp tree as corroborating evidence only.

## Exit criteria (from the phase doc)

`impl: Functor for Option` in a user module dispatches for an `Option[i64]` operand
named in that module; S2's W3/W4 goldens migrate from fixture twins to the real lib
types unchanged in behavior; no duplicate monomorphs are introduced by the widened
identity. S3's dogfood (real `Option`/`Result`/`List` through a shared bound) depends
on this slice or on twin workarounds.

All three clauses were **measured live** in m5 (probe log, battery (a)–(h)): the first
two flip from rejection to pass with the exact output pins, and the third holds with
symbol sets byte-identical on every unchanged program. S4's job is to land the change
and convert the probe battery into committed goldens — the suite observes none of this
today (F7: zero new full-suite failures under all four experiment patches, no test pins
any `c{}m{}` symbol, and the wart's only in-repo defender is one unit test's doc-pinned
applying-module assertion).

## What the brief established, and what it did not

The brief's three load-bearing results stand and this spec is built on them:

- **F1 — the wart is two producers, not a convention migration.** Of nine non-test mint
  producers of `instantiate_struct`/`instantiate_enum`, only two key on the *naming*
  module: `resolve_type_or_apply` (src/parser.rs:6866/:6884 — the declaring `owner` is
  already computed at :6833 and used only for the header lookup) and
  `poly_construct_generic`'s no-fallback arm (src/check/poly.rs:5946, which also feeds
  the :6039/:6041 construction mint and the symbolic `PolyType::Generic` record at
  :6045-6050). Every other producer — the all-concrete folds (parser.rs:5500/:5502,
  :7747/:7750), `substitute_generic_field` (ast.rs:940/:942/:969/:977), `apply_subst`
  (poly.rs:10243/:10245/:10303/:10387/:10389), impl-target pattern pins
  (parser.rs:3970) — already passes the declaring module. Soundness falls out of the
  registry layout: one `GenericTypes` per program (src/driver.rs:671, pre-pass
  :679-689) makes `idx` globally unique, so `(idx, args, lens)` identifies a header
  without any module.
- **F3/F4 — matching-blind and naive dedup each fix exactly the family the other
  misses.** B (blind the four dispatch comparisons) fixes poly bound dispatch but can
  never fix mono: same-rendering distinct-mint `Type` equality
  (`` `showopt` expected `Option[i64]`, found `Option[i64]` ``) sits outside every
  PolyType comparator. A (drop `module` from the memo keys) dedups but leaves dispatch
  module-strict and makes the minted decl's `.module` a first-minter lie.
- **F5 — candidate C meets every measured exit criterion with zero comparator edits.**
  The 3-site patch (S4-1) flipped P1 to `2`, P2 to `1`, built P4 on one truthful shared
  mint (zero `sooth_mono_*`), left P5's marker byte-identical, kept slice1/slice2 at
  16/16 and 17/17, migrated W3/W4 with output pins unchanged, collapsed the T6
  recursive duplicate, and held symbol sets byte-identical on every unchanged program.
  The full cost is exactly one unit-test re-baseline (S4-3) — and, per P7, nothing else
  (Q2).

What the brief did **not** settle, and the rulings below do: the exact re-baseline text
(Q1), the golden set and its pins (Q4), the W4 two-ctor parking (Q3), and the one
unprobed path — variant-annotation tags spelled from a non-declaring module (Q5).

## Correction to the brief

Re-verified at HEAD `ad136f3`. The brief's substance is fully confirmed (including
`driver.rs:769` = `impls.extend(bodies.impls);`, the program-wide impl registry that
makes `find_bound_impl` visibility-blind, and the m5 battery logs). Five anchors
drifted; the spec cites the re-verified lines throughout:

1. **`bare_generic_owner` is at `src/parser.rs:6928`**, not :6940 (:6937 is
   `generic_is_declared`, which it calls).
2. **Minted-decl `__m{naming}` mangling is `src/resolve.rs:791-797`** — the
   struct/enum registry mangling loops over the *minted* entries, whose `.module` is
   the minting module — with `mangle` at `src/resolve.rs:36` (`{name}__m{module}` at
   :40/:54). The brief's :803-807 is past those loops.
3. **`poly_construct_generic`'s construction mint is at `src/check/poly.rs:6039/:6041`**
   (brief: :6038/:6040), with the symbolic `PolyType::Generic` record at :6045-6050.
   Same statements, one-line drift.
4. **`mono_member_no_dispatch_error` is at `src/check/poly.rs:2404`** (brief cited
   :2409/:2413 — those are the `operands` param and the text inside it).
5. **The CtorImage identity comparison is `pattern_id == gid` at
   `src/check/poly.rs:8849-8857`** (brief's :8842-8852 is the comment block above it).

One fixture-provenance note, not an anchor correction: the `m4_twin_impls/` fixture in
/tmp evolved *after* the m4 run — u1's `run` now carries the qualified
`Functor::unbox` probe spelling (whose `unknown word` error is the F8 remedy-hole
record, log `m4-qualified-functor.log`), whereas the ambiguity capture recorded in
`mutations.md` §m4 routes through the poly word `go`. The S4-12 golden spells the
mutations.md shape (`run` → poly `go` → `unbox`), not the stale fixture body.

## Rulings on the brief's open questions

Each of Q1–Q5 is ruled here, not carried forward.

### S4 ruling on Q1 — the re-baseline is the *only* expected test change

**Ruled: rewrite `parse_qualified_generic_application_from_another_module_resolves`
(src/parser.rs:10653, assert :10691) to pin the declaring-module mint, quoting the old
doc as the wart's record.** m5(e) measured the full suite at 3077/1 under C with this
as the sole failure (`assert_eq!(generics.inst_structs[0].module, 1)`, `left: 0,
right: 1`). The test's doc comment (:10643-10651) pins the removed wart verbatim —
"stamped with the *applying* module, not the declaring one" — and is rewritten the same
way as S2's fence-text baseline: the new text states the declaring-module convention
and quotes the old sentence as the recorded wart it replaces. No other test moved
under any of the four experiment patches (F7), so this is the entire re-baseline.

### S4 ruling on Q2 — resolved by P7; the orphan rule is untouched

~~Does the concrete-pinned lib target re-home under C, tightening the orphan rule?~~
**Resolved by probe P7 (fixture `q2demo`): no.** The mapper's §5-C claim was an
over-inference contradicted by its own §1: the impl-target pin mints through the
all-concrete fold on the pattern's recorded (declaring) owner, so
`impl_target_module`'s concrete arms (src/check/declarations.rs:493-498) already read
declaring-keyed mints for impl-target pins. Measured: `impl: Show for Option[i64]` in
a user module with a core-side `Show` is orphan-rejected at declaration today and the
error is **byte-identical** under C (logs `p7-orphan-today.log` /
`p7-orphan-under-C.log`, diff-confirmed). No ruling, no behavioral golden, no orphan
change — S4-14 records the fence.

### S4 ruling on Q3 — W4's two-ctor half is parked on a future real `* -> *` lib type

**Ruled: the migrated W4 golden covers real `core::option` through the shared bound
with the `"-1\n3\n"` pin; the two-impl-per-constructor half waits for `core::list`.**
lib/core has exactly two generic headers — `Option['T]` (`* -> *`,
lib/core/option.sth:1) and `Result['T 'E]` (`* -> * -> *`, lib/core/result.sth:1) — so
a `* -> *` shared bound has no second real ctor to dispatch to. This is a **library**
block, not a compiler block; it is the same dependency the phase doc's dogfood line
already carries (`List` does not exist yet). Recorded in the ledger; nothing in S4
pretends to fix it.

### S4 ruling on Q4 — the golden set is the deliverable

**Ruled: the seven-fixture battery below is S4's test deliverable** (F7: the area is
unobserved by the suite today). Positive: P1 and P2 as new goldens over the real lib
types; P4's two-module shared-mint build; the T6 recursive single-mint fixture
(`^` indirection mandatory — the direct self-field is the pre-existing infinite-size
error, `src/check/declarations.rs:1785`); m4's per-trait control. Error /
non-regression: m4's `mono_ambiguous_member_error` as a located-error golden; P5's
same-named-ctor marker byte-pinned (S5 territory — S4 must not move it); the Q1
re-baseline. Full pins in the golden set section.

### S4 ruling on Q5 — close the variant-tag probe gap with one positive fixture

**Ruled: add the leading-variant-slot tag fixture spelled from a non-declaring
module.** The mapper flagged `variant_name_is_visible`/`module_declares_variant`
(src/parser.rs:6021/:6047-6060) as flowing through declaring-module name tables and
unprobed by m5's battery (which exercised the ctor/eliminator env paths but not the
quotation-annotation tag path, `parse_leading_variant_slot`, src/parser.rs:5992,
called at :5944). The negative pins exist
(`parse_leading_variant_slot_other_module_variant_is_not_visible` :9036 and its
generic twin :9088); the positive twin — a wildcard import of `core::option` making
`( Some )` visible as a routing tag in a non-declaring module — does not. One fixture
pins it (S4-13). No code change is expected; if the fixture fails, that is a finding,
not a pre-approved fix.

## Requirements

### The change (Phase 1)

**S4-1 — the two naming-module mint producers key on the declaring module.**
`resolve_type_or_apply`'s struct arm (src/parser.rs:6866, `instantiate_struct`) and
enum arm (:6884, `instantiate_enum`) pass the `owner` bound at :6833 instead of
`self.module`; `poly_construct_generic`'s no-fallback arm (src/check/poly.rs:5946)
keys on `generics.enums[idx].module` / `generics.structs[idx].module` instead of
`ctx.module()`. Reference shape: the 3 hunks of `/tmp/p7bs4-probes/candidate-c.diff`,
measured fmt/clippy-clean on `ad136f3`. Verifiable: the re-baselined unit test (S4-3)
asserts the declaring module, and the S4-4/S4-5 programs build and print.

**S4-2 — fence: no new registry entry type, key shape, or call site.** The memo keys
stay the `(idx, module, args, lens)` 4-tuples (src/ast.rs:645-646, pushes
:1334-1335/:1385-1386); the four dispatch comparisons (`pattern_id == gid`
poly.rs:8849-8857, `found_module != *module` at :8868/:9019/:9875), CtorImage
identity, `instantiation_symbol` mangling (ast.rs:2869/:2885), and env dispatch
(terms.rs:1399-1404) are untouched. Verifiable by inspection: the landed diff contains
exactly the three S4-1 lines and nothing else in src/.

**S4-3 — the one re-baseline.** `parse_qualified_generic_application_from_another_
module_resolves` (src/parser.rs:10653) asserts `generics.inst_structs[0].module == 0`
(the declaring module) at :10691, and its doc comment (:10643-10651) is rewritten to
pin the declaring-module mint while quoting the old applying-module sentence as the
wart's record. Verifiable: the full suite is green with the rewritten test.

**S4-9 — symbol parity on every unchanged program.** Programs that build at HEAD build
with byte-identical `nm` symbol sets after the change: the P6 twin control, `examples/
gcd.sth`, and the buildable P4-family fixtures (p4_one_module, p4m_poly_id,
p4u_two_types, p4v_dormant, p4y_b_only). Local headers have naming == declaring, so
nothing moves (m5(g)). Verifiable: `nm` diff per binary, recorded in the phase's
commit message.

**S4-14 — the orphan mechanism is untouched.** No diff hunk touches
src/check/declarations.rs: `impl_target_module` (:491), the gate (:578), the duplicate
scan (:539-560), and the error text (:648-672) are byte-identical. Verifiable by
inspection of the landed diff; the P7 record (Q2) is the behavioral evidence. No
behavioral golden is owed.

### The exit-criterion goldens (Phase 2)

**S4-4 — mono dispatch over the real lib Option (P1).** A user module that
wildcard-imports `core::option` and declares `trait: Functor['F: * -> *]` with member
`map` plus `impl: Functor for Option` dispatches a mono member call on an `Option[i64]`
operand named in that module. The golden is probe fixture `p1_mono_real_option` ported
verbatim (explicit-instantiation spelling `map[i64 i64]`, the W2 form) and prints
`2\n`.

**S4-5 — poly bound dispatch over the real lib Option (P2).** The same shape with the
shared-bound poly word `twice['F: Functor 'T] ( 'F['T] [ 'T -- 'T ] -- 'F['T] )`
builds and prints `1\n`. Golden is `p2_poly_real_option` ported verbatim.

**S4-6 — W3 migrates to real `core::result` byte-for-byte.**
`functor_map_over_result_dispatches_to_the_result_impl_and_passes_err_through`
(tests/phase7b_slice2.rs:509) replaces its local twin declaration with
`import: core::result * ;`, changes nothing else, and keeps the `"0\n2\n"` pin.

**S4-7 — W4 migrates to real `core::option` through the shared bound.**
`functor_map_through_shared_bound_dispatches_per_constructor_from_poly_body`
(tests/phase7b_slice2.rs:558) migrates to `core::option` with the `* -> *` Functor and
`twice['F: Functor 'T]` (the `candidate-c-testedit.diff` shape: the Opt/Res two-param
twins had no real same-kind second ctor), keeping the `"-1\n3\n"` pin; the slice2
suite stays 17/17 with both migrations in.

### The identity goldens (Phase 3)

**S4-8 — the two-module shared-mint build (P4).** Two user modules each naming
`Option[i64]` (fixture `p4_two_modules`: mod_a/mod_b, identical `mk`/`un`/`run`)
build and run on one truthful shared mint. Verifiable: build succeeds (pre-change:
`type mismatch in mk`, order-dependent), run exits 0, and `nm` on the binary shows
zero `sooth_mono_*` symbols (m5(b)).

**S4-10 — the recursive single-mint fixture (T6).** A recursive generic header
declared in one module (`type: L['T] | Nil | Cons 'T rest ^L['T] ;` — the `^`
indirection is mandatory, `src/check/declarations.rs:1785`) and named from another
builds and runs, minting one `Cons`. Pre-change the build fails with two identical
`Cons` overloads (`candidate: i64 ^L[i64]` twice — outer naming mint + inner declaring
mint, log `m5-t6-pre.log`); post-change it builds and runs (log `m5-t6-post.log`).

### The fence goldens (Phase 4)

**S4-11 — the S5-boundary marker does not move (P5).** The same-named-ctor
cross-pick error (`type mismatch in mk (line 7)` family — two user modules each
declaring their own `Widget['T]`, fixture `p5_same_named_ctors`) is byte-identical
post-change (m5(c) diffed against the wave-1 record). Byte-pinned as non-regression.

**S4-12 — twin-impl ambiguity stays located, and per-trait dispatch stays clean
(m4).** With two same-named traits each holding `impl: Functor for Option` (the same
widened identity now), a mono caller whose `Option[i64]` operand both impls dispatch
on gets the located `mono_ambiguous_member_error` ("`unbox` … is a trait member of
both `Functor` and `Functor`", poly.rs:2450, fired at :2256) — no silent first-win.
The fixture family carries the per-trait control: a module with only its own trait
builds and prints `39` via its poly route (`run` → `go['F: Functor 'T]` → `unbox`).
Note the delta this golden pins: pre-change the both-traits shape fails with
`mono_member_no_dispatch_error` (neither impl matches the naming-module operand);
post-change it fails with the *ambiguity* error — a strict improvement in
diagnostic precision, still located.

**S4-13 — a leading-variant-slot tag is visible from a non-declaring module (Q5).**
A module that wildcard-imports `core::option` spells a quotation annotation with a
leading variant slot (`[ ( Some ) … ]`); the tag parses as a `VariantTag` (not
`unknown type`Some``), and the program builds and runs. Positive twin of the
negative pins at src/parser.rs:9036/:9088; visibility routes through the
wildcard-desugared selective map (src/driver.rs:569-590) to
`module_declares_variant` (parser.rs:6047). No code change expected.

## Considered and rejected

- **Candidate A — drop `module` from the memo keys** (m1). Rejected: P1/P2
  byte-identical baseline errors (dispatch stays module-strict), P4 builds on a
  first-minter lie (`.module` wrong for the second module), and A-with-declaring-
  recovery converges with C observably while churning key shapes, lookup signatures,
  and every tuple consumer. The one-mint outcome C delivers comes from the convention
  change alone, with zero key edits.
- **Candidate B — blind the four dispatch comparisons** (m2/m2b). Rejected: fixes poly
  bound dispatch (P2 prints 1) but mono dies at a plain `Type` equality outside every
  PolyType comparator — same rendering, two distinct minted handles. B can never fix
  the mono half of the exit criterion. Recorded as the fallback design with its
  ceiling (F3) quoted.
- **Candidate A+B together.** Rejected without measurement: it is A's key churn plus
  B's comparator churn to reach what C reaches with three lines, and the m2b mono
  failure shows the comparator blindness is still needed *only* because the mints
  disagree — which C removes at the source.
- **m3 (orphan relaxation).** Skipped by the round on P3's evidence (declaration
  already passes; the failure is dispatch-only), and P7 killed the residual
  "tightening" claim outright. Nothing to relax.

## Deliberate limitations (ledger)

1. **Env dispatch is S5's territory and is untouched.** `poly_env` keys are
   post-mangle names (src/check.rs:655-695; poly.rs:15634), exact-match dispatch, and
   the module-blind generated-ctor first-match (terms.rs:1399-1404). C fixes the
   *same-header* cross-pick as a side effect (one mint → one ctor sig); the
   different-header same-name cross-pick (P5) stays and is byte-pinned (S4-11).
2. **The P4 exported-effect private-type wall is recorded, not fixed.** An exported
   word naming the instantiation from a module that does not export the type still
   rejects (`names private type`Option[i64]``, declarations.rs:790; probe `p4z`).
   Pre-existing, unchanged by C.
3. **The mono overload-suffix routing residual is S5-adjacent, recorded only** — mono
   member routing for colliding synthesized names still resolves through
   `poly_env`/first-match rather than the registry the poly path uses.
4. **The m4 remedy-spelling hole is recorded, not fixed.** The ambiguity error's
   documented remedy (`module::member`) cannot reference one's own module's trait —
   `u1::unbox` and `Functor::unbox` are both "unknown word" inside u1
   (`m4-qualified-functor.log`). S5-adjacent UX; the S4-12 golden pins the *located*
   error, not the remedy.
5. **W4's two-ctor half is parked on a future real `* -> *` lib type** (`core::list`)
   — a library dependency (Q3), not a compiler gap. The phase doc's dogfood line
   carries it.
6. **No orphan change, no behavioral orphan golden** (Q2). The q2demo fixture stays
   probe-side evidence; S4-14 is an inspection requirement.
7. **Symbol parity is a phase-1 measurement, not a committed golden.** Pinning
   `gcd`'s full symbol list in-tree is brittle and low-value (no generics); the
   *committed* symbol assertions are S4-8's zero-`sooth_mono_*` pin and S4-10's
   single-mint build, which are the identities the exit clause is about.

## The golden set

New file `tests/phase7b_slice4.rs` (package `p7bs4`), modeled on
`tests/phase7b_slice3.rs`: `single_file_hosted` (:53) for single-file fixtures,
`build_run_keep` (:69) wherever an `nm` assertion accompanies stdout,
`common::call_graph` available (tests/common/mod.rs:352), and per-file `t.write` for
multi-module fixtures per tests/phase7b_slice2.rs:644. Single-file fixtures pull their
own imports (`import: intrinsics * ;` + `import: hosted::show | . | ;` prefix, as the
probe fixtures do). Positive goldens assert stdout and exit code; the identity goldens
add their `nm` clause; error goldens assert the located error's shape — byte-exact
text at implementation time (freeze note above), shape here.

| # | Kind | Name | Fixture | Pin |
| --- | --- | --- | --- | --- |
| 1 | positive | mono dispatch over real Option (S4-4) | `p1_mono_real_option` verbatim | `2\n` |
| 2 | positive | poly shared-bound dispatch over real Option (S4-5) | `p2_poly_real_option` verbatim | `1\n` |
| 3 | positive (migrated) | W3 over real `core::result` (S4-6) | slice2:509, import swap only | `0\n2\n` |
| 4 | positive (migrated) | W4 over real `core::option` (S4-7) | slice2:558, testedit shape | `-1\n3\n` |
| 5 | positive + nm | two modules, one shared mint (S4-8) | `p4_two_modules` | run exits 0; zero `sooth_mono_*` |
| 6 | positive | recursive header mints once (S4-10) | `t6_recursive` | builds, runs, exits 0 |
| 7 | positive (control) | per-trait poly route clean (S4-12a) | `m4_twin_impls` u1 alone | `39\n` |
| 8 | error | twin-impl ambiguity located (S4-12b) | u1+u2 + mono caller | "trait member of both `Functor` and `Functor`" |
| 9 | non-regression | S5 marker unmoved (S4-11) | `p5_same_named_ctors` | byte-identical `type mismatch in mk` error |
| 10 | error (unit) | Q1 re-baseline (S4-3) | src/parser.rs:10653 | `module == 0`, wart-quoted doc |
| 11 | positive | variant tag from importing module (S4-13) | Q5 fixture, new | builds, runs |

The re-baseline (S4-3) is a unit test in src/parser.rs, not a file golden — it is
listed here because it is the wart's only in-repo defender and its rewrite is the
record of the convention change.

## Phased delivery plan

Each phase is independently verifiable: its goldens pass and any touched stage code
carries unit coverage before it is done (CLAUDE.md). Green =
`cargo fmt --check && cargo clippy -- -D warnings && cargo test`. Baseline at HEAD is
green (probe round: 3078 passed / 0 failed across 79 suites; re-confirmed here for
slice1 16/16 and slice2 17/17). Phases 2–4 are test-only and depend only on Phase 1;
they are sequenced because they share `tests/phase7b_slice4.rs` (and Phase 2's harness
is what Phases 3–4 extend), not because of any code dependency.

### Phase 1 — The three-line mint change, the re-baseline, and the parity measurement

S4-1, S4-2, S4-3, S4-9, S4-14.

- **Scope**:
  - Modify `src/parser.rs:6866` (`resolve_type_or_apply` struct arm) and `:6884`
    (enum arm): pass `owner` (bound at :6833) where `self.module` is passed today.
  - Modify `src/check/poly.rs:5946` (`poly_construct_generic` no-fallback arm): key
    on `generics.enums[idx].module` / `generics.structs[idx].module` instead of
    `ctx.module()`.
  - Modify `src/parser.rs:10653` (`parse_qualified_generic_application_from_another_
    module_resolves`): assert `.module == 0` at :10691; rewrite the :10643-10651 doc
    comment to pin the declaring-module mint, quoting the old applying-module sentence
    as the wart's record.
- **Out of scope for this phase (fences a naive implementer might touch)**: the memo
  key shape (ast.rs:645-646), any comparator (poly.rs:8849-8857/:8868/:9019/:9875),
  `instantiation_symbol` (ast.rs:2869), env dispatch (terms.rs:1399-1404),
  declarations.rs (orphan rule), resolve.rs, and every other mint producer — the seven
  declaring-keyed sites are already correct and must not be edited.
- **Entry conditions**: clean tree at `ad136f3`; the S4-4/S4-5 fixtures available in
  some form (the probe sources' text is reproduced in the golden table's fixture
  column; the implementer may re-spell them from this spec alone).
- **Verifiable artifacts**: full suite green (the one expected re-baseline absorbed);
  `cargo fmt --check && cargo clippy -- -D warnings` clean; the m5 battery re-run as a
  scratch harness and **recorded in the commit message**: P1 → `2`, P2 → `1`, P4 builds
  with zero `sooth_mono_*`, P5 byte-identical, q2demo byte-identical (S4-14 evidence),
  T6 single mint, `nm` parity on P6/gcd/P4-family (S4-9), slice1 16/16, slice2 17/17.
- **Parallelism**: SEQUENTIAL — first phase; every later phase verifies against its
  behavior.
- **Relative effort**: S — three changed lines plus one test rewrite; the cost is the
  measurement discipline, not the code.
- **Difficulty**: `hard` — an S1-era identity-convention change with dedup and
  monomorph-symbol implications across every consumer of the minted identity; the
  measured battery is the safety net and must actually run.
- **Open questions / blockers**: None identified.

### Phase 2 — Real-type dispatch goldens and the W3/W4 migration

S4-4, S4-5, S4-6, S4-7.

- **Scope**:
  - Create `tests/phase7b_slice4.rs`, modeled on `tests/phase7b_slice3.rs:53`/:69
    (`single_file_hosted`, `build_run_keep`), package `p7bs4`.
  - Golden #1 (S4-4): `p1_mono_real_option`'s source verbatim; pin `2\n`.
  - Golden #2 (S4-5): `p2_poly_real_option`'s source verbatim; pin `1\n`.
  - Modify `tests/phase7b_slice2.rs:509` (W3): twin declaration → `import: core::
    result * ;`, all else byte-identical, pin `0\n2\n` unchanged; refresh that test's
    doc comment (the twin-wart justification is superseded by S4; quote it as history).
  - Modify `tests/phase7b_slice2.rs:558` (W4): the `candidate-c-testedit.diff` shape
    (real `core::option`, `* -> *` Functor, `twice['F: Functor 'T]`, two Option call
    sites), pin `-1\n3\n` unchanged; doc comment refreshed the same way.
- **Out of scope for this phase**: any src/ change; the other slice2 goldens that
  legitimately keep twins (W2 :405, the S2-8/S2-9 shapes) — only W3/W4 migrate;
  the Q3 two-ctor shape (no real second `* -> *` ctor exists — do not invent one).
- **Entry conditions**: Phase 1 landed (P1/P2 build; W3/W4 twins still green).
- **Verifiable artifacts**: `tests/phase7b_slice4.rs` 2/2 with the exact stdout pins;
  `tests/phase7b_slice2.rs` 17/17 with both migrations in; slice1 16/16;
  full suite green.
- **Parallelism**: SEQUENTIAL after Phase 1 (its goldens are false without the mint
  change). No dependency on Phases 3–4, but shares the new test file with them.
- **Relative effort**: S — the fixtures exist verbatim in the probe record and the
  migration template is the measured test-edit diff.
- **Difficulty**: `standard` — test porting with measured pins; no design freedom.
- **Open questions / blockers**: None identified.

### Phase 3 — Single-mint identity goldens (P4 and T6)

S4-8, S4-10.

- **Scope**:
  - Extend `tests/phase7b_slice4.rs`:
  - Golden #5 (S4-8): port `p4_two_modules` (main + mod_a.sth + mod_b.sth, per-file
    `t.write` per tests/phase7b_slice2.rs:644). Assert build+run success and, via
    `build_run_keep`'s binary, zero symbols matching `sooth_mono_` in `nm` output
    (the m5(b) evidence, in-repo for good).
  - Golden #6 (S4-10): port `t6_recursive` (rec.sth declares
    `type: L['T] | Nil | Cons 'T rest ^L['T] ;` + `export: L ;`; main names `L[i64]`
    across the module boundary). Assert build+run, exits 0. Keep the `^` indirection
    spelling exactly (the direct self-field is the pre-existing infinite-size
    rejection, declarations.rs:1785 — do not "fix" the fixture by inlining the field).
- **Out of scope for this phase**: any src/ change; the p4z exported-effect wall
  (ledger item 2 — recorded, no golden); the P4 order-swap variants (p4x/p4w — the
  order-dependence is *gone* under C, so those variants are meaningless; port only the
  canonical two-module fixture).
- **Entry conditions**: Phase 1 landed (the fixtures build at all).
- **Verifiable artifacts**: both goldens pass under the landed change; as a mutation
  check, reverting Phase 1's poly.rs hunk alone must make golden #6 fail with the
  two-overload error (and reverting the parser hunk must break golden #5) — run once
  and record in the commit message, mirroring m5-t6-pre/post.
- **Parallelism**: SEQUENTIAL after Phase 1; sequenced after Phase 2 only because it
  extends the same new test file.
- **Relative effort**: M — two multi-file fixture ports with binary-level assertions
  and a required mutation check; more moving parts than Phase 2, still bounded.
- **Difficulty**: `standard` — fixture porting plus nm inspection; no concurrency, no
  migration.
- **Open questions / blockers**: None identified.

### Phase 4 — Fences: non-regression, located ambiguity, and the Q5 tag

S4-11, S4-12, S4-13.

- **Scope**:
  - Extend `tests/phase7b_slice4.rs`:
  - Golden #9 (S4-11): port `p5_same_named_ctors` (f.sth + a.sth + b.sth + main.sth);
    assert the exact `type mismatch in`mk`` error and its line/col shape
    (byte-pinned at implementation per the freeze note).
  - Goldens #7/#8 (S4-12): the m4 family — u1.sth/u2.sth (each its own
    `Functor['F: * -> *]` with member `unbox` and observably different impl bodies)
    - a main that (a) for the control, drives u1 alone through the poly route
    (`run` → `go['F: Functor 'T]` → `unbox`), pin `39\n`; (b) for the error, names
    `Option[i64]` itself and calls bare `unbox`, asserting the located
    `mono_ambiguous_member_error` shape. Spell `run`'s body with the `go` route, not
    the stale qualified spelling (Correction note above).
  - Golden #11 (S4-13): a single-file fixture wildcard-importing `core::option`, one
    word whose quotation annotation carries the leading variant slot
    (`[ ( Some ) … ]`), asserting build+run (the tag parsed as a `VariantTag`, not
    `unknown type`).
  - Phase-exit growth-signal re-run (CLAUDE.md) against `src/parser.rs` and
    `src/check/poly.rs` as they then stand (both gained only comments/one arm each in
    this slice — record the verdict in the commit message).
- **Out of scope for this phase**: any src/ change; the m4 remedy spelling (ledger
  item 4 — do not add module-qualified trait-member spelling); any orphan or env
  dispatch behavior.
- **Entry conditions**: Phases 1–3 landed (the ambiguity error is only reachable once
  both impls share the widened identity; the S5 marker pin presumes the landed tree).
- **Verifiable artifacts**: goldens #7–#11 pass; full suite green; growth-signal
  verdict recorded.
- **Parallelism**: SEQUENTIAL after Phases 1–3 (same test file; the S4-12 error shape
  presumes the widened identity from Phase 1).
- **Relative effort**: M — three fixture families, one of them two-module with an
  error-shape pin; plus the phase-exit bookkeeping.
- **Difficulty**: `standard` — porting and pinning; the ambiguity fixture needs care
  to route the control through the poly word, but no design freedom.
- **Open questions / blockers**: None identified.

### Parallelism summary

- Phase 1 is the sole code change; everything else verifies against it.
- Phases 2, 3, 4 are content-independent of each other and depend only on Phase 1;
  they are strictly ordered solely because all three write `tests/phase7b_slice4.rs`
  (and Phase 4's ambiguity pin reads best on the fully populated file). No phase can
  usefully run concurrently with another.

### Effort summary

Phase 1 S + Phase 2 S + Phase 3 M + Phase 4 M = roughly two weeks, of which one day is
code and the rest is converting the measured battery into permanent, pinned goldens.

## Anchor status

Re-verified against HEAD `ad136f3` while writing this spec.

| Anchor | At |
| --- | --- |
| `resolve_type_or_apply` / struct mint / enum mint / `owner` binding | `parser.rs:6814` / `:6866` / `:6884` / `:6833` |
| `poly_generic_header` / `bare_generic_owner` / `generic_is_declared` | `parser.rs:6897` / `:6928` / `:6937` |
| All-concrete folds (declaring-keyed, untouched) | `parser.rs:5500`/`:5502`, `:7747`/`:7750` |
| Impl-target pattern's declaring record | `parser.rs:3970` (`poly_generic_header` call → `:3975`) |
| `poly_construct_generic` / no-fallback arm / construction mint / symbolic record | `poly.rs:5885` / `:5946` / `:6039`/`:6041` / `:6045-6050` |
| `substitute_generic_field` (declaring-keyed, untouched) | `ast.rs:872` (mints `:940`/`:942`, `:969`/`:977`) |
| `apply_subst` (declaring-keyed, untouched) | `poly.rs:10085` (mints `:10243`/`:10245`, `:10303`, `:10387`/`:10389`) |
| Memo keys / lookups / reverse lookups / key pushes | `ast.rs:645-646` / `:1077`/`:1091` / `:1118`/`:1128` / `:1334-1335`/`:1385-1386` |
| Comparators: CtorImage identity / `found_module != *module` ×3 | `poly.rs:8849-8857` / `:8868` (`match_impl_target_rec`, fn `:8721`), `:9019` (`collect_positions`, fn `:8953`), `:9875` (`unify_poly_input`, fn `:9634`) |
| `select_most_specific` | `poly.rs:7948` |
| Dispatch funnel: `find_bound_impl` / program-wide impl registry | `poly.rs:8110` / `driver.rs:769` (`impls.extend`) |
| `resolve_user_bound` / `resolve_mono_member_call` / viable loop / ambiguity fire | `poly.rs:8227` / `:2146` / `:2216-2232` / `:2256` |
| Errors: `mono_member_no_dispatch_error` / `mono_ambiguous_member_error` / `mono_member_unroutable_error` / `unsatisfied_user_bound_error` / `ambiguity_error` | `poly.rs:2404` / `:2450` / `:2427` / `:8615` (text `:8643-8664`) / `:7988` (text `:8000`) |
| Mangling: `instantiation_symbol` / `mangle` / minted-decl loops | `ast.rs:2869` (render `:2885`) / `resolve.rs:36` / `resolve.rs:791-797` |
| Lowering lockstep: `subst_polytype` lookup+expect arms | `ir/driver.rs:567` (arms `:686-695`, `:719-727`) |
| Orphan: `impl_target_module` / gate / duplicate scan / error text | `declarations.rs:491` (concrete `:493-498`, Generic `:499-502`) / `:578` / `:539-560` / `:648-672` |
| Registry layout: `GenericTypes::with_bases` / pre-pass / flush / single-module check | `driver.rs:671` / `:679-689` / `:784-786` / `:888` (`check::check`) |
| `GenericEnumDecl.module` | `ast.rs:601` |
| Env dispatch (untouched): `poly_env` / post-mangle keys / generated-ctor first-match | `check.rs:655` (pushes `:672`, `:690`) / `poly.rs:15634` / `terms.rs:1399-1404` (fn `:1406`) |
| Variant-tag visibility: `variant_name_is_visible` / `module_declares_variant` / leading slot / negative pins | `parser.rs:6021` / `:6047-6060` / `:5992` (call `:5944`) / `:9036`, `:9088` |
| Q1 re-baseline test / wart doc / failing assert | `parser.rs:10653` / `:10643-10651` / `:10691` |
| W3/W4 goldens / hosted harness / multi-module fixture pattern | `tests/phase7b_slice2.rs:509` / `:558` / `:92` / `:644` |
| S3 test patterns: `single_file_hosted` / `build_run_keep` / `call_graph` | `tests/phase7b_slice3.rs:53` / `:69` / `tests/common/mod.rs:352` |
| Exported-effect private-type wall text / infinite-size rejection | `declarations.rs:790` / `:1785` |
| lib generic headers (exactly two) | `lib/core/option.sth:1`, `lib/core/result.sth:1` |

## Open questions & risks

- [x] ~~Q2 — does C tighten the orphan rule for pinned lib targets?~~ Resolved by P7:
      byte-identical rejection today and under C; no ruling, no golden (S4-14).
- [x] ~~Q3 — can W4's two-ctor half migrate?~~ No: library-blocked on a real `* -> *`
      type (`core::list`); parked, carried by the phase doc's dogfood line.
- [x] ~~Q1/Q4/Q5 — re-baseline text, golden set, tag fixture.~~ Ruled above (S4-3,
      golden table, S4-13).
- [ ] None outstanding that block the plan.

| Risk | Likelihood | Mitigation |
| --- | --- | --- |
| The mint change shifts identity for cross-module programs in a way the probes did not exercise | Low | The m5 battery plus the full suite (3078 tests) measured exactly this delta; Phase 1 re-runs the battery and records it; symbol parity is the tripwire |
| `/tmp/p7bs4-probes/` (candidate-c.diff, fixtures, logs) is gone by implementation time | Med | This spec reproduces the three hunks, every fixture's shape, and every pin; the /tmp tree is corroborating evidence only |
| Anchor drift between this spec and the implementation phases | Low | Every anchor pairs path:line with a symbol name; the S4-1 sites are two functions findable by name |
| A naive implementer "fixes" the P5/P4z walls or the m4 remedy while porting goldens | Med | Each phase's out-of-scope list names the exact fences; the ledger records why they stand |
| The Q5 fixture exposes a real tag-visibility gap | Low | S4-13 expects no code change; a failure is a finding for the supervisor, not a pre-approved fix |

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "Declaring-module mint keying and re-baseline", "effort": "S", "difficulty": "hard" },
    { "phase": 2, "focus": "Real-type dispatch goldens and W3/W4 migration", "effort": "S", "difficulty": "standard" },
    { "phase": 3, "focus": "Single-mint identity goldens (P4, T6)", "effort": "M", "difficulty": "standard" },
    { "phase": 4, "focus": "Fence goldens: S5 marker, twin-impl ambiguity, variant tag", "effort": "M", "difficulty": "standard" }
  ]
}
```
