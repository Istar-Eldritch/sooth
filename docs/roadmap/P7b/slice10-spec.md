# P7b.S10 — header-level export ambiguity for the third-module bare caller

> Implemented reference. Companion frozen docs: [slice10-brief](./slice10-brief.md),
> [slice10-paper-tests](./slice10-paper-tests.md), [slice10-probes](./slice10-probes.md).
> Sits beside [slice9-spec](./slice9-spec.md) (R1.1a / R2.4 / R-NFR context).
> Shipped in `87284cb` (check-stage error + goldens/units) and `465e914`
> (roadmap: S9 residual retired, S10 ∥ S6 entry). Base `a0ea485`.

## Why

S9 closed operand provenance (R1.1a) and monomorphization identity (R2.1) and
left one measured hole as its Phase-4 Residual. Two modules `a`/`b` each declare
their own same-named generic header (`Widget['T]`) with their own
`impl: Sized for Widget` (constants 1 and 2); a third module `c` imports both,
declares **no** `Widget` header, and makes a **bare** `Widget` ctor call. Before
S10 that call built and silently dispatched on whichever module happened to spell
the instantiation eagerly (with `b` eager, `c` prints `2`; with `a` eager, `1`) —
`c`'s output decided by internals its author may never have read. This is the
silent, load-bearing dependence the roadmap exists to turn into sharp compile
errors (CLAUDE.md).

S9's own-header grounding has nothing to ground at (`c` declares no header); the
only `Widget[i64]` visible to `c`'s env lookup is the single eagerly-minted
instantiation, so the pre-existing single-candidate arm took it silently. S10
replaces that silent pick with a **located compile-time ambiguity error** at the
single-candidate grounding fall-through, keyed on the generic header registry
**scoped to the caller's own import set** — not on the trait-impl matcher (S9's
R-NFR2 forbids dispatch-time machinery).

## Mechanism (as shipped)

The env is built once, before any body is checked, from parse-time **eager mints
only**. A headerless caller's bare ctor call therefore sees a candidate list of
exactly the eagerly-spelled instantiations — in the ambiguous shape, **one**,
even though **both** headers are visible in `module.generic_structs` at env-build
time. The silent pick happened at the single-candidate arm's grounding
fall-through in `bare_generated_word_own_module_grounding` (`src/check/terms.rs`).

The new check lives in `foreign_single_candidate_grounding`
(`src/check/terms.rs:1676`), called from the grounding path at
`terms.rs:1543`. A qualified type spelling in a signature (`a::Widget[i64]`) is
**not** a term-position resolution mechanism — it is a *second eager mint*, so it
reaches the pre-existing multi-candidate error, not this one (why R5's remedy
drops qualified spelling entirely).

## The rule

At the single-candidate arm, once the sole env candidate has survived **both**
the foreign check (`owning_module == caller_module` fails, `terms.rs:1504`) **and**
the candidate-identity check (`generated_word_entry` + `key`/`symbol` match,
`terms.rs:1531-1537`), the grounding raises a located ambiguity error naming the
surface name, the relevant declaring module(s), and the call span — **unless one
of four exemptions holds**.

### Guard ordering (soundness-critical)

The error may fire **only after** the foreign check (`:1504`) and the
candidate-identity check (`:1531-1537`) survive. The pre-S10 code returned
`Ok(None)` at the `find_struct(header_name, caller_module) → None` fall-through
(`:1507-1509`) *before* reaching the identity check, which sat gated behind
`own_idx`. The shipped code restructures the control flow so a headerless
caller still passes the identity check first; emitting at `:1507-1509` directly
would newly reject the "ordinary user word whose output merely happens to be
another module's instantiation" shape the identity check protects. The
`:1492-1497` "the order is free" comment was rewritten: every early exit being an
interchangeable `Ok(None)` stops holding once an arm can return `Err(...)`.

### The reachable set

`m`'s raw import set = `ModuleInfo.imports` ∪ `ModuleInfo.selective` target
values, one hop, **regardless of which name each selective entry was keyed by**
(GO). Plus the **export-origin walk-extension**: `walk_generic_header_origin`
(`terms.rs:1781`) run from every module in that raw set for the surface name;
whatever it resolves to joins the set (GN). The walk is structurally the same
chase the checker runs for concrete type names but over the **generic header
registry** (`ctx.generics().structs`) — **not** a call into
`walk_type_export_origin` / `type_origin` (`driver.rs`), which are
concrete-`StructDecl`/`EnumDecl`-only and always return `None` for a generic
header. A `None` walk (cycle/dead end) contributes nothing. Reachability-scoping
is load-bearing: a program-wide header count would flag a header some unimported
module happens to declare, breaking single-lib programs (GH).

