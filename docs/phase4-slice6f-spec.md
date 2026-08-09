# Phase 4 Slice 6f: liveness ends at last use (spec)

Make a reference bound to a local die at its **last use**, the way the same reference left on
the stack already does. Today `live_derivs` (`src/check.rs:759`) chains the virtual stack's
derivations with the scope's bindings, and the two halves disagree: a stack slot's derivation
dies when a term consumes its slot, a binding's derivation lives for the whole block. So
`&!acc &!Acc>arr | f | f 0 >usize &!> 5 ! acc drop` is rejected (the borrow via `f` outlives
its use) while the identical chained form compiles. This slice ends the binding's borrow at
its last use, and applies the same last-use rule to `aliasing_origin` (`:854`), which asks the
same lexical-vs-last-use question about a second *name* for a place rather than a second
borrow of it.

This is a **checker-acceptance-only** slice (D1): no new `Instr`/`Terminator`, no lowering
change, no `Type`/`IrType` change. `Deriv`/`DerivId` are confined to `src/check.rs` (recon 10),
so lowering cannot observe borrow liveness and no emitted code changes by construction. It is
**not a lifetime system** by DESIGN.md's Memory-model definition (no lifetime variables, no
regions, nothing binding a reference's validity to a named scope); it is a rule about *when a
borrow ends inside one block*, and the anonymous case already works that way (DESIGN.md,
"A named thing behaves like the anonymous one" tie-breaker).

The brief (`docs/phase4-slice6f-brief.md`) is the recon of record. This spec resolves its six
open questions concretely, carries its ten locked decisions as binding constraints, and states
exit criteria in the CLAUDE.md golden-test style. All line citations were re-verified against
`main` at `a5a4180`.

## Where the change lands (verified against current `main`)

- **The rule is one function.** `live_derivs` (`:759`) chains
  `stack.iter().filter_map(|s| s.deriv)` with
  `scope.bound.iter().filter_map(|b| b.deriv)`. Its doc comment already states the asymmetry.
  Only the **second** chain (the `scope.bound` half) becomes use-bounded; the stack half is
  already correct (D2).
- **Seven consumer sites inherit it**, through the `live_deriv`/`live_borrow_of`/
  `live_mutable_borrow_of` wrappers (`:769`/`:780`/`:793`) or `live_derivs` directly.
  **Corrected against the brief, which listed five and mislabelled one — verify this list
  before touching anything:**
  1. the **reborrow-suspension** check (`:5844`, `live_deriv` over `d.reborrow && d.place`)
     → `suspended_place_error` (`:6999`). **Structurally unaffected by this slice**: every
     scope binding's deriv is minted with `reborrow: false` (`Scope::bind`, `:583`), so the
     `scope.bound` half can never satisfy this predicate and relaxing it cannot change this
     site's answer. Do not "fix" it; do not add a test for it.
  2. the **consume** check (`:5858`, `live_borrow_of`): consuming a linear local while a borrow
     rooted at it is live → `consume_of_borrowed_place_error` (`:7013`). *The probe pair's
     rejection path (T3/T4).*
  3. the **naming-side** check (`:5881`, `live_mutable_borrow_of`, gated on
     `aliases.is_some()`): naming an aggregate while a live `&!` already reaches its storage
     → `naming_aliases_borrowed_place_error` (`:5882`). This is the **symmetric twin** of
     `aliasing_origin`; the source states the rule is "checked here *and* symmetrically at the
     naming" (`:7287`-`:7289`), so a naming that comes first is caught at `:7292` and one that
     comes later is caught here. The two are relaxed by **different halves**, which is easy to
     get wrong: this site asks whether a live mutable *borrow* reaches the place, so it
     inherits D2's borrow-half change automatically — **do not add a name filter here** — while
     `:7292` asks whether a live *name* denotes the region and is relaxed by D8's filter.
     **Do** add a criterion that the two directions still agree once both relaxations are in:
     a rule whose verdict depends on which of two lines came first is exactly the bug this
     symmetry exists to prevent.
  4. the **exclusivity** check in `check_reference_word` (`:7275`, `live_deriv` over
     `d.owned_root == rest`): a new borrow conflicting with a live borrow of the place →
     `conflicting_borrow_error`. *This is where D10 lands.*
  5. / 6. the **`times` identity snapshots** (`:6002`/`:6041`, `live_derivs` collected into a
     `HashSet<DerivId>` before/after the body splice). *The one existing consumer the change
     can silently break (Q2).*
  6. `aliasing_origin` (`:854`, sole caller `:7292`) — the sibling table (D8), which scans
     `scope.bound` for a live overlapping name and has its own untouched stack half (`:876`).
