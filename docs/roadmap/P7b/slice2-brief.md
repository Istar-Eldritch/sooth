# P7b.S2 brief — constructor-keyed dispatch and higher-kinded trait declarations (recon round 260831)

Scope input for the S2 spec. Produced by a recon round against the clean tree (worktree
`p7b-s2`, HEAD `5443a0d`, P7b.S1 landed): a read-only extension-point map, ~25 live
compile/run probes plus three reverted mutation experiments under `/tmp/p7bs2-probes/`
(verbatim log: [slice2-probes.md](./slice2-probes.md)). Repo untouched throughout; probe
fixtures are disposable. Diagnostic texts below are drafts — they freeze at
implementation, pinned by the goldens (S12 precedent).

S2's exit criteria (from the [phase doc](../P7b-higher-kinded-types.md)): a trait may
declare a higher-kinded variable and use type-level application in member signatures;
`impl: Functor for Option` resolves; a call to `map` on `Option[i64]` with
`~[ i64 -- bool ]` dispatches to the Option impl and produces `Option[bool]`; the same
call on `Result[i64 Err]` dispatches to a separate impl and produces `Result[bool Err]`.

## What the round established

The headline: **the phase doc's S2 is much narrower than it reads.** The applied-var
impl target already parses, matches, dispatches, dedups, and specificity-orders on this
tree (S4's pattern machinery, proven by p5b/p11b/p11c and mutation m2). What is actually
missing is concentrated in **trait member signatures** — they cannot mention `'F['T]`,
and every downstream failure traces back to that.

| # | Finding | Probes |
| --- | --- | --- |
| F1 | **Applied-var ctor targets work today.** `impl: Functor for Box['T]` parses as an S4 `Generic` pattern with a `Var` arg; `match_impl_target_rec`'s Generic arm matches it against concrete instantiations and binds the pattern var; dispatch through a `Functor` bound works (p5b, p10b). | p5b, p10b, p11c |
| F2 | **Dedup and specificity are already ctor-ready.** `Box['T]` vs `Box['U]` are alpha-equivalent duplicates (structural equality); `Box[i64]` beats `Box['T]` by the S4 specificity order. | p11a, p11b, p11c |
| F3 | **Bare ctor target is one desugar away.** `for Box` dies at the shared arity gate. Mutation m2 desugared it to `for Box['ctor0]` — everything downstream (dispatch, dedup, orphan, member synthesis) worked unchanged and ran. No new pattern representation is needed. | p4, m2 |
| F4 | **The trait header's kind annotation is inert.** `parse_header_bracket` parses the kind, `parse_trait_decl` discards it (`let (var, var_span, _kind)`); no publication, no annotation-vs-usage validation (`trait: F['F: *]` with an `'F['T]` member is silently accepted). | p6c, A-map O2 |
| F5 | **The member single-variable gate fires first.** `map ( 'F['T] [ 'T -- 'U ] -- 'F['U] )` parses (S1 grammar) and dies at `multi_variable_trait_error` before the member-shape gate could reject its App/Quotation shapes. | p6a, p6b |
| F6 | **Behind the gates sits a dispatchability rule S2 must replace.** m1 (gates lifted) exposed: "trait member `map` ... never takes `'T` (or `&'T`) directly as an input, so a call has nothing to dispatch on". The rule demands the trait var bare in an input; `map`'s dispatchable input is App-*headed*. | m1 |
| F7 | **Member sigs cannot accept App-shaped caller slots.** With `'F['T]` in the *caller's* sig (θ('F) = `CtorImage`), a Star member slot mismatches: "expects `'F`, found `'F['T]`". CtorImage dispatch is unreachable until member sigs carry applications. | p7, A-map O3 |
| F8 | **`CtorImage` matches no impl-target arm** (`Concrete` compares, `Generic` needs a `Struct`/`Enum`, `App => None`); the S4-legal bare-var target `for 'T` would *capture* a CtorImage θ and die later at S1-15.g. Mutation m3: deleting the target-App fence makes `for 'F['T]` register but never match — clean unsatisfied-bound error, no panic, no silent miss. The fence is UX, not soundness. | m3, A-map O3 |
| F9 | **The R8i cross-call fence has exact text** and fires on poly-caller → poly-callee with App slots; mono callers are fine. The W4 shared-bound dogfood will hit it. | p8, p8b |
| F10 | **The Monad.bind question is answered: not free.** Declarations *represent* App inside quotation rows; body-level `call` cannot see through it; quotation-valued outputs are fenced at the S10 slice-7 boundary; call-site inference does not reach App outputs. `Functor.map` needs none of this (its quotation parameter is App-free). | p9a-p9g |
| F11 | **The member word's `PolySig` is built from the target's tables only** — no member-local variable story exists (spans all `Span::default()`). This construction is S2's load-bearing structural change. | A-map O4 |
| F12 | **Pre-existing wart:** a bare generic ctor as a value word in a mono body fails (`unknown word 'Box' in 'main'`) while the same ctor inside a declared-sig helper resolves. Constrains dogfood ergonomics; blocks no exit criterion. | p5j, p5k |
| F13 | **Pre-existing:** the second word to call a field-carrying variant ctor after an `inline` word with a `~[` param fails with an identical-rendering type mismatch; order-dependent. Non-inline `[`-param words avoid it. | body-pinning series |
| F14 | **Pre-existing, blocks W2's body as written:** a zero-field variant ctor in a polymorphic arm does not unify with the ambient type variable — the None arm leaves mono `Option[i64]` against the Some arm's `Option['U]`. Field-carrying ctors unify fine, so W3's Result arms dodge it. S2 (or a ride-along) must fix ctor-var unification in arm contexts, or W2 needs a different shape. | body-pinning series |

Positive controls (p0-p3, re-proving S1's W2 and k6b shapes) all pass.

## Machinery map (verified anchors, this tree)

Structural facts every S2 change must respect (from the extension-point worker; all
anchors re-derived on HEAD `5443a0d`):

- **Trait surface.** `parse_trait_decl` (`src/parser.rs:3177`) requires the header
  bracket; `parse_header_bracket` (`:6333`) returns kinds that are discarded at
  `:3190`. The member effect parse (`parse_trait_member_effect`, `:3324`) pre-interns
  the header var bare at id 0 via `intern_ty_var` (`:3331`) — which never touches
  `PolyBuilder.ty_established_kind` (`:1452`) — so var 0's kind comes from member usage
  only (`mark_ty_star`/`mark_ty_arrow`), published by `finish` into `PolySig::ty_kinds`.
  Gate order inside the member parse: effect parse → `validate_pending_quotation_rows`
  → `builder.finish` (`:3347`) → **single-var gate (`:3348`) → shape gate
  (`member_shape_is_supported`, `:3351`/fn `:368`) → row gate (`:3359`)**. Both
  post-finish gates can read `sig.ty_kinds` when lifted. The member's own bound bracket
  already kind-annotates the header var (`attach_bracket_bounds` kind block,
  `:2955-3002`) — the model for header-vs-usage validation.
- **Impl target.** `parse_impl_target` (`:3447`) parses an S4 `PolyType` pattern
  (applied-var shapes admitted), fences App heads via `impl_target_app_unsupported_error`
  (`:3457-3460`), and stamps `ty_kinds: vec![Kind::Star; ..]`. `ImplTarget::is_concrete`
  (`src/ast.rs:2100`) / `concrete_ty` (`:2109`) classify targets for the desugar's
  mono-vs-poly member path. Hazard: `PolyType::Concrete(CtorImage)` would bypass the
  fence and be mis-classified value-concrete — the parser never produces it, but S2's
  target representation must keep it unreachable.
- **Member grounding.** `ground_member_type` (`src/ast.rs:1968`, concrete targets)
  supports Concrete/Var/Array/Ref and ends in a `_ => unreachable!` wildcard
  (`:1993`) — reachable from the unsatisfied-bound error path once member sigs carry
  App. `ground_member_poly` (`:1997`, generic targets) substitutes `Var(_) →
  target.clone()` (the only variable rule) and has `App => unreachable!` (`:2033`).
  The desugar's generic path (`src/parser.rs:3612-3650`) builds the member word's
  `PolySig` from the target's tables only (F11).
- **Dispatch.** `find_bound_impl` (`src/check/poly.rs:6963`) one-way-matches the bound
  ty against each impl target via `match_impl_target` (`:7245`) and resolves bounds;
  `resolve_user_bound` (`:7104`) picks the member word from `imp.resolved`, mints
  `instantiation_symbol(word, subst)` for generic winners, and records
  `(word, subst)` in `impl_monos`; `discover_transitive_instantiations` (`:6160`)
  re-derives and dedups on symbol; the composed cross-call path
  (`CrossGround::compose`, `:6658`) runs the identical bound loop. For an HKT call,
  θ('F) is `Type::CtorImage(gid)` (bound by `unify_poly_input`'s App arm, `:8445`,
  head binding `:8512`) — which matches **no** target arm today (F8).
- **Dedup/orphan.** `check_impl_decls` (`src/check/declarations.rs:466`) dedups on
  `(TraitId, pattern, bounds)` — alpha-equivalence free. `impl_target_module`
  (`:426`) matches only `Concrete(Struct/Enum)`; everything else hits a `_ => None`
  wildcard → trait-module-only orphan rule for every generic target today.
- **Mangling.** `instantiation_symbol` (`src/ast.rs:2507`) folds sorted θ;
  `ctor_image_type` renders the ctor's bare declared name (`:3019-3052`) — distinct
  ctors → distinct symbols, with the documented residual hazard that same-named ctors
  in different modules collide ("Lifting that reachability (S2) must restore
  qualification (or key the symbol on `GenericId`) or S1-12 breaks", `:3035-3046`).

## Design rulings the spec must make

**R1 — Target spelling: bare ctor = desugar to applied-fresh-var (user-approved
mechanics from m2).** `impl: Functor for Option` desugars to
`for Option['ctor0 …]` (fresh pattern variables, one per declared type variable, spans
at the ctor name). The applied-var spelling (`for Option['T]`) stays exactly as it is.
Extend the same desugar to *partially applied* ctor targets (`for Result[i64]` = prefix
of explicit args + fresh vars for the rest): mechanically identical, and it is the
natural spelling when an impl fixes leading parameters. Diagnostics that name the
target should render the user's spelling (`Box`, not `Box['ctor0]`) — keep the user's
span and name alongside the desugared pattern.

**R2 — Member-sig grounding: the leading-slot rule (options; recommendation (b)).**
How `'F[X₁…Xₙ]` in a member sig grounds against a target `Generic(C, [A₁…Aₘ])`, m ≥ n:

- (a) *Partial-application image*: `'F` binds to a partially-applied constructor
  (`Result[· E]`). New representation in `Subst`/`Type`; heaviest machinery.
- (b) *Leading-slot unification (recommended)*: the member's application args identify
  with the target's **leading** ctor slots (`Xᵢ ≡ Aᵢ` for var args); leftover target
  slots (`Aₙ₊₁…Aₘ`) are the impl's own variables and flow through untouched. `'F['T]`
  against `Result['T 'E]` grounds to input `Result['T 'E]`, output `Result['U 'E]` —
  exactly the phase-doc exit criterion. Requires `n ≤ m` (checked, located error
  otherwise) and no new Type representation: `ground_member_poly`'s App arm returns
  `Generic{C, memberArgs ⧺ targetArgs[n..]}` with member-local vars renamed into the
  member word's id space.
- (c) *Fence multi-arg ctors* — rejected: contradicts the phase-doc `Result` exit
  criterion.

Under (b) the trait header's `'F: * -> *` stays honest: the kind constrains how many
arguments *members* apply, not the ctor's total arity; the impl-check validates
`n ≤ m` per impl. Functor's convention is "map transforms the leading slot".

**R3 — Dispatch on a CtorImage θ: identity selection, args re-derived at the member
call.** `match_impl_target_rec` gains a real arm: `ty = CtorImage(g)` matches a target
pattern `Generic{idx, module} == g` on constructor identity alone, without comparing
args. The returned subst carries no arg bindings; the member word — polymorphic over
the target's variables and the member's locals — unifies its own slots at the member
call from the caller's App-grounded types (the phase doc's "instantiated per call site
with the call's concrete type arguments"). Spec work: trace exactly where the member
call's θ and symbol mint sit (`resolve_user_bound` → `impl_monos` →
`discover_transitive_instantiations`, plus `CrossGround::compose`) so the
`CtorImage`-selection subst and the member-call unification compose without a second
monomorph of the same (word, θ). Also decide the `for 'T` catch-all (F8): recommend a
guard so a bare-var target does **not** match a CtorImage ty (it can never ground its
member anyway — S1-15.g), with a dedicated diagnostic, rather than letting it win
dispatch by accident.

**R4 — Replace the member dispatchability rule (m1's gate).** HKT-aware form: a member
sig must have an input that is either the trait var directly (Star traits, unchanged)
or an application headed by the trait var (HKT traits — the dispatchable input).
Draft replacement text (modeled on m1's captured message):

```text
error: trait member `map` of `Functor` (line 4, col 8) has no input for a call to dispatch on (expected the trait's variable `'F` bare or heading an application like `'F['T]`)
```

**R5 — Lift the member single-variable gate; make the header kind travel.** Members
may declare their own locals; the header var keeps id 0 in each member's sig.
Publish the header kind (and span) on `TraitDecl`, seed each member's builder var 0
with it *before* the effect parse (so annotation-vs-usage conflicts carry both spans,
modeled on `attach_bracket_bounds`' kind block), and validate per member: the header
annotation `'F: * -> *` vs a bare `'F` mention in a member is a located error (S1's
R7 family, trait-context member). Keep `multi_variable_trait_error` for the *header*
bracket itself (one trait var per trait — unchanged).

**R6 — Member word `PolySig` construction: union id-space with real spans (the
load-bearing change).** The desugar's generic path must union the target's variables
with the member's locals (R2's identification applied inside App args), build matching
`ty_kinds`/len tables, and stop stamping `Span::default()` — every variable gets the
span of its introduction (target var → target span; member local → member sig span) so
diagnostics can name variables precisely. `member_shape_is_supported` gains real arms:
`App` (head = the trait var; arity ≤ the eventual target's — checked at impl-check
time) and `Quotation` (rows **App-free**; an App inside a member quotation row is a
located fence, F10). `ground_member_type`/`ground_member_poly` gain App arms per R2,
retiring both `unreachable!`/wildcard paths.

**R7 — Cross-call scope (R8i): lift for the member-call shape.** The W4 shared-bound
dogfood (`twice` calling `map` from a poly body) requires a poly caller to dispatch a
member call whose slots are App-shaped. Options: (i) lift only bound/member calls from
poly bodies; (ii) lift App-slot cross-calls generally (S1's fence predates S1's App
arms in `unify_poly_input`, which now decompose App slots). Recommendation: (ii), with
p8's exact fence text as the regression baseline and a dedicated cross-call golden;
fall back to (i) if general lifting turns out to reopen the soundness concern S1
fenced (the spec's probes will tell).

**Amended 260831 after the spec review round:** the fence is **not lifted**. Member calls
never reach `poly_cross_match` (`poly_trait_member_call` fronts them), so the lift was
aimed at the wrong site; the member-call path (spec S2-16) owns App slots, p8's exact
fence text stays as the regression baseline, and the fence was a scope fence (S1-17.i),
not a soundness fence.

**R8 — Orphan/coherence for ctor targets.** Extend `impl_target_module`'s Generic arm
to `Some(ctor module)` so a ctor-abstract impl may live in the constructor's module or
the trait's module — the same rule concrete targets already get. Draft diagnostic:
reuse `impl_orphan_error` (it already names trait and target).

**R9 — CtorImage symbols: key on `GenericId`.** Honor the in-tree S1-12 contract:
extend the mangling fold to carry the ctor's `GenericId` (or a qualified name) so
same-named ctors in different modules mint distinct symbols. Unit test: two
same-named ctors, two modules, one Functor impl each, both dispatched in one program.

**R10 — Keep fenced this slice.** (i) App inside member quotation rows (F10 —
declaration representable, `call` blind;Monad.bind is a later slice's extension).
(ii) Fully abstract App-headed targets (`for 'F['T]`) keep the fence (m3 shows they
degrade safely, but the fence's message is better UX and no exit criterion needs
them). (iii) Quotation-valued outputs stay an S10 slice-7 boundary. (iv) The F12
ctor-word wart (`5 Box` in `main`): record, do not fix in S2 — goldens use
declared-sig helpers, as the S1 goldens already do.

**R11 — Member-call checking mechanism (added 260831 after the spec review round;
user-approved option (a), the full member-call path).** No existing path can check an
HKT member call: the poly operand check (`poly_trait_member_call` +
`substitute_member_var`) conflates member locals with the caller's bound var, mono
bodies have no member-call path at all (a bare member is an unknown word), and splice
grounding can leak a `CtorImage` into a value-type position. Approved: unify the
member's dispatchable input against caller slots at `poly_trait_member_call` **and** add
a mono member-call path so a bare member with fully concrete operands dispatches
directly — the phase doc's exit criteria 3-4 read as direct calls. Spec requirement
S2-16.

## Witness programs

Real lib types (`lib/core/option.sth`, `lib/core/result.sth`) make the dogfood honest.
The member **body idiom is pinned** (see the body-pinning section of the probe log):
raw-stack plumbing through the eliminator —

```sth
: mapover ( Option['T] [ 'T -- 'U ] -- Option['U] )
  swap
  ~[ ( Some ) Some> swap call Some ]
  ~[ ( None ) drop drop None ]
  Option? ;
```

— verified end-to-end at concrete types; the poly version passes its Some arm and is
blocked only by F14 (zero-field ctor unification), which S2 must fix or route around.
Diagnostics texts below are drafts — they freeze at implementation, pinned by the
goldens (S12 precedent).

**W1 — the HKT trait declaration type-checks** (exit: declaration criterion):

```sth
import: intrinsics * ;
import: core * ;

trait: Functor['F: * -> *]
  : map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
;
: main ( -- ) ;
```

Today: p6a's `multi_variable_trait_error`. Post-S2: builds.

**W2 — map over `Option[i64]` with `~[ i64 -- bool ]` produces `Option[bool]`** (exit:
dispatch + output-type criterion; `impl: Functor for Option` bare-ctor spelling):

```sth
impl: Functor for Option
  : map \ ( Option['T] [ 'T -- 'U ] -- Option['U] ) — eliminate, call, reconstruct
  ;
;
: main ( -- ) 1 Some ~[ 1 - ] map ... \ Some[1] → Some[0]; print via an observable
```

Contract: runs, prints an observable proving `Some[0]` (e.g. `Option?` arms printing
`some 0` / `none`). Today: p6a gate.

**W3 — the same shape on `Result[i64 Err]` with a second impl** (exit: separate-impl
criterion; the R2 leading-slot witness):

```sth
impl: Functor for Result
  : map \ ( Result['T 'E] [ 'T -- 'U ] -- Result['U 'E] ) ;
;
```

`Ok[1]` + `~[ 1 - ]` → `Ok[0]`; `Err` passes through untouched. Contract: runs,
prints `Ok`-path observable and `Err`-path observable. Today: p6a gate. This witness
is what rules out R2(c).

**W4 — dispatch through a shared Functor bound from a poly body** (exit: the
"shared bound" dogfood seed; exercises R3 + R7):

```sth
: twice['F: Functor 'T] ( 'F['T] [ 'T -- 'T ] -- 'F['T] ) map map ;
: main ( -- ) 1 Some ~[ 1 - ] twice .  1 Ok ~[ 1 - ] twice . ;
```

Contract: both dispatch through the one bound word; prints both observables. Today:
p6a upstream; p8's fence at the body call.

**W5 — S2's error family** (one fixture per new diagnostic; see the golden list).

**W6 — non-regression trio**: the p2 applied-target shape; p11c specificity; an
S3t-style explicit instantiation; `tests/phase7b_slice1.rs` stays green.

## Golden list — `tests/phase7b_slice2.rs`

Style to copy: `tests/phase7b_slice1.rs` (`single_file` + `build_and_run` asserting
exit 0 and exact stdout; `build_error` asserting `stderr.contains(...)` on
distinguishing fragments).

Positive:

1. `hkt_trait_declaration_with_app_and_quotation_member_typechecks` — W1.
2. `functor_map_over_option_dispatches_and_produces_option_of_bool` — W2, exact stdout.
3. `functor_map_over_result_dispatches_to_the_result_impl_and_passes_err_through` — W3.
4. `functor_map_through_shared_bound_dispatches_per_constructor_from_poly_body` — W4.
5. `bare_ctor_impl_target_resolves_and_dispatches` — m2's shape as a permanent golden.
6. `zero_field_ctor_unifies_with_ambient_var_in_poly_arm` — F14's fix as a golden
   (the poly `mapover` None arm; the W2 body is its program-level form).
7. `partially_applied_ctor_impl_target_binds_explicit_prefix` — `for Result[i64]`
   spelling (R1 extension) dispatches with the pinned prefix.
8. `concrete_impl_wins_over_ctor_impl_by_specificity` — `Option[i64]` impl vs
   `for Option` at an `Option[i64]` operand (extends p11c).
9. `ctor_impl_in_ctor_module_satisfies_orphan_rule` — R8.
10. `same_named_ctors_in_two_modules_dispatch_distinct_impls` — R9.

Kind/shape errors (drafts per R4/R5/R6; texts freeze at implementation):

1. `hkt_member_without_dispatchable_input_is_located_error` — a member with no
   trait-var-headed input (R4's draft).
2. `trait_header_kind_conflicting_with_member_usage_is_error` — `trait: F['F: *]` with
   an `'F['T]` member (R5's new check; today silently accepted — p6c).
3. `member_app_arity_exceeding_target_ctor_arity_is_error` — R2's `n ≤ m` check.
4. `app_inside_member_quotation_row_is_fenced` — F10's fence, member-grammar form.
5. `bare_var_impl_target_does_not_capture_ctor_image` — R3's catch-all guard.

Non-regression: `applied_target_functor_dispatch_unchanged` (p2), `s3t_explicit_
instantiation_spelling_unchanged`, `slice1_goldens_stay_green` (suite-level).

Unit tests beside the stages (CLAUDE.md): `parse_impl_target` bare-ctor desugar
(fresh-var naming, spans, partial prefix); header-kind publication and member seeding
(R5); `member_shape_is_supported` App/Quotation arms; `ground_member_poly` App arm —
Option and Result (leading-slot) cases; member-word `PolySig` union (impl vars +
locals, kinds, spans); `match_impl_target_rec` CtorImage identity arm (match + arg
agnosticism + catch-all guard); `impl_target_module` Generic arm; mangling
two-ctor-different-modules dedup (R9); specificity App-vs-concrete pairing (the three
`unreachable!` arms in `generic_args_of`/`generic_len_args_of`/`quotation_parts` turn
real).

## Weight

M. The heavy lifting is member-signature machinery and one dispatch arm; target-side
work is a desugar (m2-proven) plus an orphan arm. Files touched: `src/parser.rs`
(header-kind threading, gate lifts + R4 replacement, bare-ctor desugar, member-word
`PolySig` union), `src/ast.rs` (`TraitDecl` kind field, `ground_member_poly`/
`ground_member_type` App arms, `ctor_image_type` qualification), `src/check/poly.rs`
(`match_impl_target_rec` CtorImage arm, `resolve_user_bound` composition, cross-call
lift, specificity App arms), `src/check/declarations.rs` (`impl_target_module`,
impl-check arity validation). The split signals do not fire: everything lands inside
existing responsibilities (parsing traits, grounding members, dispatching bounds).

## What S2 leaves for S3 and later

- **S3 (inline + HKT bounds)**: `declares_inline` members with ctor-keyed dispatch —
  S2 must keep the member word's `(word, θ)` symbol story intact so S3's splice can
  reuse it; no inline probes were run (paper-level only).
- **Monad/Applicative**: F10's verdict — declarations represent App in quotation rows,
  `call` cannot see through them, quotation-valued outputs are an S10 boundary. A
  later slice extends the quotation-effect machinery; the fence messages captured in
  p9a/p9e are its baseline.
- **The F12 ctor-word wart** (`5 Box` in `main`): candidate micro-slice; recorded with
  a minimal repro.
