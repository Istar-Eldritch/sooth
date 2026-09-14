# SOO-30 probe round 3 findings (measure-then-pin, pre-Phase-2-goldens)

Round 3 at HEAD `bc702fb` (Phase 1 gate relaxation LANDED). Temporarily
implements the R-30.2 output-side grounding route (Phase 2's real scope,
minimally), measures the at-risk goldens' live bytes, then reverts everything —
final tree = committed HEAD + untracked `probes/soo30r3_*` files only.
Reference bytes: `probes/soo30r2_findings.md` + `probes/soo30r2_baseline.md`
(walls r2b-1, r2b-1c, r2b-2). All invocations:
`cargo run -q -- build probes/<f>.sth --manifest tests/fixtures/sooth.pkg`.

## The temporary patch (Phase 2's starting point)

Three sites, minimal, per the spec's Phase 2 scope. Full diff pasted at the
bottom of this file (also the receipt for the Phase 2 implementer).

1. `src/check/poly/ground.rs` — `resolve_mono_member_call`'s S6/S8b escape
   hatch: for a candidate whose output row contains a trait-var-headed App
   (`output_trait_var_app`, new helper; one `Ref` layer unwrapped) and a SINGLE
   explicit type argument, bind the member's residual vars from the output
   App's arguments, positionally against the instantiation's ctor arguments
   (`instantiation_ctor_args`, new helper — reads the interned instantiation
   via `GenericTypes::struct/enum_instantiation_of`), and extend
   `impl_target_seed` with them. Union id of an appended member local =
   `target_var_count + v - 1` (member-sig var `v >= 1`; with no dispatchable
   input no local is identifying, so the append rank is `v - 1` —
   `build_member_var_union`, src/parser.rs:768). `find_bound_impl` untouched —
   it already keys impl selection on the dissolved ctor head.
2. `src/check/poly.rs` — `check_poly_call` gains an `output_app_route: bool`
   parameter (set only by the escape hatch's extended-seed path; both other
   callers pass `false`); the arity gate takes the route's exception
   (`&& !output_app_route`). Wrong-arity supplies on non-route shapes keep
   today's bytes.
3. `src/check/poly/ground.rs` — `mono_nullary_member_no_instantiation_error`
   renders its example via `mono_nullary_remedy_example` (new helper): the
   R-30.2 output-App spelling `member[Head[args]]` from the member's output App
   and the trait's first impl target head (`user_spelling`, else the pattern's
   name); App args resolve against the operand stack via the declared input
   that binds each var; every unrenderable shape falls back to the placeholder
   `{member}[i64]`. Line 1 byte-identical (same format prefix).

## R3-G14 — the output-App route end-to-end (THE primary target)

PREDICTION (R-30.2): builds, runs, prints the marker.

RESULT: **CONFIRMED — build exit=0, run prints `42\n`, exit=0.** The single
explicit instantiation `pure[Box[i64]]` grounds the return type at a mono call
site with no `'F` operand: r2b-1's arity wall (`declares 2 type variables ...
but was given 1 type argument`) is removed by the route, impl selection keyed
on the dissolved ctor head picked the Box impl, and dispatch lowered
end-to-end. The route is reachable only through the escape hatch (single
zero-input candidate, explicit single argument); operand-driven selection and
the parser fence are untouched.

Bytes: `probes/soo30r3_baseline.md` § g14. Fixture: `probes/soo30r3_g14.sth`.

## R3-G6 — bare-call remedy with the corrected example

PREDICTION (R-30.3): line 1 byte-identical to r2b-2; example line names the
output-App spelling.

RESULT: **CONFIRMED.** Line 1 keeps r2b-2's exact wording and span shape
(`error: \`pure\` in \`main\` (line 13, col 18) is a trait member with no
operand to dispatch on` — located at this fixture's own line/col; r2b-2's
fixture had the call at line 9). The example line now reads `e.g.
\`pure[Box[i64]]\``— the R-30.2 output-App spelling, rendered from the member's
output App (`'F['A]` → head `Box` from the impl target, arg `i64` from the
operand stack) and proven achievable by R3-G14. The r2b-2 example `pure[i64]`
is gone.

