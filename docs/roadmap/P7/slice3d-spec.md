# Phase 7 Slice 3d: rowless quotation-consumer splice (spec)

## Goal

A quotation literal in a **non-inline** polymorphic word body has exactly one legal
consumer today: an enum eliminator (P7.S3b). This slice adds the two **rowless** consumers
that need no row (`..a`/`..b`) machinery at all:

1. `call` on a quotation **literal** written in the body: splice the literal's own body in
   place against the live poly stack, the poly analogue of the concrete path's literal
   `call` (`src/check/terms.rs:299-357`).
2. A quotation **literal** passed as an argument to a **concrete** (non-poly) `env` word
   whose declared parameter is a **ground** `Type::Quotation`: ground the literal's body
   against that declared effect and let the ordinary monomorphic call proceed.

Everything else stays rejected, unchanged: `branch`/`if`/`times`/`tag` on any quotation
(P7.S3b-follow's row-typed territory), a literal against a `Type::InlineQuotation`
parameter (unrepresentable on a non-inline word, see R6), and a literal passed to a **poly**
callee (P7.S3f's `check_poly_call` R9p phantom-`'T` hazard). This slice adds **no** row
unification, **no** mid-body `Subst`, **no** phantom-`'T` binding, and is expected to add
**no** lowering beyond what the checker fix implies (open item, OQ3 exit finding).

Probe-verified at HEAD, the two distinct rejections this slice removes (from the brief):

```sooth
: caller ( 'T: Copy 'T -- 'T ) [ ] call ;
```

```text
error: `call` on a quotation in the polymorphic body of `caller` (line 1) is not yet supported
  only an enum eliminator consumes a quotation in a generic body today (P7.S3b-follow)
```

```sooth
: helper inline ( 'T ~[ 'T -- 'T ] -- 'T ) call ;
: caller ( 'T -- 'T ) [ ] helper ;
```

```text
error: `helper` is not permitted on a quotation literal in `caller` (line 2)
```

The second reproducer above is an `inline` `~[ ]` **parameter** case and stays rejected by
R6's standing gate regardless; the concrete-helper consumer this slice delivers is the
finding-6 shape (a **concrete** `Type::Quotation` parameter, not `~[ ]`), reproduced in R4.

## Recon (verified against the source at HEAD, carried from the brief and re-checked)

The brief's recon is the authoritative source; the load-bearing anchors, re-confirmed
against `src/check/poly.rs` at HEAD:

1. **`call`'s rejection is a hardcoded name-list checked against the whole stack, not the
   operand** (`poly_call_term`, `poly.rs:914-925`):
   `if matches!(name, "call" | "branch" | "if" | "times" | "tag") && stack.iter().any(|slot|
   slot.quot.is_some())`. `call` is the one member that never needs a row: it splices the
   literal's body in place (concrete twin: `terms.rs:299-357`, which fetches
   `prov.quotations[id].body` and splices via `check_terms_relaxed` with no declared effect
   when the top is a *literal*). `branch`/`if`/`times`/`tag` are genuinely row-typed and
   keep the rejection.

2. **A second, earlier rejection blocks the concrete-helper consumer**: the operand-window
   guard (`poly.rs:966-980`) rejects a `PolyType::QuotLit` slot found in any candidate's
   `BUILTIN_TABLE` operand window unconditionally, before the `env` dispatch runs.

3. **The `env`-dispatch loop below (`poly.rs:997-1043`) also cannot ground a quotation
   operand**: its per-input match is `PolyType::Concrete(t) if t == inp`, `PolyType::Var(v)
   => …`, `other => …error`; a `QuotLit` slot falls into `other` and errors. No arm
   recognizes a declared ground `Type::Quotation` input and grounds the literal against it.
   This is the actual grounding gap the concrete-helper consumer needs.

4. **Grounding machinery for a declared quotation effect exists but is wired to the wrong
   endpoints**: `unify_poly_input`'s `PolyType::Quotation` arm (`poly.rs:2566-2604`) unifies
   a declared effect against a `Type::Quotation`/`Type::InlineQuotation` pointwise, but its
   caller `check_poly_call` (`poly.rs:2383+`) is the *concrete-word-calls-poly-callee* path
   and rejects a quotation argument up front (R9p, `poly.rs:2416-2419`,
   `reject_quotation_argument`). Its `sig.inputs[i]` are a poly callee's `PolyType`s, not a
   monomorphic candidate's `Type`s. The comparison this slice needs (a `QuotLit` slot in a
   poly body against a monomorphic candidate's ground `Type::Quotation` input) has **no**
   existing call site. Port the pointwise grounding logic; do not reuse that call path.

## The consumers (exactly two, no more)

- **C1 — `call` on a body-local literal.** `[ ] call` and `[ 1 add ]`-style literals are
  consumed by splicing the literal's body against the live poly stack. Consumption, not
  materialization: identical in kind to an eliminator arm.
- **C2 — a body-local literal passed to a concrete `env` word.** The callee's declared
  parameter must be a **ground** `Type::Quotation` (no free type variable). The literal's
  body is grounded against that declared effect; the ordinary monomorphic call then
  proceeds and truncates the operand (the literal never survives the call).

Both are cheap for the same reason: no row, no phantom-`'T`, no cross-arm join (each has
exactly one body and one continuation, unlike an eliminator's N arms; see OQ1).

## Locked rules (carried from P7.S3b, unchanged and for the same reasons)

- **L1 Splice-consumed literals only.** A quotation cannot be returned, stored, captured,
  or handed to a materializing consumer. `call` on a literal is consumption, exactly like an
  eliminator arm; C2's concrete callee consumes its operand immediately (`stack.truncate`
  before its outputs land). Nothing here weakens that boundary. A non-literal /
  abstract / erased quotation operand at either consumer is a **located rejection**, reusing
  the S3b materialization diagnostic family, **never** an inherited backend panic.
- **L2 Type variables and rows stay rigid.** No mid-body `Subst`, no row inference. This
  slice adds no row machinery. C2 grounds against a **ground** `Type::Quotation` only: if the
  candidate's declared parameter carries a free variable it is not this slice's consumer.

## Delivered shape

### R1 `call` on a literal (C1) — split the name guard

Split the hardcoded list at `poly.rs:914-925`. `call` on a `PolyType::QuotLit` operand gets
its own arm, ahead of the retained guard:

- Pop the marker slot; look up the literal's stored body via `scope.quotation(quot)` (the
  `PolyQuotLit` behind the `PolyQuotRef`).
