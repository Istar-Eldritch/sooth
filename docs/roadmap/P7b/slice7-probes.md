# P7b.S7 recon probe log — quotation effects over type constructors (the `call` extension)

- Date: 2026-09-05. HEAD: `a9eca84` (suite green, all crates, no failures).
- Binary: worktree `target/debug/sooth` (dev profile, `cargo build`).
- Fixtures: `/tmp/p7bs7-probes/*` (scratch only, not committed).
- Method: minimal `.sth` fixtures per shape, built with `sooth build <fixture>`, verbatim output captured. All commands re-run once for reproducibility; outputs were stable.

---

## P1 — Monad.bind shape, plain `[ ]` quotation syntax

Hypothesis: `bind ( 'F['T] [ 'T -- 'F['U] ] -- 'F['U] )` hits the S2-15.d row fence at `trait:` declaration time, before `call` is ever involved.

Fixture (`/tmp/p7bs7-probes/p1-monad-bind/main.sth`):

```
trait: Monad['F: * -> *] : bind ( 'F['T] [ 'T -- 'F['U] ] -- 'F['U] ) ; ;

: main ( -- ) ;
```

Command:

```
sooth build /tmp/p7bs7-probes/p1-monad-bind/main.sth
```

Output (verbatim):

```
error: trait `Monad`'s member at line 1, col 28 applies a type variable inside a quotation row (`'F[...]` may not appear inside `[ ... ]`)
  note: a type application is supported only in a plain signature slot; keep quotation rows App-free