Bytes: `probes/soo30r3_baseline.md` § g6. Fixture: `probes/soo30r3_g6.sth`.

## R3-G8 — 2-arg positional stays operand-driven

PREDICTION (R-30.2 conservative face): still fails impl selection with the
r2b-1c bytes.

RESULT: **CONFIRMED — no ESCALATE.** Byte-identical to r2b-1c's template: `is a
trait member of Applicative, but no \`impl:\` in this program dispatches on
these operands`+ the operand-types line (`i64`). The route did not fire: two
written arguments are not the single-argument output-App form, so the supply
kept operand-driven selection and the no-dispatch diagnostic. R-30.2's
conservative face stands as ruled.

Bytes: `probes/soo30r3_baseline.md` § g8. Fixture: `probes/soo30r3_g8.sth`.

## R3-G13 — double-wrap idiom survives (non-leakage)

PREDICTION (R-30.7): builds and runs, prints `42\n` (r2b-3b bytes).

RESULT: **CONFIRMED — no regression.** Build exit=0, run stdout `42\n`, exit=0
— byte-identical to r2b-3b. The 2-arg positional supply on an operand-carrying
call still rides the ordinary S8b seed channel (arity 2 == 2 vars passes the
gate positionally; the extended-seed route is single-argument only), and
operand-driven selection is untouched.

Bytes: `probes/soo30r3_baseline.md` § g13. Fixture: `probes/soo30r3_g13.sth`.

## R3-G2/G3/G4 — lib ctors through the route (temporary scaffolding)

PREDICTION: build + run print `5\n` per ctor; 2-param Result (G3) is the
measured finding either way.

RESULT: **ALL THREE CONFIRMED — no ESCALATE, including the at-risk G3.**
Scaffolding per the r2c receipt (`lib/core/applicative.sth` true-shape trait +
`export: Applicative ;`; `applicative` inserted before `option` in sooth.pkg;
impls co-located in option/result/list, each headed `import: self::applicative
| Applicative | ;`, list also `import: intrinsics | ^ | ;`):

- G2 `5 pure[Option[i64]] showopt` → build exit=0, run `5\n`, exit=0.
- G3 `5 pure[Result[i64 i64]] showres` (2-param Result, 1-arg spelling) →
  build exit=0, run `5\n`, exit=0. The seed extension handles the 2-param
  target: ctor args `[i64, i64]` bind `'ctor0`/`'ctor1`, and the residual `'A`
  (union id 2 = target vars 2 + rank 0) binds from the output App's single
  argument. The partial-head ride-along intent holds without forcing.
- G4 `5 pure[List[i64]] showlist` → build exit=0, run `5\n`, exit=0 — a real
  Cons cell (the cell-boxing impl body `Nil ^ Cons` checked and dispatched).

All three impls checked under the route with no P2e-class mismatch.

Bytes: `probes/soo30r3_baseline.md` §§ g2, g3, g4. Fixtures:
`probes/soo30r3_g2.sth`, `probes/soo30r3_g3.sth`, `probes/soo30r3_g4.sth`.

## R3-G5 — shared-bound consumer dispatches two ctors

PREDICTION: `7\n7\n`.

RESULT: **CONFIRMED — build exit=0, run `7\n7\n`, exit=0.** `repure['F:
Applicative 'A] ( 'F['A] 'A -- 'F['A] ) swap drop pure ;` (the paper's
corrected body) checks and dispatches at two mono sites, called BARE — both
vars operand-grounded, no explicit instantiation (the R-30.4 consumer shape:
`'F` in an input). The body's bare `pure` rides the pre-existing poly
member-call path ('F's Applicative bound); the route flag on that path is
false, so G5 measured the poly path as it stands — it works. An intermediate
attempt (`5 nile 7 repure showlist`) failed with a main stack-effect mismatch;
recorded in the baseline as a probe-authoring bug (unconsumed literal before
`nile`), NOT a checker wall — the Option-only variant printed `7\n` with the
identical preamble.