- `poly_walk` that body **in place** against the current stack, the poly analogue of
  `check_terms_relaxed`'s splice (`terms.rs:299-357`). One body, one continuation: a
  straight-line walk, **no** per-arm clone-and-union, **no** `poly_eliminator_call` join
  (see OQ1).
- **Teardown, the poly analogue of `Scope::leave`/`leave_block` for a splice with no block
  of its own.** Before splicing, snapshot the enclosing scope's `locals`/`moves` (the keys
  already bound outside this `call`) — the same snapshot `poly_eliminator_call` takes ahead
  of each arm walk (`poly.rs:1298`, its own comment: "R3, the poly analogue of
  `Scope::leave`. The poly walk has no block scope[...] nothing removes them"). After the
  `poly_walk` above returns, reject any local still unconsumed that is not in that snapshot
  — the same leaked-arm-bound-local shape `poly_eliminator_call` already rejects, reusing
  its diagnostic rather than a fresh message (`poly_arm_local_not_consumed_error`,
  `poly.rs:3304`: "the local `{local}` of type `{ty}`, bound in an arm of `{word}` in
  `{...}` (line {N}), is never consumed"). Only then retain `locals`/`moves` back down to
  the snapshot — a **retain**, not a `Moves::join` (R3 explains why that distinction
  matters). Without this, a linear local bound inside the spliced literal leaks past `call`
  unreported, exactly as an unguarded arm would leak one past an eliminator.

  (OQ3-adjacent, recorded but not resolved here: lowering's own call-of-literal fusion
  already truncates locals at the splice point, which is *why* the checker needs this
  matching teardown — a checker that left a leaked local visible past the splice would
  disagree with what lowering already discards.)
- `call` on a **non-literal** quotation operand (an abstract/forwarded/erased quotation) is
  a **located rejection** (L1), not a splice and not a panic.

The retained guard narrows to `matches!(name, "branch" | "if" | "times" | "tag")` with the
`stack.iter().any(|slot| slot.quot.is_some())` predicate unchanged, still emitting
`poly_quotation_combinator_unsupported_error` and naming P7.S3b-follow. No name that reaches
this family may fall through to `unknown word`.

### R2 A literal passed to a concrete `env` word (C2) — carve the guard, add the grounding arm

Two edits, at the two anchors in recon 2 and 3:

- **Operand-window guard (`poly.rs:966-980`)**: a `PolyType::QuotLit` slot is **not**
  rejected here when the resolved `env` candidate for `name` is a **concrete** (non-poly)
  word whose declared input at that operand position is a **ground** `Type::Quotation`.
  Every other `QuotLit`-in-window case keeps the existing `poly_op_on_variable_error`.
- **Env-dispatch loop (`poly.rs:997-1043`)**: add one arm to the per-input match. When the
  slot is `PolyType::QuotLit` and the candidate's declared input `inp` is a ground
  `Type::Quotation(eff)`, ground the literal's body against `eff` (the ported pointwise
  logic from recon 4's `unify_poly_input::Quotation` arm — a rowless pointwise effect check,
  **no** row grounding), then let the ordinary call proceed: `stack.truncate(base)` and push
  the candidate's outputs, exactly as the existing arms do.

**Concrete-callee only, ground-`Type::Quotation` only.** Never accept against:

- `Type::InlineQuotation` (unrepresentable on a non-inline word per R6; out of scope);
- a **poly** (`PolyType`) candidate signature (P7.S3f's `check_poly_call` R9p territory:
  the phantom-`'T` hazard is real only for a poly callee with an unbound `'T`; a concrete
  callee has none, which is exactly why C2 is safe and S3f is not).

**Single-candidate (non-overloaded) concrete names only.** The grounding arm above applies
when `name` resolves the way `chosen` already requires: a lone candidate, or one exact
match among several. An **overloaded** concrete name — several candidates disagreeing in
type, one of them a ground `Type::Quotation` at this position — is out of scope for this
slice: the existing overload-selection predicate (`poly.rs:1005-1010`: `[only] =>
Some(only)`, else `all(|(s, inp)| matches!(&s.pt, PolyType::Concrete(t) if t == inp))`) can
never match a `PolyType::QuotLit` slot against `PolyType::Concrete(t)`, so an overloaded
candidate set resolves `chosen` to `None` regardless of this carve-out. Two probe-confirmed
outcomes at HEAD follow from where the literal sits in the operand window:

- If the literal is **not** the top-of-window operand, `chosen` falls through to
  `poly_delegate_op`, which also finds no match, and then to `unknown_word_error` (probed:
  a two-overload concrete `run1` with the literal as the first of two operands rejects with
  `` unknown word `run1__m0` ``). This is a **completeness gap** — a program whose intent
  is unambiguous still fails to compile — not a soundness hole: nothing is mis-selected.
- If the literal **is** the top-of-window operand, the earlier, pre-existing generic
  operand-window guard (`poly.rs:966-980`, itself unaffected by this carve-out, since the
  carve-out only fires for a single resolved concrete quotation-taking candidate) already
  catches it first and rejects with `poly_op_on_variable_error` (probed: a two-overload
  `run2` taking the literal as its sole operand rejects with `` `run2` is not permitted on
  a quotation literal in `caller` (line 3) ``) — a located rejection, not `unknown word`.

This slice does not close the first gap; both outcomes are recorded, not fixed.

### R3 No cross-arm machinery, no second join

Neither C1 nor C2 has multiple arms, so `poly_eliminator_call`'s per-arm clone, borrow-table
union, and `Moves::join` are **not** reused or modified. R1's splice teardown does reuse
one piece of that arm machinery — the `Scope::leave` analogue (a `locals`/`moves`
**retain** back down to a snapshot taken before the splice) — because a straight-line walk
with no block scope of its own leaks a spliced-in linear local exactly the way an
unconsumed arm binding would. That retain is **not** a `Moves::join`: it discards the
spliced body's own bindings against a single fixed snapshot, never reconciles two
divergent maps, so reusing it does not conflict with this section's claim below. This slice
must introduce **no** second join into `poly.rs` (the file's single borrow-table union,
over the eliminator's N arms, stays the only one; a second is a soundness regression, per
S3b-follow's L3). Confirm this at exit.

## R4 The golden

A test fixture (`tests/phase7_slice3d.rs`), not a `lib/` word. Fixtures are given below as
complete `.sth` sources, not abbreviated fragments — an abbreviated `[ 1 add ]` sketch
cannot be probed for the exact rejection a deleted guard would produce.

- **C1 behavioural (`c1_call_on_literal_splices_body_in_place`)** — a non-inline generic
  word whose literal body names a bound local (non-trivial: not `[ ] call`), run at **two
  distinct instantiations** of `'T` so it is carried rigidly rather than coincidentally
  matching:

  ```sooth
  : bump ( 'T: Copy -- 'T 'T )
    | x | [ x x ] call
  ;
  ```

  Run as `5 bump` (`'T = i64`, expect `5 5`) and `true bump` (`'T = bool`, expect
  `true true`). Mutation guard: deleting R1's `call`-on-literal arm makes this fixture fail
  to compile; confirm the mutated message is the operand-window guard's `QuotLit`
  rendering, not the C1 negative's wording below (`call` again reaches only the retained
  `branch`/`if`/`times`/`tag` guard, so the marker slot falls through to the generic
  operand-window check, which renders "a quotation **literal**" for a `QuotLit` slot,
  distinct from the C1 negative's non-literal "a quotation" wording below).

- **C1 negative (`c1_call_on_non_literal_operand_is_located_rejection`)** — `call` on a
  **non-literal** (abstract/forwarded) quotation operand in a non-inline poly body is a
  located rejection, not a panic (L1). R1's new arm reuses `poly_op_on_variable_error`'s
  renderer (the same "is not permitted on" family already used for `dup`/`over` on a
  quotation literal, `poly.rs:2946`), so the exact expected text is:

  ```text
  error: `call` is not permitted on a quotation in `<word>` (line <N>)
  ```

  never `unknown word`.

- **C2 behavioural (`c2_literal_grounds_against_concrete_quotation_param`)** — a poly body
  passing a literal to a **concrete** helper that carries real logic around its own `call`
  (finding 6: a second, unrelated argument on the same call), so the fixture is not a
  transparent wrapper that inlining `call` on a literal already covers:

  ```sooth
  : run1 ( [ i64 -- i64 ] i64 -- i64 )
    swap call
  ;

  : c2_apply_and_pass_through ( 'T: Copy -- 'T i64 )
    | x | x [ 1 add ] 2 run1
  ;
  ```

  Run as `5 c2_apply_and_pass_through` (`'T = i64`, expect `5 3`) and
  `true c2_apply_and_pass_through` (`'T = bool`, expect `true 3`) — two distinct
  instantiations of the outer `'T`, ruling out a coincidental match. Mutation guard:
  deleting R2's env-dispatch grounding arm makes this fixture fail to compile with:

  ```text
  error: `run1` is not permitted on a quotation literal in `c2_apply_and_pass_through` (line <N>)
  ```

  (`poly_op_on_variable_error` — the operand-window guard at `poly.rs:966-980` still sees
  the `QuotLit` slot once the env-dispatch carve-out is gone).

- **Negatives (each asserting the exact message text below, not merely that the build
  fails — a bare failure assertion passes identically against an unrelated `unknown word`
  fallthrough, the one regression these tests exist to catch):**

  - `branch`/`if`/`times`/`tag` on a quotation in a non-inline poly body, one test per name
    (`tag`'s guard arm is probe-confirmed reachable in S3b-follow, so it stays in the
    list), each asserting the unchanged `poly_quotation_combinator_unsupported_error`:

    ```text
    error: `<name>` on a quotation in the polymorphic body of `<word>` (line <N>) is not yet supported
      only an enum eliminator consumes a quotation in a generic body today (P7.S3b-follow)
    ```

  - A literal passed to a **poly** callee (`c2_literal_to_poly_callee_is_rejected`) still
    rejects, proving the concrete-only gate in R2 holds. **Correction (probe-verified during
    implementation):** this does not reach `check_poly_call`'s `reject_quotation_argument`
    at all — that function is the *concrete-caller-calls-poly-callee* path only
    (`Scope`/`Slot`), never invoked from `poly_call_term` (the *poly-caller* path, on
    `PolyScope`/`PolySlot`). `poly_call_term` cannot see `poly_env` (recon 1), so a poly
    callee is simply not present in `env`; the pre-existing operand-window guard rejects it
    first with the ordinary `poly_op_on_variable_error` wording:

    ```text
    error: `<word>` is not permitted on a quotation literal in `<word>` (line <N>)
    ```

    The poly→poly shape is the one that actually proves R2's concrete-only gate; the
    original wording above (a *concrete*-caller shape) would not have exercised it.

  - A **non-literal** quotation operand at `call` (C1) — see the C1 negative above.

  - **A quotation literal passed as the sole operand to an overloaded concrete name**
    (`c2_overloaded_candidate_with_quotation_literal_is_located_rejection`, R2's
    completeness-gap note) — probe-confirmed at HEAD (pre-existing, unaffected by this
    slice) that the generic operand-window guard (`poly.rs:966-980`, not R2's carve-out,
    which only fires for a single resolved concrete quotation-taking candidate) already
    catches this shape and rejects with `poly_op_on_variable_error`:

    ```text
    error: `<name>` is not permitted on a quotation literal in `<word>` (line <N>)
    ```

    never `unknown word`, never a panic. This test must **not** attempt the
    deeper-operand shape (a quotation literal at a non-top-of-window position of an
    overloaded call): that shape is the completeness gap named in R2 and currently falls
    through to `unknown_word_error` (probed: `` unknown word `run1__m0` `` for a
    two-overload `run1` with the literal as its first of two operands) — asserting a
    located rejection there would misdescribe current behaviour.

## Testing

Goldens (`tests/phase7_slice3d.rs`): the C1 and C2 behavioural fixtures above at two
instantiations; and every R4 negative, each asserting exact message text.

Unit tests beside the stage (`src/check/poly.rs`, `#[cfg(test)] mod tests`, per CLAUDE.md,
naming `thing_condition_expected`): `call` on a literal splices the body in place;
`call` on a non-literal operand is a located rejection; a `QuotLit` passed to a concrete
`env` word with a ground `Type::Quotation` input grounds and the call proceeds; a `QuotLit`
at the top-of-window operand position grounds via the carve-out (the load-bearing shape,
see the mutation-testing correction below); `poly_ground_quotation_literal`'s own three
guards each have a dedicated test (a leaked non-`Copy` local, the post-grounding local
retain, and the `eff.outputs` pointwise mismatch); the retained guard still fires for
`branch`/`if`/`times`/`tag`; a declared `~[ ]` parameter rejects at its own declaration
(R6), and even a legal `inline` word with one never reaches dispatch by name (both pinned
so this shape is never mistaken for evidence of the grounding arm's own
`Type::Quotation`-not-`Type::InlineQuotation` exclusion, which is otherwise unreachable);
against a poly candidate still rejects (see the golden negative's correction below for the
actual mechanism).

Mutation-tested guards (delete/flip the guarded code, watch the named test fail, then
restore to a clean `git status`; commit before mutation testing; a mutation copy needs
`examples/`; touch sources after any rollback):

- R1's `call`-on-literal arm (deletion → C1 fixture rejects with the located message).
- R2's env-dispatch grounding arm (deletion → C2 fixture rejects with
  `poly_op_on_variable_error`).
- R2's operand-window carve-out. **Correction (probe-verified during implementation):**
  forcing it off fails **zero** tests against the R4 C2 fixture — a non-builtin name's
  operand window is always exactly one slot (its top), and both the R4 fixture and the
  `poly_quotlit_grounds_against_concrete_quotation_param_ok` unit test park the literal
  *underneath* the window (`x [ .. ] 2 run1`), where the guard never inspects it; the
  env-dispatch loop's `other` arm produces the identical `poly_op_on_variable_error`
  message regardless of the carve-out, which is why that mutation looked covered and
  wasn't. The carve-out is only load-bearing for the quotation-*last* shape
  (`i64 [ i64 -- i64 ] -- i64`, the literal as the top-of-window operand); its own test is
  `poly_quotlit_grounds_when_it_is_the_top_of_window_operand_ok`, mutation-verified to fail
  when the carve-out is forced off.
- The retained guard's remaining names (deletion → the corresponding negative starts
  compiling or falls through to `unknown word`). **Correction:** dropping `branch` fails a
  slice3b test and dropping `times` fails a phase-2 unit test, but dropping `if` or `tag`
  failed **zero** tests until `c2_branch_if_times_tag_on_quotation_still_rejected` (this
  phase) added coverage for all four by name.

