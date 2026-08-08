# Phase 4 Slice 6e: `if` in a polymorphic body (spec)

Lift the unconditional rejection of `if` inside a non-combinator polymorphic body so a
polymorphic word may branch. This is a **checker-acceptance-only** slice: no new `Instr`,
no new `Terminator`, no lowering path (D1). The concrete instantiation path is already
correct once the poly-walk stops rejecting the body outright; lowering happens later, per
concrete instantiation, through the existing `check_word` path.

The brief (`docs/phase4-slice6e-brief.md`) is the recon of record. This spec resolves its
four open questions concretely, encodes its five locked decisions as binding constraints,
and states exit criteria in the CLAUDE.md golden-test style.

All line citations below were re-verified against current `main` (post-6d-merge). Where the
brief's numbers had drifted, the current number is used; the one drift found was the existing
rejection test (`src/check.rs:8470`, brief said `:8469`).

## Where the change lands (verified against current `main`)

- `check` dispatches each polymorphic word by `is_combinator` (`src/check.rs:1385`): a
  combinator (a poly word with a quotation parameter) goes to
  `check_poly_combinator_standalone` (`:1400`); every other poly word goes to
  `check_poly_body` (`:1415`), which walks a `Vec<PolyType>` stack via `poly_walk` →
  `poly_term`.
- The rejection is a single unconditional early-out: the `TermKind::If { .. }` arm of
  `poly_term` returns the "not yet supported" error (message at `src/check.rs:3674`) before
  touching the stack, scope, or either branch. Clean insertion point.
- The monomorphic `if` arm this ports from is `check_term`'s `TermKind::If` case
  (`src/check.rs:6137`): pop condition, `Bool`-check, clone scope into `then_scope`/
  `else_scope`, walk each branch over a cloned stack, `leave_block` each arm
  (`:5566`), compare residual stack lengths/types
  (`branch_mismatch_error` `:5610`, `branch_type_mismatch_error` `:5622`), then
  `scope.moves = Moves::join(then_scope.moves, else_scope.moves)`.
- Move-state machinery to reuse: `MoveState` (`src/check.rs:597`), `Moves` (`:607`) with
  `take` (`:615`), `moved_site` (`:629`), `unconsumed` (`:639`), and `Moves::join` (in the
  `Moves` impl at `:607`, the `fn join` that maps disagreement to `MaybeMoved`).
- `PolyType` derives `PartialEq, Eq` (`src/ast.rs:513`), so per-slot type comparison at the
  join is a direct `==`, no new comparison logic.
- `PolyScope` (`src/check.rs:3458`): `locals: HashMap<String, PolyType>` and
  `moves: HashMap<String, Option<Span>>` (`:3460`), with a name-sorted `unconsumed()`
  (`:3466`). The poly binder inserts `None` for a non-`Copy` local (`:3661`); the poly local
  read consumes it by inserting `Some(span)` and rejects a second read via
  `poly_use_after_move_error` (`:3719`-`:3723`, error fn at `:4222`).

## Locked decisions carried from the brief (binding constraints)

**D1 — No new IR, checker only.** No new `Instr`/`Terminator`, no lowering path.
`poly_term`'s job is acceptance/rejection. This slice edits `src/check.rs` only, plus one
test file, `tests/phase4_generics.rs`, `ROADMAP.md`, and one dogfood example.
`PolyType`/`PolySig`/`apply_subst`/monomorphization are untouched.

**D2 — Three move states, not a boolean.** `PolyScope`'s move tracking gains the three
`MoveState` cases (live / moved-on-both-arms / moved-on-exactly-one-arm). See Q1 below for
the concrete realization (reuse the existing `Moves`).

**D3 — No quotation machinery on the poly side.** A `PolyType` value is provably never a
quotation: the literal that would produce one is rejected eagerly at `poly_term`'s
`TermKind::Quotation` arm (`src/check.rs:3692`), upstream of any `if`. Therefore the poly
condition-pop must **not** port `reject_quotation_operand` (`:6774`), and the poly join must
**not** port `different_quotations_at_join_error` (`:6814`) or
`quotation_versus_value_at_join_error` (`:6825`). They would be dead code by construction.
The poly `if` join is strictly simpler than its monomorphic sibling on exactly this axis.

