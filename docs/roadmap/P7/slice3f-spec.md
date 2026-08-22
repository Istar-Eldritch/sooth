## Recon that shaped the design (kept because it explains the shape)

- `Slot::quot` is `Option<QuotRef>` with a single variant, `QuotRef::Known(QuotId)`, so
  `stack[base + i].quot.is_some()` at R9p can only mean a **statically-known literal**
  whose body is available to materialize — never an "abstract" marker. Once erased
  (`quot: None`), the value is an ordinary `Type::Quotation` slot that never reaches R9p
  at all, which is why an already-materialized/forwarded quotation argument was
  unaffected by this slice from the start.
- `unify_poly_input`'s `PolyType::Concrete(t)` arm is a plain equality check — no
  `Var`-binding path, hence no phantom-`'T` hazard — and that is the arm R1's narrowing
  routes a spared operand into. R1 alone is insufficient: a `Known` literal's raw
  `Slot.ty` is still the `Cstr` placeholder, so the check fails with
  `` expected `[ i64 -- i64 ]`, found `cstr` `` until R2's materialization runs first.
- `materialize_quotation_at_boundary` (`src/check/captures.rs`) is the same function the
  concrete argument boundary already calls at its own R8 site in `check_term`.
  `check_poly_call` had exactly one call site (`check_term`'s `TermKind::Call` arm), and
  `check_term` already held `env`/`cells`/`slices`/`&mut prov`/`&mut scope`, so R2's
  widening needed no plumbing above that call site.
- In a poly body, a genuine parameter bound to a ground quotation type folds to
  `PolySlot { pt: PolyType::Concrete(Type::Quotation(eff)), quot: None }` — no marker,
  nothing spliceable — so it fell into `poly_op_on_variable_error`'s `Concrete` arm,
  which renders `` `[ i64 -- i64 ]` ``. The *abstract* `PolyType::Quotation` shape hits a
  different arm rendering `"a quotation"` verbatim: one guard, two cases, two fixes.
- `check_abstract_quotation_call` (`src/check/terms.rs`) is exactly the
  pop-declared-inputs / push-declared-outputs / no-splice shape R3 needed, over
  `PolySlot`/`PolyType::Concrete` instead of `Slot`/`Type`. `QuotEffect` carries no row
  and no variable, so no row/`Subst` machinery was needed on the poly side either.

## Locked rules

- **L1 Ground only.** Every change is gated on the declared input being
  `PolyType::Concrete(Type::Quotation(eff))` after `raw_to_poly_type`'s fold. A declared
  `PolyType::Quotation(ins, outs, ..)` still carrying a free type/row variable is
  untouched: R1 does not spare it, R3 does not dispatch on it.
- **L2 R9p's real hazard stays enforced.** A bare `PolyType::Var` position (the
  `dupit`/`'T: Copy -- 'T 'T` shape) keeps rejecting.
- **L3 No splice for a genuine parameter.** R3 pops and pushes against the declared
  effect and never fetches a body; splicing stays P7.S3d's mechanism for a literal.
- **L4 No new representation, no new lowering — did not hold.** See Exit findings: R1/R2
  needed a real lowering step. L4 is additionally bounded by a pre-existing output-arity
  limit: a quotation effect declaring two or more outputs cannot be lowered at all, on
  the concrete path as much as the polymorphic one (`intern_output_bundles` walks
  `module.words`, so a quotation effect's own output tuple is never interned). R3 was not
  gated on output arity for that; the gap is registered as P7.S3m.

## Delivered shape

See the landed commits for the exact mechanism (this condensation trims the how, per
this project's spec-condensation convention, and the citations above are the durable
reference): `41118b2` (phase 1, R1+R2, argument boundary), `4576981` (phase 2, R3, body
boundary), `99c5308` (phase 3, R4, stale wording retirement), plus the review-fix
commits `c4397ef`/`2d730b3`/`2908a00` (exit findings, roadmap closeout, P7.S3l/P7.S3m
follow-ups named).