Regression, green and untouched: `tests/phase7_slice3b.rs`, `tests/phase7_slice3a.rs`, the
`tests/phase6_*` eliminator suites, `tests/qbe_baseline.rs`. Green is
`cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

## Exit findings (confirmed at implementation)

- **OQ1:** confirmed. `call`-splicing a literal (R1) is a straight-line walk with one body
  and one continuation. `poly_eliminator_call`'s own arm-cloning/union machinery is untouched
  by this slice; the file's only `Moves::join` call site remains `poly.rs:1594`, inside
  `poly_eliminator_call`. Neither C1 nor C2 adds a second join.
- **OQ3 lowering:** no new lowering work was required. Both C1 and C2 fall out of the
  existing monomorphization pass unchanged: `tests/phase7_slice3d.rs`'s C1 and C2 goldens
  build and run correctly at two instantiations of the outer `'T` with no lowering changes
  in this phase's diff.

  C2 **does** materialize the literal, once per instantiation: the C2 golden's binary
  carries `sooth_mono_c2__m0__t0_i64__quot0` and `..._bool__quot0`, and the mono body
  stores that code pointer with a null env into a `(code, env)` pair and passes it to
  `run1__m0` by value. Only the checker side (`poly_ground_quotation_literal`) walks the
  body in place; lowering treats the operand as an ordinary materialized quotation, which
  is why it needs no new lowering work rather than because nothing is materialized.

  So the capture risk this exit finding was watching for **is** reachable through C2: a
  literal capturing an aggregate local panics `an aggregate field is copied by blit, not
  scalar-stored` (`backend/qbe.rs:521`). It is **pre-existing** and shared with the
  concrete path, not introduced here — the monomorphic twin of the same program
  (`| x | x drop 1 2 P | p | [ p drop 1 add ] 2 run1`, no generics) panics identically.
  Capturing a **concrete scalar** local is fine and probe-confirmed (`| x k | x [ k add ]
  2 run1` at `'T = i64` prints `9` then `5`). Recorded as a recommendation for the phase
  that owns aggregate capture; out of scope here.

  A related, **pre-existing** gap found while probing this: a literal that captures a local
  whose type is still a bare type variable (not a concrete capture) produces a misattributed
  diagnostic rather than a located poly rejection —
  `` `add` needs 2 values, but the stack holds 0 / note: declared ( -- ) `` for
  `` : apply ( 'T: Copy -- 'T i64 ) | x | 5 [ x add ] call ; ``. This reproduces identically
  through **C1** (`call` on the literal directly, no C2 involved), so it predates this phase
  and belongs to the parse-time literal-effect inference C1 already relies on, not to R2's
  carve-out or grounding arm. Recorded as a recommendation for a future phase; not fixed
  here.
