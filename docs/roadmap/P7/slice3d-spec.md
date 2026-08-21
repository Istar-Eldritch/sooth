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

### R3 No cross-arm machinery, no join touched

Neither C1 nor C2 has multiple arms, so `poly_eliminator_call`'s per-arm clone, borrow-table
union, `Scope::leave` analogue, and `Moves::join` are **not** reused or modified. This slice
must introduce **no** second join into `poly.rs` (the file's single borrow-table union stays
the only one; a second is a soundness regression, per S3b-follow's L3). Confirm this at exit.

## R4 The golden

A test fixture (`tests/phase7_slice3d.rs`), not a `lib/` word.

- **C1 behavioural** — a non-inline generic word with `[ ] call` and a **non-trivial**
  literal body (e.g. `[ 1 add ]`, or a body that names/consumes a bound local), compiling
  and running to the correct result at **two instantiations** so `'T` is carried rigidly
  rather than coincidentally matching. Mutation guard: deleting R1's `call`-on-literal arm
  makes the fixture fail with the located rejection.
- **C2 behavioural** — a poly body passing a literal to a **concrete** helper that carries
  **real logic** around its own `call` (per the brief's finding 6: a second, unrelated
  argument on the same call, or composing/side-effecting), so the fixture is **not** a
  transparent wrapper that inlining `call` on a literal already covers. Probe from the
  brief for the helper shape:

  ```sooth
  : run1 ( [ i64 -- i64 ] i64 -- i64 ) swap call ;
  ```

  called as `dup [ 1 add ] 2 run1` from inside a generic body, with the second argument
  ruling out the transparent-wrapper placebo. Mutation guard: deleting R2's env-dispatch
  grounding arm makes the fixture fail with `poly_op_on_variable_error`.
- **Negatives (each asserting the exact message text, not merely that the build fails —
  a bare failure assertion passes identically against an `unknown word` fallthrough, the
  one regression these tests exist to catch):**
  - `branch`/`if`/`times`/`tag` on a quotation in a non-inline poly body still reject with
    the unchanged `poly_quotation_combinator_unsupported_error` naming P7.S3b-follow, one
    test per name (`tag`'s guard arm is probe-confirmed reachable in S3b-follow, so it
    stays in the list).
  - A literal passed to a **poly** callee still rejects (S3f's territory), proving the
    concrete-only gate in R2 holds.
  - A **non-literal** quotation operand at `call` (C1) hits the located rejection, no panic
    (L1).

## Testing

Goldens (`tests/phase7_slice3d.rs`): the C1 and C2 behavioural fixtures above at two
instantiations; and every R4 negative, each asserting exact message text.

Unit tests beside the stage (`src/check/poly.rs`, `#[cfg(test)] mod tests`, per CLAUDE.md,
naming `thing_condition_expected`): `call` on a literal splices the body in place;
`call` on a non-literal operand is a located rejection; a `QuotLit` passed to a concrete
`env` word with a ground `Type::Quotation` input grounds and the call proceeds; the same
`QuotLit` against a `Type::InlineQuotation` input still rejects; against a poly candidate
still rejects; the retained guard still fires for `branch`/`if`/`times`/`tag`.

Mutation-tested guards (delete/flip the guarded code, watch the named test fail, then
restore to a clean `git status`; commit before mutation testing; a mutation copy needs
`examples/`; touch sources after any rollback):

- R1's `call`-on-literal arm (deletion → C1 fixture rejects with the located message).
- R2's env-dispatch grounding arm (deletion → C2 fixture rejects with
  `poly_op_on_variable_error`).
- R2's operand-window carve-out (revert → C2 fixture rejects at the earlier guard).
- The retained guard's remaining names (deletion → the corresponding negative starts
  compiling or falls through to `unknown word`).

Regression, green and untouched: `tests/phase7_slice3b.rs`, `tests/phase7_slice3a.rs`, the
`tests/phase6_*` eliminator suites, `tests/qbe_baseline.rs`. Green is
`cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

## Exit findings (required, to confirm during implementation)

- **OQ1 (carried from the brief, confirm):** `call`-splicing a literal is a straight-line
  walk with one body and one continuation, genuinely simpler than the eliminator's per-arm
  clone-and-union, **not** a hidden case of it. Confirm no join/merge logic is needed and
  none was added (R3).
- **OQ3 lowering (carried from the brief, open — do NOT pre-decide):** the brief's
  recon is checker-only and did not trace lowering. Confirm at exit whether a spliced `call`
  and a grounded C2 call inside a non-inline poly body's IR generation fall out of the
  existing monomorphization / eliminator-splice lowering, or whether new lowering work is
  required. Record the answer; if new lowering is needed, that is an exit finding, not
  silently in-scope work.
- Re-run `poly.rs`'s five split signals (P7.S3b's deferred split; S3b-follow's OQ3) and
  record the decision. Note that this slice adds arms inside `poly_call_term`, not a second
  consumer with its own arm-walk, so the split's trigger (a second arm-walking consumer) is
  **not** met here.
- Confirm the borrow join is unique in the file (no second, non-unioning join introduced).

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
      "focus": "C1: split the poly.rs:914-925 name guard so `call` on a quotation literal splices the literal's body in place via poly_walk against the live stack (poly analogue of terms.rs:299-357); non-literal `call` operand stays a located rejection; retained guard narrows to branch/if/times/tag unchanged",
      "effort": "S",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "C2: carve the operand-window guard (poly.rs:966-980) and add one env-dispatch arm (poly.rs:997-1043) grounding a QuotLit against a concrete candidate's ground Type::Quotation input (ported from unify_poly_input's Quotation arm, rowless), concrete-callee only, never Type::InlineQuotation or a poly candidate; golden fixtures (C1 + non-wrapper C2 at two instantiations) plus exact-message negatives; mutation-test the guards; exit findings incl. the OQ3 lowering confirmation and the poly.rs split re-run",
      "effort": "S",
      "difficulty": "standard"
    }
  ]
}
```
