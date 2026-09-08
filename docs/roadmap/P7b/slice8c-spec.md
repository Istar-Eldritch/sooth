# P7b.S8c — Per-site binding for member signatures with free input type variables

Delivery plan for S8c. Recon inputs, frozen: [slice8c-brief](./slice8c-brief.md)
(scope + F1-F5 mechanism + D1/D2), [slice8c-probes](./slice8c-probes.md)
(two-round verbatim log, route taxonomy, w3r no-churn verdict),
[slice8c-paper-tests](./slice8c-paper-tests.md) (golden designs G1-G6, measured
HEAD behaviour). Roadmap scope: `P7b-higher-kinded-types.md:540`. Base HEAD
`62a928d` (post-S8b merge); baseline gate green (89 binaries, 3334 tests, 0
failed).

## Why

The P7b.S8 integrated review surfaced a pre-existing ICE: a bound-dispatched
trait member whose signature carries a **free input type variable** (not the
trait header variable, not bound by any bound bracket) passes checking and
panics at IR lowering. The panic is `subst_polytype`'s unification `expect`
(`src/ir/driver.rs:579`, "checked: unification bound every input type
variable"), reached through `concrete_effect` (`:555`) from lowering's R9
instantiation loop (`:327`). `sooth build` exits 101 and produces **no binary**
(the roadmap's "the build succeeds and the PANIC fires at run/IR time" wording
is corrected at slice exit — the *check* succeeds; the panic is a **build-time
lowering** panic).

The recon round rewrote the mechanism story into a five-route dispatch taxonomy
keyed in `resolve_user_bound` (`src/check/poly.rs:8981`) on the obligation's
`ty` (probes, "Mechanism"):

- **Route A — CtorImage** (App-headed operand, `'F['U]`): the arm at
  `poly.rs:9161-9314` composes per site — re-grounds `ob.slots` through the
  caller θ, unifies each candidate's member signature, mints that site's
  θ_call. Every member variable grounds. This is what every shipped S6/S7/S8
  dogfood rides (probes P3/P6/P7/P8).
- **Route B — non-CtorImage, generic winner** (the bound variable itself in a
  non-App position — plain slot, `&'T`, quotation row, array element): the
  P7.S4 mint arm (`poly.rs:9315-9323`) records `(member_word, subst)` where
  `subst` is `find_bound_impl`'s impl-target match substitution **alone** — the
  member row's own variables never enter θ. Lowering substitutes the whole
  member signature through that θ and hits the unbound `expect`. **This is the
  ICE** (probes P1/P5/P9/P10, plus round-2 array-element and bare-var-target
  cells).
- **Route C — non-CtorImage, concrete winner** (`poly.rs:9324-9331`): bare
  symbol, no mint, **no site-slot check**. `ground_member_type`'s
  `PolyType::Var(_) => target` arm (`src/ast.rs:2241`; round 1 cited 2239)
  collapses
  every member local to the target at registration, so the member emits as one
  mono function typed wholly at the target, and any type flows through the local
  slot unchecked. **A silent wrong-typing hole**, not a panic (probes P11/P12).
- **Route D — mono direct member call**: full call-site unification binds
  everything; healthy, and it *does* check operand slots
  (`trait_member_operand_error`, probes P2).
- **Route E — lifted mono ctor-app target**: bare symbol, no signature to
  substitute; untouched (S8's shipped Range golden).

F3/P14: output-position member locals are already fenced at the impl body check;
only input/ref/quotation/array positions are in play. F5: the IR `expect`'s
invariant holds for call-site-minted instantiations (Routes A/D) and is violated
**by construction** for Route B's match-only mint — the checker never enforced
that a member row's variables are all determined by the impl-target pattern.

## Decisions

### D1 — fix direction: **option B, the per-site binding rule** (REQ-1)

Extend the CtorImage arm's per-site unification to the P7.S4 mint arm: at the
member call site, re-ground `ob.slots` through `caller_subst`, unify the member
word's signature, mint that site's θ (sorted per the P7.S3t invariant). The
repro then *works* — it grounds like `'B` (Bifunctor's row locals) already does
through Route A — rather than being rejected.

Evidence for B over A and C:

- **Option A (declaration-time plain-slot fence) breaks green tests.** Round 2
  refuted the round-1 "zero victims" claim (probes w4): the committed suite
  carries ≥7 green test functions in 5 files riding plain-slot member locals
  through *working* Route B dispatches — including **S6's exit-criterion
  golden** `Foldable::fold` (`tests/phase7b_slice6.rs:337/372/469`), `Take::take`
  (`tests/phase7b_slice3.rs:426`), `Functor::pick`
  (`tests/phase7b_slice2.rs:146`), plus two src unit twins
  (`declarations.rs:3860`, `poly.rs:22301`). A declaration-time fence outlaws a
  shape the machinery supports. Disqualified.
- **Option B has measured nil symbol churn** (probes w3r, three reasons): (1)
  no shipped dispatch reaches the mint arm at all — Show/Write/Ord ride Route C,
  Iterator/List ride Route A, Iterator/Range[i64] rides Route E; the 49-symbol
  inventory contains no mint-arm dispatch, and `lib/` ships no
  Functor/Foldable/Monoid/Monad (those live only as test fixtures); (2) the
  len-pair encounter-order hazard has zero victims (no `Len::Var` in the shipped
  dispatch surface); (3) where both substitutions exist the binding sets agree,
  so a per-site symbol is byte-identical to today's. Predicted post-fix repro
  monomorph: `sooth_mono_odd_Odd_0_Box__T0___m0__t0_i64_t1_i64` (member word
  registered `odd;Odd;0;Box['T0]__m0`; θ binds the target var and the local,
  both `i64`).
- **Option C (mint-site fence)** is the conservative fallback: located error
  when a member variable remains unbound after the match subst. B subsumes C's
  safety once D2 folds the concrete arm in (the same unification *is* that
  check), and B turns the repro into working code rather than a fence casualty.

The evidence leans clearly to B; the spec owns the call and picks **B**.

### D2 — concrete-target hole: **fold in** (REQ-2)

Route C's silent no-site-check dispatch (`poly.rs:9324-9331`) is the same
obligation loop, one arm below the mint arm. Round 2 raised its urgency: it is
**bound-dispatch-specific** (the direct mono call is checked, the bound dispatch
is not) and **exploit-confirmed** to produce unambiguous wrong output — a member
body computing on the local slot prints a `Bool`'s discriminant `+1` or a
`List[i64]`'s head word `+1` (probes w2, P12). No type-respecting execution
explains it.

Folding in costs one slot-compatibility unification in the concrete-winner arm —
exactly option B's machinery reused — and keeps the size-S character: one
obligation loop, one diagnostic family (Route D's `trait_member_operand_error`
text is the template, probes w2). It closes the hole while B is open in the arm
directly above; carving it out would leave a live silent-wrong-typing hole
behind a slice that already has the unification in hand. **Fold in.**

## Requirements

- **REQ-1 (D1, Phase 1).** In `resolve_user_bound`'s non-CtorImage generic-winner
  mint arm (`src/check/poly.rs:9315-9323`), compose the member instantiation
  **per site**: re-ground `ob.slots` through `caller_subst`, unify the member
  word's signature against the grounded slots (the CtorImage arm's
  `unify_poly_input`/slot machinery at `poly.rs:9161-9314` is the source), and
  mint θ sorted per the P7.S3t invariant. The recorded `(member_word, θ)` must
  bind every member input variable, so lowering's `expect` (`driver.rs:579`)
  never fires on this route. Route A is untouched by construction.

- **REQ-2 (D2, Phase 2).** In the concrete-winner arm (`poly.rs:9324-9331`), add
  a site-slot compatibility check: unify `ob.slots` against the member's grounded
  (target-collapsed) signature. On mismatch, emit a **located** error naming the
  member and the mismatched operand slot, modelled on Route D's
  `trait_member_operand_error`. The bare-symbol dispatch path is otherwise
  unchanged (no mint — the member is already a mono word).

- **REQ-3 (scope, all phases).** `src/check/` only. **No `src/ir/` change** —
  `driver.rs:579`'s `expect` stays as the backstop, and G2/G4 double as the
  proof it never fires on the already-healthy routes. No new trait/impl surface,
  no `lib/` change (`lib/` is the regression net only).

- **REQ-4 (caller rule unchanged, Phase 3).** P4's caller-side rule ("output
  variable that no input binds / supply it explicitly") is pre-existing and
  stays byte-identical. The quotation-row twin (P5) reaches Route B *after* the
  caller supplies the explicit instantiation, so REQ-1 grounds it; P4's own
  located message gets no new golden.

- **REQ-5 (cells, Phase 3).** REQ-1's binding must cover all Route B cells the
  same union-var mechanism admits (`build_member_var_union`,
  `parser.rs:768/817-827`): plain slot, `&'T`, quotation-row local,
  **array-element** (`array['U 2]`), and the **bare-var catch-all** impl target
  (`impl: Odd for 'T`). Goldens cover the cheapest twins of each shape.

- **REQ-6 (unit coverage, Phases 1-2).** Unit tests beside the changed checker
  code (`src/check/poly.rs`'s `#[cfg(test)] mod tests`): message bytes, member/
  variable naming, and both the mint-arm binding (REQ-1) and the concrete-arm
  compatibility check (REQ-2). Byte-exact diagnostic pins measured from the live
  binary (diagnostics are behaviour).

- **REQ-7 (goldens both routes, Phase 3).** End-to-end goldens in
  `tests/phase7b_slice8.rs` for the mono call-site route (Route D) and the
  bound-dispatch route (Route B), per roadmap scope. S8c is an S8 follow-up; the
  file header comment gains an S8c line.

- **REQ-8 (repro twins, Phase 3).** The S8-review repro twins land
  panicked→located: G1r (Route B repro builds+runs, exit 0) and G3b
  (quotation-row twin builds+runs) are the surviving halves under direction B;
  the fence-direction halves G1/G3a do **not** ship. The phase commit message
  names which direction won.

## Goldens

Designs and measured HEAD behaviour: [slice8c-paper-tests](./slice8c-paper-tests.md).
Under direction B, exactly one of each alternatives pair survives.

| Golden | Test name | Fixture (probes) | Post-fix (direction B) |
| --- | --- | --- | --- |
| G1r | `bound_dispatch_grounds_member_locals_from_the_call_site` | `plainslot.sth` (P1) | builds+runs, exit 0; monomorph `sooth_mono_odd_Odd_0_Box__T0___m0__t0_i64_t1_i64` |
| G2 | `mono_call_site_member_local_stays_working` | `mono.sth` (P2) | build+run exit 0 (positive pin, Route D) |
| G3b | `quotation_row_member_local_grounds_with_explicit_instantiation` | `qrow3.sth` (P5) | builds+runs, exit 0 |
| G4 | `app_headed_member_local_keeps_grounding_per_site` | `mixed3.sth` (P8) | build+run exit 0 **and** symbol `sooth_mono_w2_W2_0_Box__T0___m0__t0_i64_t1_e0_Bool` byte-identical (Route A regression pin) |
| G5 | *(checklist item, no new fixture)* | S6/S7/S8 dogfoods | `cargo test` green, zero retouched baseline ILs; any retouch justified in the commit message |
| G6 | `concrete_target_member_dispatch_checks_site_slots` | `concrete4.sth` (P12) | `build_error_located` naming the member and the mismatched slot (D2 folded in) |

Plus REQ-5 cell twins: an array-element (`array['U 2]`) and a bare-var-target
(`impl: Odd for 'T`) fixture, each a G1r-style build+run pin. Harness:
`single_file_hosted` / `build_run_keep` / `build_error_located` (the latter
asserts `!stderr.contains("panicked")` — the admission-safety sweep's standing
rule). G4's symbol assert rides `emit_ssa_with_manifest` capture
(`tests/phase7b_slice8.rs:742`; `slice8b.rs:177/565` already filter
`sooth_mono_*`).

## Codebase map

| Path:line | Symbol | Role in S8c |
| --- | --- | --- |
| `src/check/poly.rs:8981` | `resolve_user_bound` | route dispatch on `ob.ty`; the changed function |
| `src/check/poly.rs:9161-9314` | CtorImage arm | per-site composition; REQ-1's source machinery (`unify_poly_input`, slot re-grounding) |
| `src/check/poly.rs:9315-9323` | P7.S4 generic-winner mint arm | **REQ-1 edit site** — replace match-only subst with per-site θ |
| `src/check/poly.rs:9324-9331` | concrete-winner bare-symbol arm | **REQ-2 edit site** — add site-slot compatibility check |
| `src/check/poly.rs` (`mod tests`, near `:22301`) | checker unit tests | REQ-6 unit twins live here |
| `src/ir/driver.rs:579` | `subst_polytype`'s `expect` | the backstop; **unchanged** (REQ-3) |
| `src/ir/driver.rs:555,327` | `concrete_effect`, `lower` R9 loop | where the panic is reached today; unchanged |
| `src/ast.rs:2241` | `ground_member_type`'s `Var` arm | why Route C collapses locals to the target (context for REQ-2); unchanged |
| `src/parser.rs:391` | `member_shape_is_supported`'s `Var` arm | admits plain-slot member locals (context); unchanged |
| `src/parser.rs:768,817-827` | `build_member_var_union` | the union-var mechanism spanning all REQ-5 cells; unchanged |
| `tests/phase7b_slice8.rs` (header; `:742`) | S8 goldens | REQ-7/REQ-8 end-to-end goldens + G4 symbol capture |

## Delivery plan

- **Phase 1 — REQ-1 (D1, per-site binding).** Extend the generic-winner mint arm
  to per-site composition, reusing the CtorImage arm's unification. Unit tests
  beside it (REQ-6): binding correctness for plain-slot and `&'T` shapes, both
  binding arms (`Box['T0]` vs `Box[i64]`). Verify the predicted post-fix
  monomorph symbol.
- **Phase 2 — REQ-2 (D2, concrete-arm compatibility check).** Add the site-slot
  unification to the concrete-winner arm; located diagnostic modelled on
  `trait_member_operand_error`. Unit tests beside it (REQ-6): the mismatch case
  (a `List[i64]` in an i64-typed local slot) is now located, the matching case
  still builds.
- **Phase 3 — goldens (REQ-5/7/8).** G1r, G2, G3b, G4, G6 in
  `tests/phase7b_slice8.rs` (header gains an S8c line), plus the two REQ-5 cell
  twins. Byte-exact pins measured from the live binary.
- **Phase 4 — roadmap/bookkeeping exit.** Apply corrigenda 1-4 to the S8c
  paragraph (`P7b-higher-kinded-types.md:540`): (1) "build succeeds…run/IR time"
  → check succeeds, panic at build-time lowering; (2) the shape is one cell of a
  five-route taxonomy, with the quotation-row/array-element/bare-var-target cells
  and the concrete hole recorded; (3) carry-forward wording; (4) citation drift.
  Note in the paragraph that S8c shipped direction B and folded in D2. Re-run the
  growth-structure signals against `poly.rs` (per CLAUDE.md; the split stays
  deferred — a localized two-arm edit adds no new signal). Full green gate:
  `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

## Phases (JSON)

```json
[
  { "phase": 1, "focus": "REQ-1: per-site binding in the generic-winner mint arm (poly.rs:9315-9323) + unit tests", "effort": "M", "difficulty": "M" },
  { "phase": 2, "focus": "REQ-2: concrete-winner site-slot compatibility check (poly.rs:9324-9331) + unit tests", "effort": "S", "difficulty": "M" },
  { "phase": 3, "focus": "REQ-5/7/8 goldens G1r/G2/G3b/G4/G6 + cell twins in tests/phase7b_slice8.rs", "effort": "M", "difficulty": "S" },
  { "phase": 4, "focus": "roadmap corrigenda 1-4 on the S8c paragraph, growth re-check, full green gate", "effort": "S", "difficulty": "S" }
]
```