- **R2's single-candidate gate is symbol-based, not just arity-based:** `env` recording one
  candidate for a name is not enough to ground through it. `ast::overload_symbols` suffixes
  a concrete word's mangled symbol (`run1__m0$$0`) merely for *sharing its surface name*
  with an unrelated **poly** word, while `env["run1__m0"]` still holds it as that name's sole
  candidate. Grounding through such a candidate records no `builtin_overloads` entry (the
  record is `exact`-gated, and a `QuotLit` operand is never `PolyType::Concrete`), so
  lowering is left to resolve a bare, un-mangled name and panics at `calls.rs:685` — an
  inherited backend panic, which L1 forbids. The **grounding arm** therefore also requires
  `chosen.symbol == name`, falling such a call through to the ordinary located rejection
  (pinned by `c2_literal_to_name_shared_with_a_poly_word_is_located_rejection`). The same
  conjunct on the operand-window carve-out's `single_candidate` is *redundant* and was not
  kept: mutation-testing it kills no test, and probing shows why — whether the carve-out
  admits the slot or not, the call still lands on either the grounding arm's `other` branch
  or `poly_delegate_op`, both of which render the identical located rejection (checked for a
  suffixed name, an unsuffixed name, and a **builtin** name, where a `QuotLit` makes `exact`
  false and skips the dispatch block entirely).

  Actually *supporting* that shape is a **future-phase lowering item**, not a checker one:
  recording the `$$`-suffixed symbol makes the checker accept it, but lowering then emits
  invalid QBE for the quotation argument. Deferred with the rest of the OQ3 lowering work.
