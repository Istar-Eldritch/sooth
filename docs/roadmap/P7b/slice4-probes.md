# P7b.S4 probe round — verbatim log (run 260902)

Recon round for P7b.S4 scoping, run 260902 against the clean tree (worktree `p7b-s4`,
HEAD `ad136f3`, the merge of `p7b-s3` into `main`). Four workers: a read-only mapper
(`/tmp/p7bs4-probes/mapper.md`), a paper-test designer (`/tmp/p7bs4-probes/paper.md`),
a live-probe runner (`/tmp/p7bs4-probes/probes.md`), a mutation runner
(`/tmp/p7bs4-probes/mutations.md`), plus a follow-up candidate-C measurement
(`/tmp/p7bs4-probes/mutations-c.md`).
Fixtures are compile/run fixture dirs under `/tmp/p7bs4-probes/` with per-probe
captures under `/tmp/p7bs4-probes/logs/`; the repo was untouched throughout
(`git status --porcelain` empty after every revert, re-verified at round end).
Probe sources carry a local `sooth.pkg` (`package: p7bs4 ; layer: hosted ;` with core
and hosted path deps) and the two-line prelude, matching
`tests/phase7b_slice2.rs`'s `single_file_hosted` helper, so fixtures are golden-ready
as written.

Baseline: `cargo test --no-fail-fast` at HEAD is **green — 3078 passed, 0 failed**
across 79 suites; `phase7b_slice1` 16/16, `phase7b_slice2` 17/17. Every rejection
below is the standing S4 wart, not baseline breakage.

Anchor drift since the S2 spec's table: `match_impl_target_rec` now at
`src/check/poly.rs:8721` (was :8265); ctor-image mangling now at `src/ast.rs:2885`
(was ~:2865). All other anchors held.

## Summary table

| Probe / experiment | Fixture or patch | Outcome |
| --- | --- | --- |
| P0 W3/W4 twin control | `cargo test --test phase7b_slice2` | 17/17 green — the migration target behavior is pinned on twins |
| P1 mono, real `core::option` | p1_mono_real_option | **rejected** at dispatch: `no impl: in this program dispatches on these operands` |
| P2 poly bound, real Option | p2_poly_real_option | **rejected**: `cannot instantiate 'F` … `does not satisfy Functor` |
| P3 gate location | code-read + P1/P2 spans | impl **declaration passes**; failure is dispatch-only (orphan rule is not the blocker) |
| P4 two modules, same `Option[i64]` | p4_two_modules | **cannot build today**: `type mismatch in mk` — order-dependent ctor cross-pick (S5-adjacent wall) |
| P4 variants | p4x/p4w/p4y/p4z/p4m/p4u/p4v | which-module + order-swap confirm first-mint-wins; `p4z`: exported effects cannot name the instantiation; symbol baselines: one shared identity → one `sooth_mono_*`, distinct identities → distinct |
| P5 S5-boundary marker | p5_same_named_ctors | `type mismatch in mk (line 7)` — same-named different-header cross-pick; the marker S4 must not move |
| P6 twin control | p6_twin_control | local `Opt` twin **builds and prints `2`** — module identity is the only delta |
| P7 orphan gate, pinned lib target | q2demo | `impl: Show for Option[i64]` (core-side trait, user module) **already orphan-rejected today**; error **byte-identical under C** — the "tightening" claim is dead |
| m1 = naive A (memo key drops module) | ast.rs key/lookup edits | P1/P2 **byte-identical baseline errors**; P4 two-module fixture **now builds** (one mint, first-minter wins); full suite 3078/0 |
| m2 narrow (blind `match_impl_target_rec` only) | poly.rs 2 compares | both errors **move deeper**, none fixed; suite 3078/0 |
| m2b = m2 + blind `collect_positions`/`unify_poly_input` | poly.rs 4 compares | P2 **passes** (prints `1`); P1 fails with `` expected `Option[i64]`, found `Option[i64]` `` — same rendering, two distinct mints; suite 3078/0 |
| m3 orphan relaxation | — | SKIPPED: P3 showed declaration already passes |
| m4 twin-impl hazard (under m2b) | m4_twin_impls | located `mono_ambiguous_member_error`, **no silent first-win**; the documented remedy (module qualifier) is unspellable for one's own trait |
| **m5 = candidate C** (declaring-module keying) | 3-site patch (`candidate-c.diff`) | **P1 PASS** (prints `2`), **P2 PASS** (prints `1`), P4 builds+runs (one truthful mint, zero `sooth_mono_*`), P5 byte-identical, slice1/2 green, full suite 3077/1 (the one expected re-baseline), W3/W4 migrate to real types, symbols stable, T6 recursive duplicate collapses |

## Verbatim captures — live probes

### P1 — mono caller over the real lib Option

