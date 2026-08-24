# P7.S3l brief — A poly body cannot `call` its own abstract quotation parameter

## Problem, confirmed live against current `main` (`9d213e6`)

A non-inline polymorphic word may declare a quotation-typed parameter whose brackets still
mention the word's own type variable (`[ 'T -- 'T ]`), and `unify_poly_input`'s `Quotation`
arm (`src/check/poly.rs:5170-5194`) unifies it correctly at the *call site* against a
concrete argument, row-pointwise, binding whatever the row mentions — this already works and
is tested (`tests/phase7_slice3f.rs` L1's accept case, `phase7_slice3d.rs`). What does not work
is the *body* side: calling `call` on that same parameter, from inside the declaring word's
own body, is rejected outright. Live probe:

```
: apply ( 'T [ 'T -- 'T ] -- 'T ) call ;
: main ( -- ) 4 [ 1 add ] apply print ;
```

```
error: `call` is not permitted on a quotation in `apply` (line 1)
```

`poly_call_term`'s `call` arm (`src/check/poly.rs:1275-1287`) has exactly two live cases: a
quotation *literal* (`top.quot`, spliced in place) and a fully **ground**
`PolyType::Concrete(Type::Quotation(eff))` operand, dispatched to
`poly_call_ground_quotation_param` (`:2450`, P7.S3f's R3). Every other operand — including a
`PolyType::Quotation(..)` that still carries a variable, exactly the shape `apply`'s own
parameter has inside its own body — falls through to `poly_op_on_variable_error`
(`:5603`), which renders "not permitted on a quotation" with no further discrimination. This
is not an accident this brief is discovering: it is a **deliberately pinned** rejection, named
at S3f's own exit as out of scope (L1) and exercised by two tests that assert this exact
wording as the *expected* behavior today: `poly_call_on_an_abstract_quotation_param_is_still_error`
(`src/check/poly.rs:7349`) and `body_boundary_rejects_an_abstract_quotation_param`
(`tests/phase7_slice3f.rs:170`). Both will need their assertions flipped from "stays rejected"
to "now succeeds" as part of this slice's exit — cited here so the pipeline does not read a
red diff on them as a regression.

The gate is unconditional on the input side too — the same rejection fires even when the
declared row is trivially satisfiable and even before any arity/type mismatch beneath the
quotation would matter, confirmed by probe (a `Bool` where `apply`'s row wants an `i64`
produces the identical "not permitted on a quotation" text, never a type-mismatch message):
there is no partial mechanism to build on, only the one blanket `Err`.

## What already exists and what does not

**Check-time unification already binds the row; nothing new needs representing.**
`PolyType::Quotation` is `#[derive(PartialEq, Eq)]` (`src/ast.rs:1845`), so the two
declared rows (`ins`, `outs`) can be compared structurally against the operand's own
`PolyType`s exactly the way `poly_eliminator_call` already compares arm exit rows
structurally under S3b's L1 ("type variables stay rigid... two arms agree on an exit position
iff the `PolyType`s are structurally equal", `:2500-2503`). `apply`'s own body never
substitutes `'T` to anything concrete while it is checked — `check_poly_body` walks it once,
generically, with every variable rigid — so this is a pop/compare/push dispatch parallel to
`poly_call_ground_quotation_param`'s structure, not a new grounding mechanism: no `Subst` is
built or consulted mid-body for this, the same absence S3b already established for the
eliminator case.

**Lowering is plausibly already sound for the concrete instantiation, unconfirmed.**
By the time a polymorphic body reaches IR construction, it has been monomorphized per
concrete instantiation, so `apply`'s `[ 'T -- 'T ]` parameter has already substituted to a
ground `Type::Quotation` at that call site (`apply_subst`'s `Quotation` arm,
`src/check/poly.rs:5421-5439`, already folds a fully-grounded row to
`Type::Quotation`/`Type::InlineQuotation`). `src/ir/func_builder/quotation.rs:205`'s
indirect-call path ("a non-literal `call` operand is a materialized quotation value") is
already exercised by the *ground*-parameter case (S3f R3) that works today end-to-end,
including at runtime. Whether this instantiated case reaches that same lowering path with no
further change, or needs its own arm, is **not yet recon'd** — S3f's own probe pass found no
representation gap, but did not walk the lowering side for this specific shape and neither
has this brief; the pipeline's phase 1 should confirm this before assuming it is free.

## Scope

**In scope:** a plain (non-`inline`, non-combinator) polymorphic word's `call` on its own
declared quotation parameter, still abstract at check time, once the caller's instantiation
has bound every variable the row mentions — the direct mirror of S3f's R3 for the ground case,
one level earlier in the pipeline. Both currently-pinned "still an error" tests
(`src/check/poly.rs:7349`, `tests/phase7_slice3f.rs:170`) become accept cases; underflow and
type-mismatch beneath the quotation get their own located diagnostics, parallel to
`poly_call_ground_quotation_param`'s existing `underflow_error`/`type_mismatch_error`/
`poly_rendered_type_mismatch_error` arms, rather than continuing to share the single blanket
"not permitted on a quotation" message once the accept path exists.

**Out of scope, named so the spec does not silently drift into it:** a **row-typed**
abstract quotation parameter (`Option<u32>` row fields on `PolyType::Quotation`, Slice 10a's
combinator machinery) — this brief's probes only exercise a fixed-arity `[ 'T -- 'T ]`, never
a row-carrying `[ 'T ~row -- ~row ]`-shaped parameter, and that machinery belongs to the
`inline`-combinator family this slice does not touch. A poly *combinator* calling its own
abstract quotation parameter is likewise untouched — combinators are checked through
`check_poly_combinator_standalone`, a different, term-splicing path (`src/check.rs:761-784`),
not `poly_call_term`'s ordinary body walk this brief probed.

**Exit** (per `docs/roadmap/P7-language-prereqs.md`'s existing S3l entry): a poly body may
`call` its own declared, still-abstract quotation parameter once bound by the caller's
instantiation, popping/pushing against the row grounded through that call's `Subst`, the same
way S3f's R3 does for the already-ground case.