- **`poly.rs` split signals, re-run:** at P7.S3b's deferred split, 3 of 5 signals fired and
  both candidate splits (`poly/diagnostics.rs`, `poly/eliminator.rs`) were rejected as wrong
  splits, with the deferral's own stated expiry condition being "a second quotation
  consumer lands" (`project_poly_rs_split_deferred`). This slice adds exactly that — C1 and
  C2 are a second and third consumer beyond the eliminator — so the condition has fired.
  Re-running the signals against `poly.rs` as it now stands (5945 lines, still a single
  `use super::*`, no circular dependency): the same 3 of 5 fire, and the two previously
  rejected splits are still wrong for the same reasons (a layer-shaped `diagnostics.rs` has
  no precedent elsewhere in the checker; `poly_call_term` → `poly_eliminator_call` →
  `poly_walk` → `poly_call_term` is still one mutually-recursive walk, and C1/C2 hang off
  `poly_call_term` directly, so a split drawn around the eliminator alone would not even
  capture the new consumers). **Decision: still defer.** The file has grown, not
  reorganized into separable responsibilities; a split is recommended once an actual
  distinct-module boundary appears (e.g. P7.S3b-follow's real row-typed
  `branch`/`if`/`times`/`tag` implementation, not just the deferred-rejection stub that
  exists today), not as a line-count response.