**D4 — Nested `if` falls out of recursion.** `poly_term`'s new `if` arm recurses through
`poly_walk` into each branch; an inner `if` is just another term the recursive walk
dispatches through the same `poly_term` match. No dedicated "nested poly-if" mechanism. One
dogfood example nests, to check the claim rather than assume it.

**D5 — The rejection test is rewritten, not duplicated.**
`check_poly_body_with_if_is_rejected` (`src/check.rs:8470`) becomes the primary **acceptance**
test for `choose`, renamed to say so (see criterion T1). It is not left beside a new test.

## Resolved open questions

### Q1 — How `PolyScope` tracks a local bound inside one `if` arm only

**Decision: reuse the monomorphic `Moves` wholesale for move state, and add a keys-snapshot
`leave_arm` for arm-local scoping. No ordered `Vec`.**

Two coupled changes to `PolyScope` (`src/check.rs:3458`):

1. **Move state.** Change `moves: HashMap<String, Option<Span>>` to `moves: Moves` (the
   existing struct at `:607`). This is the D2-sanctioned "reuse `MoveState`" path — `Moves`
   is exactly a newtype around `HashMap<String, MoveState>` and already carries the three
   states, `take`, `moved_site`, `unconsumed`, and `join`. Reusing it makes the poly move
   discipline structurally identical to the monomorphic side rather than a parallel
   invention (the brief and CLAUDE.md both want this). Downstream edits this forces, all
   mechanical:
   - `PolyScope::unconsumed()` (`:3466`) delegates to `self.moves.unconsumed()`.
   - The poly binder (`:3661`) inserts `MoveState::Live` (via `self.moves.states.insert`)
     instead of `None` for a non-`Copy` local.
   - The poly local read (`:3719`-`:3723`) becomes
     `scope.moves.take(name, span).map_err(|site| poly_use_after_move_error(ctx, span, name, site))?`.
     `take` returns `Ok(())` for an absent (`Copy`) local — a no-op — and `Err(site)` for a
     `Moved`/`MaybeMoved` one, exactly the current behaviour.
   - `check_poly_body`'s final leak check (in `:3565`-`:3601`) reads `scope.unconsumed()`
     unchanged; it now correctly counts a `MaybeMoved` local (consumed on one arm only) as
     still-leaked, which is the whole point of D2.

2. **Arm scoping.** Add two methods to `PolyScope`, mirroring `Scope::depth`/`leave_block`
   but keyed by name rather than depth:

   ```rust
   impl PolyScope {
       /// The names bound before an `if` arm is walked; a name absent from
       /// this set after the arm was bound inside it.
       fn snapshot(&self) -> HashSet<String> {
           self.locals.keys().cloned().collect()
       }

       /// leave_block's poly twin: reject an arm-local non-`Copy` value never
       /// consumed inside the arm, then drop every arm-local from scope so the
       /// two arms' name sets agree at the join. `token` names the arm's
       /// closing keyword ("else" or "end") for the diagnostic.
       fn leave_arm(&mut self, before: &HashSet<String>, token: &str) -> Result<(), String>;
   }
   ```

   `leave_arm` computes the arm-local names (`locals.keys()` not in `before`), name-sorted;
   for the first that is still in `self.moves.unconsumed()` it returns
   `poly_arm_local_unconsumed_error` (Q2); then it removes **every** arm-local name from both
   `locals` and `moves.states` and returns `Ok(())`. The removal happens whether or not a
   leak fired earlier in the sort order, so a successful arm always leaves the pre-`if` name
   set behind and `Moves::join` (which indexes `else_arm.states[name]`) cannot panic on a
   key mismatch — the same invariant `leave_block` upholds for the monomorphic side.