Fixture: `import: core::option * ;`, user-declared `trait: Functor['F: * -> *]` with
member `map`, `impl: Functor for Option` in the user module, `main` names
`Option[i64]` via `Some`, mono caller with the W2 explicit-instantiation spelling.

```text
error: `map` in `main` (line 12, col 33) is a trait member of Functor, but no `impl:` in this program dispatches on these operands
  the operand types here are `Option[i64] cstr`; declare an impl of one of those traits for the operand's type, or import a word that claims this name
```

Emitted by `mono_member_no_dispatch_error` (`src/check/poly.rs:2409`, text `:2413`)
from the S2-16 mono member path (`resolve_mono_member_call`, `src/check/poly.rs:2146`).

### P2 — poly caller through a Functor bound over the real Option

```text
error: cannot instantiate `'F` of `twice` with `Option` in `main` (line 16, col 33)
  `Option` does not satisfy `Functor`: no `( 'F['T] [ 'T -- 'U ] -- 'F['U] )` found
```

Message built at `src/check/poly.rs:8645-8653` (`unsatisfied_user_bound_error`).

### P4 — the two-module wall (pre-existing, order-dependent)

```text
error: type mismatch in `mk` (line 3)
  body leaves `Option[i64]` where the declaration requires `Option[i64]`
  note: declared ( i64 -- Option[i64] )
```

Variants: shifting the marker comment moves the error to the other module
(`p4x`); swapping the import order moves it back (`p4w`); each module alone builds
(`p4y`). Mechanism: the FIRST-processed module's ctor-word mint wins the module-blind
generated-ctor env dispatch (`src/check/terms.rs:1401`), so the second module's `Some`
call produces the first module's identity — same rendering, different type. Corollary
wall (`p4z_main_wraps`, main wraps and exports a consumer):

```text
error: exported word `use` (line 4, col 3) names private type `Option[i64]`, which is not exported
  export `Option[i64]` too, or remove it from the effect