- **The position is already in hand.** `check_terms` (`:5748`) walks
  `for (i, term) in terms.iter().enumerate()`, and each `if` arm / spliced body / inlined
  quotation re-enters `check_terms` over its own term list (the monomorphic `if` arm at `:6197`
  walks each branch through `check_terms` on a cloned scope at `scope.depth()`; the `times`
  body at `:6002`; `inline_combinator`/`call`). Every block is its own `check_terms`
  invocation with its own index space.
- **The scan already exists as precedent.** `alpha_rename_locals` (`src/ast.rs:1002`) walks the
  term tree tracking which names a block binds, rewriting the `Call`s that reference them —
  and its `rename_call` (`src/ast.rs`) shows a *use* of a local is a `TermKind::Call(s)` where
  `s` is the bare name **or** a `&`/`&!`-sigilled borrow of it. `Bind`'s extent is the rest of
  its block; a nested quotation or `if` arm inherits the outer binds by value. The last-use
  scan is modelled on this, not invented.
- **No shadowing to handle** (recon 6, D4): re-binding a name in scope is rejected outright
  (`rebound_local_error`, reached from the `Bind` arm at `:5809`), so within a block a name
  maps to one binding and a last-use scan needs no stop-at-rebind rule.

## Locked decisions carried from the brief (binding constraints)

**D1 — Checker only.** No new `Instr`/`Terminator`, no lowering path, no `Type`/`IrType`
change. Edits are confined to `src/check.rs`, `tests/phase3_refs.rs`, one dogfood example, and
`ROADMAP.md` (see Sanctioned files).

**D2 — Only the scope half of each table becomes use-bounded.** In `live_derivs` the
`scope.bound` chain is filtered; the `stack.iter()` chain is left byte-for-byte. In
`aliasing_origin` the `scope.bound` name loop is filtered; the stack loop (`:876`) is left
alone. A change touching either stack half is out of scope and presumed a regression.

**D3 — Last use is per `check_terms` invocation, over the slice being walked.** A spliced
combinator body, an inlined quotation body, and an `if` arm each get their own scan over their
own term list, in their own index space. No global or cross-body index. This is what keeps the
change composable with the inliner's alpha-renaming (recon 5).

**D4 — No shadowing rule.** If implementation finds a path where a name *can* be re-bound in
scope, that is a separate defect to report, not something to handle here.

**D5 — The two flipping tests are rewritten to use-after-consume, not deleted.**
`move_of_place_borrowed_in_locals_is_error` (`tests/phase3_refs.rs:750`) and
`dispose_of_borrowed_place_is_error` (`:765`) both bind a reference local and never use it
again, so after this slice the borrow is dead and the consume is legal — they would flip to
accepting. Their intent (a held reference blocks consuming its place) is correct and kept, by
adding a use of the reference *after* the consume so the reference is genuinely live there. The
stack control `move_of_place_borrowed_on_stack_is_error` (`:735`) is untouched and must keep
rejecting.

**D6 — A loop body is not straight-line code.** A reference bound *outside* a loop and used
*inside* it is live for the whole body: iteration N's "earlier" use follows iteration N-1
reaching the body end while the reference is still live. Realized precisely by Q1's
invocation-scoping (below): the relaxation only expires bindings *created within* the current
`check_terms` invocation, so a reference inherited into a loop-body invocation is never relaxed
by it, while a body-local reference (re-minted each iteration) expires straight-line inside the
body.

**D7 — Ending a borrow disposes nothing and emits nothing.** No implicit drop is introduced,
and none may be added to make the slice work. `Scope::bind` (`:719`) registers a move-state
entry only for a linear value; `Scope::leave` (`:742`) reports a leak only where such an entry
exists; a reference carries neither. `leave_block` (`:5625`) already runs and reports nothing
for a reference local going out of scope, and `Deriv`/`DerivId` never leave `src/check.rs`
(recon 10). The slice moves only the point at which the checker *stops refusing* a consume,
never anything the compiler emits.

