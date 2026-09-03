# P7b.S4 brief — declaring-module identity for generic instantiations (recon round 260902)

Scope input for the S4 spec. Produced by a recon round against the clean tree (worktree
`p7b-s4`, HEAD `ad136f3`, the merge of P7b.S3 into `main`): a read-only module-identity
map, nine live compile/run probes plus seven supporting variants, a four-experiment
reverted mutation round (m1, m2, m2b, m4; m3 skipped on evidence), and a follow-up
candidate-C measurement (m5, a nine-item battery), all under `/tmp/p7bs4-probes/`
(verbatim log: [slice4-probes.md](./slice4-probes.md)). Repo untouched throughout
(`git status --porcelain` empty after every revert); probe fixtures are disposable.
Baseline `cargo test --no-fail-fast` at HEAD is green (3078 passed / 0 failed, 79
suites), so every rejection below is the standing wart, not baseline breakage.
Diagnostic texts below are drafts — they freeze at implementation, pinned by the
goldens (S12 precedent).

S4's exit criteria (from the [phase doc](../P7b-higher-kinded-types.md)): `impl:
Functor for Option` in a user module dispatches for an `Option[i64]` operand named in
that module; S2's W3/W4 goldens migrate from fixture twins to the real lib types
unchanged in behavior; no duplicate monomorphs are introduced by the widened identity.
S3's dogfood (real `Option`/`Result`/`List` through a shared bound) depends on this
slice or on twin workarounds.

## What the round established

The headline is three-part, and the third part is the decision.

**First: the roadmap's framing is precise but incomplete — and the fix is narrower
than the scope note feared.** Of nine non-test mint producers of
`instantiate_struct`/`instantiate_enum`, only **two** key on the *naming* module:
`resolve_type_or_apply` (src/parser.rs:6866/:6884 — the declaring `owner` is already
computed at :6827-6833 and used only for the lookup) and `poly_construct_generic`'s
no-fallback arm (src/check/poly.rs:5946, which also feeds the :6038/:6040 construction
mint and the symbolic `PolyType::Generic` record). Every other producer — signature
mentions, field applications, `apply_subst`, substitution of captured generics —
already passes the **declaring** module. The impl-target pattern records declaring
(parser.rs:3970→:3975). So S4 is a two-producer convention change: **no new registry
entry type, no key-shape change, no new call site** — memo keys stay `(idx, module,
args, lens)` 4-tuples, and the feared S3o-pattern (new registry entry kind) does not
materialize. Soundness falls out of the registry layout: one `GenericTypes` per
program (driver.rs:671, headers appended in the driver pre-pass :679-689), so `idx`
is globally unique and `(idx, args, lens)` identifies a header without any module.

**Second: the orphan rule is not the blocker, and matching-blind has a structural
ceiling.** P3 pins the gate location: a user-module `impl: Functor for Option` (trait
declared in the same user module) **passes declaration checks today**
(`check_impl_decls`' short-circuit `impl_module == trait_decl_module`,
declarations.rs:578) and fails only at dispatch. The mutation round then measured the
two roadmap-mentioned alternatives separately: naive **A** (drop `module` from the
memo keys) fixes the identity-collision family — P4's two-module fixture builds on one
mint — but dispatch stays module-strict (P1/P2 byte-identical baseline errors) and the
minted decl's `.module` becomes a first-minter lie. **B** (blind the four dispatch
comparisons) fixes poly bound dispatch — P2 prints `1` — but mono dies at
`` `showopt` expected `Option[i64]`, found `Option[i64]` ``: same rendering, two
distinct minted handles, a plain `Type` equality **outside every PolyType
comparator**. B can never fix the mono path. Each mechanism fixes exactly the family
the other misses.

**Third: candidate C measured — it qualifies as the slice's primary design.** m5
applied the 3-site patch (`/tmp/p7bs4-probes/candidate-c.diff`; the two naming-module
producers read the header's declaring module instead) and ran the full battery: P1
prints `2`, P2 prints `1`, P4 builds+runs on **one truthful shared mint** (zero
`sooth_mono_*`), P5's marker is byte-identical, slice1/slice2 stay 16/16 and 17/17,
W3/W4 migrate to real `core::option`/`core::result` with their output pins unchanged
(`0\n2\n` / `-1\n3\n`), `nm` symbol sets are identical on every unchanged program
(local headers have naming == declaring), and the latent recursive duplicate collapses
(T6: a self-referential generic header named from another module mints **two**
identical `Cons` overloads today — outer naming mint + inner declaring mint — and one
mint under C). The full cost is exactly one unit-test re-baseline
(`parse_qualified_generic_application_from_another_module_resolves`,
src/parser.rs:10691, whose doc comment pins the removed wart verbatim: "stamped with
the *applying* module, not the declaring one") — and, per probe P7, nothing else:
the orphan gate is byte-identical under C (Q2). Patch is fmt/clippy-clean
and applies cleanly to `ad136f3`.

| # | Finding | Probes |
| --- | --- | --- |
| F1 | **Only two mint producers are naming-keyed; `owner` is already in scope at both.** The other seven callers pass declaring modules; signature mentions already mint declaring, so the wart is ctor calls without an output fallback plus `resolve_type_or_apply`'s concrete readers. | mapper §1 |
| F2 | **The orphan rule already admits the exit spelling.** `impl_target_module` (declarations.rs:491) is a registry-home annotation consumed once by the gate (:576-586); a trait-module impl short-circuits at :578. A core-trait-over-lib-ctor impl from a user module is orphan-rejected at declaration today and stays so under C (P7: byte-identical error, both today and patched) — S4 enables user-trait impls over lib ctors, the dogfood shape. | P3, P7 |
| F3 | **B's ceiling is a same-rendering distinct-mint `Type` equality** outside all PolyType comparators — proof that matching alone cannot deliver the mono half of the exit criterion. | m2, m2b |
| F4 | **Naive A's dedup is real but provenance-lying**: one mint, first-minter wins, `.module` wrong for the second module, dispatch still strict. | m1 |
| F5 | **C meets every measured exit criterion with zero comparator edits**; keys, lookups, reverse-lookups, lowering lockstep (`driver.rs` `expect`s check-time keying) all untouched. | m5 |
| F6 | **The two-module wall is pre-existing and order-dependent** (first-processed module's ctor mint wins module-blind env dispatch, terms.rs:1401; exported effects cannot even name the instantiation — "names private type `Option[i64]`"). C fixes the same-header case as a side effect and leaves the different-header cross-pick byte-identical: the S4/S5 boundary is now precisely observable. | P4 (+variants), P5, m5(b)(c) |
| F7 | **The area is unobserved by the suite.** Zero new full-suite failures under all four patches; no test pins a `c{}m{}` symbol; the wart's only in-repo defender is one unit test's doc-pinned applying-module assertion. S4 must *add* the goldens for the new behavior. | m1/m2/m2b/m5(e), mapper §2.8 |
| F8 | **Twin-impl ambiguity is located, not silent — but its remedy is unspellable.** The impl-pattern `ambiguity_error` (poly.rs:8000) is unreachable (orphan confinement + per-module duplicate scan + TraitId filtering); what fires is `mono_ambiguous_member_error` at mono call sites, and its documented module-qualifier remedy cannot reference one's own module's trait (`u1::unbox`, `Functor::unbox` both "unknown word"). | m4 |

Positive controls: P0 (W3/W4 twin goldens) and P6 (twin twin of P1, prints `2`) hold
throughout; module identity is the isolated delta.

## Machinery map (verified anchors, this tree)

All anchors re-derived on HEAD `ad136f3`; drift from the S2 spec's table noted.

- **Mint producers (9 non-test).** Naming-keyed: `resolve_type_or_apply`
  (parser.rs:6866/:6884; `owner` computed :6827-6833) and `poly_construct_generic`'s
  no-fallback arm (poly.rs:5946 → mint :6038/:6040). Declaring-keyed: the
  all-concrete folds parser.rs:5500/:5502 and :7747/:7750 (via
  `poly_generic_header`'s `owner`, parser.rs:6897; `bare_generic_owner` :6940;
  wildcard imports desugar to selective entries, driver.rs:556-576);
  `substitute_generic_field` ast.rs:940/:942/:969/:977; `apply_subst`
  poly.rs:10243/:10245/:10303/:10387/:10389. Impl-target patterns record declaring at
  parser.rs:3970→:3975.
- **Identity surfaces.** Memo keys `(idx, module, args, lens)`: ast.rs:630-647,
  pushes :1334/:1385, lookups :1077/:1091, reverse :1118-1136. Comparators:
  CtorImage identity `pattern_id == gid` poly.rs:8842-8852; `found_module != *module`
  at :8868 (`match_impl_target_rec`, fn :8721 — moved from S2's :8265), :9019
  (`collect_positions`, feeding `select_most_specific` :7948), :9875
  (`unify_poly_input`). Dispatch funnel: `find_bound_impl` :8110 (program-wide,
  driver.rs:769, no visibility filter) ← `resolve_user_bound` :8227 (poly bounds) and
  `resolve_mono_member_call` :2146 (mono members). Errors:
  `mono_member_no_dispatch_error` poly.rs:2409/:2413;
  `unsatisfied_user_bound_error` :8643-8664.
- **Mangling.** `instantiation_symbol` ast.rs:2876/:2885 renders `c{idx}m{module}_…`
  from the CtorImage's gid (moved from ~:2865); minted decl names get `__m{naming}`
  via resolve.rs:803-807. No test pins any `c{}m{}` symbol.
- **Lowering lockstep.** `subst_polytype` driver.rs:691-697/:719-721 looks up by the
  same key and `expect`s the check-time mint — C keeps keys and both sides declaring,
  so lockstep is free.
- **Orphan rule.** `impl_target_module` declarations.rs:491 — Generic arm returns the
  pattern's declaring module (:497-510); concrete arms read the **minted** decl's
  module (:493-496) — for impl-target pins those mints are already declaring-keyed
  (P7), so no home shifts under C. Gate :576-586; text :648-672.
- **Registry layout.** One `GenericTypes` per program (driver.rs:671, pre-pass
  :679-689) ⇒ `idx` globally unique. Duplicate-impl scan per-module
  (declarations.rs:525-560) over a program-wide registry.
- **Env dispatch — S5's territory, untouched.** `poly_env` keys are post-mangle names
  (check.rs:655-695; poly.rs:15634), exact-match :5778,
  `mono_member_unroutable_error` :2427, generated-ctor first-match terms.rs:1401.
  None read the memo key or the pattern's module.

## Open questions and scope recommendation

**The design question is settled by measurement, not argument.** The spec should
mandate **candidate C** (declaring-module keying at the two naming producers;
`candidate-c.diff` is the reference shape) with B recorded as the fallback and its
ceiling (F3) quoted. A is not standalone: naive A fails the exit criterion outright,
and A-with-declaring-recovery converges with C observably while churning key shapes,
lookup signatures, and every tuple consumer.

**Q1 — the one expected re-baseline.** `parse_qualified_generic_application_from_another_module_resolves`
(parser.rs:10691) asserts the applying-module mint and its doc comment pins the wart
verbatim. The spec should rewrite both to pin the declaring-module mint, quoting the
old doc as the wart's record — the same move as S2's fence-text baseline.

**Q2 — ~~the orphan tightening~~ resolved by probe P7: no ruling needed.** The round's
claim that a concrete-pinned lib target homes at the *naming* module (mapper §5-C) was
an over-inference — the mapper's own §1 records that the pin mints through the
all-concrete fold on the pattern's recorded (declaring) owner, and the live probe
sides with §1. Verified: `impl: Show for Option[i64]` in a user module with a
core-side `Show` is orphan-rejected at declaration **today**, and the error is
**byte-identical** under C (fixture `q2demo`; logs `p7-orphan-today.log` /
`p7-orphan-under-C.log`, diff-confirmed). The concrete arms of `impl_target_module`
(declarations.rs:493-496) already read declaring-keyed mints for impl-target pins.
The orphan rule is untouched by C; no behavioral golden is owed.

**Q3 — W4's two-ctor half is library-blocked, not compiler-blocked.** lib/core has
exactly two generic headers (`Option['T]`, `* -> *`; `Result['T 'E]`,
`* -> * -> *`), so a `* -> *` shared bound has no second real ctor to dispatch to.
The migrated W4 golden covers real `Option` through the shared bound (`"-1\n3\n"`
pin); the two-impl half depends on a future real `* -> *` type (e.g. `core::list`) —
the same dependency the phase doc's dogfood line already carries.