```

Symbol baselines (`nm`): a second module merely naming `Option[i64]` mints a registry
entry and **zero symbols** (`p4v_dormant`); a shared poly `id['T]` called at
`Option[i64]` from two call sites mints exactly one
`sooth_mono_id__m2__t0_Option_i64_` (`p4m_poly_id`); two distinct instantiations mint
distinct symbols with the type rendering embedded (`p4u_two_types`).

### P5 — the S5-boundary marker (must not move)

Two user modules each declare their own `Widget['T]` (same name, identical payload
shape), an impl each, `: mk ( i64 -- Widget[i64] ) Widget ;` in both:

```text
error: type mismatch in `mk` (line 7)
  body leaves `Widget[i64]` where the declaration requires `Widget[i64]`
  note: declared ( i64 -- Widget[i64] )
```

Same rendering twice — the env-dispatch first-match cross-pick. Under m5 this error
is **byte-identical** (diffed against this record): S4 does not move it.

### P6 — twin control

Byte-for-byte the P1 shape with a fixture-local `type: Opt['T] | None | Some 'T ;`:
builds and prints `2`. P1 fails, P6 succeeds, everything else identical — module
identity is the only delta.

### P7 — the orphan gate on a pinned lib target (kills the "tightening" claim)

Fixture `q2demo/`: a user module wildcard-imports `core::option` and `core::show`
(`Show` is core-side) and declares a concrete-pinned impl over the lib generic:

```sth
impl: Show for Option[i64]
  : show drop drop ; \ consume the value, then the buffer borrow
;
```

Today, on the clean tree:

```text
error: `impl: Show for Option[i64]` at line 8, col 1 must live in the module declaring `Show` or the module declaring `Option[i64]`
```

Under the m5 C patch: **byte-identical** (logs `p7-orphan-today.log` /
`p7-orphan-under-C.log`, diff-confirmed). The mapper's §5-C claim that a
concrete-pinned lib target "currently homes at the naming module" was an
over-inference contradicted by its own §1: the pin mints through the all-concrete
fold (parser.rs:5500/:5502) on the pattern's recorded (declaring) owner, so
`impl_target_module`'s concrete arms already read declaring-keyed mints for
impl-target pins. The orphan rule is untouched by C; no ruling and no behavioral
golden is owed. (P3's verdict stands, now verified on the pinned spelling too.)

## Verbatim captures — mutation round

### m2b — the structural ceiling of matching-blind (candidate B)

With all four dispatch comparisons blinded (CtorImage identity :8842-8852,
`found_module != *module` at :8868, `collect_positions` :9013ff, `unify_poly_input`
:9866ff), P2 passes (prints `1`) but P1's mono caller now fails at:

```text
error: `showopt` expected `Option[i64]`, found `Option[i64]`
```

Same rendering, two distinct mints (naming-module operand vs declaring-module member
output) — a plain `Type` equality outside every PolyType comparator. **B can never
fix the mono path.** m2 (narrow, two compares) moves both errors deeper instead:
`` `map;Functor;0;Option['T0]` expected `Option['ctor0]`, found `Option[i64]` `` and
`ctor_image_member_site_mismatch_error`. m1 (naive A) leaves P1/P2 byte-identical
(recovery's first-minter module still loses the U-vs-core compare at poly.rs:8868)
while fixing P4's build — each mechanism fixes exactly the family the other misses.

### m4 — twin impls: what ambiguity actually looks like

u1 and u2 each declare their own `Functor` + `impl: Functor for Option` with
observably different members. The paper's predicted impl-pattern `ambiguity_error`
(poly.rs:8000) is **unreachable** (orphan rule confines a trait's impls to its
declaring module, where the per-module duplicate scan rejects identical patterns;
separate same-named traits never compete — `find_bound_impl` filters by TraitId).
What fires at mono call sites:

```text
error: `unbox` in `main` (…) is a trait member of both `Functor` and `Functor`
  … qualify with the claiming trait's module …
```

No silent first-win. Surprise: the remedy is unspellable for the module's own trait —
`u1::unbox` and `Functor::unbox` are both "unknown word" inside u1. Poly-bound
dispatch is per-trait clean. Two-module builds are blocked one level earlier by the
P4/P5 wall, unchanged.

## m5 — candidate C (declaring-module keying), the full battery

Patch (`candidate-c.diff`, 3 hunks; applies cleanly to `ad136f3`; fmt/clippy-clean):

- `src/parser.rs:6866`/`:6884` — `resolve_type_or_apply` mints with `owner` (the
  header's declaring module, already computed at :6827-6833) instead of `self.module`.
- `src/check/poly.rs:5946` — `poly_construct_generic`'s no-fallback arm keys on the
  header's declaring module (`generics.enums[idx].module` / `structs[idx].module`)
  instead of `ctx.module()`, which also fixes the symbolic `PolyType::Generic` record
  and the :6038/:6040 construction mint it feeds.

| item | verdict |
| --- | --- |
| (a) P1 mono real Option | **PASS** — builds (was `no impl: … dispatches`), prints `2` |
| (a) P2 poly real Option | **PASS** — builds (was `does not satisfy Functor`), prints `1` |
| (b) P4 two-module | **PASS** — builds+runs; nm: user words only, zero `sooth_mono_*` — one shared truthful mint |
| (c) P5 S5 marker | **PASS** — error byte-identical to the wave-1 record |
| (d) slice1/slice2 | **PASS** — 16/16, 17/17 (W3/W4 twins still green) |
| (e) full suite | **3077 passed / 1 failed** — sole failure `parse_qualified_generic_application_from_another_module_resolves` (`src/parser.rs:10691`), whose doc comment pins the removed wart verbatim ("stamped with the *applying* module, not the declaring one"); asserts `module == 1` (applying), now `0` (declaring). Expected one-line re-baseline on landing |
| (f) W3/W4 rehearsal | **PASS** — W3 migrates byte-for-byte (`import: core::result * ;`, `"0\n2\n"` pin unchanged); W4 migrates to real `core::option` with the shared-bound `twice` (`"-1\n3\n"` pin unchanged); slice2 17/17 with both migrated |
| (g) symbol stability | **PASS** — P6 twin, `gcd`, and all five buildable P4-family fixtures nm-identical pre/post; local headers (naming == declaring) do not move |
| (h) T6 recursive collapse | **CONFIRMED** — `t6_recursive/` (`type: L['T] \| Nil \| Cons 'T rest ^L['T] ;` declared in one module, named from another): PRE — build fails with **two identical `Cons`overloads** (`candidate: i64 ^L[i64]` twice), the outer naming mint + inner declaring mint; POST — builds, runs, one mint |

(f) caveat: W4's two-ctor half cannot migrate for **library** reasons — lib/core has
exactly two generic headers, `Option['T]` (`* -> *`) and `Result['T 'E]`
(`* -> * -> *`), so a `* -> *` shared bound has no second real ctor to dispatch to.
Recorded as depending on a future real `* -> *` type (e.g. `core::list`, per the
phase doc's dogfood line).

Working-tree integrity: `git diff` verified to contain exactly the three C hunks
before the patched build and immediately before the final revert; test edit reverted;
source reverted; `git status --porcelain` empty at `ad136f3`; nothing committed.

## Artifacts

- Diffs: `/tmp/p7bs4-probes/candidate-c.diff`, `candidate-c-testedit.diff` (reverted),
  `logs/m1.diff`, `logs/m2-narrow.diff`, `logs/m2b-extended.diff`.
- Reports: `mapper.md`, `paper.md`, `probes.md`, `mutations.md`, `mutations-c.md`.
- Logs: `/tmp/p7bs4-probes/logs/` (p-baseline, p0–p7 + variants, m4-*, m5-*).
- Fixtures: `/tmp/p7bs4-probes/` (p1…p6 dirs, p4 variants, m4_twin_impls, t6_recursive, q2demo).
