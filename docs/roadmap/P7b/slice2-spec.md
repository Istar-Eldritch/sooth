# P7b.S2 spec — constructor-keyed dispatch and higher-kinded trait declarations

Technical specification for compiler slice P7b.S2. Scope input is the recon
[brief](./slice2-brief.md) (findings F1-F14, machinery map, rulings R1-R10, witnesses
W1-W6, golden list) and the verbatim [probe log](./slice2-probes.md); exit criteria are
from the [phase doc](../P7b-higher-kinded-types.md). Sibling structure: the S1 spec
([slice1-spec.md](./slice1-spec.md)). All anchors below were re-verified against HEAD
`5443a0d` this session (see [Anchor status](#anchor-status)).

Design rulings R1-R10 in the brief are user-approved as written (260831): the Solution
Approach and phases implement them, and every deviation is called out in
[Open questions](#open-questions).

Diagnostic texts here pin **shape**, not wording (located `(line, col)`, single `error:`
prefix, names the offending position and its origin, parenthetical advice). Exact strings
freeze when the goldens are written, per the S12 precedent (brief W-list preamble).

## Exit criteria (from the phase doc)

1. A trait may declare a higher-kinded variable and use type-level application in member
   signatures (`trait: Functor['F: * -> *] : map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;`).
2. `impl: Functor for Option` resolves.
3. A call to `map` on `Option[i64]` with `~[ i64 -- bool ]` dispatches to the Option impl
   and produces `Option[bool]`.
4. The same call on `Result[i64 Err]` dispatches to a separate impl and produces
   `Result[bool Err]`.

## Background the requirements depend on

Established by the probes; cited so each requirement is traceable. `F#` = brief finding;
`p#`/`m#` = probe/mutation.

- **The applied-var impl target is already complete** (F1/F2; p5b, p10b, p11a-c). `impl:
  Functor for Box['T]` parses as an S4 `Generic` pattern with a `Var` arg;
  `match_impl_target_rec`'s `Generic` arm matches concrete instantiations and binds the
  pattern var; dedup (alpha-equivalence via structural equality), specificity
  (`Box[i64]` beats `Box['T]`), and bounded dispatch all work. The *concrete-operand*
  half of constructor-keyed dispatch exists today. **The whole gap is in trait member
  signatures** — they cannot mention `'F['T]` — and every downstream failure traces back
  to that (F5/F6/F7).

- **The bare ctor target is one desugar away** (F3; p4, m2). `for Box` dies at the shared
  arity gate (`generic_arity_error`, not `parse_impl_target`). Mutation m2 desugared it to
  `for Box['ctor0]`; everything downstream (dispatch, dedup, orphan, member synthesis) ran
  unchanged. No new pattern representation is needed.

- **The trait header's kind annotation is inert** (F4; p6c). `parse_header_bracket` parses
  the kind; `parse_trait_decl` discards it (`let (var, var_span, _kind)`, `parser.rs:3190`).
  No publication, no annotation-vs-usage validation: `trait: F['F: *]` with an `'F['T]`
  member is silently accepted.

- **The member single-variable gate fires first** (F5; p6a, p6b). `map ( 'F['T] [ 'T -- 'U ]
  -- 'F['U] )` parses (S1 grammar landed) and dies at `multi_variable_trait_error`
  (`parser.rs:3349`) before `member_shape_is_supported` (`:368`) could reject its
  App/Quotation shapes.

- **Behind the gates sits a dispatchability rule S2 must replace** (F6; m1). Lifting the
  gates exposes: "trait member `map` … never takes `'T` (or `&'T`) directly as an input, so
  a call has nothing to dispatch on". `map`'s dispatchable input is App-*headed*, not a
  bare var; the rule was written for single-var traits.

- **Member sigs cannot accept App-shaped caller slots** (F7; p7). With `'F['T]` in the
  caller's sig, `unify_poly_input`'s App arm binds `θ('F) = Type::CtorImage(gid)`; a Star
  member slot mismatches ("expects `'F`, found `'F['T]`"). CtorImage dispatch is unreachable
  until member sigs carry applications.

- **`CtorImage` matches no impl-target arm today** (F8; m3). `Concrete` compares, `Generic`
  needs a `Struct`/`Enum`, `App => None`. The S4-legal bare-var target `for 'T` would
  *capture* a CtorImage θ and die later at S1-15.g. m3 (target-App fence deleted) shows a
  fully-abstract `for 'F['T]` registers but never matches: a clean unsatisfied-bound error,
  no panic. The fence is UX, not soundness.

- **The R8i cross-call fence has exact text** (F9; p8, p8b). It fires on poly-caller →
  poly-callee with App slots (`poly_cross_match`, `poly.rs:2517`); mono callers are fine.
  The W4 shared-bound dogfood (`twice`) will hit it.

- **The Monad.bind question is answered: App-in-quotation-rows is not free** (F10; p9a-g).
  Declarations *represent* App inside quotation rows; body-level `call` cannot see through
  it; quotation-valued outputs are fenced at the S10 slice-7 boundary; call-site inference
  does not reach App outputs. `Functor.map` needs none of this — its quotation parameter is
  App-free (`[ 'T -- 'U ]`).

- **The member word's `PolySig` is built from the target's tables only** (F11). The
  generic-path desugar (`parser.rs:3612-3650`) stamps every variable `Span::default()`; no
  member-local variable story exists. This construction is S2's load-bearing structural
  change.

- **Three pre-existing warts constrain the dogfood** (F12/F13/F14):
  - **F12** — a bare generic ctor as a value word in a mono body fails (`unknown word 'Box'
    in 'main'`) while the same ctor inside a declared-sig helper resolves. Constrains
    dogfood ergonomics; blocks no exit criterion. **Recorded, not fixed** (R10 iv).
  - **F13** — the second word to call a field-carrying variant ctor after an `inline` word
    with a `~[` param fails with an identical-rendering mismatch; order-dependent.
    Non-inline `[`-param words avoid it. The pinned body idiom (below) sidesteps it.
  - **F14** — a zero-field variant ctor in a polymorphic arm does not unify with the ambient
    type variable: the poly `mapover`'s None arm leaves mono `Option[i64]` against the Some
    arm's `Option['U]`. Field-carrying ctors unify fine (W3's Result arms dodge it); **W2's
    None arm needs the fix.** S2 must fix ctor-var unification in arm contexts or route W2
    around it (S2-13).

- **The member body idiom is pinned** (probe body-pinning series). Raw-stack plumbing
  through the eliminator, verified end-to-end at concrete types:

  ```sth
  : mapover ( Option['T] [ 'T -- 'U ] -- Option['U] )
    swap
    ~[ ( Some ) Some> swap call Some ]
    ~[ ( None ) drop drop None ]
    Option? ;
  ```

  Idiom facts: eliminator arms inherit the ambient stack below the scrutinee (`swap`
  before the arms); `call` pops the quotation on top and its input beneath; `~[` params
  require `inline` (`[` params work on non-inline words, and non-inline is what dodges
  F13); `&field` is blocked in generic bodies, `Some> | x |` destructure is the arm-level
  alternative; ctors reject explicit instantiation (mono declared-sig helpers are the
  ctor-in-`main` workaround, F12).

## Requirements

Each requirement cites its brief ruling and supporting evidence. `R#` = ruling; `F#` =
finding; `p#`/`m#` = probe/mutation.

### Trait surface (Phase 1)

**S2-1 (R5).** Publish the header kind, lift the member single-variable gate, seed each
member's var 0. `parse_trait_decl` (`parser.rs:3177`) stops discarding the parsed kind at
`:3190`: `TraitDecl` (`ast.rs`) gains a `kind: Kind` field (default `Star` for a plain
`trait: F['F]`) **and** the header var's span, so a member's annotation-vs-usage conflict
carries both spans. `parse_trait_member_effect` (`:3324`) seeds the member builder's var 0
with the header kind *before* the effect parse (modeled on `attach_bracket_bounds`' kind
block, `:2955-3002`) so `mark_ty_star`/`mark_ty_arrow` (`:1563`/`:1589`) check against it;
today `intern_ty_var` (`:1539`) pre-interns var 0 bare and never touches
`ty_established_kind` (`:1452`). The single-var gate at `:3349` is **lifted** — members may
declare their own locals, the header var keeps id 0 in each member's sig. Keep
`multi_variable_trait_error` for the *header bracket* itself (one trait var per trait,
`:3188`/`:3222` — unchanged). Validate per member: the header annotation `'F: * -> *` vs a
bare `'F` mention in a member is a located error (S2-15.b), both spans.

**S2-2 (R4).** Replace the member dispatchability rule (m1's checker gate). HKT-aware form:
a member sig must have an input that is either the trait var directly (Star traits,
unchanged) or an **application headed by the trait var** (HKT traits — the dispatchable
input). Located error otherwise (S2-15.a; draft text modeled on m1's captured message):

```text
error: trait member `map` of `Functor` (line 4, col 8) has no input for a call to dispatch on (expected the trait's variable `'F` bare or heading an application like `'F['T]`)
```

The rule reads `sig.ty_kinds`/the header var id, not the member-local variable table (m1's
gate named `'T`, the member-local, because it read the sig's variable table — that shape
must not survive).

**S2-3 (R6).** `member_shape_is_supported` (`parser.rs:368`) gains real arms:

- **`App`** — supported iff the head is the trait var (id 0); arity is checked later at
  impl-check time against the target (S2-11), not here (the target is unknown at member
  parse).
- **`Quotation`** — supported iff its rows are **App-free**; an `App` inside a member
  quotation row is a located fence (S2-15.d, F10 — declaration representable but `call`
  blind, a later slice's extension).

The gate at `:3351` reads `sig.ty_kinds` when it needs the head's kind. The row gate
(`:3359`) is unchanged.

### Target and member-word construction (Phase 2)

**S2-4 (R1).** Bare and partially-applied ctor targets desugar to applied-fresh-var
(m2-proven mechanics). In the impl-target slot path (`parse_impl_target`, `:3447`), a bare
generic name (no following `[`) desugars to the ctor applied to fresh pattern variables,
one per declared type variable: `for Option` ≡ `for Option['ctor0 …]`, spans at the ctor
name. Extend to the **partial-application** case: `for Result[i64]` = explicit prefix + fresh
vars for the remaining slots (`for Result[i64 'ctor1]`), mechanically identical. The
applied-var spelling (`for Option['T]`) is unchanged. Keep the user's span and name
alongside the desugared pattern so diagnostics render the user's spelling (`Option`, not
`Option['ctor0]`). The App-head fence (`impl_target_app_unsupported_error`, `:3457-3460`)
stays (R10 ii — fully-abstract `for 'F['T]`); the representation hazard
`PolyType::Concrete(CtorImage)` (which would bypass the fence and be mis-classified
value-concrete, `ast.rs:2100`/`:2110`) must stay unreachable — the parser never produces it.

**S2-5 (R6).** Member-word `PolySig` construction — the load-bearing change. The desugar's
generic path (`parser.rs:3612-3650`) unions the target's variables with the member's locals
(R2's identification applied inside App args), builds matching `ty_kinds`/len tables, and
**stops stamping `Span::default()`**: every variable gets the span of its introduction
(target var → target span; member local → member sig span) so diagnostics name variables
precisely. Today the sig is built from the target's tables only (F11).

**S2-6 (R2).** Member-sig grounding — the leading-slot rule (recommendation (b)).
`ground_member_poly` (`ast.rs:1997`, generic targets) gains an `App` arm (retiring the
`unreachable!` at `:2038`); `ground_member_type` (`:1968`, concrete targets) gains its own
`App` arm (retiring the `_ => unreachable!` at `:1985`). For a member's `'F[X₁…Xₙ]` against a
target `Generic(C, [A₁…Aₘ])` with `m ≥ n`: the member's application args identify with the
target's **leading** ctor slots (`Xᵢ ≡ Aᵢ` for var args); leftover target slots
(`Aₙ₊₁…Aₘ`) are the impl's own variables and flow through untouched. `'F['T]` against
`Result['T 'E]` grounds to input `Result['T 'E]`, output `Result['U 'E]` — the phase-doc
Result exit criterion. The arm returns `Generic{C, memberArgs ⧺ targetArgs[n..]}` with
member-local vars renamed into the member word's id space (S2-5). No new `Type`/`Subst`
representation (rejects R2(a)); multi-arg ctors are **not** fenced (rejects R2(c) — W3 rules
it out).

**S2-7 (R2).** Impl-check arity validation. At impl-check, validate `n ≤ m` per impl (the
member's application arity must not exceed the target ctor's arity); located error otherwise
(S2-15.c). This keeps the trait header's `'F: * -> *` honest without constraining the ctor's
total arity: the kind constrains how many arguments *members* apply, the impl-check validates
the fit per impl. Anchor: `check_impl_decls` (`declarations.rs:466`), beside the existing
member-body checks.

### Dispatch, orphan, mangling (Phase 3)

**S2-8 (R3).** `match_impl_target_rec` (`poly.rs:7266`) gains a real `CtorImage` arm:
`ty = CtorImage(g, _)` matches a target pattern `Generic{idx, module}` on **constructor
identity alone** (`idx`/module == `g`), without comparing args. The returned subst carries
no arg bindings — the member word, polymorphic over the target's variables and the member's
locals (S2-5/S2-6), unifies its own slots at the member call from the caller's App-grounded
types. Also add the **`for 'T` catch-all guard** (F8): a bare-var target must **not** match a
CtorImage ty (it can never ground its member anyway — S1-15.g), a dedicated diagnostic
(S2-15.e) rather than letting it win dispatch by accident.

**S2-9 (R3).** Dispatch composition. `resolve_user_bound` (`poly.rs:7073`) picks the member
word from `imp.resolved`, mints `instantiation_symbol(word, subst)` for generic winners, and
records `(word, subst)` in `impl_monos`; `discover_transitive_instantiations`
(`poly.rs:6158`) re-derives and dedups on symbol; `CrossGround::compose` (`poly.rs:6620`)
runs the identical bound loop. For an HKT call the CtorImage-selection subst (S2-8, no arg
bindings) and the member-call unification (which binds the member's own slots from the
caller's App-grounded types) must **compose without a second monomorph of the same
(word, θ)**. Trace and pin exactly where the member call's θ and symbol mint sit across these
four sites so the composition holds.

**S2-10 (R7).** Cross-call fence lift for the member-call shape. `poly_cross_match`
(`poly.rs:2517`, exact fence text p8) currently rejects a poly caller dispatching a member
call whose slots are App-shaped; W4's `twice` needs it lifted. Recommendation (ii): lift
App-slot cross-calls generally (S1's fence predates S1's App arms in `unify_poly_input`,
which now decompose App slots). Fall back to (i) — lift only bound/member calls from poly
bodies — if general lifting reopens the soundness concern S1 fenced (the phase's probes tell
which). Keep p8's exact fence text as a regression baseline (a non-member App cross-call
that *should* still reject) with a dedicated cross-call golden (W4).

**S2-11 (R8).** Orphan/coherence for ctor targets. Extend `impl_target_module`'s Generic arm
(`declarations.rs:426`, currently a `_ => None` for everything but `Concrete(Struct/Enum)`)
to `Some(ctor module)` so a ctor-abstract impl may live in the constructor's module or the
trait's module — the same rule concrete targets get. Reuse `impl_orphan_error` (it already
names trait and target).

**S2-12 (R9).** CtorImage symbols keyed on `GenericId`. `Type::CtorImage(GenericId,
&'static str)` already carries the gid (S1 landed the two-field variant), but the mangler
renders `ty.name()` only (`ast.rs:3020`), so same-named ctors in different modules collide —
the documented S1-12 residual hazard (`ast.rs:3040-3046`). Lifting ctor-target reachability
(this slice) makes that reachable, so extend the mangling fold (`instantiation_symbol`,
`ast.rs:2507`) to key on the ctor's `GenericId` (or a qualified name) so distinct ctors mint
distinct symbols. Unit test: two same-named ctors, two modules, one Functor impl each, both
dispatched in one program (golden #10).

**S2-13 (F14).** Zero-field variant ctor unification in a poly arm. S2 must fix ctor-var
unification in arm contexts (a zero-field ctor like `None` in a polymorphic eliminator arm
must unify with the ambient type variable, not a mono mint) **or** route W2 around it. The
fix is preferred (golden #6 pins it directly). If the fix proves out of slice scope, W2's
body takes a field-carrying shape that dodges F14 and the routing is recorded as an
[open question](#open-questions). Field-carrying ctors already unify fine, so W3/W4's Result
arms are unaffected.

**S2-14 (R10).** Kept fenced this slice, each a deliberate non-change:

- (i) App inside member quotation rows (F10 — declaration representable, `call` blind;
  Monad.bind is a later slice). Enforced by S2-3's Quotation arm (S2-15.d).
- (ii) Fully-abstract App-headed targets (`for 'F['T]`) keep the fence
  (`impl_target_app_unsupported_error`); m3 shows they degrade safely, but the focused
  message is better UX and no exit criterion needs them.
- (iii) Quotation-valued outputs stay an S10 slice-7 boundary (unchanged).
- (iv) The F12 ctor-word wart (`5 Box` in `main`): record, do not fix; goldens use
  declared-sig helpers, as the S1 goldens already do.

**S2-15 (R4/R5/R6 diagnostics family).** Draft shapes; single `error:` prefix, `(line, col)`,
both spans where a conflict has an origin, parenthetical advice. Texts freeze at
implementation, pinned by the goldens.

- a. **member with no dispatchable input** (S2-2): the R4 draft above.
- b. **header kind conflicting with member usage** (S2-1): `trait: F['F: *]` with an `'F['T]`
  member — the header-annotation span and the member-usage span (today silently accepted,
  p6c).
- c. **member application arity exceeds target ctor arity** (S2-7): the `n ≤ m` check.
- d. **App inside a member quotation row** (S2-3): F10's fence, member-grammar form.
- e. **bare-var impl target capturing a CtorImage** (S2-8): the `for 'T` catch-all guard.

## Considered and rejected

- **R2(a) partial-application image** (`'F` binds to a partially-applied constructor, new
  `Subst`/`Type` representation) — rejected; heaviest machinery, and the leading-slot rule
  (S2-6) meets the exit criteria with no new representation.
- **R2(c) fence multi-arg ctors** — rejected; contradicts the phase-doc `Result` exit
  criterion (W3 is the witness that rules it out).
- **A fresh impl-target pattern representation for bare ctors** — rejected; m2 proved the
  applied-fresh-var desugar (S2-4) makes bare ctors expressible as existing S4 applied-var
  patterns, everything downstream unchanged. The only question is sugar and diagnostic
  naming, handled by carrying the user's span/name.
- **Comparing args in the CtorImage dispatch arm** (S2-8) — rejected; identity selection
  with args re-derived at the member call composes with the existing per-call-site
  monomorph (the phase doc's "instantiated per call site with the call's concrete type
  arguments"), avoiding a second monomorph of the same (word, θ).
- **Letting `for 'T` win a CtorImage dispatch** — rejected; it can never ground its member
  (S1-15.g), so a dedicated guard + diagnostic (S2-8/S2-15.e) beats a silent capture that
  fails downstream.

## Phased delivery plan

Each phase is independently verifiable: its goldens pass and its new stage code carries
unit coverage before it is done (CLAUDE.md). Green = `cargo fmt --check && cargo clippy --
-D warnings && cargo test`. Golden file `tests/phase7b_slice2.rs`, style from
`tests/phase7b_slice1.rs` (`single_file` + `build_and_run` asserting exit 0 and exact
stdout; `build_error` asserting `stderr.contains(...)` on distinguishing fragments). Real
lib types (`lib/core/option.sth`, `lib/core/result.sth`) make the dogfood honest.

### Phase 1 — Trait surface

Requirements S2-1, S2-2, S2-3, and S2-15.a/b/d. Publish the header kind on `TraitDecl` and
seed each member's var 0; lift the member single-var gate (keep the header-bracket gate);
replace the member dispatchability rule with the HKT-aware form; `member_shape_is_supported`
App/Quotation arms with the App-free-quotation-row fence; per-member header-vs-usage
validation.

- **Unit tests:** header-kind publication and member seeding (annotation-vs-usage carries
  both spans); `member_shape_is_supported` App arm (head = trait var accepted, non-trait-var
  head rejected) and Quotation arm (App-free row accepted, App-in-row fenced); the
  replacement dispatchability rule (trait-var-headed input accepted, no-dispatchable-input
  rejected).
- **Golden (positive #1):** `hkt_trait_declaration_with_app_and_quotation_member_typechecks`
  — W1 (`trait: Functor['F: * -> *] : map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;` +
  `: main ( -- ) ;`) builds; today p6a's `multi_variable_trait_error`.
- **Goldens (errors #1, #2, #4):**
  `hkt_member_without_dispatchable_input_is_located_error` (S2-15.a),
  `trait_header_kind_conflicting_with_member_usage_is_error` (S2-15.b, p6c today accepts),
  `app_inside_member_quotation_row_is_fenced` (S2-15.d).

### Phase 2 — Target and member-word construction

Requirements S2-4, S2-5, S2-6, S2-7, S2-11, and S2-15.c. The bare/partial ctor desugar in
`parse_impl_target`; the member-word `PolySig` union with real spans; `ground_member_poly`/
`ground_member_type` App arms (leading-slot, Option and Result cases); impl-check `n ≤ m`
arity validation; the orphan Generic arm; the specificity/collector `unreachable!` arms
that App now reaches (`generic_args_of` `poly.rs:7779`, `generic_len_args_of` `:7797`,
`quotation_parts` `:7809`) turn real.

- **Unit tests:** bare-ctor desugar (fresh-var naming, spans, partial prefix
  `for Result[i64]`); member-word `PolySig` union (impl vars + locals, kinds, spans);
  `ground_member_poly` App arm — Option (`'F['T]` → `Option['T]`/`Option['U]`) and Result
  leading-slot (`Result['T 'E]`/`Result['U 'E]`); `impl_target_module` Generic arm;
  arity-exceeds check.
- **Golden (positive #5):** `bare_ctor_impl_target_resolves_and_dispatches` — m2's shape as
  a permanent golden.
- **Golden (positive #7):** `partially_applied_ctor_impl_target_binds_explicit_prefix` —
  `for Result[i64]` dispatches with the pinned prefix.
- **Golden (positive #9):** `ctor_impl_in_ctor_module_satisfies_orphan_rule` (S2-11).
- **Golden (error #3):** `member_app_arity_exceeding_target_ctor_arity_is_error` (S2-15.c).

### Phase 3 — Dispatch, mangling, and the arm-unification fix

Requirements S2-8, S2-9, S2-10, S2-12, S2-13, and S2-15.e. The `match_impl_target_rec`
CtorImage identity arm + `for 'T` catch-all guard; the `resolve_user_bound` /
`discover_transitive_instantiations` / `CrossGround::compose` composition (no second
monomorph); the cross-call fence lift for the member-call shape; the mangling GenericId
keying; the F14 zero-field-ctor arm-unification fix (or the recorded W2 routing).

- **Unit tests:** `match_impl_target_rec` CtorImage arm (matches on identity, arg-agnostic;
  `for 'T` catch-all guard rejects a CtorImage ty); the composition path mints one symbol
  per (word, θ); mangling two-ctors-different-modules mint distinct symbols (S2-12); a
  regression probe that a non-member App cross-call still rejects with p8's fence text
  (S2-10 baseline).
- **Golden (positive #2):**
  `functor_map_over_option_dispatches_and_produces_option_of_bool` — W2, exact stdout
  (an observable proving `Some[0]`, e.g. `Option?` arms printing `some 0` / `none`).
- **Golden (positive #6):** `zero_field_ctor_unifies_with_ambient_var_in_poly_arm` — F14's
  fix (the poly `mapover` None arm; W2's body is its program-level form).
- **Golden (error #5):** `bare_var_impl_target_does_not_capture_ctor_image` (S2-15.e).

### Phase 4 — End-to-end goldens and non-regression

The remaining runtime witnesses and the full suite green.

- **Golden (positive #3):**
  `functor_map_over_result_dispatches_to_the_result_impl_and_passes_err_through` — W3
  (`Ok[1]` + `~[ 1 - ]` → `Ok[0]`; `Err` passes through), exact stdout. Rules out R2(c).
- **Golden (positive #4):**
  `functor_map_through_shared_bound_dispatches_per_constructor_from_poly_body` — W4
  (`twice` calling `map` from a poly body over both `Some` and `Ok`), exercises S2-8 + S2-10.
- **Golden (positive #8):** `concrete_impl_wins_over_ctor_impl_by_specificity` —
  `Option[i64]` impl vs `for Option` at an `Option[i64]` operand (extends p11c).
- **Golden (positive #10):** `same_named_ctors_in_two_modules_dispatch_distinct_impls`
  (S2-12).
- **Non-regression (W6):** `applied_target_functor_dispatch_unchanged` (p2),
  `s3t_explicit_instantiation_spelling_unchanged`, `slice1_goldens_stay_green`
  (`tests/phase7b_slice1.rs` suite stays green).

## Open questions

- **S2-13 / F14 routing.** The plan fixes ctor-var arm unification directly (golden #6). If
  implementation shows the fix exceeds slice scope, W2's body must take a field-carrying
  shape that dodges F14 and this becomes a recorded deviation from R10's "S2 must fix or
  route around" — flag it here rather than silently reshaping the witness.
- **S2-10 cross-call lift breadth.** R7 recommends lifting App-slot cross-calls generally
  (ii) with the fallback of lifting only bound/member calls from poly bodies (i). The choice
  is decided by whether general lifting reopens the S1 soundness concern; the phase's probes
  resolve it, and the fallback is not a deviation.

## Anchor status

Re-verified against HEAD `5443a0d` this session; load-bearing anchors accurate (brief line
refs carried minor 1-3 line drift, corrected here).

| Anchor | Brief | Verified |
| --- | --- | --- |
| `parse_trait_decl` | `parser.rs:3177` | `:3177` ✓ (kind discard `:3190`) |
| `parse_trait_member_effect` | `:3324` | `:3324` ✓ |
| member single-var gate | `:3348` | `:3349` (drift +1) |
| `member_shape_is_supported` | `:368` | `:368` ✓ |
| `multi_variable_trait_error` (header) | `:3188`/`:3222` | both ✓ |
| `intern_ty_var` / `ty_established_kind` | `:1470`/`:1452` | `:1539` / `:1452` (fn drift) |
| `mark_ty_star` / `mark_ty_arrow` | usage marks | `:1563` / `:1589` ✓ |
| `attach_bracket_bounds` kind block | `:2955-3002` | present ✓ |
| `parse_impl_target` / App fence | `:3447` / `:3457-3460` | both ✓ |
| generic-path member `PolySig` build | `:3612-3650` | present ✓ (`Span::default` stamps) |
| `ImplTarget::is_concrete` / `concrete_ty` | `ast.rs:2100`/`:2109` | `:2100` / `:2110` (drift +1) |
| `ground_member_type` / wildcard | `ast.rs:1968`/`:1993` | `:1968` / `_ => unreachable! :1985` |
| `ground_member_poly` / App arm | `ast.rs:1997`/`:2033` | `:1997` / `App => unreachable! :2038` |
| `find_bound_impl` | `poly.rs:6963` | `:6963` ✓ |
| `resolve_user_bound` | `:7104` | `:7073` (drift -31) |
| `discover_transitive_instantiations` | `:6160` | `:6158` (drift -2) |
| `CrossGround::compose` | `:6658` | `:6620` (drift -38) |
| `match_impl_target` / `_rec` | `:7245` | `:7253` / `_rec :7266` |
| `unify_poly_input` App arm | `:8445`/`:8512` | fn `:8148` (App arm tested `:11244`) |
| `poly_cross_match` | `:2673` | `:2517` (drift) |
| `generic_args_of` / `_len_` / `quotation_parts` | — | `:7779`/`:7797`/`:7809` |
| `check_impl_decls` / `impl_target_module` | `declarations.rs:466`/`:426` | both ✓ |
| `instantiation_symbol` / `ctor_image_type` | `ast.rs:2507`/`:3019-3052` | `:2507` / `:3045`; `CtorImage(GenericId, &'static str)` `:2740`, name render `:3020` |

## Phases (JSON)

```json
[
  { "phase": 1, "focus": "Trait surface: publish the header kind on TraitDecl and seed each member's var 0; lift the member single-variable gate (keep the header-bracket gate); replace the member dispatchability rule with the HKT-aware trait-var-headed-input form; member_shape_is_supported App/Quotation arms with the App-in-row fence; per-member header-vs-usage validation; W1 golden plus error goldens #1/#2/#4", "effort": "M", "difficulty": "M" },
  { "phase": 2, "focus": "Target and member-word construction: bare/partial ctor desugar to applied-fresh-var in parse_impl_target (user span/name retained); member-word PolySig union with real spans; ground_member_poly/ground_member_type App arms (leading-slot, Option+Result); impl-check n<=m arity validation; orphan Generic arm; the collector unreachable arms App now reaches; goldens #5/#7/#9 and error #3", "effort": "M", "difficulty": "H" },
  { "phase": 3, "focus": "Dispatch, mangling, arm-unification: match_impl_target_rec CtorImage identity arm + for 'T catch-all guard; resolve_user_bound/discover_transitive_instantiations/CrossGround::compose composition with no second monomorph; cross-call fence lift for the member-call shape; instantiation_symbol keyed on GenericId; F14 zero-field-ctor arm unification fix; W2 golden, #6, error #5, symbol-distinctness unit test", "effort": "L", "difficulty": "H" },
  { "phase": 4, "focus": "End-to-end goldens and non-regression: W3 (Result, Err pass-through) and W4 (shared bound from a poly body) run goldens; specificity #8; same-named-ctors #10; the W6 non-regression trio incl. tests/phase7b_slice1.rs staying green; full suite and unit coverage green", "effort": "M", "difficulty": "M" }
]
```