- **Borrow join uniqueness:** confirmed. `grep -n "Moves::join"` finds one call site,
  `poly.rs:1594`, inside `poly_eliminator_call`. R1's splice teardown and R2's grounding
  teardown both use a `retain`-to-snapshot, never a join (R3).

## Out of scope

- `branch`/`if`/`times`/`tag` on any quotation (row-typed or not) — P7.S3b-follow.
- A `~[ ]` (`InlineQuotation`) parameter on a non-inline word — a standing gate this slice
  does not touch (R6): a non-inline word cannot declare a `~[ ]` parameter, which is what
  keeps `sort`/`bin_search`'s comparator *parameter* out of reach. The only way a rowless
  quotation reaches a non-inline poly body is as a **literal** written in the body.
- Passing a literal (or an abstract quotation parameter) to a **poly** callee — P7.S3f's
  `check_poly_call` R9p phantom-`'T` hazard. C2's carve-out is concrete-callee only.
- Trait bounds (P7.S3e) and self-recursion (P7.S3g) — no interaction traced.
- Row unification, mid-body `Subst`, phantom-`'T` binding (L2).
- Any lowering change beyond what falls out of the checker fix, pending the OQ3/R7 exit
  finding.

## R6 The standing `~[ ]`-parameter gate (recorded, untouched)