**Q4 — golden set.** The suite observes none of this today (F7), so the spec's
goldens are the deliverable: real-type mono dispatch (P1 → `2`) and poly dispatch
(P2 → `1`) — the migrated W3/W4; the two-module shared-mint build (P4); the P5 marker
as a byte-pinned non-regression; a T6 recursive single-mint fixture (`^` indirection
mandatory — the direct self-field is the pre-existing infinite-size error); m4's
`mono_ambiguous_member_error` as a located-error golden; the Q1 re-baseline.

**Q5 — probe gap to close in the spec's test list.** The mapper flagged
variant-annotation-tag visibility (parser.rs:6047-6060) as flowing through
declaring-module name tables; m5's fixtures exercised the ctor/eliminator env paths
(they build under C) but not a leading-variant-slot tag spelled from a non-declaring
module. One fixture should pin it.

**Scope fences to keep.** Orphan-rule mechanism untouched (F2); env dispatch
untouched (P5 byte-identical); m4's remedy-spelling hole recorded, not fixed
(S5-adjacent UX); the P4 exported-effect private-type wall recorded, not fixed; S5's
residuals (mono overload-suffix routing, same-named ctor env dispatch) untouched; no
new registry entry type, key shape, or call site — the S1-era convention change lands
as three lines in two files.