**D8 — `aliasing_origin`'s scope half gets the same treatment.** One last-use analysis, two
scope tables, both stack halves untouched. A name never used again does not count as a second
denoting name, exactly as a reference never used again does not count as a live borrow.
Splitting the analysis would answer one question twice.

**D9 — The alias half is mutation-tested, the borrow half is merely tested.** The failure modes
differ: a wrong answer on the borrow half accepts or rejects a *program*; a wrong answer on the
alias half silently produces a wrong *value*, the class this language exists to turn into a
compile error. Every one of the **17** `aliasing_origin` guard tests
(`tests/phase3_refs.rs`, enumerated below) must be shown to go **red** when the guard it names
is removed, not merely observed to pass. This is a phase exit criterion, not a footnote (see
Load-bearing criteria).

**D10 — The exclusivity relaxation is intended.** Two sequential mutable borrows of one place
where the first is never used again become legal (`&!v | f | &!v ...` with `f` unread). This is
not two live mutable references: `| f |` *pops* the reference into the binding table
(`stack.split_off(...)`, `:5825`), and a binding never named again is read through by nothing.
It is a visible relaxation of per-place exclusivity, recorded as deliberate and pinned with its
own test (T7).

## Resolved open questions

### Q1 — Where last use lives: a per-invocation pre-pass, keyed by names bound in that invocation

**Decision: option (c), narrowed.** At the top of each `check_terms` invocation, compute a
`Liveness` over *that invocation's own term list*; thread `&Liveness` plus the current index
`i` down through `check_term` into the six consumers. Reject options (a) and (b):