Bytes: `probes/soo30r3_baseline.md` § g5. Fixture: `probes/soo30r3_g5.sth`.

---

## Full diff receipt (Phase 2's starting point)

(pasted below before the revert)

```diff
// === the temporary Phase-2 route patch (src/), reverted after the round ===
diff --git a/src/check/poly.rs b/src/check/poly.rs
index 080ecf7..b938820 100644
--- a/src/check/poly.rs
+++ b/src/check/poly.rs
@@ -4108,7 +4108,15 @@ pub(super) fn check_poly_call(
     // `type_args` channel, whose positional contract (P7.S3t, below) binds
     // variable `i` from argument `i`. `None` at every other caller, which
     // keeps that contract exactly as it was.
+    //
+    // P7b.S15 R-30.2: the supply rode the output-App route
+    // (`resolve_mono_member_call`'s extended seed): the single written type
+    // argument is the output-App instantiation, not one value per dissolved
+    // variable, so the positional arity comparison below is exempted for
+    // exactly that shape. Every other caller passes `false` and keeps
+    // today's bytes.
     impl_target_seed: Option<&Subst>,
+    output_app_route: bool,
     stack: &mut Vec<Slot>,
     ctx: &Ctx,
     env: &HashMap<String, Vec<Overload>>,
@@ -4160,7 +4168,13 @@ pub(super) fn check_poly_call(
     // after pass 2.
     let mut seeded: Vec<u32> = Vec::new();
     if !type_args.is_empty() {
-        if type_args.len() != sig.ty_var_names.len() {
+        // P7b.S15 R-30.2: the output-App route's exception. A supply routed
+        // through the extended impl-target seed carries ONE written argument
+        // (the output-App instantiation, e.g. `pure[Box[i64]]`) for a
+        // dissolved member word with target params + member locals; it is not
+        // compared positionally against the var count. Wrong-arity supplies
+        // on any non-route shape keep this gate's bytes.
+        if type_args.len() != sig.ty_var_names.len() && !output_app_route {
             return Err(instantiation_arity_error(span, name, &sig, type_args.len()));
         }
         // P7b.S8b Phase 1 (R4): the arity gate above still reads the
diff --git a/src/check/poly/ground.rs b/src/check/poly/ground.rs
index be9db2d..d53afe7 100644
--- a/src/check/poly/ground.rs
+++ b/src/check/poly/ground.rs
@@ -841,6 +841,7 @@ pub(in crate::check) fn resolve_splice_member_call(
                 &[],
                 &[],
                 None,
+                false,
                 stack,
                 ctx,
                 env,
@@ -1180,6 +1181,12 @@ pub(in crate::check) fn resolve_mono_member_call(
     // `Opt[Opt[i64]]`, whose variant words then clobbered lowering's
     // bare-name map and re-typed every `Some`/`Cons` in the program.
     let mut impl_target_seed: Option<Subst> = None;
+    // P7b.S15 R-30.2: true when the escape hatch routed a single explicit type
+    // argument through the member's trait-var-headed output App (the extended
+    // seed below). `check_poly_call`'s arity gate exempts the routed supply:
+    // the written argument is the output-App instantiation, not one positional
+    // value per dissolved variable.
+    let mut output_app_route = false;
     if viable.is_empty() {
         if let (Some(&ty), [(zero_tid, zero_m)]) = (
             type_args.first(),
@@ -1203,8 +1210,53 @@ pub(in crate::check) fn resolve_mono_member_call(
                 refs,
                 &mut visited,
             )? {
+                let mut seed = subst;
+                // P7b.S15 R-30.2: for an R-30.1-admitted member (no
+                // dispatchable input, output row carries a trait-var-headed
+                // App) the single written type argument IS the output-App
+                // instantiation: `find_bound_impl` above already keyed impl
+                // selection on the dissolved ctor head and bound the target's
+                // own variables (`'ctor0 := i64` for `pure[Box[i64]]`). What
+                // is missing is the member's residual locals: bind each from
+                // the output App's arguments, positionally against the
+                // instantiation's ctor arguments (`'A := i64`), so θ is
+                // complete before the operand unifies and a disagreeing
+                // operand stays the seeded-conflict diagnostic (P7.S3t)
+                // instead of silently re-grounding the written instantiation.
+                // The union id of an appended member local is `target vars +
+                // declaration rank` (`build_member_var_union`,
+                // src/parser.rs:768): with no dispatchable input no local is
+                // identifying, so member-sig variable `v >= 1` lands at
+                // `target_var_count + v - 1`.
+                if type_args.len() == 1 {
+                    if let Some(app_args) = output_trait_var_app(&zero_m.sig) {
+                        let target_var_count =
+                            poly.trait_resolve.impls[imp_idx].target.ty_var_names.len() as u32;
+                        for (i, arg) in app_args.iter().enumerate() {
+                            let PolyType::Var(v) = arg else {
+                                continue;
+                            };
+                            if *v == 0 {
+                                continue;
+                            }
+                            let uid = target_var_count + v - 1;
+                            if seed.ty_of(uid).is_some() {
+                                continue;
+                            }
+                            let Some(ctor_args) = instantiation_ctor_args(ty, ctx) else {
+                                continue;
+                            };
+                            let Some(val) = ctor_args.get(i) else {
+                                continue;
+                            };
+                            let pos = seed.ty.partition_point(|(id, _)| *id < uid);
+                            seed.ty.insert(pos, (uid, *val));
+                            output_app_route = true;
+                        }
+                    }
+                }
                 viable.push((*zero_tid, *zero_m, imp_idx));