A **non-inline** word cannot declare a `~[ ]` (`InlineQuotation`) parameter (probe-verified:
`` word `X` declares an inline-quotation parameter … but is not `inline` ``). This slice does
not lift that gate; `sort`/`bin_search`'s `~[ 'T 'T -- i64 ]` comparator parameters stay
`inline`-only and non-monomorphizable after this slice. This is why the C2 consumer is a
literal passed to a concrete `Type::Quotation` parameter, never a `~[ ]` parameter.

## References

- `docs/roadmap/P7/slice3d-brief.md` (probe-grounded brief; the authoritative recon and the
  finding-6 concrete-helper carve-out)
- `docs/roadmap/P7/slice3b-follow-spec.md` (the row-typed consumers this slice's rejections
  defer to; the two-guard structure at `poly.rs:922`/`:1997`)
- `docs/roadmap/P7/slice3c-spec.md` (format precedent)
- `docs/roadmap/P7-language-prereqs.md` (P7.S3f R9p phantom-`'T`; P7.S3g self-recursion)

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "C1: split the poly.rs:914-925 name guard so `call` on a quotation literal splices the literal's body in place via poly_walk against the live stack (poly analogue of terms.rs:299-357), with R1's snapshot/retain teardown rejecting a leaked arm-bound linear local; non-literal `call` operand stays a located rejection; retained guard narrows to branch/if/times/tag unchanged. Full C1 coverage lands here: poly.rs unit tests for the splice and the non-literal rejection, the C1 behavioural golden at two instantiations, and C1's own negative",
      "effort": "S",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "C2: carve the operand-window guard (poly.rs:966-980) and add one env-dispatch arm (poly.rs:997-1043) grounding a QuotLit against a concrete, single-candidate (non-overloaded) declared ground Type::Quotation input (ported from unify_poly_input's Quotation arm, rowless); never Type::InlineQuotation, a poly candidate, or an overloaded name. C2's own coverage plus the shared negatives: poly.rs unit tests, the non-wrapper C2 behavioural golden at two instantiations, the branch/if/times/tag and poly-callee negatives, the new overloaded-candidate-with-quotation-literal negative (R2's completeness gap), all mutation tests for both phases, and exit findings incl. the OQ3 lowering confirmation and the poly.rs split re-run",
      "effort": "S",
      "difficulty": "standard"
    }
  ]
}
```