### Declaring vs. instantiating module

Exemptions 2 and 4 both compare against the sole candidate's **declaring** module
(`GenericStructDecl.module` / `guard.structs[gi].module`), never
`struct_instantiation_of`'s *instantiating*-module component. The two diverge in
GE/GF (module instantiates a foreign header in its own signature), but there the
divergence is moot: the caller is the instantiating module, so the call exits at
`:1504` before any exemption runs. The distinction is load-bearing for GL/GN,
where the raw target is neither the instantiating nor (yet) the declaring module.

### The four exemptions

1. **Own header** — `m` declares its own header; S9's R1.1a grounds the caller's
   own mint (untouched).
2. **≤1 reachable header AND declaring module reachable** (both halves required,
   over the fully-resolved reachable set). A header some unimported/un-walked-to
   module declares does not count against the ≤1 threshold (GH). Separately, a
   reachable header that never mints its own instantiation does not license
   silently borrowing a *different*, unreachable module's instantiation (GM — the
   sole existing mint belongs to `z`, which `app` never imports; the error fires).
3. **Multi-candidate arm** (≥2 env candidates) — the existing S5 `select_overload`
   path governs unchanged, including tier-2 pinning for declared type imports (GJ).
4. **Matching explicit resolution** — a **named** selective import (`| Widget |`)
   whose target, **resolved through any hub re-export chain first**
   (`walk_generic_header_origin`), lands on the sole candidate's declaring module
   (GI/GL). A `*` wildcard's per-export desugar never counts, however identically
   it populates `ModuleInfo.selective` (GP). This required threading the
   assembly-time named-vs-wildcard distinction through as
   `ModuleInfo.named_selective` (`ast.rs:195`, built in `driver.rs`: named inserts
   update both maps, wildcard desugars only the flattened one), since the raw
   `HashMap<String,u32>` cannot recover it. A wildcard-reached module still counts
   toward **reachability** (exemption 2).

