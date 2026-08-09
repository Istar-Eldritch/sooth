# Phase 4 Slice 6f: liveness ends at last use (spec)

A reference bound to a local now dies at its **last use**, matching the anonymous stack case. `live_derivs` (`src/check.rs:759`) chains the virtual stack's derivations with the scope's bindings; the two halves disagreed (a stack slot's deriv dies when its slot is consumed, a binding's lived for the whole block). This slice ends a binding's borrow at its last use, and applies the same rule to `aliasing_origin` (`:854`), which asks the same lexical-vs-last-use question for a second *name* of a place.

**Checker-acceptance-only (D1):** no new `Instr`/`Terminator`, no lowering/`Type`/`IrType` change. `Deriv`/`DerivId` stay confined to `src/check.rs`, so no emitted code changes by construction. Not a lifetime system (no lifetime variables, regions, or scope-bound validity); a rule about *when a borrow ends inside one block*.

## Where the change landed

- **The rule is one function:** `live_derivs` (`:759`). Only the `scope.bound` half becomes use-bounded; the `stack.iter()` half is left byte-for-byte (D2).
- **Seven consumer sites** inherit it via `live_deriv`/`live_borrow_of`/`live_mutable_borrow_of` (`:769`/`:780`/`:793`) or `live_derivs` directly:
  1. reborrow-suspension (`:5844` → `:6999`): **structurally unaffected** — scope bindings mint `reborrow: false` (`Scope::bind`, `:583`). Do not touch or test.
  2. consume (`:5858`, `live_borrow_of` → `:7013`): the probe pair's reject path (T3/T4).
  3. naming-side (`:5881`, `live_mutable_borrow_of`, gated on `aliases.is_some()` → `:5882`): symmetric twin of `aliasing_origin` (`:7287`-`:7289`). Inherits D2's borrow-half change automatically — **no name filter here**; `:7292` gets D8's name filter. Both directions must still agree once both relaxations land.
  4. exclusivity in `check_reference_word` (`:7275`, `owned_root == rest` → `conflicting_borrow_error`): where D10 lands.
  5/6. `times` identity snapshots (`:6002`/`:6041`, `live_derivs` into a `HashSet<DerivId>`): the one existing consumer relaxation could silently break (Q2).
  7. `aliasing_origin` (`:854`, sole caller `:7292`): sibling table (D8), own untouched stack half (`:876`).
- **Position already in hand:** `check_terms` (`:5748`) walks `terms.iter().enumerate()`; each `if` arm / spliced body / inlined quotation re-enters `check_terms` in its own index space.
- **Scan precedent:** `alpha_rename_locals` (`src/ast.rs:1002`); a *use* of a local is a `TermKind::Call(s)` with `s` bare or `&`/`&!`-sigilled (`rename_call`).
- **No shadowing** (D4): re-binding a name in scope is rejected (`rebound_local_error`, `Bind` arm `:5809`).

## Locked decisions

