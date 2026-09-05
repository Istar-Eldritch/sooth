# P7b.S7 brief — quotation effects over type constructors (the `call` extension)

- Date: 2026-09-05. Base: `a9eca84` (P7b.S10 recon docs merged; suite green).
- Recorded in the roadmap as S2's residual (`slice2-spec.md` "Deliberate limitations"
  S2-15.d: "App inside member quotation rows is fenced — declarations represent it,
  `call` cannot see through it; Monad.bind awaits a later slice") and in the roadmap's
  own S7 entry (`P7b-higher-kinded-types.md`).
- Source: recon probe round [slice7-probes](./slice7-probes.md) (verbatim, P1–P5 +
  precedent check).

## Problem

Two container traits can't be declared today because of two independent parser-level
fences, both predating S7:

1. **`Monad.bind`** — `bind ( 'F['T] [ 'T -- 'F['U] ] -- 'F['U] )` puts an App
   (`'F['U]`) *inside* a quotation row. `app_in_member_quotation_row_error`
   (`src/parser.rs:452`) rejects the whole `trait:` declaration at parse time — before
   `check`, before dispatch, before `call` is ever reached (P1, P2).
2. **`Applicative.ap`** — `ap ( 'F[ [ 'A -- 'B ] ] 'F['A] -- 'F['B] )` puts a quotation
   *as the argument* of an App headed by a type variable. `app_arg_quotation_error`
   (`src/parser.rs:2630`, raised at `:5213`/`:7846` from `parse_poly_app_arg`) rejects this unconditionally,
   trait or no trait — it's an S1-era fence unrelated to traits (P3, P4). A **named**
   constructor's application list (`Box[[i64 -- i64]]`) has no such fence (P5): the
   restriction is specific to a type-*variable* head, not to "constructor applied to a
   quotation" in general.

Both are declaration-time parse rejections. Neither trait exists yet as a checkable
`TraitDecl`, so there is no dispatch-time behavior to fix — the work is admitting the
shape, then grounding it.

## Adjudicated mechanism (probe round precedent check; revised after a follow-up

verification pass — see slice7-spec.md's "Why" section for the full citation trail)

- `apply_subst`'s `App` arm (`src/check/poly.rs:10448`) already grounds a plain-slot
  HKT output (`'F['U]`) via `Subst`-based `CtorImage` substitution — this is S2's
  shipped `Functor.map` mechanism. It has never been asked to descend into a
  `PolyType::Quotation`'s rows, because no such row can exist (fence #1).
- **Superseded finding, kept for the record:** the probe round originally read
  `poly_call_abstract_quotation_param` (`src/check/poly.rs:4426`) as the precedent
  needing extension, reasoning it grounds an abstract quotation parameter by
  **structural equality** and citing `src/check/poly.rs:1352` as existing
  `PolyType::App` `Eq`/pattern-match support. **This was wrong on both counts**: line
  1352 sits inside `unify_member_operand`, a one-way *unifier*, not structural
  equality; and a follow-up verification pass found `poly_call_abstract_quotation_param`
  never actually sees an unresolved App at all — `ground_member_poly`'s existing `App`
  arm (`src/ast.rs:2316`) already dissolves a row-nested App into a plain `Generic` at
  parse time, before the member's own body is ever poly-checked, and the "`'F` still
  abstract" case is structurally impossible (a non-`Generic` impl target is already a
  located error, `src/ast.rs:2054`).
- **Conclusion (revised):** no new grounding code is needed anywhere. What the slice
  actually needs is *verification* that two already-shipped mechanisms —
  `ground_member_poly`'s App-in-Quotation-row combination (callee side, untested only
  because the shape could not parse) and `unify_member_operand`/`render_member_decl`
  (caller side, `src/check/poly.rs:1335`/`:1407`, already exercised by
  `Functor.map`/`twice` dispatch but never with an App nested inside a row) — correctly
  ground `bind`'s shape once the parse-time fences lift. See slice7-spec.md's REQ-5.

## Scope

Per the roadmap entry: *"probe-first... one probe round answers both shapes before the
brief... Scope is the quotation-effect grounding paths only — no new trait surface, no
new declaration syntax."* Concretely, in order:

1. Lift fence #1 (`app_in_member_quotation_row_error`) to admit `PolyType::App` inside
   a member's quotation row.
2. Lift fence #2 (`app_arg_quotation_error`) to admit a quotation-typed argument to a
   variable-headed `App`, for the trait-member-declaration site (does **not** need to
   be lifted at every signature site — P5 shows the named-constructor path already
   has no fence, so this is only closing the gap for the abstract-head grammar).
3. Audit every function whose comments assert the shape can't exist, since these are
   invariants other code relies on, not incidental gaps. Probe round named three:
   `ground_member_type`'s Quotation arm, `fence_member_app_against_concrete_target`
   (`src/ast.rs:2116`, "Quotation rows never carry an App"), and
   `poly_type_app_head`'s row-blind-by-design contract (`src/parser.rs:902-913`).
   **Follow-up verification found all three need comment-only corrections, no code
   changes** — each guard's scan already covers the row-nested case defensively; see
   slice7-spec.md's REQ-3 for the resolved audit.
4. **Revised (was: "extend the grounding"; resolved to "verify it").** No new
   grounding logic is needed — `poly_call_abstract_quotation_param` never sees an
   unresolved App (`ground_member_poly` dissolves it into a `Generic` first, before
   the member's own body is checked), and the "still abstract `'F`" case cannot occur.
   The actual work is verifying `ground_member_poly`'s and
   `unify_member_operand`/`render_member_decl`'s existing arms against this specific
   shape; see slice7-spec.md's REQ-5.