- **(a) an expiry field on `Binding`** hits an index-space wall. A `Binding.last_use` computed
  at bind time is an index in the block that bound it; when a query runs inside a *nested*
  invocation (an `if` arm, a `times` body), its current index is in a different space, so the
  two are not comparable. Working around that reintroduces exactly the invocation-scoping (c)
  gives for free, on top of a new `Binding` field that also disturbs the struct the stack-half
  machinery reads (against D2's "leave the working half alone").
- **(b) re-scan forward per query** re-walks the term subtree at every one of the six
  consumers, several of which fire per term; it is the same information (a) would cache, paid
  for repeatedly.

The pre-pass is O(size of this invocation's subtree) once per invocation, not quadratic, and
lives in the query's own index space by construction.

```rust
/// Q1/D3: the last-use index, within one `check_terms` invocation's term list,
/// of every reference/aggregate name that invocation *binds*. A name used but
/// not bound here (an outer local inherited into this block) is deliberately
/// absent, so it is never relaxed by this invocation (D6). A name is *used* by
/// a `TermKind::Call(s)` whose bare-or-sigilled target resolves to it, at any
/// depth; a use inside a nested `if` arm or quotation is attributed to the index
/// of the top-level term of *this* list that contains it (Q3's conservative max).
struct Liveness {
    last_use: HashMap<String, usize>,
}

impl Liveness {
    fn scan(terms: &[Term]) -> Self { /* mirrors alpha_rename_locals' walk */ }

    /// A binding is dead at term index `at` iff this invocation bound it and its
    /// last use is strictly before `at`. Absent (outer) names are never dead here.
    fn dead(&self, name: &str, at: usize) -> bool {
        self.last_use.get(name).is_some_and(|&last| last < at)
    }
}
```

`scan` walks the term list with a running index `i` and a `bound: HashSet<String>` of names
introduced by `Bind` terms seen so far (forward, since there is no forward reference and no
shadowing, D4). For a `TermKind::Call(s)`, it strips a leading `&!`/`&` (per `rename_call`) and,
if the result is in `bound`, records `last_use[name] = i`. For a `TermKind::If` or
`TermKind::Quotation` at index `i`, it recurses to find any `Call` of a currently-`bound` name
*inside* it and, for each found, sets `last_use[name] = max(existing, i)` — attributing the
nested use to the containing top-level index. It does **not** collect bindings from nested
bodies: those belong to the nested invocation's own `scan`.

**Filtering, in `live_derivs`.** Only the `scope.bound` chain gains the filter; the stack chain
is untouched (D2). Because `scan` keys only on names *this* invocation binds, an outer binding
inherited into the current scope is never in `last_use` and so is never relaxed here — which is
exactly D6 for loop bodies and the conservative choice inside `if` arms:

```rust
fn live_derivs<'a>(
    stack: &'a [Slot], scope: &'a Scope, live: &'a Liveness, at: usize,
) -> impl Iterator<Item = DerivId> + 'a {
    stack.iter().filter_map(|s| s.deriv)                          // stack half: unchanged (D2)
        .chain(scope.bound.iter()
            .filter(move |b| !live.dead(&b.name, at))            // scope half: use-bounded
            .filter_map(|b| b.deriv))
}
```

### Q2 — The `times` before/after snapshots evaluate at the `times` term's index

**Decision: both snapshots use the outer invocation's `Liveness` at the `times` term's index**
(recon 9). The identity check compares "the same borrows are live before and after the body".
Evaluating both `live_derivs` calls (`:6002`, `:6041`) with the same `(live, at = i)` keeps the
comparison apples-to-apples: any binding whose last use precedes the `times` term is excluded
from *both* sets equally, so relaxation cannot manufacture a false difference, while a borrow
genuinely carried across the back-edge still differs and still trips
`times_body_borrow_across_loop_error`. A test proves a body that genuinely leaves a borrow live
is still caught (T8); it is `times_body_borrow_across_loop_error`'s existing guard
(`:5589`/`tests/phase3_refs.rs:1311`), which must stay red.

### Q3 — Branches: conservative max, the `if` term's index

**Decision: max across arms.** A reference bound *before* an `if` and last used inside one arm
only expires when the `if` ends, not per-arm. This falls out of Q1 with no special case: the
outer invocation's `scan` recurses into both arms and attributes any use to the index of the
`if` term itself, so the binding's `last_use` is the `if` index and it is live through the whole
construct. Inside an arm's own invocation the outer binding is not in that arm's `Liveness`
(it was bound outside), so it is treated as live there too. Per-arm precision is not pursued: it
cannot be more permissive in any dogfood shape, and max cannot be unsound. This is the
highest-risk question because six of the 17 alias guards are merge cases (recon 12); Q6 states
why max composes with region unioning.

### Q4 — `leave_block` needs nothing

**No change, confirmed by reading it (`:5625`), not assumed.** `leave_block` calls
`scope.leave(depth)` (`:742`), which inspects only `scope.moves.states`; a reference local has
no such entry (D7), so a reference going out of scope already runs and reports nothing.
Last-use relaxation changes only the mid-block *borrow-conflict* queries, never the end-of-block
*leak* check, and it stores nothing that `leave_block` would need to retire. The `Liveness` for
a block is dropped when its `check_terms` invocation returns; there is no state to unwind.

### Q5 — REPL: unchanged, degrades sensibly

**A REPL line is one `check_terms` invocation; its `Liveness` scans that line's term list and
nothing crosses to the next line.** A reference local bound on a line with no later use has its
last use at its bind point, and simply expires at line end — which is already the behaviour
(`reference_local_expires_without_drop`, `tests/phase3_refs.rs:590`), unchanged. The one REPL
rejection, `reference_surviving_repl_line_is_error` (`:518`), is about a reference left on the
**stack** carrying into the next line (the stack half, D2, untouched), not a bound local, so it
is unaffected. A regression run of the existing session test confirms this rather than
asserting it.

### Q6 — The alias half shares the borrow half's scan, and it composes with overlap

**Decision: one `Liveness`, filtered per binding by its own name; overlap is preserved.**
`aliasing_origin` keys on alias-set *overlap*, not name identity, so the worry is a region
denoted by name A whose last use has passed while a *different* overlapping name B is still
live. Filtering each candidate binding by **its own** last use composes correctly: A is dropped
(dead), but B still overlaps `place` and is still live, so `aliasing_origin` still returns B and
still rejects. The borrow is accepted only when *no* live name overlaps — which is precisely the
hazard's absence (no live second name can observe the mutation). The filter is added to the name
loop's predicate, alongside the existing `moved_site(...).is_none()`:

```rust
.filter(|b| {
    b.name != place
        && b.aliases.is_some_and(&overlaps)
        && scope.moves.moved_site(&b.name).is_none()
        && !live.dead(&b.name, at)          // Q6/D8: a name never used again is not a second name
})
```

The merge cases (Q3) are where this is load-bearing: a merged value `p` carries *both* arms'
regions, so when the borrowed place is one arm's `v` and `p` is used after the borrow, `p` is
live and overlaps `v`, and the rejection stands (the three sampled and all six are written to
use the aliasing name after the borrow). The `place` binding itself is never relaxed here (the
loop skips `b.name == place`), and `&!place` is a `check_reference_word` term, not a `Call`, so
it never registers as a "use" that could confuse the scan — which is correct, since the borrow
we are checking *is* that use.

## The mechanism (design, not final code)

1. **`Liveness::scan` + `dead`** as in Q1, a new private struct/impl near `Scope` in
   `src/check.rs`.
2. **`check_terms`** builds `let live = Liveness::scan(terms);` once, and passes `&live` and the
   loop index `i` into each `check_term` call. `check_term` gains `live: &Liveness` and `at:
   usize` parameters (its signature is already wide; this is mechanical). Nested bodies
   re-enter `check_terms`, which rebuilds its own `Liveness` — no outer `Liveness` leaks
   downward (D3).
3. **`live_derivs`/`live_deriv`/`live_borrow_of`/`live_mutable_borrow_of`** gain `live:
   &Liveness, at: usize` and apply the scope-half filter (Q1). The six consumers pass the
   `check_term`'s `(live, at)` through. The `times` snapshots pass `(live, i)` to both calls
   (Q2).
4. **`aliasing_origin`** gains `live: &Liveness, at: usize` and adds the `!live.dead(...)`
   clause to its name-loop predicate only (Q6/D8). Its stack tail (`:876`) is unchanged.
5. **No `Binding` field, no `Slot` field, no lowering touch** (D1/D2/D7).

## Sanctioned files

- `src/check.rs` — `Liveness`; the `live_derivs` family and `aliasing_origin` signature/filter
  changes; the `check_terms`/`check_term` threading; the `times`-snapshot call updates; all new
  **unit** tests (the `#[cfg(test)] mod tests` block and its `check_src`/`run_src` helpers).
- `tests/phase3_refs.rs` — rewrite the two flip tests (D5); add the borrow-half probe pair
  (T3/T4), the D10 exclusivity test (T7), the alias-half relaxation pair (T9/T10), and the
  mutation-test coverage assertions the reviewer drives (see below). Uses the file's existing
  `check_error`/`run_src`/`run_session` harnesses.
- `examples/inplace_fold.sth` — the dogfood (new file), at both a `Copy` and a linear `Acc`.
- `ROADMAP.md` — mark 6f implemented; confirm the slice-7 dependency note.

No other files. No staged changes outside these.

## Exit criteria (golden tests)

`thing_condition_expected` naming. Compile-time checks use `check_error`/`check_src`; run
goldens use `run_src`; REPL uses `run_session`. Source in → expected output or diagnostic out.

| ID | Test (file) | Kind | Phase | Source in → expected out |
|----|-------------|------|-------|--------------------------|
| T1 | `move_of_place_borrowed_in_locals_is_error` rewritten (`tests/phase3_refs.rs:750`) | reject | 1 | bind `&b \| r \|`, `b sink`, **then use `r`** → `cannot consume the borrowed local` + `still live`, borrow now live *because used after* |
| T2 | `dispose_of_borrowed_place_is_error` rewritten (`:765`) | reject | 1 | same shape with `&!b \| r \|`, consume, then use `r` → `cannot consume the borrowed local` + `the mutable borrow taken at` |
| T3 | `borrow_via_local_dead_before_consume_is_accepted` (`tests/phase3_refs.rs`) | accept | 1 | probe B: `&!acc &!Acc>arr \| f \| f 0 >usize &!> 5 ! acc drop` → builds, prints `5` |
| T4 | `use_of_borrow_local_after_consume_is_error` (`tests/phase3_refs.rs`) | reject | 1 | probe A: `&!acc &!Acc>arr \| f \| acc drop f 0 >usize &!> 5 !` → `cannot consume the borrowed local` |
| T5 | `move_of_place_borrowed_on_stack_is_error` unchanged (`:735`) | reject | 1 | control: borrow left on stack, still rejected (regression anchor) |
| T6 | `naming_a_place_after_its_borrow_ends_is_accepted` unchanged (`:1225`) | accept | 1 | borrow ends at `+!`, naming after is reuse (regression anchor) |
| T7 | `two_sequential_mutable_borrows_first_unused_is_accepted` (`tests/phase3_refs.rs`) | accept | 1 | D10: `&!v \| f \| &!v &!V>x 1 +!` (`f` unread) → builds; a variant that *does* use `f` after → still `conflicts with a live borrow` |
| T8 | `times_body_borrow_across_loop_error` unchanged (`:1311`) | reject | 1 | Q2: a body genuinely leaving a borrow live is still caught |
| T9 | `mutable_borrow_of_name_aliased_place_dead_is_accepted` (`tests/phase3_refs.rs`) | accept | 2 | Copy `V`: `v v \| p q \| q V> . . &!p &!V>x 1 +!` (`q` used *before* the borrow) → builds |
| T10 | `mutable_borrow_of_name_aliased_place_is_error` (`:895`, unchanged) | reject | 2 | the same shape with `q` used *after* → `cannot borrow \`p\` mutably` + `aliased by \`q\`` (the T9/T10 pair is the alias analogue of T3/T4) |
| T11 | all 17 `aliasing_origin` guards still red (`tests/phase3_refs.rs`) | reject | 2 | every guard below still fails for its stated reason (each uses its alias name after the borrow) |
| T12 | `inplace_fold_copy_lowers_without_per_iteration_blit` (`tests/phase4_*.rs`) | accept+QBE | 3 | dogfood at `Copy Acc`: builds, prints expected, emitted QBE loop body has **no** per-iteration `blit` |
| T13 | `inplace_fold_linear_lowers_without_per_iteration_blit` (`tests/phase4_*.rs`) | accept+QBE | 3 | dogfood at linear `Acc`: same, borrow-half only (no aliasing rule fires) |
| T14 | ROADMAP 6f marked implemented; slice-7 dependency confirmed (`ROADMAP.md`) | doc | 4 | prose exit line; no test |

### The 17 `aliasing_origin` guards (T11 / mutation-test surface)

Name/peek/getter aliases (9): `mutable_borrow_of_name_aliased_place_is_error` (`:823`),
`..._peek_aliased_place_...` (`:842`), `..._struct_aliased_by_peeked_field_...` (`:858`),
`..._peeked_field_aliased_by_struct_...` (`:875`), `..._struct_aliased_by_gotten_field_...`
(`:891`), `..._gotten_field_aliased_by_struct_...` (`:907`),
`..._one_of_two_gotten_fields_from_same_struct_...` (`:940`),
`..._a_place_over_duplicated_...` (`:1059`), `..._an_array_over_duplicated_...` (`:1074`).

Branch/merge (6, the highest-risk group, Q3/Q6): `mutable_borrow_aliased_by_if_join_result_...`
(`:956`), `..._by_one_if_arm_only_...` (`:973`), `..._by_the_second_if_arm_only_...` (`:992`),
`..._a_merge_of_two_aliased_arms_...` (`:1008`), `..._a_place_a_merge_may_denote_...` (`:1026`),
`..._a_place_a_merged_peek_may_denote_...` (`:1103`).

Stack aliases (2, the untouched positional half, D2): `..._place_aliased_on_the_stack_...`
(`:1151`), `..._struct_aliased_by_peek_on_the_stack_...` (`:1171`).

### Load-bearing / mutation-test-required criteria (D9)

Placebo tests have shipped on this project repeatedly; a criterion not flagged here tends not to
get mutation-tested. The reviewer **must** prove each of the following can fail, by reverting
the specific guard it protects in a **throwaway copy** of the checker (not the shared worktree).

- **M0 (guards T11, the whole alias half — 17 tests).** Short-circuit `aliasing_origin`'s
  **name loop** to yield no name (return before the `stack` tail), leaving the stack half. The
  15 name/peek/getter and branch/merge tests must go red; the 2 stack tests must stay green
  (they are caught by the untouched tail). Then short-circuit the **stack** tail instead: the 2
  stack tests must go red and the other 15 stay green. Together this proves each of the 17 is
  load-bearing against the half that actually catches it. Any test that stays green under the
  half that names it is a placebo.

- **M1 (guards the alias last-use filter itself — T9/T10).** Delete the `!live.dead(...)` clause
  from `aliasing_origin`'s predicate. **T9 must go red** (`q`-dead-before-borrow no longer
  accepted — the relaxation is gone). Independently, mutate `Liveness::dead` to always return
  `true`: **T10 (and several of the 17) must go red** (a still-live aliasing name wrongly
  relaxed → hazard missed). If T9 still fails or T10 still passes under these, the filter is not
  actually wired into the alias half.

- **M2 (guards the six merge cases specifically — Q3/Q6).** In `Liveness::scan`, drop the
  recursion into `if` arms (attribute no nested use, so a name used only inside an arm looks
  dead at the `if`). At least one of the six branch/merge guards (e.g.
  `mutable_borrow_aliased_by_one_if_arm_only_is_error`, `:973`) must go **red**: the aliasing
  name, used inside an arm after the borrow, is wrongly seen as dead and the hazard is missed.
  This is the concrete proof that Q3's conservative max is implemented, not assumed.

- **M3 (guards the borrow half — T3/T4, "merely tested" per D9).** T3 and T4 are a mutation
  pair by construction (same terms, reordered): mutate the scope-half filter in `live_derivs` to
  a no-op (revert to whole-block liveness) and **T3 must go red** (probe B no longer compiles);
  mutate it to always-dead and **T4 must go red** (probe A wrongly compiles, the consume-while-
  borrowed hazard missed). D9 rates this half by program-accept/reject, not silent wrong value,
  so it is tested, not mutation-gated to the 17's standard — but the pair is cheap and stated.

- **M4 (guards D6 — the loop-body carve-out).** Change `Liveness::dead` to also relax names
  *not* bound in the current invocation (i.e. include inherited names). A test that carries a
  reference into a `times` body and uses it there (add one if none exists,
  `reference_carried_into_loop_body_stays_live`) must go **red** or the `times` identity check
  (T8) must — proving the "only names bound here" scoping is what makes the loop sound, not an
  accident.

## Phased delivery plan

Four phases; each independently green under
`cargo fmt --check && cargo clippy -- -D warnings && cargo test`. The alias half (phase 2)
reuses phase 1's `Liveness` verbatim (D8: one analysis, two tables), so it is a filter-and-tests
phase, not a second scan.

**Phase 1 — The borrow half (`live_derivs` family).** Add `Liveness` (`scan`/`dead`); thread
`&Liveness`/`at` through `check_terms`→`check_term`→the `live_derivs` family; filter the scope
half only (D2). Update the two `times` snapshots to pass `(live, i)` to both calls (Q2). Rewrite
the two flip tests to use-after-consume (T1/T2, D5). Add T3/T4 (probe pair), T7 (D10), and keep
T5/T6/T8 green. Verify M3/M4. Hard phase (the substantive change).

**Phase 2 — The alias half (`aliasing_origin`).** Add `&Liveness`/`at` to `aliasing_origin` and
the `!live.dead(...)` clause to its name loop only (Q6/D8); stack tail untouched. Confirm all 17
guards still red (T11) and add T9/T10 (the relaxation pair). Drive M0/M1/M2 mutation callouts.
Hard phase (D9's mutation-test surface concentrates here).

**Phase 3 — Dogfood + emitted-QBE goldens.** Add `examples/inplace_fold.sth` (the named-
accumulator in-place `fold`, D8/D10 payoff) and T12/T13 in the phase-4 test file: build+run at a
`Copy` and a linear `Acc`, assert stdout, and assert the emitted QBE loop body carries **no**
per-iteration `blit` (the measurable claim of the slice, checked against emitted QBE, not
asserted). Standard-to-hard: the QBE assertion is the fiddly part.

**Phase 4 — ROADMAP.** Mark slice 6f implemented (`ROADMAP.md:1468`) and confirm in prose that
slice 7's stated dependency (a settled borrow-end rule for closure captures to point at) is now
satisfied. No code; T14.

## Explicitly out of scope

- **Declarable linearity.** `is_copy` (`:239`) derives linearity structurally; opting a struct
  out is spelled "give it a `drop` overload". Real, recorded in ROADMAP 6f and DESIGN.md's
  Memory model, entangled with slice 1's `Copy`-as-constraint question and slice 8's polymorphic
  `drop`. Do not settle it here, do not foreclose it.
- **Closure captures** (slice 7): this slice lands first so slice 7 points a settled rule at a
  new carrier; nothing about capture is decided here.
- **The `times` reference-across-back-edge rejection** (`times_body_borrow_across_loop_error`,
  `:5589`): carrying a reference as loop-carried state is a different feature. This slice only
  ensures its guard still means what it says (Q2/T8).
- **Any change to what a borrow *is***: projection, reborrow chaining, `owned_root`
  propagation, or the shape of per-place exclusivity. Only *when a borrow ends* changes.
- **Either stack half** of `live_derivs` or `aliasing_origin` (D2).

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