exit: 1
```

Verdict: **confirmed, at declaration time.** This is `app_in_member_quotation_row_error` (`src/parser.rs:452`), raised from the shape gate at `parse_trait_member_effect`'s post-parse loop (`src/parser.rs:3950`), which calls `member_shape_is_supported` (`src/parser.rs:379`) — the `Quotation` arm (`src/parser.rs:391`) rejects any row where `member_quotation_row_mentions_app` (`src/parser.rs:421`) is true. `call` is never reached; the trait itself never parses to a registered `TraitDecl`. This matches the doc note verbatim: "declarations represent it, `call` cannot see through it" is slightly imprecise — declarations do *not* represent it either; the declaration itself is rejected. (Existing golden: `app_inside_member_quotation_row_is_fenced`, `tests/phase7b_slice2.rs:204`, pins the same fixture shape with `Functor.map`.)

Col 28 is the App inside the *output* row's `'F['U]`; the input row `'F['T]` (col 3ish) is a plain slot and is fine on its own (that's exactly S2's existing `Functor.map` shape).

---

## P2 — same shape, isolating which row triggers it

Hypothesis: does the fence fire on the *input* row's `'T -- 'F['U]` regardless of whether the row's App is on the input or output side? (Already implicit in P1's column, but confirm with a minimal case that only has the App on the input side of the quotation row, output side plain.)

Fixture:

```
trait: Peek['F: * -> *] : peek ( 'F['T] [ 'F['T] -- 'U ] -- 'U ) ; ;
```

Command:

```
sooth build /tmp/p7bs7-probes/p2b-app-in-input-row/main.sth
```

Output:

```
error: trait `Peek`'s member at line 1, col 27 applies a type variable inside a quotation row (`'F[...]` may not appear inside `[ ... ]`)
  note: a type application is supported only in a plain signature slot; keep quotation rows App-free
exit: 1
```

Verdict: **confirmed.** Symmetric — `member_quotation_row_mentions_app` scans both `ins` and `outs` (`src/parser.rs:428`), so the fence is row-position-blind: an App on either side of the quotation's declared effect fires it. This is in fact the exact fixture already pinned by `parse_trait_decl_app_inside_member_quotation_row_is_fenced` (`src/parser.rs:11861`), just with `Peek` renamed from `Functor`. No new information beyond confirming symmetry, but rules out "maybe only the output-row case is fenced."

---

## P3 — Applicative.ap shape: constructor applied over an effect type itself

Hypothesis: `ap ( 'F[ [ 'A -- 'B ] ] 'F['A] -- 'F['B] )` — where the App's *argument* is a quotation type, not the App sitting inside a quotation's row — is a distinct, earlier fence (S1, not S2-15.d), and may not even parse as a type independent of any trait context.

Fixture:

```
trait: Applicative['F: * -> *] : ap ( 'F[ [ 'A -- 'B ] ] 'F['A] -- 'F['B] ) ; ;
```

Command:

```
sooth build /tmp/p7bs7-probes/p3-applicative-ap/main.sth
```

Output:

```
error: expected a type, found `[` at line 1, col 43 (a type application's arguments are types, not quotations)
exit: 1
```

Verdict: **confirmed, and it's a different fence than P1's.** This is `app_arg_quotation_error` (`src/parser.rs:2630`), raised by `parse_poly_app_arg` (`src/parser.rs:5206`) — the moment the parser sees `[` as a type-application *argument*, it errors immediately, before the application even finishes parsing (this is `raw_to_poly_type`'s `App` fold, `src/parser.rs:5611`, upstream of `member_shape_is_supported` entirely). This fence is documented as **P7b.S1** (`S1-6`), i.e. it predates S2's trait/HKT work by a full slice and has nothing to do with traits at all.

---

## P4 — is Applicative.ap's shape parseable as a *type* outside any trait/member context?

Hypothesis (from the task): probe whether `'F[ [ 'A -- 'B ] ]` is even kind-checkable as a type, independent of `call` or trait declarations.

Fixture (plain poly word, no trait):

```
: h ( 'F[ [ i64 -- i64 ] ] -- ) drop ;
```

(`/tmp/p7bs7-probes/p6-var-headed-app-quotation-arg/main.sth`, with `import: intrinsics | drop | ;`)

Command:

```
sooth build /tmp/p7bs7-probes/p6-var-headed-app-quotation-arg/main.sth
```

Output:

```
error: expected a type, found `[` at line 2, col 11 (a type application's arguments are types, not quotations)
exit: 1
```

Verdict: **confirmed — same S1 fence, unconditional.** `'F[...]` (a type-*variable*-headed application) never admits a quotation argument anywhere in the grammar, trait or no trait, `call` or no `call`. This is strictly a parser-level restriction on `RawTy::App`'s argument grammar (`parse_poly_app_arg`), not a checker-level or dispatch-level one.

## P5 — control: does a *named-constructor* application (not a type-variable App) admit a quotation argument at a signature site?

Hypothesis: `Box[T]`-style named-generic applications go through a different parse path (`Generic`, not `App`) and might not carry the same S1 fence — worth knowing since it bears on whether the restriction is fundamental to "constructor applied to a quotation" or an artifact of the `'F[...]` variable-headed grammar specifically.

Fixture:

```
import: intrinsics | drop | ;
type: Box['T] v 'T ;
: h ( Box[ [ i64 -- i64 ] ] -- ) drop ;
```

Command:

```
sooth build /tmp/p7bs7-probes/p5-plain-app-of-quotation/main.sth
```

Output: no parse/check error (fails only at link time with `undefined reference to sooth_main`, expected — no `main` word defined in this scratch fixture). The quotation-typed generic argument parses and type-checks cleanly.

Verdict: **the S1 fence is specific to `RawTy::App` (`'F[...]`, a type-*variable* head), not to "a constructor applied to a quotation" in general.** A named header's application list (`Box[...]`) goes through `parse_generic_field_application`/the signature-site `Generic` argument parser, which has no analogous quotation-argument fence — `Box[[ i64 -- i64 ]]` is representable today as `PolyType::Generic`. This means Applicative.ap's shape is blocked specifically because `'F` is abstract (a trait's own higher-kinded variable), not because "constructor of a quotation" is inherently unrepresentable — the concrete case already works. The gap is that the *abstract*-head grammar (`App`) was never given the same argument freedom the *named*-head grammar (`Generic`) has.

---

## Existing precedent check: does any non-HKT mechanism already ground a quotation whose declared effect mentions an unbound output var?

Grep/read: `src/check/poly.rs`.

- `apply_subst` (`src/check/poly.rs:10210`) already has a working `App` arm (`src/check/poly.rs:10448`): given a ground `θ`, it grounds a plain-slot `'F['U]` output by looking up `θ`'s binding for `'F`'s var (expects a `Type::CtorImage`), then substituting the application's arguments through the constructor's declared parameters — this is exactly what already grounds `Functor.map`'s `'F['U]` output today (S2's shipped mechanism). It is *not* wired to descend into a `PolyType::Quotation`'s rows, because no such row can exist yet (S2-15.d fences it at parse time before this code ever runs).
- `apply_subst`'s `Quotation` arm (`src/check/poly.rs:10265`-ish) deliberately leaves quotation rows ungrounded ("substituting the caller region into an interned effect would mint one no literal could ever equal") — grounding for a quotation *parameter's* rows happens callee-side instead, in two functions:
  - `poly_call_ground_quotation_param` (`src/check/poly.rs:4377`) — fully concrete `QuotEffect`, no vars, no `Subst` involved at all.
  - `poly_call_abstract_quotation_param` (`src/check/poly.rs:4426`) — the **actual existing precedent for the shape S7 needs**: it grounds an abstract quotation parameter (rows still carrying unbound `PolyType::Var`s, e.g. today's ordinary generic `[ 'T -- 'U ]`) by **structural equality**, no `Subst` built or consulted (S3b's "L1: variables stay rigid" discipline, matching P7.S3o/S3l/S3b). It compares the operand's declared `PolyType` against each row-slot's declared `PolyType` via derived `Eq`, and `PolyType::App` already implements structural `Eq`/pattern-matching for exactly this purpose elsewhere (`src/check/poly.rs:1352`, the `unify_poly_input`-style App/App structural-match arm used by dispatch).

**Conclusion on grounding machinery:** the mechanism S7 would extend is `poly_call_abstract_quotation_param`'s structural-equality path, not `apply_subst`'s Subst-based one — `bind`'s quotation parameter is called with `'F` still abstract at the point of the trait member's own body-level `call` (it's the *member's own* declared quotation, not a caller-supplied literal being substituted). Extending it to accept `PolyType::App { head: 0, .. }` row-slots (which already have working structural-equality code elsewhere) looks like the smaller, better-grounded extension than inventing new machinery — but the parse-time fences (S2-15.d for the row case, S1's `app_arg_quotation_error` for Applicative's constructor-of-quotation case) both have to be lifted *before* any of this is reachable, and lifting them safely requires auditing every other place `member_shape_is_supported`/`poly_type_app_head` assume "an App inside a quotation row can't exist" (e.g. `ground_member_type`'s Quotation arm, `fence_member_app_against_concrete_target`'s "Quotation rows never carry an App" comment at `src/ast.rs:2116`) — those are load-bearing invariants elsewhere in the codebase today, not just declaration-time noise.

---

## Summary

| Shape | Fence | Location | Timing |
|---|---|---|---|
| Monad.bind: App inside a quotation row (`[ 'T -- 'F['U] ]`) | `app_in_member_quotation_row_error` | `src/parser.rs:452`, triggered via `member_shape_is_supported` (`src/parser.rs:379`) called from `parse_trait_member_effect` (`src/parser.rs:3950`) | Parse time, at `trait:` declaration — before `check`, before dispatch, before `call` |
| Applicative.ap: App applied to a quotation argument (`'F[ [ 'A -- 'B ] ]`) | `app_arg_quotation_error` | `src/parser.rs:2630`, raised at `:5213` from `parse_poly_app_arg` (`src/parser.rs:5206`) | Parse time — an even earlier, S1-era fence, unrelated to traits; fires for *any* `'F[...]` application, in or out of a trait context |
| Same shape via a *named* constructor (`Box[[i64--i64]]`) | none | — | Parses and checks fine; the S1 fence is specific to variable-headed `App`, not to constructor-over-quotation in general |

**What grounds today, unextended:** the abstract-quotation-parameter `call` path (`poly_call_abstract_quotation_param`) already grounds a declared quotation effect that mentions an unbound plain type var (ordinary non-HKT generics, e.g. `[ 'T -- 'U ]`), via structural equality with no `Subst`. `apply_subst`'s `App` arm already grounds a plain-slot HKT output (`'F['U]`, S2's shipped `Functor.map`) via `Subst`-based `CtorImage` substitution. Neither path has ever had to combine the two (an App *inside* a quotation row) because the parser refuses to construct that `PolyType` at all.

**What's genuinely new work:** (1) lifting the S2-15.d row fence to let `PolyType::App` appear inside a member's quotation row `PolyType`, (2) lifting the S1 `app_arg_quotation_error` fence to let a variable-headed `App`'s argument be a quotation type, (3) auditing every function whose comments currently assert "a quotation row/App-argument can never carry the other" (at minimum: `ground_member_type`'s Quotation arm, `fence_member_app_against_concrete_target` in `src/ast.rs:2116`, `poly_type_app_head`'s row-blind-by-design contract in `src/parser.rs:902-913`) since those are invariants other diagnostics rely on, not incidental gaps, and (4) extending `poly_call_abstract_quotation_param`'s structural-equality comparison to handle an `App`-headed row-slot where `'F` is bound in the *caller's* θ (dispatchable) versus still fully abstract (the member's own recursive declaration) — this second part (dispatch-time grounding of the App itself, as opposed to just admitting the shape structurally) has no probed precedent at all and is the real open question for the brief.

**Post-recon correction (follow-up verification pass, not part of the original probe round):** this conclusion's part (4) does not hold. `poly_call_abstract_quotation_param` never sees an unresolved App at all — `ground_member_poly`'s existing `App` arm (`src/ast.rs:2316`) already dissolves a row-nested App into a `Generic` at parse time, and the still-abstract-`'F` case is structurally impossible (a non-`Generic` impl target is already a located error). See `slice7-spec.md`'s "Why" section and REQ-5 for the full citation trail; this log is kept verbatim above as the historical record of the recon round's own reasoning.