5. Wire dispatch so `bind ( 'F['T] [ 'T -- 'F['U] ] -- 'F['U] )` through a shared
   `Monad` bound splices/dispatches per constructor and produces IR equivalent to a
   hand-written inline call.

Applicative.ap ships only if step 1-4's extension generalizes for free to the mirror
shape (App-of-quotation rather than quotation-containing-App) — the roadmap allows
this ("if shape (b) grounds for free in the same extension"); it is not an independent
exit requirement.

**Final ruling (Phase 4, measured): it does not generalize for free.** Lifting the
second fence is sufficient to *parse* `ap`'s declaration, but `impl: Applicative for Option`
(no call site needed) immediately hits a second, independent checker-level fence —
`audit_poly_input_quotation`'s `Generic` arm (`src/check/audits.rs:431`) calls into
`reject_poly_quotation_anywhere`'s own `Quotation` arm (`src/check/audits.rs:484`),
which rejects a quotation nested inside a type application's argument list. This is a different
mechanism from `bind`'s: `bind`'s quotation is a *direct* member parameter (the `App`
only nested inside the effect's row), while `ap`'s quotation is nested *inside* `'F`'s
own application — the same rejection class as the pre-existing
`quotation_smuggled_as_generic_arg_is_rejected` precedent (`Box[['T -- 'T]]`).
Grounding `ap` would need new logic in that audit, outside this slice's scope (no new
grounding code beyond what Phase 2 verified for `bind`). Fence #2's lift was reverted;
`ap` ships no golden and is recorded as future work, gated on whichever slice takes up
`reject_poly_quotation_anywhere`'s constructor-of-quotation case. See
slice7-spec.md's "What shipped" section for the full citation trail.

## Explicitly out of scope

- No new trait declaration syntax — `trait:`/`impl:` grammar is otherwise unchanged.
- No new kinds, no kind polymorphism (already out of scope for all of P7b).
- `Applicative.pure` (return-type-polymorphic construction) — S6 territory, not S7's
  quotation-effect grounding.
- Lifting fence #2 at *every* signature site (plain words, non-member generics) is not
  required; only the trait-member declaration path needs it for `Applicative.ap`.
  If the fix naturally lands in the shared `RawTy::App` parser path (P4's finding —
  the fence is unconditional, not trait-scoped), that is acceptable and not scope
  creep, since it is the same code path either way; it is not a goal to pursue
  separately.

## Dogfood / exit

Matches the roadmap's stated exit: `bind` through a shared `Monad` bound type-checks,
dispatches per constructor (`Option`/`Result`), and splices to the same IR a
hand-written inline call would produce, zero call frame. Goldens for `Option` and
`Result` dispatch and short-circuit correctly on each arm (early-exit error handling —
the errors-as-values idiom). Applicative.ap does
not ground for free (see the Adjudicated mechanism section's final ruling) and ships
no golden; this was a bonus, not a requirement, so the slice exit is unaffected.