Exemption 4's match is load-bearing, not decorative: `named_selective` records
what the caller *asked for*, not what it *gets*. When they disagree (caller
selects `X`'s `Widget` but the sole mint belongs to reachable `Y`), the exemption
must **not** apply — handing `Y`'s value silently is the exact defect S10 closes,
relabeled behind a selective import (GK). A blanket "selective import present ⇒
exempt" reading is unsound.

## Diagnostics (R3/R4/R5)

A new located message in house style. Names the surface name, the call site, and
the declaring module(s) using the caller's own import qualifier — module names
**sorted lexicographically** before joining (determinism: import-order and
mint-order independence). When the sole candidate's declaring module has no
caller-side qualifier at all (GM), it is named **structurally** rather than
fabricating a name, following the `drop`-visibility diagnostic precedent
(`word_families.rs`). When reached only through a wildcard import, the existing
"its wildcard-imported module" phrasing applies. GM is framed as a reach failure
("unresolved"), not an ambiguity. The existing 2-candidate `no_overload_matches_error`
text is byte-unchanged (GC). No canonical module-name registry exists; every
diagnostic renders the caller's own qualifier.

Remedy note is two-part: (1) declare your own header + impl (cures alone); (2)
selectively import the module whose instantiation you want — which cures **alone**
only when that module is the sole eager minter (GI) or both mint and tier-2 pins
your selection (GJ). When the selected module is reachable but never itself minted
(GK), selective import alone does **not** cure; the caller must additionally spell
the concrete type in an intermediate word's own signature
(`: mk ( i64 -- Widget[i64] ) Widget ;`), making the caller the instantiating
module. Qualified type spelling is dropped from the note (it only mints a second
candidate, routing into the pre-existing 2-candidate error).

When `ctx.modules` is `None` (e.g. a retained poly word, off the whole-program
build path), the check never fires — the same discipline the D1 drop gate follows.

## Guardrails

- **R-NFR1** — check-stage only. S9's `ir/layout.rs` exception does **not** carry
  into S10.
- **R-NFR2** — `match_impl_target` / `find_bound_impl` untouched; the dispatched
  `size` member call routes exactly as today whenever grounding succeeds.
- **R-NFR3** — no run-count / ratio assertions; determinism is pinned by
  byte-exact text across import orders and minter placements.
- Only the new message is new; no existing diagnostic text churns.

## Out of scope (pre-existing warts, not fixed)

`export: Widget[i64]` parse error and the R18 gate's unsatisfiable remedy;
qualified ctor **term** `a::Widget` → unknown word (P7b2); the concrete-type
same-name collision dimension; a distinguishable phrasing for 2+ declaring modules
reached *only* through wildcard imports (the both-wildcard sub-case is unexercised).

## Goldens (16, `tests/phase7b_slice10.rs`)

| Golden | Test name | Behaviour |
| --- | --- | --- |
| GA | `third_module_bare_caller_with_ambiguous_headers_is_a_located_error` | ambiguous headers → located error naming `a`/`b`, identical for both import orders |
| GB | `third_module_bare_caller_error_is_independent_of_the_eager_minter` | `a` sole minter → same text as GA (minter-independent) |
| GC | `both_modules_eager_2_candidate_ambiguity_error_unchanged` | 2-candidate arm; pre-existing `no_overload_matches_error`, byte-identical |
| GD | `single_declaring_header_bare_caller_still_resolves` | one header program-wide → resolves, prints `7` |
| GE | `single_reachable_header_with_selective_import_still_resolves` | `c` mints `a`'s header itself → exits at `:1504`, prints `1` |
| GF | `single_reachable_header_with_qualified_signature_still_resolves` | same `:1504` mechanism via qualified signature, prints `1` |
| GG | `unimported_foreign_type_annotation_is_still_an_error` | type-position `unknown type` error, unchanged |
| GH | `unimported_declaring_module_does_not_count_toward_ambiguity` | second header exists but unreachable → exemption 2, prints `7` |
| GI | `matching_selective_import_of_the_sole_minter_still_resolves` | exemption 4 matches sole minter, prints `1` |
| GJ | `selective_import_pins_the_named_exporter_when_both_mint` | multi-candidate arm, tier-2 pins, prints `1` |
| GK | `mismatched_selective_import_is_still_a_located_error` | selects `a` but `b` mints → same error as GA (exemption 4's soundness case) |
| GL | `hub_reexported_selective_import_still_resolves` | exemption 4 resolves through hub re-export chain, prints `1` |
| GM | `unreachable_minter_bare_call_is_a_located_error` | sole mint in unreachable `z` → reach-failure error, module named structurally |
| GN | `hub_reexport_reachable_through_plain_import_still_resolves` | reachability walk-extension through a plain hub import, prints `1` |
| GO | `selective_import_of_different_name_still_grants_reachability` | selective import of a *different* name still makes the target reachable, prints `4` |
| GP | `wildcard_import_does_not_exempt_the_ambiguity_check` | wildcard desugar does not satisfy exemption 4 → same error as GA |

Error goldens are byte-exact (measure-then-pin). GE/GF pin the `:1504`
own-instantiating-module arm, not exemption 2.

## Units (12, beside `terms.rs`)

`ambiguous_foreign_headers_grounding_is_located_error`,
`single_foreign_header_grounding_still_borrows`,
`own_header_still_grounded_first`,
`ambiguous_header_error_names_declaring_modules`,
`reachable_header_count_excludes_unimported_declaring_modules`,
`matching_selective_import_exempts_the_ambiguity_check`,
`mismatched_selective_import_does_not_exempt_the_ambiguity_check`,
`hub_reexported_selective_import_resolves_through_origin_walk`,
`unreachable_declaring_module_of_the_sole_candidate_is_still_an_error`,
`reachable_set_includes_selective_targets_regardless_of_selected_name`,
`reachable_set_extends_through_reexport_origin_walk`,
`wildcard_desugar_is_not_explicit_resolution`.

## S9 golden inversion

S9's `third_module_bare_caller_dispatches_the_single_shared_env_instantiation`
(`tests/phase7b_slice9.rs`) pinned the exact silent `2`/`1` outputs GA/GB now
replace with an error; it was retired in `87284cb`. Every other S9 golden stays
green and byte-unchanged. Gate: fmt / clippy / test 3202/0.

## Roadmap

`465e914` updated both stale S9 sentences (dispatch-mechanism Exit clause and
Residual note) to closed-by-P7b.S10, and added an S10 entry recording the
**S10 ∥ S6** parallel sequencing (both branch from base `a9eca84`; maintainer
ruling 2026-09-05) with the landing rule: if S6's probes demand a `terms.rs`
grounding change or the merges interact, S10's checker change lands first and S6
rebases onto it. Growth signals (R6) re-run on every touched file: no file shows
2+ signals; `terms.rs`'s additions form one cluster in the existing grounding
call graph.
