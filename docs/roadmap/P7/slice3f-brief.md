# Phase 7 Slice 3f: runtime quotation values crossing the polymorphism boundary (brief)

Ordinary `[ ]` quotations (`Type::Quotation`) already have a real runtime
representation, landed in Phase 4 Slices 7a/7b: a **concrete** word declaring one as a
parameter and `call`ing it indirectly works today, probe-verified —

```
import: intrinsics * ;
: apply_quot ( [ i64 -- i64 ] i64 -- i64 )
  swap call
;
: main ( -- ) [ 1 add ] 7 apply_quot drop ;
```

compiles clean. What does not work is that same value crossing the **polymorphism**
boundary, on either side. Discovered while scoping P7.S3d, not planned — S3d's own
gap is a quotation *literal* written inside a poly body; this slice's gap is an
*abstract parameter* and the *call-site argument boundary*, a different code path in
the same file (`src/check/poly.rs`). **Independent of S3d, not a prerequisite either
way**, and orthogonal to S3e's trait-bound work.

## Recon (read and probe-verified directly against the built compiler, this session)

1. **Gap 1 — calling an abstract (parameter-bound, non-literal) quotation slot from
   inside a poly body is still rejected, but with a different, more precise message
   than the roadmap entry currently cites.** `poly_call_term`'s `call` handling
   (`src/check/poly.rs:953-958`) checks the top-of-stack slot's `.quot` marker; when
   it is `None` — the operand is a real parameter, not a spliced literal — it falls
   to `poly_op_on_variable_error(ctx, span, name, &pt, sig)` (`poly.rs:958`), which
   renders a `PolyType::Quotation(..)` operand as `` `call` is not permitted on a
   quotation in `{word}` `` (`poly.rs:~3806-3819`, the `PolyType::Quotation(..) =>
   "a quotation".to_string()` arm of `poly_op_on_variable_error`). Probe:

   ```
   : call_it ( 'T: Copy [ i64 -- 'T ] -- 'T ) 1 swap call ;
   : main ( -- ) [ 5 ] call_it drop ;
   ```

   fails with exactly that message. The roadmap's citation
   (`` unknown word `call` ``) is stale — some intervening slice (plausibly S3d, which
   added dispatch machinery ahead of this same guard) gave the abstract-quotation case
   a located rejection where it used to fall through unnamed. The gap itself has not
   closed; only the diagnostic changed.