**Why keys-snapshot, not an ordered `Vec` matching `Scope.bound`.** `PolyScope` has no depth
concept today and reports leaks name-sorted (`unconsumed()` sorts), so it has no ordering
semantics to preserve. A keys-snapshot is the minimal faithful port and matches the
existing sorted-reporting convention. An ordered `Vec<Binding>` twin would import `Scope`'s
depth machinery that nothing else in `PolyScope` uses, for a strictly larger diff and no
behavioural gain — against the craft-project "smallest change that fits the existing style"
rule. A value bound inside one arm cannot become `MaybeMoved` (it does not exist in the
other arm), so the arm-local leak is always a plain unconsumed, never a maybe-moved; the
snapshot approach needs no `every_path` distinction.

### Q2 — Naming and signatures of the new error functions

**Decision: three new `poly_`-prefixed functions in the `ctx`/`span` family** (beside
`poly_use_after_move_error` at `:4222`), **not** the `word`/`sig` family
(`poly_local_unconsumed_error` `:4204`, `poly_output_mismatch_error` `:4302`) and **not**
generalizations of the monomorphic `branch_*_error`.

Rationale: an `if`-arm error fires mid-body at the `if`'s `span`, with a `Ctx` in hand
(`poly_term` takes `ctx: &Ctx`), exactly like `poly_use_after_move_error`. The `word`/`sig`
family exists only for word-boundary errors that have no span. And the monomorphic
`branch_*_error` cannot simply be reused because they take `Type`, whereas the poly join
compares `PolyType`; the `&PolyType` argument is precisely why a new function is needed
rather than a call into the existing one. `Ctx::Word` already embeds the word name, so the
messages still name the word.

```rust
fn poly_branch_mismatch_error(ctx: &Ctx, span: Span, d_then: usize, d_else: usize) -> String;
fn poly_branch_type_mismatch_error(ctx: &Ctx, span: Span, t_then: &PolyType, t_else: &PolyType) -> String;
fn poly_arm_local_unconsumed_error(ctx: &Ctx, span: Span, local: &str, pt: &PolyType, token: &str) -> String;
```

Each has a `Ctx::Word` phrasing (naming the word and line) and a `Ctx::Line` phrasing, the
two-arm shape every error fn in this module already uses. Message text mirrors the
monomorphic siblings: "`if` branches leave different stack depths", "`if` branches leave
different types", and for the arm-local "local `{local}` (`{pt}`) bound in the `{token}` arm
is never consumed in it".

### Q3 — The second concrete type for the "two instantiations" test

