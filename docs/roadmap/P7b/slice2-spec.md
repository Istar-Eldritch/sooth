# P7b.S2 spec — constructor-keyed dispatch and higher-kinded trait declarations

Technical specification for compiler slice P7b.S2. Scope input is the recon
[brief](./slice2-brief.md) (findings F1-F14, machinery map, rulings R1-R10, witnesses
W1-W6, golden list) and the verbatim [probe log](./slice2-probes.md); exit criteria are
from the [phase doc](../P7b-higher-kinded-types.md). Sibling structure: the S1 spec
([slice1-spec.md](./slice1-spec.md)). All anchors below were re-verified against HEAD
`5443a0d` this session (see [Anchor status](#anchor-status)).

Design rulings R1-R10 in the brief are user-approved as written (260831): the Solution
Approach and phases implement them, and every deviation is called out in
[Open questions](#open-questions). Revised 260831 after a three-lane review round of this
spec: **R7 was amended** (the cross-call fence is not lifted — see S2-10) and **R11 was
added** (the member-call checking mechanism — S2-16, full member-call path); both changes
are recorded in the brief's rulings section. Second revision 260831 after the round-2
targeted review: the S2-8 tie rule is compatibility-conditioned and dispatch goes to the
fullest grounded type (P1-2); S2-9's deferred θ_call gained its binding-map data path and
canonical-sort pin (P1-1); S2-15.f is a non-raising variant (P2); the mono trigger order,
lowering record, and several anchor citations were corrected (P2). A third pass (round 3)
settled four wording clarifications: S2-6's identification reading (D1), S2-16's declared-sig
unification source (D2), S2-9's subject attribution (D3), and the per-site scope of the
tie-break and θ_call (D4).

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
  No publication, no annotation-vs-usage validation. Precisely: p6c's accepted fixture has
  a **bare** member (`size ( 'F -- i64 )`); an App-member fixture dies today at
  `multi_variable_trait_error` (p6a/p6b) before any kind could be checked — the
  annotation-vs-usage conflict becomes detectable only once S2-1 lifts that gate.

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

- **The R8i cross-call fence has exact text** (F9; p8, p8b) — and it does **not** block
  member calls. It fires on *named poly-word* cross-calls with App slots
  (`poly_cross_match`, `poly.rs:2517`, fence arms `:2673-2682`); a member call never
  reaches it because `poly_trait_member_call` fronts member dispatch (`poly.rs:1617`).
  The W4 shared-bound dogfood (`twice`) therefore fails earlier, at the member-call
  operand check (`trait_member_operand_error`, p7's path), not at this fence. The fence
  stays untouched this slice (R7 amended; S2-10).

- **The Monad.bind question is answered: App-in-quotation-rows is not free** (F10; p9a-g).
  Declarations *represent* App inside quotation rows; body-level `call` cannot see through
  it; quotation-valued outputs are fenced at the S10 slice-7 boundary; call-site inference
  does not reach App outputs. `Functor.map` needs none of this — its quotation parameter is
  App-free (`[ 'T -- 'U ]`).

- **The member word's `PolySig` is built from the target's tables only** (F11). The
  generic-path desugar (`parser.rs:3612-3650`) stamps every variable `Span::default()`; no
  member-local variable story exists. This construction is S2's load-bearing structural
  change.

- **No existing path can check an HKT member call** (review round P0; every site below was
  read this session). (a) *Poly caller*: `poly_trait_member_call` (`poly.rs:1395`, fronted
  at `:1617`) checks operands by **structural equality** after `substitute_member_var`
  (`:1089`; the check at `:1495-1507`) — single-var machinery whose `Var` rewrite maps
  *every* bare variable to the caller's bound var, conflating member locals `'T`/`'U`, and
  whose `App` arm rewrites through the same map while the `Quotation` arm clones
  member-local ids through (`other => other.clone()`). For `map`'s sig
  `'F['T] [ 'T -- 'U ] -- 'F['U]` no caller slot can ever compare equal →
  `trait_member_operand_error`. (b) *Mono caller*: the mono checker has zero
  bound-dispatch calls — a bare member falls through `env.get` as an unknown word
  (`terms.rs:830-845`); the only mono member path is the splice-gated
  `resolve_splice_member_call` (`poly.rs:1139`), active only inside combinator splices.
  (c) *Splice grounding*: `resolve_splice_member_call` grounds member inputs via
  `ground_member_type` with `ty = θ(var)` (`poly.rs:1204-1210`), and `ground_member_type`'s
  `Var` arm returns its target **unchecked** (`ast.rs:1974`) — a `CtorImage` θ would flow
  into a value-type position, the exact misclassification `apply_subst` already guards at
  `poly.rs:8623-8640` (S1-15.g, `poly_ctor_image_as_type_error`). S2-16 specifies the
  replacement paths.

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

The rule reads `sig.ty_kinds`/the header var id. History, corrected from the probe log's
reading: today's gate (`member_binds_trait_var`, `declarations.rs:385-394`) matches
`PolyType::Var(0)` — the **header** var, interned first (`parser.rs:3331`) — and accepts
it bare or under `Ref` (`&'F`); the `'T` in m1's captured message is a hardcoded literal
in `nested_receiver_member_error` (`declarations.rs:396-402`), not the checked variable.
The shape that must not survive is *bare-or-ref-only* acceptance: no App-headed input is
recognized. S2 keeps `&'F` dispatchable (dropping it would break every S1-style
ref-member trait — `Shw` at `tests/phase7b_slice1.rs:390`) and extends the same courtesy
to `&'F['T]`: ref-ness is an addressing mode, not a type identity, so a reference to a
trait-var-headed application dispatches on the same head.

**S2-3 (R6).** `member_shape_is_supported` (`parser.rs:368`) gains real arms:

- **`App`** — supported iff the head is the trait var (id 0); the application's arity is
  validated against the target ctor at grounding time (S2-7), not here (the target is
  unknown at member parse).
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

The union scheme is pinned because the desugar clones `bounds: target.bounds.clone()`
(`parser.rs:3656`), keyed by **target** variable ids (`parse_impl_bounds`,
`parser.rs:3482`, resolves names against the target's table): target/header variables
keep their ids and order; member locals **append after** them and are never renumbered,
so every `where`-bound survives the merge; `ty_kinds`/name/span tables grow in the same
order (target kinds kept; appended locals are `Star` — the member grammar declares no
kinds). Within a member sig, a name in an identifying position (an App arg of the
dispatchable input) binds that name to the target slot variable for the whole sig; a
member-local name not in an identifying position must not collide with a target variable
name — located error at desugar (not part of the golden-backed S2-15 family), since the
collision would make the sig text ambiguous.

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
it out). The arm validates `n ≤ m` **first**, erroring located (S2-15.c) — at parse time, inside
this arm: the desugar is the only point where both the member's application arity and the
target ctor's arity are in hand (S2-7), and the `targetArgs[n..]` slice in the return
formula above would panic
on `n > m` before any later validation could run. Non-Generic targets are specified too:
a member `App` grounding against a `PolyType::Var` target (S4-legal `impl: Functor for
'T`) is a located error — a fully-abstract target names no constructor to dissolve into
(the dispatch-side twin is S2-8's `for 'T` guard, S2-15.e). The concrete-target `App` arm
(`ground_member_type`) is a located error as well: there is no mono representation for
member locals, so an HKT member is never checked by grounding against a concrete `Type`
— it goes through S2-16's unification paths; the arm exists only to retire the
`unreachable!` with a located message (same family as the Var-target case). Edge cases,
pinned: a member
with two App-headed inputs (an Applicative-shaped `ap`) grounds each App positionally
against the same leading target slots — the slot variables coincide, which is the
intended reading for one trait head; a concrete leading member arg (`'F[i64]`) grounds to
`Concrete(i64)` and mismatches at the call site if the target's slot is not `i64`.
Likewise, pinned leading slots from a partially-applied target (`for Result[i64]`'s
`i64`) stay `Concrete(i64)` in the member's grounded sig — golden #7's pinned-prefix
dispatch depends on it. Reading rule, so the formula above cannot be misapplied
per-App-position: for the **dispatchable input**, its App args identify with the target's
slot *contents* — a variable arg aliases the target slot variable (S2-5's name binding),
a pinned arg binds to it so the pin stays `Concrete`. For every **other** App in the
sig, the member's own args occupy the slot, displacing the target slot variable — which
the W3 example already requires (output `'F['U]` against `Result['T 'E]` grounds to
`Result['U 'E]`, slot 0 displaced). S2-8's tie example presupposes this reading.

**S2-7 (R2).** Member-application arity validation — at grounding, not impl-check. The
`n ≤ m` check (the member's application arity must not exceed the target ctor's arity)
lives **inside the grounding App arms** (S2-6), firing as a located parse-time error
(S2-15.c) before the `targetArgs[n..]` slice: the parse-time desugar (`parser.rs:3613`
concrete, `:3644`/`:3649` generic) is where both arities are in hand, and a
check-time-only validation would be reached only after the arm had already sliced out of
bounds (`check_impl_decls` runs later, `driver.rs:835`). This keeps the trait header's
`'F: * -> *` honest without constraining the ctor's total arity: the kind constrains how
many arguments *members* apply, the grounding validates the fit per impl.

### Dispatch, orphan, mangling, and the arm fix (Phases 2–3)

**S2-8 (R3).** `match_impl_target_rec` (`poly.rs:7266`) gains a real `CtorImage` arm:
`ty = CtorImage(g, _)` matches a target pattern `Generic{idx, module}` on **constructor
identity alone** (`idx`/module == `g`), without comparing args. The returned subst carries
no arg bindings — the member word, polymorphic over the target's variables and the member's
locals (S2-5/S2-6), unifies its own slots at the member call from the caller's App-grounded
types. Also add the **`for 'T` catch-all guard** (F8): a bare-var target must **not** match a
CtorImage ty (it can never ground its member anyway — S1-15.g), a dedicated diagnostic
(S2-15.e) rather than letting it win dispatch by accident. Dispatch rule, pinned as one
formulation: **dispatch on the fullest grounded operand type available**. In the mono
path the caller's operand is fully concrete, so it dispatches through the existing
`Concrete`/`Generic` arms, which compare and bind args — a pinned target whose pin
mismatches the operand (`for Option[i64]` vs an `Option[bool]` operand) is simply **not
a match**, and the CtorImage arm never sees a mono call. The CtorImage arm exists only
where the head is genuinely abstract: the poly resolve path, where θ('F) is a bare
CtorImage (the App unification bound the head; the args live in the binding map, S2-9).
There, identity matches on ctor alone, and the tie rule is **compatibility-conditioned**:
a concrete-pinned target's pins must unify with the caller's grounded operand args
(re-grounded from the binding map) — incompatible pins disqualify the candidate
entirely (it is not a match); among compatible candidates prefer more pins (the same
concrete-beats-generic principle `select_most_specific` (`poly.rs:6801`, fed from
`find_bound_impl`'s candidate list at `:7029`) applies to `Struct`/`Enum` tys); an
identical pin-shape tie is the ambiguity error. Without the compatibility condition,
`for Option[i64]` would win over `for Option` at every CtorImage call site and then
fail unification at its own pinned member input (`Option[i64]` vs the caller's
`Option[bool]`) — an error where the polymorphic impl should serve. This is selection
*among* identity matches, not arg comparison inside a match — R3's identity-only rule
is untouched.

**S2-9 (R3).** Dispatch composition. `resolve_user_bound` (`poly.rs:7073`) picks the member
word from `imp.resolved`, mints `instantiation_symbol(word, subst)` for generic winners, and
records `(word, subst)` in `impl_monos`; `discover_transitive_instantiations`
(`poly.rs:6158`) re-derives and dedups on symbol; `CrossGround::compose` (`poly.rs:6620`)
runs the identical bound loop. For an HKT call the CtorImage-selection subst (S2-8, no arg
bindings) and the member-call unification (which binds the member's own slots from the
caller's App-grounded types) must **compose without a second monomorph of the same
(word, θ)**. The composition is pinned, not traced: a CtorImage-selected winner must
**not** follow the generic-winner path — `resolve_user_bound`'s unconditional mint
(`instantiation_symbol(word_sym, &subst)` + `impl_monos.push`, `poly.rs:7141-7145`)
would record the degenerate `(word, ∅)`, and `discover_transitive_instantiations`' seed
loop (`poly.rs:6201-6213`) would monomorph the member word at the empty subst, where
`apply_subst` on `map`'s output `Option[Var('U)]` fails with `poly_unbound_output_ty_error`
(`poly.rs:8642`, via `impl_mono_seed`, `:6397`). Instead, for a CtorImage winner the only
record is the obligation already pushed at body-check by `poly_trait_member_call`
(`poly.rs:1525-1530` — the push site, not `resolve_user_bound`); at resolve time
`resolve_user_bound` performs no mint/record for a CtorImage winner and defers to the
per-site rule below. The data path is pinned, because θ_call
cannot exist at body-check time: there the caller's slots are still abstract
(`App{0, [Var(1)]}` in the caller's own sig space), and
`impl_monos: Vec<(String, Subst)>` with `Subst.ty: Vec<(u32, Type)>` (`ast.rs:2400-2402`)
admits only concrete types. So S2-16's body-check unification records its result — the
**member-local→caller-slot binding map** — on the obligation itself: `TraitObligation`
(`poly.rs:14-20`, today only `{span, var, trait_id, member}`) or its `WordObligations`
holder gains that map. At the obligation/resolve loop (`resolve_user_bound`), where the
caller's θ is concrete, the map is **re-grounded through the caller's θ** — member header
variable → the caller's App-grounded `CtorImage`; member locals → the caller's now
concrete slot args — producing θ_call = {target pattern vars → ctor arguments via S2-6's
leading-slot grounding; member locals → grounded slots}. That θ_call is what gets
minted: `instantiation_symbol(word, θ_call)` (the phase doc's "instantiated per call
site with the call's concrete type arguments"), recorded `(word, θ_call)` in
`impl_monos`, with the obligation's symbol linked to that `CallInst`. **Canonical order
is load-bearing**: θ_call is sorted at construction per the P7.S3t invariant
(`poly.rs:5946-5957`, "the mangled symbol depends on it") — without the sort, two
construction sites could mint two symbols for the same (word, θ) and reproduce exactly
the duplicate-monomorph this requirement forbids. Scope of the per-site work, pinned:
`find_bound_impl` runs once per (trait, bound variable, ty) — the single resolution
(`imp`/`is_generic`, `poly.rs:7121-7122`) precedes the per-obligation loop (`:7124`) — so
CtorImage identity selection is per (trait, bound variable), as today. The
compatibility-conditioned tie-break (S2-8) and the θ_call construction run **per member
call site**, using that site's re-grounded binding map: different sites on one shared
bound variable may ground different `(word, θ_call)` instances of the same winner (each
dedups on its own canonically-sorted θ_call). If two identity-matched targets both
survive compatibility at one site, that site errors with S2-8's ambiguity diagnostic.
The dedup invariant
holds by construction: θ is *defined* as the member word's call-site-grounded type
arguments, so the same (word, θ) at two sites mints the same symbol and dedups in the
seed loop as today. Star-trait winners keep the existing resolve-time mint unchanged.
`CrossGround::compose` (`poly.rs:6620`) applies the same rule: call-site-recorded
`(word, θ_call)` entries seed; the empty subst for a CtorImage-selected word never does.

**S2-10 (R7, amended 260831).** The cross-call fence is **not lifted**. `poly_cross_match`
(`poly.rs:2517`) and its App fence (`:2673-2682`, p8's exact text) govern *named
poly-word* cross-calls and stay untouched this slice. The original recommendation rested
on a misreading the review corrected: member calls never reach `poly_cross_match` —
`poly_trait_member_call` fronts member dispatch (`poly.rs:1617`) — so W4's `twice` was
never blocked by this fence; it is unblocked by S2-16's poly-caller unification instead.
The fence was a scope fence, not a soundness fence (S1-17.i: "S2 owns constructor-keyed
dispatch", `slice1-spec.md:349-353`); lifting it remains available to a later slice if a
witness ever needs it. p8's exact fence text is the regression baseline — a non-member
App cross-call still rejects — which is now trivially passable, since the site does not
change.

**S2-11 (R8).** Orphan/coherence for ctor targets. Extend `impl_target_module`'s Generic arm
(`declarations.rs:426`, currently a `_ => None` for everything but `Concrete(Struct/Enum)`)
to `Some(ctor module)` so a ctor-abstract impl may live in the constructor's module or the
trait's module — the same rule concrete targets get. Reuse `impl_orphan_error` (it already
names trait and target). Builtin-shaped ctor images (`impl: Functor for usize`) have no
module for `impl_target_module` to return (`_ => None`, `declarations.rs:429-437`), so
such impls are trait-module-only under the orphan rule — coherent, and now stated.

**S2-12 (R9).** CtorImage symbols keyed on `GenericId`. `Type::CtorImage(GenericId,
&'static str)` already carries the gid (S1 landed the two-field variant), but the mangler
renders `ty.name()` only (`ast.rs:3020`), so same-named ctors in different modules collide —
the documented S1-12 residual hazard (`ast.rs:3040-3046`). Lifting ctor-target reachability
(this slice) makes that reachable, so extend the mangling fold (`instantiation_symbol`,
`ast.rs:2507`) to key on the ctor's `GenericId` (or a qualified name) so distinct ctors mint
distinct symbols. Unit test: two same-named ctors, two modules, one Functor impl each, both
dispatched in one program (golden #10).

**S2-13 (F14).** Zero-field variant ctor unification in a poly arm — **the fix is
committed** (golden #6 pins it; Phase 2). The symptom surfaces at the arms-disagree error
(`poly.rs:9674`: "a type variable is rigid across arms"): a zero-field ctor arm (`None`)
mints a mono type (`Option[i64]`) where a field-carrying arm unifies with the ambient
variable (`Option['U]`). The fix direction: in the eliminator-arm checking path, a
zero-field variant ctor's minted type must unify with the arm's ambient type variable
instead of minting a mono instantiation — the same unification field-carrying ctors
already get. Unit test beside the checker code plus golden #6 (both in Phase 2 — the fix
is checker-side and independent of dispatch). If implementation shows the fix exceeds
slice scope, routing W2 around it (field-carrying shape) is the recorded fallback in
[Open questions](#open-questions) — a flagged deviation, never a silent reshape.
Field-carrying ctors already unify fine, so W3/W4's Result arms are unaffected.

**S2-14 (R10).** Kept fenced this slice, each a deliberate non-change. Ledger assigned:
(i) enforced in Phase 1 (S2-3); (ii) enforced in Phase 2 (S2-4); (iii)–(v) are non-changes
covered by the Phase 4 non-regression trio.

- (i) App inside member quotation rows (F10 — declaration representable, `call` blind;
  Monad.bind is a later slice). Enforced by S2-3's Quotation arm (S2-15.d).
- (ii) Fully-abstract App-headed targets (`for 'F['T]`) keep the fence
  (`impl_target_app_unsupported_error`); m3 shows they degrade safely, but the focused
  message is better UX and no exit criterion needs them.
- (iii) Quotation-valued outputs stay an S10 slice-7 boundary (unchanged).
- (iv) The F12 ctor-word wart (`5 Box` in `main`): record, do not fix; goldens use
  declared-sig helpers, as the S1 goldens already do.
- (v) The specificity/collector `unreachable!` arms (`generic_args_of` `poly.rs:7779`,
  `generic_len_args_of` `:7797`, `quotation_parts` `:7809`) stay unreachable: they fire
  only for an App-shaped impl-target *candidate* pattern, and candidates stay App-fenced
  (R10 ii; `poly_type_app_head`, `parser.rs:540`). The brief's "specificity
  App-vs-concrete pairing" unit test is deferred with them — no reachable input exists
  in S2.

**S2-15 (R4/R5/R6 diagnostics family).** Draft shapes; single `error:` prefix, `(line, col)`,
both spans where a conflict has an origin, parenthetical advice. Texts freeze at
implementation, pinned by the goldens.

- a. **member with no dispatchable input** (S2-2): the R4 draft above.
- b. **header kind conflicting with member usage** (S2-1): `trait: F['F: *]` with an `'F['T]`
  member — the header-annotation span and the member-usage span. Today the single-var gate
  rejects the App-member form first (p6a/p6b; p6c's accepted fixture has a bare member),
  so this diagnostic becomes observable only after S2-1 lifts the gate.
- c. **member application arity exceeds target ctor arity** (S2-7): the `n ≤ m` check.
- d. **App inside a member quotation row** (S2-3): F10's fence, member-grammar form.
- e. **bare-var impl target capturing a CtorImage** (S2-8): the `for 'T` catch-all guard.
- f. **member signature grounded against a `CtorImage`** (S2-16): the grounding twin of
  `apply_subst`'s S1-15.g guard (`poly_ctor_image_as_type_error`, `poly.rs:8623-8640`) —
  `ground_member_type`'s `Var` arm (`ast.rs:1974`) must reject a `CtorImage` target with a
  located error instead of returning it unchecked into a value-type position.
  **Signature ripple, pinned:** `ground_member_type` returns `Type`, not `Result`
  (`ast.rs:1968-1993`), and six call sites consume it — including the diagnostic builder
  `unsatisfied_user_bound_error`, which grounds member sigs at a ty that can be a
  `CtorImage` (`poly.rs:7179-7186`). The guard therefore lands as a **non-raising
  variant** (an `Option`/error-returning form) used by diagnostic paths; the raising
  form is used only where a real `Type` is required, so the error builder can never
  itself raise.

**S2-16 (R11, approved 260831).** The member-call-site machinery — the missing half of
dispatch. No existing path can check an HKT member call (see Background), so S2 specifies
both caller shapes:

- **Poly caller — unification at the existing front door.** `poly_trait_member_call`
  (`poly.rs:1395`, fronted at `:1617`) replaces the structural-equality operand check
  (`:1495-1507`) and `substitute_member_var` (`:1089`, whose `Var` rewrite conflates every
  bare variable with the caller's bound var) with **unification**: the source is the
  trait's **declared member sig** — what today's `substitute_member_var` substitutes
  (`poly.rs:1492-1497`) — not the member word's grounded `PolySig`: at body-check no
  impl is selected, and the word's `PolySig` is ctor-headed (S2-6 dissolved `'F`), so
  unifying it against an abstract App operand would wrongly concretize the caller's
  bound variable. The declared sig's App-headed dispatchable input (head = the abstract
  header variable) unifies against the caller's operand slots. The member word's
  `PolySig` is the source for the **mono** path and for θ_call construction
  (declared→word-space translation via S2-5's name binding, which S2-9 pins). For W4's
  poly-poly member call the caller's
  slot is App-headed by the caller's **bound** variable (`twice` before instantiation),
  and plain App-vs-App unification binds the member's header variable to that bound
  variable and member locals to the caller's slot arguments — no `CtorImage` exists yet
  at body-check; the head becomes one only when the caller's own instantiation grounds it
  (S2-9's re-grounding). Exit criterion #4 depends on this reading. Mismatch stays a
  located member-call operand error
  (the existing `trait_member_operand_error` shape, updated to name the slot positions).
  Success records the obligation **plus the member-local→caller-slot binding map**
  (S2-9's data path) and defers the mint to the resolve loop (where the tie-break and
  θ_call construction are per call site — S2-9).
- **Mono caller — new path.** A bare member word in a mono body currently falls through
  `env.get` as an unknown word (`terms.rs:830-845`; the only mono member path is the
  splice-gated `resolve_splice_member_call`, `poly.rs:1139`). S2 adds a mono member-call
  path: the lookup sits in the `env.get`-miss branch **after** `mint_fallback_candidates`
  (`terms.rs:862-876` — check-time monomorph mints take precedence) and **before** the
  `unknown_word_error` fallthrough, which is unchanged on NO-match so existing
  unknown-word goldens hold; the builtin/intrinsic handlers keep their current
  precedence (an imported, mangled `eq` resolves through `env` as today). A member named
  like a builtin (`eq`, `lt` — legal member names per `poly.rs:1605-1610`) resolves
  consistently: poly bodies claim it ahead of the intrinsic gate (the existing fronting,
  `poly.rs:1617`); mono claims it here — only when no imported word claims the name and
  the operand types match an impl. With fully concrete operand types it dispatches via
  `find_bound_impl` **on the operand's full grounded type** (the existing
  `Concrete`/`Generic` arms compare and bind args — no `CtorImage` is constructed for a
  mono call, per S2-8's dispatch rule), checks the remaining slots by unifying the
  member's grounded sig (S2-6) against the caller's concrete types, and records
  `(word, θ_call)` per S2-9. **Lowering learns the call via a span-keyed record**, the
  same pattern as `builtin_overloads` (`check.rs:117-121`, relayed onto the module and
  read by `ir.rs`): a mono caller has no obligation/`CallInst` path, so the resolved impl
  symbol + θ_call are recorded `span → (symbol, θ_call)` at check time for lowering to
  emit. No match, or an
  ambiguous member name (two traits exposing the same member name in scope), is a
  located error naming the candidates. Member words are module-qualified
  (`synth_member_word_name`, `parser.rs:651`), so the lookup keys on the member name plus
  the operand's ctor, not on a bare global name.
- **The splice path keeps working, guarded.** `resolve_splice_member_call` grounds member
  sigs through `ground_member_type` (`poly.rs:1204-1210`); S2-15.f's `CtorImage` guard
  turns the silent misclassification into a located error there. The splice path's
  standalone-check mode (`splice_uid` `None`) is unchanged.

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
- **Lifting the p8 cross-call fence (R7's original recommendation (ii))** — rejected by
  amendment (260831): the review showed member calls never reach `poly_cross_match`
  (`poly_trait_member_call` fronts them, `poly.rs:1617`), so the lift was aimed at the
  wrong site, and a general lift would invalidate p8's regression baseline. The
  member-call path (S2-16) owns App slots; the fence stays (S2-10).

## Phased delivery plan

Each phase is independently verifiable: its goldens pass and its new stage code carries
unit coverage before it is done (CLAUDE.md). Green = `cargo fmt --check && cargo clippy --
-D warnings && cargo test`. Golden file `tests/phase7b_slice2.rs`, style from
`tests/phase7b_slice1.rs` (`single_file` + `build_and_run` asserting exit 0 and exact
stdout; `build_error` asserting `stderr.contains(...)` on distinguishing fragments). The
runtime goldens that print (#2/#3/#4) run through `single_file_hosted`
(`tests/phase7b_slice1.rs:94` — it adds the hosted manifest and the `hosted::show | . |`
import; bare `single_file` writes no manifest, and importing `core` without a `sooth.pkg`
`depends:` entry fails — `anonymous_package_error`, `packages.rs:323-328`, when no
ancestor manifest exists; `missing_depends_error`, `packages.rs:390-394`, when a manifest
lacks the entry; `driver.rs:2123-2129` is the bare-package-name test, not this failure).
`lib/core` ships no `Show` instance for
`Option` (`lib/core/option.sth:1-4` exports only `Option`/`Some`/`None`), so those
goldens pin a **fixture-local printer helper** for their observable rather than core
printing. Real lib types (`lib/core/option.sth`, `lib/core/result.sth`) make the dogfood
honest.

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
  `trait_header_kind_conflicting_with_member_usage_is_error` (S2-15.b; p6c's accepted
  fixture has a bare member — the App-member form dies at today's single-var gate, so the
  conflict becomes observable only after S2-1),
  `app_inside_member_quotation_row_is_fenced` (S2-15.d).

### Phase 2 — Target and member-word construction

Requirements S2-4, S2-5, S2-6, S2-7, S2-11, S2-13, and S2-15.c. The bare/partial ctor
desugar in `parse_impl_target`; the member-word `PolySig` union with real spans and the
pinned id-space scheme; `ground_member_poly`/`ground_member_type` App arms (leading-slot,
Option and Result cases) with the in-arm `n ≤ m` arity check; the orphan Generic arm; the
F14 zero-field-ctor arm-unification fix (moved here from Phase 3 — it is checker-side and
independent of dispatch).

- **Unit tests:** bare-ctor desugar (fresh-var naming, spans, partial prefix
  `for Result[i64]`); member-word `PolySig` union (impl vars + locals, kinds, spans);
  `ground_member_poly` App arm — Option (`'F['T]` → `Option['T]`/`Option['U]`) and Result
  leading-slot (`Result['T 'E]`/`Result['U 'E]`); `ground_member_type` App arm
  (located error — no mono representation for member locals) and the in-arm `n ≤ m`
  located error; `impl_target_module` Generic arm;
  arity-exceeds check; the S2-13 fix (zero-field ctor arm unifies with the ambient
  variable — checker-level, beside the `poly.rs:9674` site).
- **Golden (positive #5):** `bare_ctor_impl_target_resolves_and_dispatches` — m2's shape as
  a permanent golden.
- **Golden (positive #7):** `partially_applied_ctor_impl_target_binds_explicit_prefix` —
  `for Result[i64]` dispatches with the pinned prefix.
- **Golden (positive #9):** `ctor_impl_in_ctor_module_satisfies_orphan_rule` (S2-11).
- **Golden (error #3):** `member_app_arity_exceeding_target_ctor_arity_is_error` (S2-15.c).
- **Golden (positive #6):** `zero_field_ctor_unifies_with_ambient_var_in_poly_arm` —
  F14's fix (the poly `mapover` None arm; W2's body is its program-level form).

### Phase 3 — Dispatch and the member-call path

Requirements S2-8, S2-9, S2-10, S2-12, S2-16, and S2-15.e/f. The `match_impl_target_rec`
CtorImage identity arm (with the compatibility-conditioned tie rule) + `for 'T` catch-all
guard; the pinned call-site mint composition (S2-9: the obligation binding-map data
path, canonical-sort θ_call); the member-call-site machinery
(S2-16: poly-caller unification, the mono member-call path, the guarded splice path); the
cross-call fence **stays** (S2-10, amended R7); the mangling GenericId keying.

- **Unit tests:** `match_impl_target_rec` CtorImage arm (matches on identity, arg-agnostic;
  `for 'T` catch-all guard rejects a CtorImage ty); the tie rule (pins compatible with
  the caller's grounded args required — incompatible pins are not a match; more pins
  preferred among compatible; identical pin-shape tie ambiguous); the composition path
  mints one symbol per
  (word, θ_call) and never seeds the empty subst for a CtorImage winner; the obligation
  binding-map record and its re-grounding through the caller's θ; poly-caller
  member-call unification (App slots unify, member locals bind to caller slots, mismatch
  located); the mono member-call path (concrete operand dispatches, unknown/ambiguous
  member name located); the S2-15.f `CtorImage` grounding guard; mangling
  two-ctors-different-modules mint distinct symbols (S2-12); a regression probe that a
  non-member App cross-call still rejects with p8's fence text (S2-10 baseline).
- **Golden (positive #2):**
  `functor_map_over_option_dispatches_and_produces_option_of_bool` — W2, exact stdout
  (an observable proving `Some[0]`, via a fixture-local printer — `single_file_hosted` +
  `sooth.pkg` manifest, see the harness note).
- **Golden (error #5):** `bare_var_impl_target_does_not_capture_ctor_image` (S2-15.e).

### Phase 4 — End-to-end goldens and non-regression

The remaining runtime witnesses and the full suite green.

- **Golden (positive #3):**
  `functor_map_over_result_dispatches_to_the_result_impl_and_passes_err_through` — W3
  (`Ok[1]` + `~[ 1 - ]` → `Ok[0]`; `Err` passes through), exact stdout. Rules out R2(c).
- **Golden (positive #4):**
  `functor_map_through_shared_bound_dispatches_per_constructor_from_poly_body` — W4
  (`twice` calling `map` from a poly body over both `Some` and `Ok`), exercises S2-8 +
  S2-16 (poly-caller unification; the fence stays, per S2-10).
- **Golden (positive #8):** `concrete_impl_wins_over_ctor_impl_by_specificity` —
  `Option[i64]` impl vs `for Option` at an `Option[i64]` operand (extends p11c).
- **Golden (positive #10):** `same_named_ctors_in_two_modules_dispatch_distinct_impls`
  (S2-12).
- **Non-regression (W6):** `applied_target_functor_dispatch_unchanged` (p2),
  `s3t_explicit_instantiation_spelling_unchanged`, `slice1_goldens_stay_green`
  (`tests/phase7b_slice1.rs` suite stays green).

## Open questions

- **S2-13 / F14 routing.** The plan fixes ctor-var arm unification directly (golden #6,
  Phase 2). If implementation shows the fix exceeds slice scope, W2's body must take a
  field-carrying shape that dodges F14 and this becomes a recorded deviation from the
  brief's "S2 must fix or route around" — flag it here rather than silently reshaping the
  witness.
- ~~**S2-10 cross-call lift breadth.**~~ Resolved by the R7 amendment (260831, after the
  three-lane spec review): the fence is **not lifted** — member calls never reach
  `poly_cross_match`, so the lift was aimed at the wrong site; S2-16 owns App slots and
  p8's baseline stays passable. Recorded in S2-10 and [Considered and
  rejected](#considered-and-rejected).
- **S2-6 `ground_member_type` App arm — deliberate location-of-check deviation.** The
  spec's "retire the `unreachable!` with a located message" is implemented parser-side
  instead: `fence_member_app_against_concrete_target` raises the located S2-6 error at
  the desugar before grounding, so the arm stays `unreachable!` (a non-raising backstop,
  consistent with S2-15.f's non-raising-signature constraint — `ground_member_type`
  returns `Type`, not `Result`). Observable behavior conforms; recorded deliberately,
  not silently (Phase 2 review round).
- **W2's two recorded deviations.** (1) The golden's call carries the explicit
  instantiation `map[i64 Bool]` rather than the bare spelling: `map`'s `'U` is bound
  only by the quotation's rows, and the pre-existing P7.S3t unbound-row-var behavior
  makes the bare spelling unspellable from a mono caller — the dispatch machinery
  (CtorImage identity, per-site composition, θ_call) is fully exercised either way.
  (2) The observable is `true`, proving the mapped inner value is a `Bool` (i.e.
  `Option[Bool]`), rather than the brief sketch's `some 0` — the exit criterion
  governs the exact stdout, and the fixture-local printer proves the type-directed
  dispatch.
- **W3/W4 run over fixture-local twins — the S1-era module-identity wart (Phase 4,
  supervisor-approved 260831).** Goldens #3/#4 use local `Result`/`Opt`/`Res` twins
  rather than the lib `core::result`/`core::option` types: a lib-declared generic
  header cannot take a ctor-keyed `impl:` from a user module yet. The wart, precisely:
  the operand's instantiation is minted at the *naming* module (`resolve_type_or_apply`
  → `instantiate_*` with the parsing module, `parser.rs:6862-6880`; memo key
  `(idx, module, args, lens)`), while the impl target pattern records the header's
  *declaring* module (`parse_impl_target_pattern` → `poly_generic_header`), and both
  dispatch paths compare the two for equality — `match_impl_target_rec`'s `Generic`
  arm (`found_module != *module`) and the CtorImage identity match
  (`unify_poly_input`'s App arm binds θ('F) from `enum_instantiation_of`, which reads
  the same recorded module). Observed verbatim: a mono caller reports "no `impl:` in
  this program dispatches on these operands"; a poly caller reports "cannot
  instantiate `'F` … does not satisfy `Functor`". This is the same convention golden #7
  documents and routed around; committed W2 (golden #2) set the twin precedent. What
  W3/W4 prove — leading-slot displacement and shared-bound App-vs-App unification with
  S2-9's re-grounding — is the machinery under test, not the type's provenance; the
  wart fix (memo key or module-blind matching) is an S1-era convention change deferred
  to a future slice/ruling.
- **W4's sketch deltas (Phase 4).** (1) The brief's `map map` consumes the quotation
  parameter on the first call; plain quotations are `Copy`, so the working form binds
  it to a local and re-reads it (`| q | q map q map`). (2) The sketch's one
  `Functor['F: * -> *]` over both `Some` and `Ok` is kind-inconsistent — a `* -> *`
  Functor's `'F['T]` cannot unify against a two-argument `Result` operand (App-vs-App
  unification requires equal argument counts) — so the shared trait is
  `* -> * -> *` (as the S2-8 tie-rule unit fixtures already are) and both ctor twins
  are two-parameter types, `Opt`'s `None` exercising golden #6's zero-field arm
  unification. (3) Local twins per the bullet above.

## Anchor status

Re-verified against HEAD `5443a0d` this session; load-bearing anchors accurate (brief line
refs carried minor 1-3 line drift, corrected here). Revised 260831: the review-round anchors
below were read personally during the spec-revision pass (two review-cited lines corrected:
the bounds clone is `parser.rs:3656`, not `:3667`; `member_binds_trait_var` is
`declarations.rs:385-394`).

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
| `poly_trait_member_call` / fronting / operand check / obligation | — | `:1395` / `:1617` / `:1495-1507` / `:1525-1530` ✓ (round-2: fn is `:1395`, not `:1396`) |
| `substitute_member_var` | — | `:1089` (App arm `:1104-1110`, `other` catch-all) ✓ |
| `resolve_splice_member_call` / splice grounding | — | `:1139` / `:1204-1210` ✓ |
| mono unknown-word fallthrough | — | `terms.rs:830-845` ✓ |
| `member_binds_trait_var` / m1 message literal | — | `declarations.rs:385-394` / `:396-402` ✓ |
| resolve-time mint/record / seed loop / `impl_mono_seed` / unbound-output raise | — | `poly.rs:7141-7145` / `:6201-6213` / `:6397` / `:8642` ✓ |
| `apply_subst` S1-15.g CtorImage guard | — | `poly.rs:8623-8640` (`poly_ctor_image_as_type_error`) ✓ |
| F14 arms-disagree site | — | `poly.rs:9674` ✓ |
| cross-call fence arms / S1-17.i scope note | — | `poly.rs:2673-2682` ✓; `slice1-spec.md:349-353` ✓ |
| bounds clone / `parse_impl_bounds` | — | `parser.rs:3656` / `:3482` ✓ |
| `select_most_specific` / its caller | — | `poly.rs:6801` / `:7029` ✓ |
| `single_file_hosted` / manifest errors | — | `tests/phase7b_slice1.rs:94` (round-2: not `:84`) ✓; `anonymous_package_error` `packages.rs:323-328` / `missing_depends_error` `:390-394` ✓ (`driver.rs:2123-2129` is the bare-package-name test) |
| `TraitObligation` / `Subst`+`impl_monos` shape | — | `poly.rs:14-20` / `ast.rs:2400-2402` ✓ |
| P7.S3t canonical-sort invariant | — | `poly.rs:5946-5957` ("the mangled symbol depends on it") ✓ |
| `unsatisfied_user_bound_error` sig grounding | — | `poly.rs:7179-7186` ✓ |
| `mint_fallback_candidates` / env-miss branch | — | `terms.rs:874` / `:862-876` ✓ |
| `builtin_overloads` span-keyed lowering record | — | `check.rs:117-121` ✓ |
| `lib/core` `Option` exports | — | `lib/core/option.sth:1-4` (no `Show`) ✓ |

## Phases (JSON)

```json
[
  { "phase": 1, "focus": "Trait surface: publish the header kind on TraitDecl and seed each member's var 0; lift the member single-variable gate (keep the header-bracket gate); replace the member dispatchability rule with the HKT-aware trait-var-headed-input form; member_shape_is_supported App/Quotation arms with the App-in-row fence; per-member header-vs-usage validation; W1 golden plus error goldens #1/#2/#4", "effort": "M", "difficulty": "M" },
  { "phase": 2, "focus": "Target and member-word construction + the F14 arm fix: bare/partial ctor desugar to applied-fresh-var in parse_impl_target (user span/name retained); member-word PolySig union with the pinned id-space scheme and real spans; ground_member_poly/ground_member_type App arms (leading-slot, Option+Result) with the in-arm n<=m arity check; orphan Generic arm; S2-13 zero-field-ctor arm unification fix anchored at the arms-disagree site; goldens #5/#6/#7/#9 and error #3", "effort": "L", "difficulty": "H" },
  { "phase": 3, "focus": "Dispatch and the member-call path: match_impl_target_rec CtorImage identity arm + identity-tie rule + for 'T catch-all guard; S2-9 call-site mint composition (never seeds the empty subst for a CtorImage winner); S2-16 member-call machinery (poly-caller unification, mono member-call path, guarded splice path); cross-call fence stays (S2-10, amended R7); instantiation_symbol keyed on GenericId; W2 golden, error #5, symbol-distinctness and p8-baseline unit tests", "effort": "L", "difficulty": "H" },
  { "phase": 4, "focus": "End-to-end goldens and non-regression: W3 (Result, Err pass-through) and W4 (shared bound from a poly body) run goldens; specificity #8; same-named-ctors #10; the W6 non-regression trio incl. tests/phase7b_slice1.rs staying green; full suite and unit coverage green", "effort": "M", "difficulty": "M" }
]
```