- **D1 — Checker only.** Edits confined to `src/check.rs`, `tests/phase3_refs.rs`, one dogfood, `ROADMAP.md`.
- **D2 — Only the scope half becomes use-bounded.** Both stack halves (`live_derivs`, `aliasing_origin:876`) left byte-for-byte; touching either is presumed a regression.
- **D3 — Last use is per `check_terms` invocation**, over its own term list in its own index space. No global/cross-body index — this is what composes with the inliner's alpha-renaming.
- **D4 — No shadowing rule.** A re-bind path found in impl is a separate defect to report.
- **D5 — The two flipping tests are rewritten, not deleted.** `move_of_place_borrowed_in_locals_is_error` (`tests/phase3_refs.rs:750`) and `dispose_of_borrowed_place_is_error` (`:765`) bind a reference and never reuse it, so the borrow would go dead and the consume legal. Keep their intent by adding a use of the reference *after* the consume. Stack control `move_of_place_borrowed_on_stack_is_error` (`:735`) untouched, still rejects.
- **D6 — A loop body is not straight-line.** A reference bound outside a loop and used inside is live for the whole body. Realized by Q1's invocation-scoping: relaxation only expires bindings *created within* the current invocation, so an inherited reference is never relaxed while a body-local one (re-minted per iteration) expires straight-line.
- **D7 — Ending a borrow disposes nothing, emits nothing.** No implicit drop. `Scope::bind` (`:719`) registers move-state only for linear values; `leave` (`:742`) leaks only where such state exists; `leave_block` (`:5625`) already reports nothing for a reference. Only the point at which the checker *stops refusing* a consume moves.
- **D8 — `aliasing_origin`'s scope half gets the same treatment.** One analysis, two scope tables, both stack halves untouched. A name never used again is not a second denoting name.
- **D9 — The alias half is mutation-tested, the borrow half merely tested.** A wrong borrow-half answer accepts/rejects a *program*; a wrong alias-half answer silently produces a wrong *value* — the class this language exists to catch. All **17** `aliasing_origin` guards must be shown to go red when the guard they name is removed. Phase exit criterion.
- **D10 — The exclusivity relaxation is intended.** Two sequential mutable borrows where the first is never reused become legal (`&!v | f | &!v ...`, `f` unread). `| f |` *pops* the reference (`stack.split_off`, `:5825`) and a binding never named again is read through by nothing. Pinned with T7.

## Resolved questions

- **Q1 — Per-invocation pre-pass, keyed by names bound in that invocation (option c, narrowed).** Compute a `Liveness` over the invocation's own term list; thread `&Liveness` + index `i` through `check_term` into the consumers. Rejected: (a) a `Binding.last_use` field hits an index-space wall across nested invocations and disturbs the stack-half struct (violates D2); (b) re-scanning per query pays repeatedly for cacheable info. Pre-pass is O(subtree) once per invocation.

  ```rust
  struct Liveness { last_use: HashMap<String, usize> }
  impl Liveness {
      fn scan(terms: &[Term]) -> Self { /* mirrors alpha_rename_locals' walk */ }
      fn dead(&self, name: &str, at: usize) -> bool {
          self.last_use.get(name).is_some_and(|&last| last < at)
      }
  }
  ```

  `scan` walks forward with a `bound: HashSet<String>` (no forward reference, no shadowing). For `Call(s)`, strip leading `&!`/`&`; if in `bound`, record `last_use[name] = i`. For a nested `If` at index `i`, recurse to find `Call`s of currently-bound names and set `last_use[name] = max(existing, i)` (attribute nested use to the containing top-level index; both arms execute synchronously there, so this is exact).

  **Correction from review round 1 (a real regression, not anticipated by this paragraph as first written):** a `Quotation` is not an `If` arm. An *unbound* quotation (passed straight to a word, e.g. a combinator argument) keeps the same at-its-own-index attribution, since it's consumed there. But a quotation *bound* to a local (`[ ... ] | q |`, detected when the literal is immediately followed by the `Bind` naming it — nothing else can have intervened, so it's exactly the bind's last name) does not execute at its own literal position; it executes wherever `q` is later used, arbitrarily far away. Treating it like an `If` arm attributed a capture to the literal's index, so the captured place looked dead the moment the literal was written, silently letting a second live `&!` to the same place through. The fix: a bound quotation's captures are recorded into a separate `captures: HashMap<String, HashSet<String>>` (bound name → names its body reaches) instead of `last_use` directly; once `scan` finishes, a fixpoint (`while changed`, not a single pass — a quotation capturing a quotation, e.g. `[ q1 call ] | q2 |`, needs the chain to settle) propagates each bound name's own eventual last use through `captures`, transitively. The capture graph is acyclic (a quotation can only capture names already bound earlier in program order), so this always terminates. Does **not** collect bindings from nested bodies. Filter added only to the `scope.bound` chain in `live_derivs`; outer bindings inherited into scope are absent from `last_use` and never relaxed (D6).