**Decision: `f64`.** `mymax`'s body needs `'T: Copy Ord`. `is_ord` (`src/check.rs:3427`) is
`Type::is_numeric`, and `is_copy` (`:239`) is true for `f64`, so `f64` satisfies both bounds
and is the minimal type distinct from `i64` in the emitted comparison and the printed
result. A user struct is explicitly **not** a candidate: `is_ord` admits only the numeric
tower, so no struct can instantiate an `Ord`-bounded body — the struct alternative the brief
floated is ruled out for `mymax` on that ground, not just for convenience. `choose` is
unbounded (`( 'T 'T bool -- 'T )`) so it instantiates at both `i64` and `f64` as well. The
matrix is therefore `{i64, f64}` for both words, deterministic.

### Q4 — Does `check_poly_combinator_standalone` need any change

**No change, confirmed by reading `check_poly_combinator_standalone` (`src/check.rs:3493`)
and the dispatch (`:1385`), not asserted.** The dispatch routes a poly *combinator* (a poly
word with a quotation parameter, `is_combinator`) to `check_poly_combinator_standalone`
(`:1400`), which builds a concrete `WordDef` (every type variable → `i64`, length variables
→ `STANDALONE_LEN`) and calls the **monomorphic** `check_word` on it. `check_word` runs
`check_term`'s own `TermKind::If` arm (`:6137`), which already handles `if` fully. It never
touches `poly_term`/`poly_walk`. Consequences:

- An `if`-bearing combinator body was already accepted before this slice, via the
  monomorphic arm on the `i64` stand-in; 6e neither changes nor regresses that path.
- 6e's change is confined to the non-combinator poly path
  (`check_poly_body`/`poly_walk`/`poly_term`), which is exactly where the rejection lives.

## The new `poly_term` `if` arm (design, not final code)

Replaces the early-out at `src/check.rs:3674`. Destructures
`TermKind::If { then_branch, else_branch, else_span, end_span }` (fields per
`src/ast.rs:976`). Mirrors the monomorphic arm minus all quotation handling (D3):

1. Pop the condition; underflow → `underflow_error(ctx, span, "if", 1, 0)`.
2. Require it be `PolyType::Concrete(Type::Bool)`; otherwise `type_mismatch_error`. **No**
   `reject_quotation_operand` guard (D3): a `PolyType` is never a quotation.
3. `let before = scope.snapshot();` then clone `scope` into `then_scope`/`else_scope` and the
   stack into each arm.
4. `poly_walk(then_branch, stack.clone(), &mut then_scope, …)` →
   `then_scope.leave_arm(&before, then_token)` where `then_token` is `"else"` when
   `else_span.is_some()` else `"end"`.
5. `poly_walk(else_branch, stack, &mut else_scope, …)` →
   `else_scope.leave_arm(&before, "end")`.
6. `scope.moves = Moves::join(then_scope.moves, else_scope.moves);`
7. Length check: `then_stack.len() != else_stack.len()` → `poly_branch_mismatch_error`.
8. Per-slot: zip; `t_then != t_else` (direct `PolyType` `==`, `src/ast.rs:513`) →
   `poly_branch_type_mismatch_error`. **No** quotation-identity case and **no** deriv/place
   merge (those are monomorphic-only, borrow-provenance and quotation concerns absent from
   `PolyType`).
9. Return the merged residual stack (either arm's; they are equal by step 7-8).

Recursion (step 4/5 into `poly_walk`) gives nested `if` with no special case (D4).

## Sanctioned files

This is a checker-acceptance-only slice (D1). Edits are confined to:

- `src/check.rs` — the `poly_term` `if` arm, the `PolyScope` move-state/`leave_arm` changes,
  the three new error functions, and all new **unit** tests (the `#[cfg(test)] mod tests`
  block begins at `src/check.rs:7742`; `check_src` helper at `:7747`; the rewritten
  acceptance test at `:8470`). Confirmed: the existing rejection test lives in a
  `#[cfg(test)] mod tests` **inside `src/check.rs` itself**, not a separate `tests/` file.
- `tests/phase4_generics.rs` — the end-to-end run-at-two-instantiations goldens (uses the
  file's existing `run_src` compile-and-run harness).
- `examples/poly_if.sth` — the nested-`if` dogfood (new file).
- `ROADMAP.md` — mark 6e implemented and confirm the slice-7 dependency note.

No other files. No staged changes outside these.

## Exit criteria (golden tests)

`thing_condition_expected` naming; unit tests use `check_src` (compile-time) in
`src/check.rs`; end-to-end tests use `run_src` (build-and-run) in `tests/phase4_generics.rs`.
Source in → expected output or diagnostic out.

| ID | Test (file) | Kind | Phase | Source in → expected out |
|----|-------------|------|-------|--------------------------|
| T1 | `check_poly_body_with_if_accepts_choose` (`src/check.rs`) — rewrite of `check_poly_body_with_if_is_rejected` | unit accept | 1 | `: choose ( 'T 'T bool -- 'T ) \| a b flag \| flag if a b drop else b a drop end ;` + `main` → `check_src(...).is_ok()` |
| T2 | `check_poly_arm_local_unconsumed_is_error` (`src/check.rs`) | unit reject | 1 | `: arm_leak ( 'T 'T bool -- 'T ) \| a b flag \| flag if a b \| y \| else a drop b end ;` → err naming `y`, "never consumed" |
| T3 | `check_poly_if_moved_on_both_arms_is_accepted` (`src/check.rs`) | unit accept | 1 | `: both ( 'T 'T bool -- ) \| a b flag \| flag if a drop b drop else b drop a drop end ;` → ok |
| T4 | `check_poly_if_moved_on_one_arm_leaks` (`src/check.rs`) | unit reject | 1 | `: one ( 'T bool -- ) \| x flag \| flag if x drop else end ;` → err naming `x` (leak) |
| T5 | `check_poly_if_moved_on_neither_arm_leaks` (`src/check.rs`) | unit reject | 1 | `: none ( 'T bool -- ) \| x flag \| flag if else end ;` → err naming `x` (leak) |
| T6 | `check_poly_if_condition_not_bool_is_error` (`src/check.rs`) | unit reject | 1 | `: bad ( 'T 'T -- 'T ) if drop else drop end ;` → type-mismatch, `if` wants `Bool` |
| T7 | `check_poly_if_branch_depth_mismatch_is_error` (`src/check.rs`) | unit reject | 1 | `: bad ( 'T: Copy bool -- 'T ) \| x flag \| flag if x else x x end ;` → branch-depth mismatch (**not** use-after-move: `x` needs the `Copy` bound the brief's example omitted, or the else-arm's second `x` read hits `poly_use_after_move_error` first since an unbounded `'T` is linear, masking the depth check this test exists to prove) |
| T8 | `check_poly_if_use_after_join_is_error` (`src/check.rs`) | unit reject | 1 | `: bad ( 'T bool -- ) \| x flag \| flag if x drop else x drop end x drop ;` → both arms consume `x` (join: Moved), the `x drop` after `end` is a second read → use-after-move naming it |
| T9 | `poly_mymax_runs_at_i64_and_f64` (`tests/phase4_generics.rs`) | e2e accept | 2 | `mymax` at `i64` (`3 7 mymax`→`7`) and `f64` (`3.0 7.0 mymax`→`7`) in one program |
| T10 | `poly_choose_runs_at_i64_and_f64` (`tests/phase4_generics.rs`) | e2e accept | 2 | `choose` at `i64` and `f64`, each prints the kept operand |
| T11 | `poly_nested_if_dogfood_runs` (`tests/phase4_generics.rs`) | e2e accept | 2 | build+run `examples/poly_if.sth` (nested-`if` `mymax3`) → expected line(s) |
| T12 | ROADMAP 6e marked implemented; slice-7 dependency note confirmed (`ROADMAP.md`) | doc | 3 | prose exit line satisfied; no test |

### Load-bearing / mutation-test-required criteria

Per this project's convention (placebo tests have shipped repeatedly; a criterion not flagged
here tends not to get mutation-tested during review), the reviewer **must** prove each of the
following can fail by reverting the specific guard it protects. Each mutation is done in a
throwaway copy of the checker, not on the shared worktree.

- **M1 (guards T1) — the join must not treat per-arm-different-order consumption as a leak.**
  `choose` consumes `a` and `b` on both arms but at different sites. Mutate `Moves::join`'s
  `(Moved, Moved) => Moved` case to yield `MaybeMoved`. T1 must then **fail** (`choose` no
  longer compiles: `a`/`b` read as leaked at the word end). This is the exact regression to
  slice 1's half-built behaviour the brief warns of. If T1 still passes after the mutation,
  T1 is a placebo.

- **M2 (guards T2) — the per-arm leak check must actually fire.** Delete the leak-detection
  branch inside `PolyScope::leave_arm` (keep the arm-local *removal*, so the join still does
  not panic). T2 must then **fail to fail**: `arm_leak` compiles when it should not
  (`y`'s leak goes unreported). If T2 still errors after the mutation, T2 is a placebo.

- **M3 (guards T3/T4/T5) — the three-state join must neither over- nor under-approximate.**
  Three distinct programs, three distinct outcomes: `both` (Moved+Moved stays Moved → no
  leak, T3 accepts), `one` (Moved+Live → MaybeMoved → leak, T4 rejects), `none` (Live+Live →
  leak, T5 rejects). Mutate the disagreement case of `Moves::join` to yield `Moved` (instead
  of `MaybeMoved`): T4 must then **fail to fail** (`one` wrongly compiles). Mutate
  `Moves::unconsumed` to exclude `MaybeMoved`: T4 must again **fail to fail**. T3 must keep
  passing through both mutations (proving it is not just accepting everything).

- **M4 (guards T5) — `none`'s leak is not incidentally covered by M3.** T5's local (`x`) is
  untouched on both arms (`Live`+`Live`), so neither of M3's two mutations (disagreement→
  `Moved`, or `unconsumed` excluding `MaybeMoved`) can break it: `Live`+`Live` is not a
  disagreement and is never `MaybeMoved`. T5 needs its own mutation: change `Moves::join`'s
  `(Live, Live) => Live` case to `(Live, Live) => Moved`. T5 must then **fail to fail**
  (`none` wrongly compiles, `x`'s leak goes unreported). Without M4, T5 has no mutation
  proving it can fail and is not load-bearing in practice despite the table's flag.

## Phased delivery plan

Three small phases. Note a deliberate divergence from the brief's suggested phase-1/2 split:
the brief floated "PolyScope move-state upgrade + per-arm leak machinery as a standalone
unit" in phase 1 and the `if`-arm wiring in phase 2. But the move-state and `leave_arm`
machinery have **no source-level entry point except the `if` arm** — testing them
"standalone" would mean unit tests against private functions with hand-built `PolyScope`
values, which is precisely the placebo-prone shape this project keeps getting burned by.
So phase 1 lands the machinery **and** the `if` arm together as one cohesive checker unit,
every test of which drives real Sooth source through `check_src`. Phases stay small; each is
independently green under `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

**Phase 1 — Poly-`if` checker arm (the whole `src/check.rs` change).** Reuse `Moves` in
`PolyScope` (moves field, binder, read, `unconsumed`); add `PolyScope::snapshot`/`leave_arm`;
add `poly_branch_mismatch_error`/`poly_branch_type_mismatch_error`/
`poly_arm_local_unconsumed_error`; replace the `poly_term` `if` early-out (`:3674`) with the
full arm. Rewrite `check_poly_body_with_if_is_rejected` (`:8470`) into
`check_poly_body_with_if_accepts_choose` (T1). Add unit tests T2-T8. Green on `cargo test`
(unit only). Mutation callouts M1-M3 verified. Hard phase (the substantive change).

**Phase 2 — End-to-end goldens and dogfood.** In `tests/phase4_generics.rs` add T9/T10
(run `mymax`/`choose` at `i64` and `f64` via `run_src`, assert stdout) and T11 (build+run
`examples/poly_if.sth`). Create `examples/poly_if.sth` with `mymax`, `choose`, and a
nested-`if` `mymax3 ( 'T: Copy Ord 'T 'T 'T -- 'T )` (D4 proof), plus a `main` that prints at
both instantiations. Green on full `cargo test`.

**Phase 3 — ROADMAP.** Mark slice 6e implemented in `ROADMAP.md` (the "6e —" entry near
line 1391 and its exit line), and confirm in prose that slice 7's stated dependency on the
polymorphic `if` is now satisfied. No code; T12.

## Explicitly out of scope

- Moving `max`/`mymax` from `BUILTIN_WORDS` into the library (a separate decision; this slice
  only removes the compiler-side wall).
- The quotation-in-a-polymorphic-body rejection (`src/check.rs:3692`) — slice 7's wall.
- Any change to `PolyType`, `PolySig`, `apply_subst`, or monomorphization.
- `check_poly_combinator_standalone` (`:3493`) — unaffected (Q4).
- `while`/`times` interaction inside a poly `if` arm: recon found no blocked or special-cased
  interaction, and the recursive `poly_walk` dispatches a loop term through its existing
  `poly_term` arm unchanged; if implementation surfaces one, call it out rather than silently
  handle it.

```json
{
  "phases": [
    { "phase": 1, "focus": "poly-if checker arm: PolyScope Moves reuse, leave_arm, join, three new errors, unit tests T1-T8", "difficulty": "hard" },
    { "phase": 2, "focus": "end-to-end run goldens at i64 and f64 for mymax and choose plus nested-if dogfood example", "difficulty": "standard" },
    { "phase": 3, "focus": "ROADMAP 6e marked implemented and slice 7 dependency note confirmed", "difficulty": "standard" }
  ]
}
```