2. **Gap 2 is real, but its actual boundary is narrower and more precisely locatable
   than "any caller ... rejected outright" suggested, and one prior recon pass this
   session mischaracterized it as a routing bug before that finding was traced to a
   malformed probe and retracted (see the pitfall below).** `check_poly_call`'s R9p
   guard (`src/check/poly.rs:3270-3271`):

   ```rust
   // R9p: `unify_poly_input` binds a `Var` to *any* concrete type, so a
   // quotation would silently bind `'T` to the placeholder and
   // monomorphize a call over a phantom. Reject before unification.
   if stack[base + i].quot.is_some() {
       return Err(reject_quotation_argument(ctx, span, name));
   }
   ```

   runs **before** `unify_poly_input` is consulted at all, for **every** input
   position, regardless of what `sig.inputs[i]` actually declares. Confirmed by
   reading `unify_poly_input` (`poly.rs:3346-3552`): its `PolyType::Concrete(t)` arm
   (a fully-ground declared type, which is exactly what a concrete quotation
   parameter like `[ i64 -- i64 ]` folds to per `raw_to_poly_type`,
   `src/parser.rs:2316-2438`) does a plain `*t != slot_ty` check — no unsound `Var`
   binding risk at all. So R9p's comment names the real hazard (binding a bare `Var`
   to a quotation type), but the guard as written doesn't discriminate that case from
   a legitimate `PolyType::Concrete(Type::Quotation(..))` parameter, which should be
   allowed through and materialized exactly as the concrete twin (`apply_quot` above)
   already does. Probe-verified this rejects at **every** input position (first,
   middle, last), once the probe programs are correctly formed:

   ```
   : run_it ( 'T: Copy [ i64 -- i64 ] -- 'T ) drop ;
   : main ( -- ) 7 [ 1 add ] run_it drop ;
   ```

   fails with `` a quotation cannot be passed to `run_it`; only `call` accepts one (a
   runtime quotation value is slice 7) in `main` ``. The "(a runtime quotation value
   is slice 7)" clause in `reject_quotation_argument` (`src/check.rs:3032-3039`) is
   stale in the same way the roadmap flagged: 7a/7b shipped this long ago on the
   concrete side.
   **Pitfall, recorded so it isn't repeated:** the first two recon passes this
   session (including two subagent dispatches) reported this as an *order-dependent
   silent miscompilation* — quotation-first signatures rejected correctly,
   quotation-not-first signatures silently passed the check and failed later with a
   confusing body-effect mismatch. That finding was **entirely a probe-construction
   bug**, not a compiler bug: every probe involved wrote a bound's binding occurrence
   (`'T: Copy`) *and* a separate bare `'T` afterward, which are two distinct stack
   inputs, not one annotated one — the exact mistake this project's own memory
   already warns about (a bound's binding site is itself an input slot). Every
   "miscompiled" probe had accidentally declared one more input than it supplied
   arguments for; the compiler's diagnostics were correct throughout. Re-verified
   with `eprintln!` instrumentation on a scratch copy of the tree
   (`sig.inputs` printed three entries for a signature the probe author believed
   declared two) and with correctly-formed 2- and 3-input signatures at all three
   quotation positions, all consistently hitting R9p. **Do not re-litigate the
   "order matters" claim without triple-checking arity first.**

3. **The concrete-side materialization boundary (P4.S7a's own R8, distinct from
   R9p) is the mechanism this slice's fix extends, not invents — confirmed by OQ1's
   probe below.** A concrete word's `Type::Quotation`-typed parameter already
   accepts and materializes a literal argument (`apply_quot` above); once R9p is
   narrowed to only reject a quotation argument destined for a genuine
   `PolyType::Var` position, the poly-callee case needs that same
   `materialize_quotation_at_boundary` step invoked, which `check_poly_call` cannot
   do without a signature change (OQ1).

## Open questions

1. ~~Does narrowing R9p to spare a `PolyType::Concrete(Type::Quotation(..))`
   position reuse P4.S7a's existing materialization path unchanged, or does a poly
   callee's per-instantiation body-check need its own?~~ **Resolved: it needs the
   same materialization step, but `check_poly_call` cannot reach it today without a
   signature change.** Probed by patching a scratch copy of the tree: narrowing R9p
   to spare a declared ground `Type::Quotation` input lets the argument's `.quot`
   marker through, but `unify_poly_input`'s subsequent `PolyType::Concrete(t)` arm
   then compares the operand's *raw* `Slot.ty` — still the `Cstr` placeholder a
   literal carries before materialization — against the declared `Type::Quotation`,
   and fails (`` `run_it` expected `[ i64 -- i64 ]`, found `cstr` ``). The concrete
   side's own materialization boundary, `materialize_quotation_at_boundary`
   (`src/check/captures.rs:287-`), is exactly the function that needs to run here
   too — it is what turns a `Known` literal into a real, `quot: None`,
   `Type::Quotation`-typed value, invoked at the concrete call-argument loop's own
   R8 site (`src/check/terms.rs:773-785`). But `check_poly_call`
   (`poly.rs:3262-3274`) cannot call it as-is: it takes `prov: &Provenance` and
   `scope: &Scope` (both read-only), while `materialize_quotation_at_boundary`
   needs both `&mut`, plus `env`, `cells`, and `slices`, none of which
   `check_poly_call` currently receives. **This is real, scoped plumbing, not a new
   IR/lowering mechanism** — the same shape of signature-threading change S3b-follow
   already made to `poly_walk` (adding `combinators`/`poly_words`) — but it is not
   free, and a spec must budget for widening `check_poly_call`'s signature and every
   one of its call sites.

2. ~~Does the abstract-parameter case (Gap 1) need new machinery, or does closing
   Gap 2 change what "abstract" means here?~~ **Resolved: they are the same guard,
   and both need their own fix; narrowing one does not subsume the other.** Probed
   directly: once R9p is narrowed (OQ1's probe), the very next thing that fails is
   Gap 1's own guard in `poly_call_term`'s `call` handling (`poly.rs:953-958`) — it
   still rejects, now with a different rendering (`` `call` is not permitted on
   `[ i64 -- i64 ]` `` — the `PolyType::Concrete(t)` arm of
   `poly_op_on_variable_error`, not the `PolyType::Quotation(..)` arm Gap 1's own
   probe hits). This confirms they share one code path, but resolving Gap 2 does not
   implicitly resolve Gap 1: `poly_call_term`'s `call` handling only knows how to
   splice a literal's interned body (`scope.quotation(quot)`); a genuine parameter
   has no body to splice; a probe patch adding a poly analogue of the concrete
   side's `check_abstract_quotation_call` (`src/check/terms.rs:1123-1145` — pop the
   declared inputs, push the declared outputs, no splice) checked correctly once its
   own test program's stack shape was fixed. **The spec needs two changes at one
   shared guard, not one:** OQ1's materialization fix at the call-site argument
   boundary, and a new `call`-on-a-ground-parameter arm (the poly analogue of R8) at
   the body's own `call` site.

3. **What does `PolyType::Quotation(ins, outs, ..)` (the *non-folded*, genuinely
   abstract case — e.g. `[ 'T -- 'T ]`) need for Gap 1, distinct from the
   already-concrete case in OQ2?** This is the shape `unify_poly_input`'s own
   `PolyType::Quotation` arm (`poly.rs:~3413-3450`) already unifies correctly at the
   *call-site* boundary (row-pointwise, binding any variable the row mentions) — the
   open gap is purely that the poly *body* itself can't `call` it once bound. Confirm
   this doesn't need a new representation, only a new dispatch arm in `poly_call_term`
   parallel to the literal-splice arm S3d added, but grounded (calling the concretely
   *bound* instantiation's quotation, not a body-local literal).

4. **Message text.** `reject_quotation_argument`'s "(a runtime quotation value is
   slice 7)" clause needs retiring regardless of scope — it's user-facing and has
   been wrong since 7a/7b shipped, independent of whatever this slice narrows the
   guard to.

## Out of scope

- `~[ ]` (`InlineQuotation`) crossing this boundary at all. A non-inline word cannot
  declare one, and that gate is correct, not a gap: `~[ ]` is splice-only by design
  (`src/check/word_entry.rs:112-142`), has no runtime representation, and giving it
  one would just reinvent plain `[ ]` under a different sigil.
- Anything already covered by P7.S3d (a quotation *literal* written inside a poly
  body, splicing in place, or grounding against a concrete callee's declared
  quotation parameter from *within* a poly body). This slice's gap is the
  *parameter*/*argument-boundary* pair, a different code path.

## Ready to spec?

**Yes.** Both open questions are now probe-verified against a patched scratch copy of
the compiler, not inferred:

- **Three changes, not one, are needed**, all at or adjacent to the same guard
  family in `src/check/poly.rs`:
  1. Narrow R9p (`check_poly_call`, `poly.rs:3270-3271`) to spare a declared ground
     `Type::Quotation` input, rejecting only the genuinely unsound case (a bare
     `PolyType::Var` position).
  2. Thread `materialize_quotation_at_boundary`
     (`src/check/captures.rs:287-`) into `check_poly_call` for that spared case —
     which needs `check_poly_call`'s signature widened (`prov`/`scope` from `&`/`&`
     to `&mut`/`&mut`, plus `env`, `cells`, `slices` added) and every call site
     updated, the same shape of change S3b-follow already made to `poly_walk`.
  3. Add a poly analogue of the concrete side's `check_abstract_quotation_call`
     (`src/check/terms.rs:1123-1145`) to `poly_call_term`'s `call` handling
     (`poly.rs:953-958`), for when the top-of-stack operand is a genuine
     (materialized, non-literal) ground `Type::Quotation` parameter: pop the
     declared inputs, push the declared outputs, no splice.
- **No new IR, lowering, or runtime representation is needed** for the concrete
  (ground `Type::Quotation`) case — confirmed by literally reusing the concrete
  side's own two small helper functions rather than inventing poly-specific ones.
  The abstract (`PolyType::Quotation(ins, outs, ..)`, genuinely variable-bearing)
  case from OQ3 is untouched by this probe pass and remains open for the spec to
  scope in or explicitly defer.
- Both retracted-and-recovered findings above (the position-independence
  correction, and this probe pass) should be treated as load-bearing recon, not
  just narrative: they're what makes the fix's shape ordinary plumbing instead of
  an unknown-sized redesign.
