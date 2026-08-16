# Phase 4 Slice 6g (implemented): combinator splices learn 6f's granting rule

Derived from [`docs/phase4-slice6g-brief.md`](./phase4-slice6g-brief.md). Landed in four
phases on `impl/phase4_slice6g_spec-2608130839`. Checker-acceptance plus one new diagnostic:
no `Instr`/`Terminator`, no `Type`/`IrType`, no lowering, no `qbe.rs` touched, and no corpus
baseline regenerated (`examples/filter_while.sth` was left as-is so the byte-for-byte lowering
claim stays literally true). Anchors below are post-implementation.

## The bug

Every combinator body is spliced at its call site by `inline_combinator`
(`src/check/combinators.rs:227`), whose body-check ran the plain `check_terms`
(`src/check/terms.rs:11`) — the root entry point 6f reserves for "a word body, a REPL line, a
`case` clause: nothing is ancestor to those" — instead of `check_terms_relaxed`
(`src/check/terms.rs:50`) with a `releasable_into`-computed grant. `call`, `times` and the `if`
arms all did the relaxed thing; the splice was the one nested-invocation shape on the wrong side
of 6f's fork. Every array is `Copy` (`check_no_linear_array_elements` rejects non-`Copy` element
types at registry time), so naming one never enters move-tracking, leaving `Liveness::dead` as the
only guard — and the splice granted nothing into it. Pre-fix, a `filter` over a *bound* array was
rejected `cannot borrow arr__inl0 mutably … it is aliased by a`, while the identical shape routed
through `times`/`call` compiled.

## Why the naive fix (D1 alone) was not shippable

Splicing a grant into `inline_combinator` (D1) closes the false rejection, but three measured
holes forced it to be sequenced behind a tightening and a new diagnostic:

- **The granting rule was already wrong at a loop back-edge (R1, recon 9).** `releasable_into`
  granted an ancestor name on `!references(rest, name)`, where `rest` is only the *remaining*
  siblings. Inside a `back_edge = true` body, execution wraps to the first term, so a name used
  *earlier* in the body was still live where the grant was handed down — a silent wrong value, no
  combinator required. 6g multiplies the blast radius (every combinator call inside a loop becomes
  a fourth doorway; 10b/10c turn `times`/`if` into splices), so R1 had to land and be validated
  before any relaxation could mask it.
- **The splice's `back_edge` was observable through a name-hygiene defect (D5, recon 10).**
  `alpha_rename_locals` renames a callee's locals but `rename_call` leaves calls to a word/builtin
  untouched, so a caller local sharing a builtin's name was read *in place of that builtin* inside
  a spliced body, silently (`| len |`, `len` a builtin, printed the wrong value with no
  diagnostic). D5 closes it at the root, making D4's "the splice's flag is unobservable" argument
  hold by construction.
- **The argument path had its own wrong-side `check_terms` (R2, recon 8).**
  `check_literal_against_declared_effect` runs the caller's quotation literal against the declared
  parameter effect in the caller's scope through the plain root entry point, and rejects *before*
  the body splice, so D1 alone could not discharge the `while`-nested-in-a-combinator shape.

## What landed

- **R1** — `releasable_into` (`src/check/engine.rs:825`) gained `live: &Liveness, at: usize` and a
  split filter: a name bound in the current invocation (`idx >= base_depth`) keeps the old
  `!references(rest, name)` rule verbatim; an ancestor name is granted only if
  `outer_releasable.contains(name) && live.dead(name, at + 1)`. The index is `at + 1`, not `at`:
  `nested_uses` attributes a use inside `terms[at]` to `at` itself, so `dead(name, at)` would be
  false whenever the granted-into term is the user of the name (the whole point of granting).
  Asking at `at + 1` reproduces "no residual use after this term" while still catching the
  wrap-around, because a use anywhere in a `back_edge = true` body is recorded `IMMORTAL_IN_BODY`
  (`usize::MAX`, never `<` any index). **R1 grants a strict subset of HEAD's grants**
  (`scan`/`references` traverse the identical three nesting `TermKind` variants, so
  `dead(name, at+1)` implies `!references(rest, name)`), so over-permissiveness relative to HEAD is
  impossible by construction. The honest cost: R1 also *rejects* some sound programs that named an
  ancestor array by name inside a back-edge body (write-only, or read-and-mutate across the edge —
  the `danger` shape that also printed a stale value on HEAD); the checker cannot distinguish
  "mentioned but never read across the edge" from "read across the edge" without machinery beyond
  this slice. A splice never triggers this (the array arrives on the stack, callee locals are
  alpha-renamed), which is why the splice / `while` / `sort` shapes all survive.