- **Q2 — `times` snapshots evaluate at the `times` term's index.** Both `live_derivs` calls (`:6002`/`:6041`) use the same `(live, at = i)`, so any binding dead before the `times` term is excluded from both sets equally; relaxation can't manufacture a false difference, and a genuinely carried borrow still trips `times_body_borrow_across_loop_error` (T8).
- **Q3 — Branches: conservative max at the `if` term's index.** A reference used inside one arm expires only when the `if` ends. Falls out of Q1: the outer scan attributes any arm use to the `if` index; inside an arm's own invocation the outer binding isn't in that arm's `Liveness`. Per-arm precision not pursued (never more permissive in any dogfood; max can't be unsound). Highest-risk (six of 17 alias guards are merge cases).
- **Q4 — `leave_block` needs nothing** (read at `:5625`). `scope.leave` inspects only `moves.states`; a reference has none (D7). Relaxation touches only mid-block borrow-conflict queries; `Liveness` is dropped when the invocation returns.
- **Q5 — REPL unchanged.** A line is one `check_terms` invocation; nothing crosses lines. A reference with no later use expires at line end (`reference_local_expires_without_drop`, `:590`). `reference_surviving_repl_line_is_error` (`:518`) is a stack carry (D2, untouched). Confirmed by regression run.
- **Q6 — One shared `Liveness`, filtered per binding by its own name; overlap preserved.** `aliasing_origin` keys on alias-set *overlap*, not name identity. Filtering each candidate by its own last use composes: name A (dead) is dropped but overlapping live name B still returns and rejects; accepted only when *no* live name overlaps. Filter added alongside the existing `moved_site(...).is_none()`:

  ```rust
  .filter(|b| {
      b.name != place
          && b.aliases.is_some_and(&overlaps)
          && scope.moves.moved_site(&b.name).is_none()
          && !live.dead(&b.name, at)
  })
  ```

  The `place` binding is never relaxed (loop skips `b.name == place`); `&!place` is a `check_reference_word` term, not a `Call`, so it never registers as a "use."

## Mechanism

1. `Liveness::scan`/`dead` (Q1) as a private struct/impl near `Scope`.
2. `check_terms` builds `let live = Liveness::scan(terms);` once, passes `&live` + `i` into each `check_term`. `check_term` gains `live: &Liveness, at: usize`. Nested bodies rebuild their own `Liveness` (D3).
3. `live_derivs`/`live_deriv`/`live_borrow_of`/`live_mutable_borrow_of` gain `(live, at)` and apply the scope-half filter; consumers pass through; `times` snapshots pass `(live, i)` to both (Q2).
4. `aliasing_origin` gains `(live, at)` and the `!live.dead(...)` clause on its name loop only (Q6/D8); stack tail (`:876`) unchanged.
5. No `Binding`/`Slot` field, no lowering touch (D1/D2/D7).

## Sanctioned files

- `src/check.rs` — `Liveness`; `live_derivs` family + `aliasing_origin` signature/filter; `check_terms`/`check_term` threading; `times` snapshot calls; new unit tests.
- `tests/phase3_refs.rs` — rewrite the two flip tests (D5); probe pair (T3/T4); D10 test (T7); alias relaxation pair (T9/T10); mutation-test assertions.
- `examples/inplace_fold.sth` — dogfood (new), at `Copy` and linear `Acc`.
- `ROADMAP.md` — mark 6f implemented; confirm slice-7 dependency note.

## Exit criteria (golden tests)

| ID | Test (file) | Kind | Phase | Source in → expected out |
|----|-------------|------|-------|--------------------------|
| T1 | `move_of_place_borrowed_in_locals_is_error` rewritten (`:750`) | reject | 1 | `&b \| r \|`, `b sink`, **then use `r`** → `cannot consume the borrowed local` + `still live` |
| T2 | `dispose_of_borrowed_place_is_error` rewritten (`:765`) | reject | 1 | same with `&!b \| r \|`, consume, then use `r` → `cannot consume the borrowed local` + `the mutable borrow taken at` |
| T3 | `borrow_via_local_dead_before_consume_is_accepted` | accept | 1 | `&!acc &!Acc>arr \| f \| f 0 >usize &!> 5 ! acc drop` → builds, prints `5` |
| T4 | `use_of_borrow_local_after_consume_is_error` | reject | 1 | `&!acc &!Acc>arr \| f \| acc drop f 0 >usize &!> 5 !` → `cannot consume the borrowed local` |
| T5 | `move_of_place_borrowed_on_stack_is_error` (`:735`) | reject | 1 | control: borrow on stack still rejected |
| T6 | `naming_a_place_after_its_borrow_ends_is_accepted` (`:1225`) | accept | 1 | borrow ends at `+!`, naming after is reuse |
| T7 | `two_sequential_mutable_borrows_first_unused_is_accepted` | accept | 1 | `&!v \| f \| &!v &!V>x 1 +!` (`f` unread) → builds; variant using `f` after → `conflicts with a live borrow` |
| T8 | `times_body_borrow_across_loop_error` (`:1311`) | reject | 1 | body genuinely leaving a borrow live still caught |
| T9 | `mutable_borrow_of_name_aliased_place_dead_is_accepted` | accept | 2 | `v v \| p q \| q V> . . &!p &!V>x 1 +!` (`q` used *before*) → builds |
| T10 | `mutable_borrow_of_name_aliased_place_live_is_error` (`:823`) | reject | 2 | same with `q` used *after* → `cannot borrow \`p\` mutably` + `aliased by \`q\`` |
| T11 | all 17 `aliasing_origin` guards still red | reject | 2 | each uses its alias name after the borrow |
| T12 | `inplace_fold_copy_lowers_without_per_iteration_blit` (`tests/phase4_slice6f.rs`) | accept+QBE | 3 | Copy `Acc`: builds, prints expected, emitted QBE loop body has **no** per-iteration `blit` |
| T13 | `inplace_fold_linear_lowers_without_per_iteration_blit` | accept+QBE | 3 | linear `Acc`: same, borrow-half only |
| T14 | ROADMAP 6f implemented; slice-7 dependency confirmed | doc | 4 | prose exit line |

### The 17 `aliasing_origin` guards (T11 / mutation surface)

Name/peek/getter (9): `..._name_aliased_place_is_error` (`:823`), `..._peek_aliased_place_...` (`:842`), `..._struct_aliased_by_peeked_field_...` (`:858`), `..._peeked_field_aliased_by_struct_...` (`:875`), `..._struct_aliased_by_gotten_field_...` (`:891`), `..._gotten_field_aliased_by_struct_...` (`:907`), `..._one_of_two_gotten_fields_from_same_struct_...` (`:940`), `..._a_place_over_duplicated_...` (`:1059`), `..._an_array_over_duplicated_...` (`:1074`).

Branch/merge (6, highest-risk, Q3/Q6): `..._by_if_join_result_...` (`:956`), `..._by_one_if_arm_only_...` (`:973`), `..._by_the_second_if_arm_only_...` (`:992`), `..._a_merge_of_two_aliased_arms_...` (`:1008`), `..._a_place_a_merge_may_denote_...` (`:1026`), `..._a_place_a_merged_peek_may_denote_...` (`:1103`).

Stack (2, untouched positional half, D2): `..._place_aliased_on_the_stack_...` (`:1151`), `..._struct_aliased_by_peek_on_the_stack_...` (`:1171`).

### Mutation-test-required criteria (D9)

Revert each guard in a **throwaway copy** (not the shared worktree).

- **M0 (T11, the alias half).** Short-circuit the **name loop** (return before the stack tail): the 15 name/peek/getter+branch/merge tests go red, the 2 stack tests stay green. Then short-circuit the **stack tail**: the 2 go red, the other 15 stay green. Any test green under the half that names it is a placebo.
- **M1 (alias last-use filter, T9/T10).** Delete `!live.dead(...)` → **T9 red**. Make `Liveness::dead` always `true` → **T10 (and several of 17) red**.
- **M2 (merge cases, Q3/Q6).** Drop the `if`-arm recursion in `scan` → at least one branch/merge guard (e.g. `..._by_one_if_arm_only_...`, `:973`) goes **red**.
- **M3 (borrow half, T3/T4).** Make the scope-half filter a no-op → **T3 red**; make it always-dead → **T4 red**. Tested, not gated to the 17's standard.
- **M4 (D6 loop carve-out).** Make `dead` also relax inherited names → a reference carried into a `times` body and used there (`reference_carried_into_loop_body_stays_live`, add if none) goes **red**, or T8 does.

## Out of scope

- **Declarable linearity** (`is_copy`, `:239`): entangled with slice 1's Copy-as-constraint and slice 8's polymorphic `drop`. Recorded in ROADMAP 6f; don't settle or foreclose.
- **Closure captures** (slice 7): this lands first so slice 7 points a settled rule at a new carrier. Slice 7 inherits an obligation: the bound-quotation capture propagation above is sound only because a quotation cannot currently escape the block that binds its captures (returning one is rejected outright, "a runtime quotation value is slice 7"). Once slice 7 makes quotations first-class values that can be returned or stored, this propagation is no longer sufficient on its own and must be revisited.
- **The `times` reference-across-back-edge rejection** (`:5589`): loop-carried reference state is a different feature; only ensure its guard still means what it says (Q2/T8).
- **What a borrow *is*:** projection, reborrow chaining, `owned_root` propagation, per-place exclusivity shape. Only *when a borrow ends* changes.
- **Either stack half** of `live_derivs` or `aliasing_origin` (D2).

## Phased delivery

**Phase 1 — Borrow half.** Add `Liveness`; thread `&Liveness`/`at` through `check_terms`→`check_term`→the `live_derivs` family; filter scope half only (D2); `times` snapshots at the `times` index (Q2); rewrite the two flip tests (T1/T2, D5); add T3/T4, T7; keep T5/T6/T8 green; verify M3/M4.

**Phase 2 — Alias half.** Add `&Liveness`/`at` to `aliasing_origin` + `!live.dead(...)` on the name loop only (Q6/D8); confirm all 17 guards red (T11); add T9/T10; drive M0/M1/M2. Reuses phase 1's `Liveness` verbatim (D8).

**Phase 3 — Dogfood + QBE goldens.** Add `examples/inplace_fold.sth`; T12/T13 build+run at Copy and linear `Acc`, assert stdout, assert no per-iteration `blit` in emitted QBE loop body.

**Phase 4 — ROADMAP.** Mark 6f implemented (`ROADMAP.md:1468`); confirm slice-7's borrow-end dependency satisfied. T14.

```json
{
  "phases": [
    { "phase": 1, "focus": "borrow half: Liveness scan/dead, thread through check_terms/check_term and the live_derivs family, filter scope half only, times snapshots at the times index, rewrite the two flip tests to use-after-consume, probe pair T3/T4, D10 exclusivity T7", "difficulty": "hard" },
    { "phase": 2, "focus": "alias half: reuse Liveness in aliasing_origin name loop only, confirm all 17 guards still red, relaxation pair T9/T10, drive M0/M1/M2 mutation callouts", "difficulty": "hard" },
    { "phase": 3, "focus": "dogfood examples/inplace_fold.sth at Copy and linear Acc plus emitted-QBE goldens asserting no per-iteration blit in the loop body", "difficulty": "hard" },
    { "phase": 4, "focus": "ROADMAP 6f marked implemented and slice 7 dependency note confirmed", "difficulty": "standard" }
  ]
}
```