-                impl_target_seed = Some(subst);
+                impl_target_seed = Some(seed);
             }
         }
     }
@@ -1215,8 +1267,17 @@ pub(in crate::check) fn resolve_mono_member_call(
                     .iter()
                     .any(|(_, m)| dispatchable_input_pos(&m.sig).is_none())
             {
+                let example = mono_nullary_remedy_example(
+                    member,
+                    &candidates,
+                    &poly.trait_resolve.impls,
+                    stack,
+                );
                 return Err(mono_nullary_member_no_instantiation_error(
-                    ctx, span, member,
+                    ctx,
+                    span,
+                    member,
+                    &example,
                 ));
             }
             return Err(mono_member_no_dispatch_error(
@@ -1470,6 +1531,7 @@ pub(in crate::check) fn resolve_mono_member_call(
             type_args,
             len_args,
             impl_target_seed.as_ref(),
+            output_app_route,
             stack,
             ctx,
             env,
@@ -1511,15 +1573,116 @@ fn mono_member_no_dispatch_error(
 /// zero-dispatchable-input member with no explicit type argument to ground
 /// its trait variable. Q1 rules out consuming-context inference for this
 /// slice, so this is a located error naming the remedy, not a lookahead.
-fn mono_nullary_member_no_instantiation_error(ctx: &Ctx, span: Span, member: &str) -> String {
+fn mono_nullary_member_no_instantiation_error(
+    ctx: &Ctx,
+    span: Span,
+    member: &str,
+    example: &str,
+) -> String {
     format!(
-        "error: `{member}` in {name} (line {}, col {}) is a trait member with no operand to dispatch on\n  a monomorphic body cannot infer the trait's type here; write an explicit type argument, e.g. `{member}[i64]`",
+        "error: `{member}` in {name} (line {}, col {}) is a trait member with no operand to dispatch on\n  a monomorphic body cannot infer the trait's type here; write an explicit type argument, e.g. `{example}`",
         span.line,
         span.col,
         name = ctx.rendered_word(),
     )
 }
 
+/// P7b.S15 R-30.2: the member's trait-var-headed output App, if its output
+/// row contains one -- the App whose arguments bind the member's residual
+/// locals. One `Ref` layer is unwrapped, mirroring the declaration gate's
+/// courtesy (`member_binds_trait_var`'s output arm).
+fn output_trait_var_app(sig: &PolySig) -> Option<&[PolyType]> {
+    sig.outputs.iter().find_map(|t| match t {
+        PolyType::App { head: 0, args } => Some(args.as_slice()),
+        PolyType::Ref(referent, _) => match referent.as_ref() {
+            PolyType::App { head: 0, args } => Some(args.as_slice()),
+            _ => None,
+        },
+        _ => None,
+    })
+}
+
+/// P7b.S15 R-30.2: the concrete ctor arguments an instantiation type carries
+/// (`Box[i64]` -> `[i64]`), read off the interned instantiation the parser
+/// minted for the call-site type argument.
+fn instantiation_ctor_args(ty: Type, ctx: &Ctx) -> Option<Vec<Type>> {
+    let generics = ctx.generics()?;
+    let g = generics.borrow();
+    match ty {
+        Type::Struct(id, _) => {
+            g.struct_instantiation_of(id)
+                .map(|(_, _, args, _)| args.to_vec())
+        }
+        Type::Enum(id, _) => {
+            g.enum_instantiation_of(id)
+                .map(|(_, _, args, _)| args.to_vec())
+        }
+        _ => None,
+    }
+}
+
+/// P7b.S15 R-30.3: the bare-call remedy's example token -- the achievable
+/// output-App spelling `member[Head[args]]`, rendered from the member's
+/// trait-var-headed output App and the trait's impl target head. App
+/// arguments resolve against the operand stack: an App argument naming a
+/// member local takes the declared input's operand type (the input that
+/// binds it), so the advice substituted at the call site builds and
+/// dispatches under R-30.2. The single-impl case is the pinned one (G6);
+/// every shape the rendering cannot name keeps the placeholder spelling.
+fn mono_nullary_remedy_example(
+    member: &str,
+    candidates: &[(TraitId, &TraitMember)],
+    impls: &[ImplDecl],
+    stack: &[Slot],
+) -> String {
+    let fallback = || format!("{member}[i64]");
+    let zero: Vec<&(TraitId, &TraitMember)> = candidates
+        .iter()
+        .filter(|(_, m)| dispatchable_input_pos(&m.sig).is_none())
+        .collect();
+    let [(tid, m)] = zero.as_slice() else {
+        return fallback();
+    };
+    let Some(app_args) = output_trait_var_app(&m.sig) else {
+        return fallback();
+    };
+    if app_args.is_empty() {
+        return fallback();
+    }
+    let Some(imp) = impls.iter().find(|i| i.trait_id == *tid) else {
+        return fallback();
+    };
+    let head = imp
+        .target
+        .user_spelling
+        .as_ref()
+        .map(|(n, _)| &**n)
+        .unwrap_or_else(|| match &imp.target.pattern {
+            PolyType::Generic { name, .. } => &**name,
+            _ => "",
+        });
+    if head.is_empty() {
+        return fallback();
+    }
+    let n_in = m.sig.inputs.len();
+    let parts: Vec<String> = app_args
+        .iter()
+        .map(|arg| match arg {
+            PolyType::Var(v) if *v >= 1 => m
+                .sig
+                .inputs
+                .iter()
+                .position(|i| matches!(i, PolyType::Var(w) if *w == *v))
+                .filter(|_| stack.len() >= n_in)
+                .map(|p| stack[stack.len() - n_in + p].ty.name().to_string())
+                .unwrap_or_else(|| "i64".to_string()),
+            PolyType::Concrete(t) => t.name().to_string(),
+            _ => "i64".to_string(),
+        })
+        .collect();
+    format!("{member}[{head}[{}]]", parts.join(" "))
+}
+
 /// P7b.S2 (S2-16, mono caller). `poly_env` is in fact built once,
 /// whole-program (`src/check.rs:668-706`), so every member word a found impl
 /// dispatches to is present in it -- the non-inline call site (S2-16's
diff --git a/src/check/terms.rs b/src/check/terms.rs
index 65bfc41..58d8729 100644
--- a/src/check/terms.rs
+++ b/src/check/terms.rs
@@ -900,8 +900,8 @@ fn check_term(
             // has already established is where a non-empty one may arrive.
             if poly.env.contains_key(name) && !fall_through_to_env {
                 return check_poly_call(
-                    name, span, type_args, len_args, None, &mut stack, ctx, env, scope, arrays,
-                    cells, refs, slices, prov, live, at, poly,
+                    name, span, type_args, len_args, None, false, &mut stack, ctx, env, scope,
+                    arrays, cells, refs, slices, prov, live, at, poly,
                 );
             }
             // P7.S3o Phase 3: a bare trait member call (like `cmp` directly)

// === the temporary lib scaffolding (lib/ + applicative.sth, reverted after the round) ===
// applicative.sth content (new file, deleted on revert):
\ core::applicative -- return-type-polymorphic construction (P7b.S15, SOO-30).
\ `Applicative['F: * -> *]` declares `pure`, the R-30.1-admitted output-App
\ member: its nonempty inputs mention no dispatchable trait-var head and its
\ output names the trait var as an application head. Grounding happens at the
\ mono call site through the explicit instantiation (`5 pure[Option[i64]]`,
\ R-30.2's output-side route); there is no consuming-context inference (the
\ S6 Q1 rule). Member words are synthesized per impl and never bare-nameable,
\ so no member name is exported (the `core::iterator` export convention).
export: Applicative ;
trait: Applicative['F: * -> *]
  : pure ( 'A -- 'F['A] ) ;
;

diff --git a/lib/core/list.sth b/lib/core/list.sth
index 047ce64..aa66757 100644
--- a/lib/core/list.sth
+++ b/lib/core/list.sth
@@ -1,4 +1,10 @@
+import: self::applicative | Applicative | ;
+import: intrinsics | ^ | ;
 type: List['T] | Nil | Cons 'T rest ^List['T] ;
 export: List ;
 export: Nil ;
 export: Cons ;
+
+impl: Applicative for List
+  : pure Nil ^ Cons ;
+;
diff --git a/lib/core/option.sth b/lib/core/option.sth
index 63eb654..9da35d2 100644
--- a/lib/core/option.sth
+++ b/lib/core/option.sth
@@ -1,4 +1,9 @@
+import: self::applicative | Applicative | ;
 type: Option['T] | None | Some 'T ;
 export: Option ;
 export: Some ;
 export: None ;
+
+impl: Applicative for Option
+  : pure Some ;
+;
diff --git a/lib/core/result.sth b/lib/core/result.sth
index b292ea3..2b9421a 100644
--- a/lib/core/result.sth
+++ b/lib/core/result.sth
@@ -1,4 +1,9 @@
+import: self::applicative | Applicative | ;
 type: Result['T 'E] | Ok 'T | Err 'E ;
 export: Result ;
 export: Ok ;
 export: Err ;
+
+impl: Applicative for Result
+  : pure Ok ;
+;
diff --git a/lib/core/sooth.pkg b/lib/core/sooth.pkg
index 2896f05..d7cda95 100644
--- a/lib/core/sooth.pkg
+++ b/lib/core/sooth.pkg
@@ -5,4 +5,4 @@
 \ on the typed surface instead of one per module.
 package: core ;
 layer: core ;
-module: bool cmp prelude combinators option result show list iterator range ;
+module: bool cmp prelude combinators applicative option result show list iterator range ;
```