- **D5** — the mono `TermKind::Bind` arm (`src/check/terms.rs:151`) and the poly one
  (`src/check/poly.rs`) reject a local whose name collides with a callable, via the new
  `callable_local_error`, modelled on `reject_variant_local`/`extern_redeclaration_error`. Coverage
  differs by site deliberately: the mono arm checks builtins, `env`, `poly.env` and
  `poly.combinators`; the poly arm checks **builtins and `env` only**, because `poly_term` has no
  `PolyCtx` and reaching the poly maps would mean changing `check_poly_body`'s signature (out of
  scope). **Recorded gap:** a polymorphic word may still bind a local named after a combinator or
  poly word; nobody could construct a wrong-value witness through the poly arm (the hygiene defect
  needs an `alpha_rename_locals` splice and no shape routes a poly-bound shadow into one), so it is
  scoped as uniformity, not soundness. Closing it is a separate slice that owns the signature
  change.
- **D1** — `inline_combinator`'s body-check (`src/check/combinators.rs`) became
  `check_terms_relaxed(..., granted, true)`; its call site in `check_term`
  (`TermKind::Call` dispatch) computes the grant via `releasable_into` exactly as its three
  neighbours do and threads `granted: &HashSet<String>` through `inline_combinator` and
  `check_poly_combinator_args` (`src/check/combinators.rs:418`).
- **R2** — `check_literal_against_declared_effect` (`src/check.rs:1345`) took the same `granted`
  set and its `check_terms` became `check_terms_relaxed(..., granted, true)`. Its three
  non-combinator callers (`src/check/captures.rs`, the two `if`-arm merge sites in `terms.rs`) pass
  `&HashSet::new()`, preserving their behaviour exactly.
