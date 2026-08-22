# Phase 7 Slice 3f: runtime quotation values crossing the polymorphism boundary (spec)

## Goal

An ordinary `[ ]` quotation (`Type::Quotation`) already has a real runtime
representation (Phase 4 Slices 7a/7b) and already crosses a **concrete** call boundary:
a concrete word declaring one as a parameter and `call`ing it works today
(`apply_quot` in the brief). This slice makes the same **ground** (fully concrete, no
free type variable) `Type::Quotation` value cross the **polymorphism** boundary, on
both sides that a single guard family currently blocks outright, regardless of the
parameter's position:

1. **Argument boundary** — a caller (concrete or poly) passing a real quotation value
   to a poly callee's declared, ground `Type::Quotation` parameter is rejected
   unconditionally by `check_poly_call`'s R9p guard (`src/check/poly.rs:3267-3271`),
   before `unify_poly_input` is ever consulted.
2. **Body boundary** — a poly body `call`ing its own ground `Type::Quotation`
   parameter (a real value, not a literal it can splice) is rejected by
   `poly_call_term`'s `call` handling (`src/check/poly.rs:953-1010`), which only knows
   how to splice a literal's interned body and has no arm for a genuine parameter.

Three independently phaseable changes close this, all in `src/check/poly.rs` or its one
call site:

- **R1** — narrow R9p to spare a declared ground `Type::Quotation` input, rejecting
  only the position that is actually unsound: a bare `PolyType::Var`.
- **R2** — thread `materialize_quotation_at_boundary` (`src/check/captures.rs:287`)
  into `check_poly_call` for the spared case, which needs `check_poly_call`'s signature
  widened (`prov`/`scope` from `&`/`&` to `&mut`/`&mut`, plus `env`, `cells`, `slices`
  added) and its one call site (`src/check/terms.rs:710-712`) updated to match — the
  same shape of change P7.S3b-follow already made to `poly_walk`.
- **R3** — add a poly analogue of the concrete path's `check_abstract_quotation_call`
  (`src/check/terms.rs:1123-1147`) to `poly_call_term`'s `call` handling: pop the
  declared ground inputs, push the declared ground outputs, no splice, for when the
  top-of-stack operand is a genuine (materialized, non-literal) ground `Type::Quotation`
  parameter.

Also retired: the stale "(a runtime quotation value is slice 7)" clause in
`reject_quotation_argument` (`src/check.rs:3032-3039`) — the clause is a deliberate
prior rewording (the function's own doc comment names it R26, off a stale "Phase 6"
parenthetical it replaced), not simply forgotten cruft, but it has been wrong again
since 7a/7b shipped a real runtime representation, and is user-facing on every message
this slice's own R1 leaves rejecting.

**Not in scope:** the genuinely abstract `PolyType::Quotation(ins, outs, ..)` case (a
declared `[ 'T -- 'T ]` parameter that is not fully concrete). `unify_poly_input`'s own
`PolyType::Quotation` arm (`poly.rs:3495-3535`) already unifies this shape correctly at
the call-site boundary; the gap left open is that a poly *body* cannot `call` a bound
instantiation of it. Nothing in this slice's probe pass touched that case; it is
recorded as a follow-up (see Out of scope).

Probe-verified at HEAD, the two rejections this slice removes for the ground case:

```sooth
: run_it ( 'T: Copy [ i64 -- i64 ] -- 'T ) drop ;
: main ( -- ) 7 [ 1 add ] run_it drop ;
```

```text
error: a quotation cannot be passed to `run_it`; only `call` accepts one (a runtime
quotation value is slice 7) in `main` (line 2)
```

```sooth
: call_it ( 'T: Copy [ i64 -- 'T ] -- 'T ) 1 swap call ;
: main ( -- ) [ 5 ] call_it drop ;
```

```text
error: `call` is not permitted on a quotation in `call_it` (line 1)
```

## Recon (verified against the source at HEAD)

1. **`check_poly_call` (`poly.rs:3234-3336`) is the sole caller of R9p and has exactly
   one call site**, `check_term`'s `TermKind::Call` arm (`terms.rs:710-712`):

   ```rust
   if poly.env.contains_key(name) && !fall_through_to_env {
       return check_poly_call(
           name, span, &mut stack, ctx, scope, arrays, refs, prov, live, at, poly,
       );
   }
   ```

   `check_term` already holds `env: &HashMap<String, Vec<Overload>>`,
   `cells: &mut Vec<OwnedCellDecl>`, `slices: &mut Vec<SliceDecl>`, `prov: &mut
   Provenance`, and `scope: &mut Scope` in its own parameter list (`terms.rs:103-119`),
   so R2's signature widening needs no new plumbing above this call site — everything
   `materialize_quotation_at_boundary` needs is already a variable in scope here, it
   is simply not threaded into `check_poly_call`'s signature today.

2. **R9p's guard, unconditional on stack shape, not on the declared input**
   (`poly.rs:3267-3271`):

   ```rust
   // R9p: `unify_poly_input` binds a `Var` to *any* concrete type, so a
   // quotation would silently bind `'T` to the placeholder and
   // monomorphize a call over a phantom. Reject before unification.
   if stack[base + i].quot.is_some() {
       return Err(reject_quotation_argument(ctx, span, name));
   }
   ```

   `stack` here is `Vec<Slot>` (the monomorphic caller's stack), and `Slot::quot` is
   `Option<QuotRef>` with a single variant, `QuotRef::Known(QuotId)` (`check.rs:190-192`,
   "a single variant: two *different* quotations at a branch join are rejected at the
   join... so no poisoned/merged marker is ever carried"). So `stack[base + i].quot.is_some()`
   at this call site can only mean one thing: the operand is a **statically-known literal**
   whose body is available to materialize — never an "abstract" marker, because the
   `Slot` representation has no such marker. Once erased (`materialize_quotation_at_boundary`
   runs, `quot` set to `None`), the value is an ordinary `Type::Quotation`-typed slot and
   R9p's `.quot.is_some()` check does not fire on it at all — this is exactly why a
   forwarded/already-materialized quotation argument to a poly callee is unaffected by
   this slice: it never reached R9p to begin with, and `unify_poly_input`'s existing
   `PolyType::Quotation` arm (concrete or abstract) already unifies it.

3. **`unify_poly_input`'s `PolyType::Concrete(t)` arm does a plain equality check**
   (`poly.rs:3361-3364`):

   ```rust
   PolyType::Concrete(t) => {
       if *t != slot_ty {
           return Err(type_mismatch_error(ctx, span, name, *t, slot_ty));
       }
   }
   ```

   No `Var`-binding path, hence no phantom-`'T` hazard, for a callee input that folded
   to `PolyType::Concrete(Type::Quotation(eff))` (`raw_to_poly_type`,
   `parser.rs:2316-2438`, folds a fully-ground `[ ... ]` signature to `Concrete`). This
   is the arm R1's narrowing routes a spared operand into. But probing this directly
   (patch R1's narrowing alone, no R2) shows it is not sufficient on its own: a `Known`
   literal's raw `Slot.ty` is still the `Cstr` placeholder it carries before
   materialization, so `*t != slot_ty` fails with `` `run_it` expected `[ i64 -- i64 ]`,
   found `cstr` `` — the concrete side's own materialization step
   (`materialize_quotation_at_boundary`) has to run first, which is R2.

4. **`materialize_quotation_at_boundary` (`captures.rs:287-`) is the exact function the
   concrete argument boundary already calls**, at its own R8 site (`terms.rs:773-785`
   in `check_term`'s builtin-overload dispatch arm):

   ```rust
   if let Type::Quotation(eff) = *want {
       if let Some(QuotRef::Known(id)) = found.quot {
           stack[base + i] = materialize_quotation_at_boundary(
               id, eff, false, name, span, ctx, env, arrays, cells, refs, slices,
               prov, scope, poly,
           )?;
           continue;
       }
       // An already-erased runtime quotation value falls through
       // to the ordinary `match_slot` (Exact) below.
   }
   ```

   Its signature (`captures.rs:287-300`) takes `prov: &mut Provenance`,
   `scope: &mut Scope`, `env: &HashMap<String, Vec<Overload>>`, `arrays: &mut
   Vec<ArrayDecl>`, `cells: &mut Vec<OwnedCellDecl>`, `refs: &mut Vec<RefDecl>`,
   `slices: &mut Vec<SliceDecl>`, `poly: &mut PolyCtx`, `escaping: bool`, and returns
   `Result<Slot, String>` — the exact same shapes `check_poly_call` needs to gain (R2),
   already all live at its one call site (recon 1). `check_poly_call` currently takes
   `prov: &Provenance` and `scope: &Scope` (read-only) and receives no `env`, `cells`,
   or `slices` at all.

5. **`poly_call_term`'s `call` handling (`poly.rs:953-1010`) has exactly one branch: a
   literal, or a located rejection.**

   ```rust
   if name == "call" {
       let Some(top) = stack.last() else {
           return Err(underflow_error(ctx, span, name, 1, 0));
       };
       let Some(quot) = top.quot else {
           let pt = top.pt.clone();
           return Err(poly_op_on_variable_error(ctx, span, name, &pt, sig));
       };
       stack.pop();
       let lit = scope.quotation(quot);
       let body = lit.body.clone();
       // ... splice body via poly_walk, R1's snapshot/retain teardown ...
   }
   ```

   `top.quot` here is `Option<PolyQuotRef>` (`PolySlot::quot`, `poly.rs:122`) — the
   poly-body twin of `Slot::quot`, set only for a `QuotLit` marker slot pushed by a
   quotation literal written in the body (`PolySlot::quotation`, `poly.rs:130-136`). A
   genuine parameter bound to a ground `Type::Quotation` type folds to `PolySlot { pt:
   PolyType::Concrete(Type::Quotation(eff)), quot: None, .. }` — it carries no marker,
   because nothing spliceable is attached to it. So `top.quot` is `None` for exactly
   this slice's case, and the `else` branch renders `poly_op_on_variable_error` against
   `pt.clone()`. That renderer's `PolyType::Concrete(t)` arm (`poly.rs:3809`) formats
   `` `{t}` `` — `Type::Quotation`'s `Display` renders `[ i64 -- i64 ]` — which is the
   `` `call` is not permitted on a quotation in `call_it` `` wording quoted above once
   demangled by `crate::resolve::demangle_call` (`poly.rs:3804`). Note this is a
   **different** renderer arm than the one Gap 1's original probe (an *abstract*
   `PolyType::Quotation(..)` parameter) hits — that shape renders `"a quotation"`
   verbatim (`poly.rs:3810`), confirming the two cases share one guard but need two
   different fixes (recon of this file's own out-of-scope boundary).

6. **The concrete twin already exists and needs no new logic invented, only ported.**
   `check_abstract_quotation_call` (`terms.rs:1123-1147`):

   ```rust
   fn check_abstract_quotation_call(
       eff: &QuotEffect,
       span: Span,
       mut stack: Vec<Slot>,
       ctx: &Ctx,
       op: &str,
   ) -> Result<Vec<Slot>, String> {
       let n = eff.inputs.len();
       if stack.len() < n {
           return Err(underflow_error(ctx, span, op, n, stack.len()));
       }
       let base = stack.len() - n;
       for (i, want) in eff.inputs.iter().enumerate() {
           let found = stack[base + i];
           match match_slot(found, *want) {
               SlotMatch::Exact | SlotMatch::LiteralSizeType => {}
               _ => return Err(type_mismatch_error(ctx, span, op, *want, found.ty)),
           }
       }
       stack.truncate(base);
       for out in &eff.outputs {
           stack.push(Slot::computed(*out));
       }
       Ok(stack)
   }
   ```

   is exactly the pop-declared-inputs / push-declared-outputs / no-splice shape R3
   needs, over `PolySlot`/`PolyType::Concrete` instead of `Slot`/`Type` (`eff.inputs`
   and `eff.outputs` are plain, fully ground `Type`s either way — `QuotEffect` carries
   no row and no variable, `ast.rs:1721-1725` — so no row/`Subst` machinery is needed on
   the poly side either).

7. **`reject_quotation_argument`'s stale clause** (`check.rs:3032-3039`):

   ```rust
   fn reject_quotation_argument(ctx: &Ctx, span: Span, word: &str) -> String {
       let word = crate::resolve::demangle_word(word);
       format!(
           "error: a quotation cannot be passed to `{word}`; only `call` accepts one (a runtime quotation value is slice 7){} (line {})",
           in_word(ctx),
           span.line,
       )
   }
   ```

   is the one function this slice's own OQ4 names to retire the parenthetical from. It
   is shared between R9p (`check_poly_call`) and the concrete argument boundary's own
   R9 (`terms.rs:786-788`); both call sites keep saying "is slice 7" about a value type
   that has had a runtime representation since 7a/7b. **Out of scope, noted but not
   touched:** two other sibling functions carry the identical stale clause for
   different diagnostics. `reject_quotation_operand` (`check.rs:3018-3024`) renders it
   for a quotation passed as a *shuffle* operand, not a call argument.
   `call_needs_quotation_error` (`terms.rs:1104-1114`) renders it for a bare `call`
   with nothing quotation-shaped on the stack (`` a quotation cannot be a runtime
   value; a runtime quotation value is slice 7 ``). Neither is named by the brief's
   OQ4 and neither is touched by this slice — retiring either is an unowned,
   separately-scoped sweep (per `project_diagnostics_double_error_prefix`-style
   precedent: a shared stale phrase across sibling diagnostics is not, by itself,
   licence to widen this slice's edit).

8. **Only one call site of `check_poly_call` exists** (`grep -n
   "check_poly_call("`: `terms.rs:710`, `poly.rs:3234` (the definition)), so R2's
   signature widening touches exactly one call site, not several.

## Locked rules (carried, unchanged)

- **L1 Ground only.** Every change in this slice is gated on the callee's declared
  input being a fully concrete `Type::Quotation` (i.e. `PolyType::Concrete(Type::Quotation(eff))`
  after `raw_to_poly_type`'s fold). A declared `PolyType::Quotation(ins, outs, ..)` that
  still carries a free type/row variable is untouched: R1 must not spare it, R3 must not
  dispatch on it. See Out of scope.
- **L2 R9p's real hazard stays enforced.** A bare `PolyType::Var` position (the
  `dupit`/`'T: Copy -- 'T 'T` shape already pinned by
  `check_poly_call_rejects_a_quotation_argument`, `poly.rs:4586-4600`) keeps rejecting.
  Narrowing R9p must not widen it into accepting a quotation at a `Var` position by
  accident of a loose predicate.
- **L3 No splice for a genuine parameter.** R3 pops and pushes against the declared
  effect; it never fetches a body to splice (there is none — `top.quot` is `None`
  precisely because this is a real value, not a literal marker). Splicing stays R1's
  (P7.S3d's) mechanism for a literal; this slice's `call` arm is the disjoint case.
- **L4 No new representation, no new lowering.** `Type::Quotation` already lowers to a
  `(code, env)` value (7a/7b); a poly callee's monomorphized body already receives and
  returns ordinary `Type`-typed values through its existing ABI. This slice's exit
  criterion is expected to fall out of the existing monomorphization pass unchanged,
  the same finding P7.S3d's OQ3 confirmed for its own two consumers — record whether
  this holds at exit (see Exit findings) rather than assuming it.
  L4 is bounded by an output-arity limit that **predates this slice**: a quotation
  effect declaring two or more outputs cannot be lowered at all, on the concrete path
  as much as the polymorphic one. `intern_output_bundles` (`check.rs:913`) walks
  `module.words`, so a quotation effect's own output tuple is never interned;
  `bundle_of` then returns `None` in `lower_indirect_call`
  (`ir/func_builder/quotation.rs:226`), the declared outputs are never pushed, and the
  next consumer panics in the backend. R3 does not cause this:
  `: call_it ( [ i64 -- i64 i64 ] -- i64 ) 1 swap call add ;` reproduces it with no
  polymorphic word in the program. But it does open a second door to it, since before
  R3 such a body was a located rejection. So L4 holds only for declared effects with at
  most one output; phase 3 records the gap (see Exit findings) rather than gating R3 on
  output arity.

## Delivered shape

### R1 Narrow R9p to spare a declared ground `Type::Quotation` input

At `poly.rs:3267-3271`, replace the unconditional `stack[base + i].quot.is_some()`
check with one that additionally inspects `sig.inputs[i]`:

- If `stack[base + i].quot.is_some()` **and** `sig.inputs[i]` is
  `PolyType::Concrete(Type::Quotation(_))`: spared. Fall through to R2's materialization
  step before `unify_poly_input` runs for this `i`.
- If `stack[base + i].quot.is_some()` and `sig.inputs[i]` is anything else (a bare
  `PolyType::Var`, an abstract `PolyType::Quotation(ins, outs, ..)`, an `Array`, a
  `Generic`, ...): unchanged, `reject_quotation_argument`.

This is a per-position check, matching the brief's re-probed finding that R9p fires
identically regardless of the quotation's position in the signature (first, middle,
last) — the narrowing preserves that position-independence, it only adds a second
predicate (the *declared* type at that same position) alongside the existing one (the
*operand's* marker).

### R2 Thread `materialize_quotation_at_boundary` into `check_poly_call`

- Widen `check_poly_call`'s signature: `prov: &Provenance` → `prov: &mut Provenance`;
  `scope: &Scope` → `scope: &mut Scope`; add `env: &HashMap<String, Vec<Overload>>`,
  `cells: &mut Vec<OwnedCellDecl>`, `slices: &mut Vec<SliceDecl>`.
- Update the one call site (`terms.rs:710-712`) to pass `env`, `cells`, `slices`
  (already bound in `check_term`'s own parameter list, recon 1) alongside the existing
  arguments.
- In the per-input loop (`poly.rs:3263-3282`), for the spared case R1 identifies: fetch
  `eff` from `sig.inputs[i]`, call
  `materialize_quotation_at_boundary(id, eff, false, name, span, ctx, env, arrays, cells,
  refs, slices, prov, scope, poly)?` where `id` is the `QuotId` inside the operand's
  `QuotRef::Known(id)`, and write the returned `Slot` back into `stack[base + i]` before
  calling `unify_poly_input` for that position — matching the concrete boundary's own
  R8 site (recon 4) exactly: `escaping: false` (a call argument is an in-frame boundary,
  same reasoning as R8's own call), same materialization function, same erasure
  contract (`quot: None` on return, `ty` now the ground `Type::Quotation(eff)`).
  `unify_poly_input`'s existing `PolyType::Concrete(t)` arm then runs unchanged and
  succeeds by ordinary equality (recon 3), since the materialized slot's `ty` now
  matches `t` exactly (same interned `eff`).
- A capturing literal (one whose body names an enclosing local) runs through
  `materialize_quotation_at_boundary`'s existing R15 admission rule exactly as the
  concrete boundary's does — no new capture logic. A literal that fails admission (an
  escaping capture, a captured `~[ ]`, ...) surfaces that function's existing located
  diagnostics, not a new one.
- **R9p's own second check-site, `unify_poly_input`'s `PolyType::QuotLit`
  arm (`poly.rs:3359-3360`, `unreachable!("a quotation-literal marker never reaches a
  signature")`) is untouched**: `sig.inputs[i]` is the *callee's declared* `PolyType`,
  never `QuotLit` (that variant only exists on the *caller's* poly-walk stack, and this
  slice's whole boundary is the concrete-caller/poly-callee shape, where the caller's
  stack is `Vec<Slot>`, not `Vec<PolySlot>`, and carries no `QuotLit` variant at all —
  recon 2). Confirm this at exit rather than assume it (a poly-caller/poly-callee
  argument boundary is a different, untouched code path — see Out of scope).

### R3 A poly analogue of `check_abstract_quotation_call` in `poly_call_term`'s `call` handling

At `poly.rs:953-960`, before falling to `poly_op_on_variable_error`: when `top.quot` is
`None` **and** `top.pt` is `PolyType::Concrete(Type::Quotation(eff))`, dispatch to a new
helper (working name `poly_call_ground_quotation_param`) instead of erroring:

- `n = eff.inputs.len()`; underflow (reusing `underflow_error`) if the stack beneath the
  popped top has fewer than `n` slots.
- For each declared input `want` (deepest-first), check the corresponding operand slot's
  `pt` is `PolyType::Concrete(*want)`; anything else is a located type mismatch, and
  **the renderer choice must split on the mismatched operand's own shape** — there is
  no single function that renders both sides of every case:
  - If the operand's `pt` is `PolyType::Concrete(t)` (a ground type that simply isn't
    `*want`): use `type_mismatch_error(ctx, span, op, *want, t)` (`check.rs:1377`,
    takes two ground `Type`s), matching `unify_poly_input`'s own `Concrete` arm
    (recon 3) exactly.
  - If the operand's `pt` is anything else (a bare `PolyType::Var`, an abstract
    `PolyType::Quotation`, ...) — it has no ground `Type` to hand `type_mismatch_error`
    for `found`, so that function cannot render it. Use
    `poly_rendered_type_mismatch_error(ctx, span, op, expected_str, found_str)`
    (`poly.rs:3580`) instead, rendering both sides through `poly_type_str`.
  An implementer following only "pick one renderer for consistency" would hit a type
  error trying to hand a non-`Concrete` `PolyType` to `type_mismatch_error`'s `Type`
  parameter; the split above is what the two functions' actual signatures require.
- Truncate to the base and push one `PolySlot::new(PolyType::Concrete(out))` per
  declared output. No `int_val`, no `quot` marker — an ordinary value slot, matching how
  every other `PolyType::Concrete` result is pushed elsewhere in this file
  (`poly.rs:872`, `poly.rs:1254-1255`).
- No splice, no `poly_walk` recursion, no snapshot/retain teardown (L3): this is a
  straight pop/push against a declared effect, the same shape
  `check_abstract_quotation_call` already is on the concrete side.

Only the ground case dispatches here. A `PolyType::Quotation(ins, outs, ..)` (abstract,
still carrying a variable) top-of-stack operand still falls through to
`poly_op_on_variable_error`'s `"a quotation".to_string()` arm, unchanged — recorded as
the open follow-up (Out of scope), not silently subsumed.

**R3's own unit tests are independently writable before Phase 1 lands** — they can
construct a `PolySlot` stack directly and call `poly_call_term`/the new dispatch arm
without going through `check_poly_call`'s argument boundary at all (the same shape as
the existing `poly_walk_arms_rejects_an_arm_local_left_unconsumed` test,
`poly.rs:4876-4908`, which seeds a stack directly rather than driving it through a
full program). **`body_boundary_calls_ground_quotation_param` (the golden) is not**:
it drives a real `.sth` program through `main`, so the literal argument reaching
`call_it` must first survive `check_poly_call`'s R9p guard — without Phase 1's
narrowing landed first, that golden fails at the argument boundary before `call_it`'s
body — where R3's fix lives — is ever reached. The three changes are independently
*implementable*, not independently *golden-testable*: sequence Phase 1 before Phase 2
for that reason, not just for tidiness.

### R4 Retire the stale clause

`reject_quotation_argument` (`check.rs:3032-3039`): drop the `(a runtime quotation
value is slice 7)` parenthetical. New wording:

```text
error: a quotation cannot be passed to `{word}`; only `call` accepts one{in_word} (line {N})
```

This message is now reached in strictly fewer cases than before this slice (R1 spares
the ground case), but every case it still fires for is a genuine phantom-`'T` hazard
(L2) or an out-of-scope abstract-row case, so the message text itself needs no other
change beyond dropping the stale clause. Every existing caller of this function
(`check_poly_call`'s R9p, and the concrete argument boundary's own R9 at
`terms.rs:786-788`) picks up the reworded message automatically; do not special-case
either call site.

**This wording change breaks an existing test, and fixing it is part of R4, not a
separate cleanup:** `tests/phase4_combinators.rs`'s
`quotation_against_non_quotation_parameter_is_error` (line ~455) asserts a `"slice 7"`
substring against this exact function's output; update that assertion to drop the
`"slice 7"` half (see Testing's regression section for the precise before/after).

## Testing

Unit tests beside the stage (`src/check/poly.rs`, `#[cfg(test)] mod tests`, per
CLAUDE.md, naming `thing_condition_expected`):

- R1/R2: a `Known` literal quotation argument at a declared ground `Type::Quotation`
  input position materializes and the call succeeds (position varied: first, middle,
  last input, mirroring the brief's own re-probed position-independence finding — one
  test per position, not just one).
- R1's negative, re-pinned: `check_poly_call_rejects_a_quotation_argument`
  (`poly.rs:4586-4600`, the existing `dupit`/bare-`'T` test) stays green **unmodified** —
  the mutation-test proof that L2 is not accidentally widened.
- R1's second negative, new: a quotation literal argument at a declared **abstract**
  `PolyType::Quotation(ins, outs, ..)` position (still carrying a free variable) stays
  rejected by the same `reject_quotation_argument` path — proves R1's `Concrete(...)`
  match arm does not accidentally widen to cover `PolyType::Quotation`.
- R2: a capturing literal at the argument boundary runs the existing R15 admission
  path (reuse, do not duplicate, the capture fixtures `check_capture_admission`'s own
  tests already exercise) — one test confirming the admission function is actually
  invoked from this new call site, not just present in the diff.
- R3: `call` on a genuine (materialized, non-literal) ground `Type::Quotation`
  parameter pops/pushes correctly; the exact prior rejection text
  (`` `call` is not permitted on a quotation in `{word}` (line {N}) ``) for the abstract
  `PolyType::Quotation(..)` case is unchanged (assert the exact string, not just that it
  still fails — a placebo risk this project's memory already names).
- R3's negative: a type mismatch inside the declared effect (wrong operand type at one
  of the popped positions) is a located rejection, not a panic and not a silent
  coercion.
- R4: `reject_quotation_argument`'s new exact wording, asserted at both surviving call
  sites (`check_poly_call`'s R9p on a `Var` position; the concrete boundary's own R9 on
  a non-quotation parameter position) — one test per call site, since the same function
  now serves both and a fix to one message text is not proof the other picked it up.

Goldens (`tests/phase7_slice3f.rs`), fixtures given as complete `.sth` sources:

- **Argument-boundary behavioural** (`argument_boundary_materializes_ground_quotation_param`):
  a poly word declaring both a real type variable and a ground `Type::Quotation`
  parameter, called from a concrete body with a literal quotation argument, run at two
  distinct instantiations of the variable so it is carried rigidly rather than
  coincidentally matching:

  ```sooth
  : run_it ( 'T: Copy [ i64 -- i64 ] -- 'T ) drop ;
  : main ( -- ) 7 [ 1 add ] run_it drop ;
  ```

  plus a `bool`-instantiated call at a second call site, both in the same `main`, so
  `'T` is exercised at two concrete types in one build.

- **Body-boundary behavioural** (`body_boundary_calls_ground_quotation_param`): a poly
  word declaring a ground `Type::Quotation` parameter and `call`ing it inside its own
  body, run at two instantiations of its other type variable:

  ```sooth
  : call_it ( 'T: Copy [ i64 -- 'T ] -- 'T ) 1 swap call ;
  : main ( -- ) [ 5 ] call_it drop [ true ] call_it drop ;
  ```

  (the quotation itself is monomorphic — `[ i64 -- 'T ]`'s `'T` grounds per
  instantiation, exactly as `run_it`'s own `'T` does; this is not a generic quotation
  body, just a generic *caller*).

- **Round trip** (`argument_and_body_boundary_together`): a poly word that both
  receives a ground `Type::Quotation` parameter as an argument and `call`s it inside
  its body — proving R1/R2 and R3 compose, not just work in isolation. The quotation
  parameter must be ground **at the signature itself** (`[ i64 -- i64 ]`, no `'T`
  inside the brackets) — a `'T` inside the brackets (`[ 'T -- 'T ]`) folds to the
  abstract `PolyType::Quotation` shape this slice defers (Out of scope), not the ground
  `PolyType::Concrete(Type::Quotation(..))` shape R1-R3 target, even though it becomes
  ground *after* monomorphization. `'T` stays outside the brackets, as an unrelated
  passthrough value carried alongside the quotation, so the fixture still exercises two
  instantiations of the poly word itself:

  ```sooth
  : apply_it ( 'T: Copy 'T [ i64 -- i64 ] i64 -- 'T i64 ) swap call ;
  : main ( -- ) 9 [ 1 add ] 7 apply_it drop drop true [ 1 add ] 7 apply_it drop drop ;
  ```

- **Negatives, each asserting exact message text** (not merely that the build fails):

  - A quotation argument at a bare `'T` position (L2, the existing `dupit` shape,
    reproduced as a golden too, not just a unit test, since it is the exit criterion's
    own boundary).
  - A quotation argument at a declared **abstract** `PolyType::Quotation` position
    (out of scope, stays rejected with the same message).
  - `call` on the poly body's own bare `'T`-bound local (unrelated to a quotation at
    all — confirms this slice's new arm in `poly_call_term` is gated on
    `PolyType::Concrete(Type::Quotation(..))` specifically, not on "not a `QuotLit`").
  - `call` on a declared **abstract** `PolyType::Quotation` parameter inside a poly
    body (out of scope, stays rejected with the pre-existing "a quotation" wording).

Mutation-tested guards (delete/flip the guarded code, watch the named test fail, then
restore to a clean `git status`; commit before mutation testing; a mutation copy needs
`examples/`; touch sources after any rollback; end each cycle on a clean `git status`):

- R1's narrowing (revert to the unconditional check → the argument-boundary golden
  starts rejecting with the old message).
- R2's materialization call (delete it, leaving R1's narrowing in place → the
  argument-boundary golden fails with the `` found `cstr` `` mismatch quoted in recon 3,
  not the located rejection — confirms R1 alone is not the fix, R2 is load-bearing).
- R3's new dispatch arm (delete it → the body-boundary golden falls back to the prior
  `` `call` is not permitted on a quotation `` rejection).
- R4's wording change (revert → the negative tests assert the new text and fail against
  the reverted one, proving they are not placebos that merely check for failure).

**Regression requiring an update, not left untouched (R4's blast radius):**
`tests/phase4_combinators.rs`'s `quotation_against_non_quotation_parameter_is_error`
(line ~455) asserts `err.contains("a quotation cannot be passed to \`f\`") &&
err.contains("slice 7")` against `reject_quotation_argument`'s output at the concrete
argument boundary (`terms.rs:786-788`, the same function R4 edits). R4 deletes the
"slice 7" substring outright, so this assertion fails the moment R4 lands. **Phase 3's
task list must include updating this test's assertion** (drop the`"slice 7"` half of
the `&&`, keep the `` a quotation cannot be passed to`f` `` half and the sibling
`!err.contains("Phase 6")` assertion, both unaffected by R4) — this is not optional
cleanup, it is a required part of landing R4 without breaking the suite.

Regression, green and untouched: `tests/phase7_slice3d.rs`, `tests/phase7_slice3b.rs`,
`tests/phase7_slice3b_follow.rs`, `tests/phase7_slice3a.rs`, the `tests/phase6_*`
eliminator suites, `tests/qbe_baseline.rs`, and
`tests/phase4_generics.rs`'s `quotation_passed_to_polymorphic_word_is_error`
(line ~780, the golden-level twin of the `dupit` unit test L2 re-pins,
`poly.rs:4586-4600`) — verified it asserts only
`` err.contains("a quotation cannot be passed to `dupit`") ``, no `"slice 7"` substring,
so R4 does not touch it. Green is
`cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

## Exit findings (required)

- **Lowering confirmation (L4) — the finding is negative, not confirmed.** Phase 1
  needed a real lowering change: `lower_poly_call` had no step materializing a
  monomorphized callee's phantom quotation argument into a `(code, env)` aggregate
  before the call, so the argument-boundary golden crashed QBE without it. Phase 1
  added `CallInst::quot_inputs` (`src/ast.rs:1500`) to carry the materialized positions
  and a `materialize_quot_args` step (`src/ir/func_builder/quotation.rs:361`) that runs
  it in `lower_poly_call`. This does not fall out of the existing monomorphization pass
  unchanged, and does not match the finding P7.S3d's OQ3 recorded for its own two
  consumers — that finding held for a *literal* spliced in place; a genuine argument
  crossing the poly call boundary needed its own materialization step. R3's `call` on
  the resulting value is checker-only bookkeeping (that half of L4 does hold); R1/R2's
  argument-boundary crossing is not.
- **R9p's second check-site (R2's note).** Still genuinely unreachable, confirmed by
  construction rather than by test: `check_poly_call` operates over `Vec<Slot>` (the
  monomorphic caller's stack), and `Slot` has no `QuotLit`-shaped variant at all --
  that marker exists only on `Vec<PolySlot>`, the poly *body's* own stack. No value of
  type `Slot` can ever carry a `QuotLit` tag, so `unify_poly_input`'s
  `PolyType::QuotLit => unreachable!(...)` arm (`poly.rs:3438`) cannot be reached from
  `check_poly_call`'s call site regardless of what this slice changed -- the two stack
  representations are disjoint types, not merely disjoint in practice.
- **`poly.rs` split signals, re-run.** `poly.rs` is 7387 lines after this slice (up
  from 6299 at P7.S3b-follow's own exit re-run). Import divergence still does not fire
  (a single `use super::*`); no circular dependency forces a split. The
  responsibility-mix and uncalled-neighbour signals fire for the same reason
  S3b-follow's exit already recorded (the abstract term walk, shuffle/ref ops,
  eliminator dispatch, combinator dispatch, and ~40 diagnostics coexist, many
  formatters uncalled by their neighbours) -- this slice did not change that shape; it
  added one function (`poly_call_ground_quotation_param`) and two diagnostic arms to
  the existing dispatch, not a new responsibility category. S3b-follow's named trigger
  ("a third consumer, or an unrelated phase that already has to touch this
  neighbourhood") targets a specific extraction shape --
  `poly_eliminator_call`/`poly_combinator_call`/`poly_walk_arms`/`poly_row_combinator`/
  `poly_declared_arm` into `check/poly_arms.rs` -- and this slice touched none of
  those functions; it added a sibling arm to `poly_call_term` (the *caller* of that
  machinery), not to the machinery itself. Trigger not met. **Deferred again**, same
  reason and same named extraction shape as S3b-follow's exit.
- **The >=2-output lowering gap (qualifies L4).** Confirmed present after this slice,
  on both the poly and concrete paths: `: call_it ( 'T: Copy [ i64 -- i64 i64 ] -- 'T )
  3 swap call . . ;` panics at `src/ir/func_builder/calls.rs:514: print: value`, and the
  concrete twin (`( [ i64 -- i64 i64 ] -- ) ...`) panics identically, because
  `intern_output_bundles` (`check.rs:913`) interns output tuples only for declared
  words, never for a quotation effect's own output row. R3 reaches this gap (a poly
  body `call`ing a two-or-more-output quotation parameter was a located rejection
  before this slice; now it type-checks and panics in the backend instead) without
  causing it — the identical concrete-path panic proves the gap predates this slice.
  Registered as **P7.S3m** in `P7-language-prereqs.md` for the `intern_output_bundles`
  fix. Do **not** close it by gating R3 on `outputs.len() < 2`: that would diverge from
  the concrete `call` twin, which admits the same shape, and would reject programs that
  become legal the moment the interning is fixed.
- **Phase 2's fixture deviations from this spec.** Three of this spec's own body-boundary
  fixtures were wrong and phase 2 corrected them; record the corrections so the spec is
  not read back as the delivered shape. (1) The body-boundary golden written here as
  `( 'T: Copy [ i64 -- 'T ] -- 'T )` contradicts L1, since a `'T` inside the brackets
  folds to the abstract `PolyType::Quotation` this slice defers, so it landed with a ground
  `[ i64 -- i64 ]`, and the spec's own shape is pinned as *still rejected* by
  `body_boundary_rejects_an_abstract_quotation_param`. (2) The round-trip fixture
  `( 'T: Copy 'T [ i64 -- i64 ] i64 -- 'T i64 )` is off by one input, since the bound
  site `'T: Copy` is itself an input slot; the duplicate `'T` was dropped. (3) `call` on
  a bare `'T`-bound local was listed among the goldens but is a checker fact with no
  runnable program, so it landed as the unit test
  `poly_call_on_a_variable_local_is_still_error`.
- **The abstract `PolyType::Quotation` follow-up, named.** Yes, it needs its own
  roadmap slice: registered as **P7.S3l** in `P7-language-prereqs.md`. A poly body
  still cannot `call` a *bound instantiation* of a still-abstract declared quotation
  parameter (`[ 'T -- 'T ]`); `unify_poly_input`'s `PolyType::Quotation` arm already
  unifies it correctly at the call-site boundary, so the gap is body-side only, and
  the fix is expected to be a `poly_call_term` dispatch arm parallel to this slice's
  R3, grounded against the caller's `Subst` rather than a body-local literal.
- **The roadmap's "any caller" wording is broader than what this slice delivers --
  said explicitly when closing out the entry.** `P7-language-prereqs.md`'s S3f entry
  is marked `[ done ]` with its `Exit:` line narrowed to "the argument boundary of a
  ground ... parameter" rather than "any caller", and a new paragraph names both gaps
  this slice does not close: `resolve_poly_overload`'s multi-candidate `saw_quotation`
  short-circuit (`poly.rs:3038-3067`), which rejects a quotation argument
  unconditionally for any **overloaded** poly name without ever reaching R1's
  per-position check, and the abstract-`PolyType::Quotation` body-call gap now named
  P7.S3l above.

## Out of scope

- **The abstract `PolyType::Quotation(ins, outs, ..)` case** — a declared quotation
  parameter that still carries a free type or row variable (e.g. `[ 'T -- 'T ]`).
  `unify_poly_input`'s own `PolyType::Quotation` arm (`poly.rs:3495-3535`) already
  unifies this correctly at the call-site boundary (row-pointwise, binding any variable
  the row mentions); the open gap is that the poly body itself cannot `call` a bound
  instantiation of it once bound. This slice's probe pass found no evidence this needs
  a new representation, only a new `poly_call_term` dispatch arm parallel to R3's —
  grounded against the concretely *bound* instantiation, not a body-local literal — but
  it is untouched here and must not be silently folded in. Flag as a follow-up slice at
  exit (see Exit findings), not expanded into this one.
- `~[ ]` (`InlineQuotation`) crossing this boundary at all — a non-inline word declaring
  one is rejected by a located diagnostic at word-declaration checking
  (`check_inline_quotation_requires_inline`, `word_entry.rs:122-146`), not a parser-level
  restriction, and that gate is correct, not a gap.
- Anything already covered by P7.S3d (a quotation *literal* written inside a poly body,
  splicing in place, or grounding against a **concrete** callee's declared quotation
  parameter — that slice's own C1/C2). This slice's gap is the parameter/argument-boundary
  pair for a **poly** callee, a different code path.
- Row-typed inline combinator dispatch (`if`/`times`/`unless`/...) — P7.S3b-follow's own
  territory, untouched.
- `resolve_poly_overload`'s multi-candidate `PolyOverloadMiss::Quotation` classification
  (`poly.rs:3038-3067`) — an **overloaded** poly name with a quotation argument still
  rejects outright even if one candidate's signature would admit it at a ground
  position, mirroring the completeness gap P7.S3d's own R2 recorded (and did not fix)
  for an overloaded concrete name. Recorded, not fixed.
- Trait bounds (P7.S3e) and self-recursion (P7.S3g) — no interaction traced.
- Any lowering change beyond what falls out of the checker fix (L4; confirm at exit,
  do not pre-emptively add).

## References

- `docs/roadmap/P7/slice3f-brief.md` (probe-grounded brief; the authoritative recon and
  the resolved open questions this spec turns into R1-R4)
- `docs/roadmap/P7/slice3d-spec.md` (the sibling rowless-splice slice this one is
  independent of either way; format precedent for a literal-vs-parameter split)
- `docs/roadmap/P7/slice3b-follow-spec.md` (precedent for widening a poly-side
  function's signature to thread new plumbing to its one call site)
- `docs/roadmap/P7-language-prereqs.md` (P7.S3f's roadmap entry; the abstract-quotation
  follow-up's eventual home)

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Argument boundary (R1+R2): narrow check_poly_call's R9p guard (poly.rs:3267-3271) to spare a declared ground PolyType::Concrete(Type::Quotation(..)) input, rejecting only a bare PolyType::Var (L2) or an abstract PolyType::Quotation position (out of scope, unchanged); widen check_poly_call's signature (prov/scope to &mut, add env/cells/slices) and update its one call site (terms.rs:710-712); thread materialize_quotation_at_boundary into the spared case before unify_poly_input runs. Coverage: per-position unit tests (first/middle/last), the dupit negative re-pinned unmodified, the abstract-position negative, a capturing-literal admission test, and the argument-boundary golden",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "Body boundary (R3): add a poly analogue of check_abstract_quotation_call (terms.rs:1123-1147) to poly_call_term's `call` handling (poly.rs:953-960), dispatching when the top-of-stack operand's pt is PolyType::Concrete(Type::Quotation(eff)) and quot is None -- pop declared inputs, push declared outputs, no splice, no poly_walk recursion. R3's own unit tests are independently writable now (seed a PolySlot stack directly, no Phase 1 dependency), but the body-boundary and round-trip goldens require Phase 1 to have landed first (the literal argument must survive check_poly_call's R9p guard before call_it's body is ever reached). Coverage: the pop/push unit test, the type-mismatch negative, the unchanged abstract-PolyType::Quotation rejection (exact text), the body-boundary golden, and the round-trip golden (argument and body boundary composing)",
      "effort": "S",
      "difficulty": "standard"
    },
    {
      "phase": 3,
      "focus": "Retire the stale reject_quotation_argument wording (R4, check.rs:3032-3039) and assert the new exact text at both surviving call sites; update tests/phase4_combinators.rs's quotation_against_non_quotation_parameter_is_error (line ~455) to drop its now-stale 'slice 7' substring assertion, which R4 breaks; run all mutation-tested guards for R1-R3; write the required exit findings (lowering confirmation, unify_poly_input's QuotLit-unreachable re-check, the poly.rs split-signal re-run, naming the abstract-PolyType::Quotation follow-up, and flagging that the roadmap's 'any caller' exit wording overclaims against the untouched multi-candidate overload gap)",
      "effort": "S",
      "difficulty": "standard"
    }
  ]
}
```