- **D4** — both new relaxed calls pass `back_edge = true`. At the literal check this is required
  for soundness (the terms scanned are the caller's own literal, re-executed per iteration). At the
  body splice it is a uniformity choice once D5 closes the hygiene defect — **not pinned by any
  test, and deliberately so** (flipping it, with D5 in place, changes no test).
- **D2** — unchanged. Copy-array move-blindness is a permanent property (`aliasing_origin`'s
  `moved_site` filter can never exclude an array because a `Copy` local never enters the move map).
- **D3** — the spec's plan was to delete a stale aliasing-rationale paragraph from an existing
  `lib/arrays.sth` and drop the dogfood if the file was absent. **Deviation:** the file did not
  exist, so Phase 4 authored a fresh `lib/arrays.sth` (`bin_search` + a comparator-driven merge
  `sort`, no stale paragraph to delete) rather than dropping T-sort. The dogfood is therefore
  measured, not skipped.

The mechanism plumbing: `check_terms_relaxed` was bumped from bare `fn` to `pub(super)` and
`check.rs` gained `use self::terms::check_terms_relaxed;` — the import landed in the *same* Phase 3
change as R2's call site, since an earlier import would be a dead-code `clippy -D warnings` failure.

## Two invariants this slice is the sole record of

- **Grants are capture-blind.** `releasable_into` never consults captures; capture-awareness lives
  at the *use* site (`live_derivs`, `aliasing_origin`, each via `capture_alive_names`). R1 added a
  `live.dead(&b.name, at + 1)` call inside `releasable_into` — a *grant* site, not an aliasing-use
  site — so it correctly needs no capture disjunct, but it is now a fifth `live.dead` caller. A
  future consumer of `live.dead` on an aliasing path without the capture disjunct would break this
  silently.
- **`dead`'s `None` arm is fail-open** (returns `outer_releasable.contains(name)`), which is safe
  only because `scan` and `references` traverse the identical three nesting variants — the same
  fact that makes R1 a strict subset of HEAD. A future `TermKind` with nested terms added to
  `references` but not to `scan` would silently open a hole (checked safe against 6h's
  `ArrayCtor(Type)`, which carries a type, not terms).

## Implementation

| Phase | Decisions | feat | review fixes |
| --- | --- | --- | --- |
| 1 | R1 (the tightening, alone) | `b5e36ec` | `99412a6`, `e74cabb` |
| 2 | D5 (bind-collision diagnostic) | `4dff92c` | `496f613` |
| 3 | D1 + R2 (the relaxation) | `475dc32` | `883b866` |
| 4 | T-sort dogfood, `lib/arrays.sth`, ROADMAP | `630f910` | `f1fdae4`, `20ec596` |

Phase 1 landed and was validated green with no relaxation present. Phase 2 landed the diagnostic
ahead of the relaxation so D4's splice-side argument held by construction. Phase 3 threaded the
grant. Phase 4 added the dogfood and marked 6g implemented in `ROADMAP.md` (repointing the
"Next action" to 10b/10c, and correcting the 6g entry's stale `self_tail`-conditioned `back_edge`
and "no other array constructor exists" texts).

Goldens in `tests/phase4_slice6g.rs`, plus the R1 unit test
`releasable_into_withholds_a_name_used_in_a_back_edge_body` (`src/check/engine.rs:2027`), which
covers both halves — the `IMMORTAL_IN_BODY` withhold and the `None`-arm still-granted case — so the
tightening is shown not to over-tighten (the e2e goldens reach `releasable_into` only through
several nested invocations; precedent is 6f's R6 walk-stop unit test).

| Test | Kind | Phase | Pins |
| --- | --- | --- | --- |
| `if_inside_a_loop_reading_an_alias_is_an_error` (P-wrap) | reject | 1 | R1 wrap-around; ran-and-printed `0`/`9` on HEAD |
| `read_and_mutate_inside_a_looped_grant_is_an_error` (danger) | reject | 1 | R1's second wrong-value shape; printed `0`/`9` on HEAD |
| `single_call_body_naming_the_alias_is_an_error` (b-call) | reject | 1 | R1, single invocation, no use after |
| `write_only_across_a_back_edge_is_an_error` (d2) | reject | 1 | R1, write-only across the edge |
| `two_level_execute_once_grant_still_accepted` (nest2) | accept | 1 | discriminates `at + 1` from `at` |
| `times_doorway_grants_the_bound_alias` | accept | 1 | doorway grant still fires |
| `later_use_withholds_the_times_grant` | reject | 1 | doorway boundary guard (not a 6g pin) |
| `binding_a_local_named_after_a_builtin_is_rejected` | reject | 2 | D5 mono arm; printed `1` silently on HEAD |
| `binding_a_poly_local_named_after_a_builtin_is_rejected` | reject | 2 | D5 poly arm (builtin/`env`) |
| `bound_array_passed_to_filter_is_accepted` (P-splice) | accept | 3 | clean D1 witness (R2 revert leaves it green) |
| `while_over_an_aliased_array_local_is_accepted` | accept | 3 | joint D1+R2 witness (reverting either reds it) |
| `while_over_an_aliased_array_local_rejects_if_the_original_name_is_read_in_the_loop` | reject | 3 | T-while soundness bound (R1) |
| `while_over_an_aliased_array_local_rejects_if_the_original_name_is_used_after_the_loop` | reject | 3 | T-while soundness bound (`references`) |
| `sort_called_with_bound_array_locals_runs` | accept | 4 | dogfood; reads sorted `ra` not scratch `rs`; prints `1 2 3 4` |

`while` is itself a combinator, so its own body-splice (D1) and the argument-path literal check
(R2) both touch the same literal; T-while therefore reds under *either* revert and pins D1+R2
jointly. T-splice's predicate `[ 4 > ]` never mentions the array, so it is the clean D1 witness
that stays green when R2 alone is reverted — the half of the story that discriminates R2 from D1.

## Out of scope

Restructuring `sort`'s code (D3 was rationale-only). `Copy`-type move-tracking (D2). Whether the
three non-combinator `check_literal_against_declared_effect` callers deserve a grant (R2 — 7b/10c's
question). The `PolyType::Ref` gap. Closing D5's poly-arm coverage gap (owns the `check_poly_body`
signature change). Any lowering, IR, or diagnostic-text change beyond D5's new diagnostic. Editing
any corpus-pinned example.
