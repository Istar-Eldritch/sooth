# P7b.S2 probe round — verbatim log (run 260831)

Recon round for P7b.S2 scoping, run 260831 against the current tree (worktree `p7b-s2`,
HEAD `5443a0d`, P7b.S1 landed). Three workers: a read-only extension-point mapper
(condensed findings live in [slice2-brief.md](./slice2-brief.md)), a live-probe runner,
and a witness/golden paper designer. Probes are compile/run fixtures under
`/tmp/p7bs2-probes/` with per-probe captures under `/tmp/p7bs2-probes/logs/`; the repo
was untouched throughout (`git status --porcelain` empty at finish). The probe runner
timed out before its final report but after completing every fixture probe and mutations
m1/m2 (diffs + captures saved); m3 was re-run by the orchestrator with the same protocol.
Fixture sources for the load-bearing probes are inline below; all others sit in
`/tmp/p7bs2-probes/`.

Positive controls all pass on this tree, so every rejection below is attributable to the
missing S2 surface, not baseline breakage:

- p0 smoke (poly word + call, prints `42`), p1 S1 W2 control (`'F['T]` pass-through,
  prints `5`), p3 header kind annotation (`: pass['F: * -> * 'T] ( 'F['T] -- 'F['T] ) ;`
  builds).
- p2 applied-target Functor control (S1 k6b re-run): `trait: Functor['F] : size ;`
  - `impl: Functor for Box[i64]` + a `Functor`-bounded word dispatching at `Box[i64]`
  compiles, runs, prints `1`. Note this shape binds `'F` as a plain Star-kind variable
  to the whole concrete type — no `CtorImage` is involved (p7 is the App-head variant).

Note: bare `/tmp` fixtures cannot `import: hosted::show` without a package manifest;
the probes carry a local `sooth.pkg` (`package: p7bs2probes ; layer: hosted ;` with core
and hosted path deps). Probes that avoid `show` need only `import: intrinsics * ;`.

## Summary table

| Probe | File | Outcome |
| --- | --- | --- |
| p0 smoke | p0_smoke.sth | compiles+runs, prints `42` |
| p1 S1 W2 control | p1_s1_w2_control.sth | compiles+runs, prints `5` |
| p2 applied-target Functor control | p2_applied_target_functor.sth | compiles+runs, prints `1` (θ('F) is a concrete `Struct`, Star var — not CtorImage) |
| p3 header kind annotation | p3_kind_annotation_header.sth | builds (S1 grammar landed) |
| p4 bare ctor target (O1a) | p4_bare_ctor_target.sth | arity gate: `generic type 'Box' declares 1 type variable, but none were supplied` (exit 1) |
| p5a member restates sig | p5a_applied_var_target_restat.sth | `impl member 'size' must not restate its signature ... inherited from trait` (exit 1) |
| p5b applied-var target (O1b) | p5b_applied_var_target_inherited.sth | **compiles** — `impl: Functor for Box['T]` parses and registers today (S4 pattern machinery) |
| p5c-p5i dispatch attempts | p5c..p5i | blocked by the ctor-word wart (F10 below), not by target machinery |
| p5j minimal ctor elim | p5j_minimal_ctor_elim.sth | `error: unknown word 'Box' in 'main'` — bare generic ctor as a value word in a mono body fails (pre-existing wart) |
| p5k ctor inside declared-sig helper | p5k_w1_plus_bare_ctor_main.sth | compiles+runs — the wart is position-specific, helpers with declared sigs work |
| p6a HKT trait decl (full map shape) | p6a_hkt_trait_decl_app_quot.sth | member single-variable gate: `trait 'Functor' names more than one type variable` (exit 1) |
| p6b HKT trait decl (App only) | p6b_hkt_trait_decl_app_only.sth | same gate, same message — multi-var fires before the member-shape gate |
| p6c bare arrow-kind header var in trait | p6c_hkt_trait_decl_bare_arrow_var.sth | compiles — arrow kinds parse in the trait header but kinds are **unenforced** in trait context (header annotation is discarded) |
| p7 CtorImage bound dispatch | p7_bound_dispatch_ctorimage.sth | `` `size` of `Functor` in `sized` ... expects `'F`, found `'F['T]` `` — a member sig over a Star `'F` cannot accept an App-shaped caller slot |
| p8 cross-call App fence (R8i) | p8_cross_call_app_fence.sth | exact fence text captured (below); p8b mono caller compiles fine |
| p9a/b bind shape, declared + called | p9a/p9b | body-level: `` `call` expected `'T`, found `'F['T]` `` — declared quotation rows admit App; `call` on one does not |
| p9c/d sig-only bind shape | p9c/p9d | `stack effect mismatch in 'bindq' / body leaves 'F['T] [ 'T -- 'F['U] ], but the declared outputs are 'F['U]` |
| p9e quotation-valued App output | p9e_quotation_app_identity_sig.sth | fenced: `a quotation type [ 'T -- 'F['U] ] cannot appear as the output of 'qpass': a quotation is only legal as a direct parameter of a word this slice, and a runtime quotation value is slice 7` |
| p9f call-site App output inference | p9f_bind_shaped_call_concrete.sth | `` `callq` in `main` ... has output variable `'F` that no input binds `` + explicit-instantiation advice |
| p9g explicit instantiation, bare ctor | p9g | hits the p4 arity gate again |
| p10a impl member restates App sig | p10a | same restatement rejection as p5a |
| p10b member with App-free sig on applied-var target | p10b_impl_member_app_inherited.sth | compiles — confirms p5b beyond doubt |
| p11a duplicate applied target | p11a_duplicate_applied_impl.sth | `duplicate 'impl:' for 'Box[i64]'` (exit 1) |
| p11b alpha-equivalent ctor targets | p11b_alpha_variant_impls.sth | `duplicate 'impl:' for 'Box['U]' ... first declared` — alpha-equivalence is free via structural equality |
| p11c specificity concrete vs generic | p11c_specificity_concrete_vs_generic.sth | compiles+runs, prints `1` — the concrete `Box[i64]` impl wins over `Box['T]` |
| m1 member gates lifted | mutation, parser.rs | new gate fires: member dispatchability (below) |
| m2 bare ctor target desugar | mutation, parser.rs | **compiles+dispatches** — `for Box` ≡ `for Box['ctor0]`; bound dispatch at `Box[i64]` prints `9` |
| m3 target-App fence deleted | mutation, parser.rs (orchestrator) | App-head target registers; dispatch degrades to a clean unsatisfied-bound error — no panic, no silent miss |
| w2p body-pinning series | w2body_pin / cap_word / cap_arm / cap_arm2 / cap_arm3 / ctor_helper / bisect_* / t6_noninline / mapopt_poly | the map member body idiom pinned (F13), plus two new pre-existing blockers (F14) |

## Verbatim captures

### p4 — bare ctor target (O1a)

```sth
import: intrinsics * ;

type: Box['T] v 'T ;
trait: Functor['F] : size ( 'F -- i64 ) ; ;
impl: Functor for Box
  : size drop 1 ;
;
: main ( -- ) ;
```

```text
error: generic type `Box` declares 1 type variable, but none were supplied at line 5, col 19 (apply it as `Box[T]`, one type argument per declared variable)
```

(S1 k6 re-confirmed on this tree. The gate sits in the shared type-expression path
(`generic_arity_error`, `src/parser.rs:4219`), not in `parse_impl_target` — it also
fires for non-impl uses, e.g. p9g.)

### p5b / p10b — applied-var ctor target works today (O1b)

```sth
import: intrinsics * ;

type: Box['T] v 'T ;
trait: Functor['F] : size ( 'F -- i64 ) ; ;
impl: Functor for Box['T]
  : size drop 1 ;
;
: main ( -- ) ;
```

Builds. The target parses as `PolyType::Generic { args: [Var] }` — S4's pattern
machinery already admits the applied-var spelling, `match_impl_target_rec`'s `Generic`
arm matches it against concrete instantiations binding the pattern var, and
`check_impl_decls` accepts it. Combined with p11c (specificity) and p11b (dedup), the
*concrete-operand* half of "constructor-keyed dispatch" already exists on this tree.

### p6a / p6b — the HKT trait declaration's first gate

```sth
import: intrinsics * ;

type: Box['T] v 'T ;
trait: Functor['F: * -> *]
  : map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
;
: main ( -- ) ;
```

```text
error: trait `Functor` names more than one type variable at line 5, col 5 (only single-type-variable traits are supported)
```

Identical for the App-only variant `map ( 'F['T] -- 'F['T] )` (p6b). The member's effect
parse itself succeeds (S1's application grammar handles `'F['T]` beside a quotation
slot); the single-variable gate fires first, before `member_shape_is_supported` could
reject the App/Quotation shapes. p6c (member `size ( 'F -- i64 )` under an arrow-kind
header) compiles — confirming the header's kind annotation is parsed then discarded
(no annotation-vs-usage validation in trait context).

### p7 — App-shaped caller slot vs Star member sig (O3)

```sth
import: intrinsics * ;
import: hosted::show | . | ;

type: Box['T] v 'T ;
trait: Functor['F] : size ( 'F -- i64 ) ; ;
impl: Functor for Box[i64]
  : size drop 1 ;
;
: sized['F: Functor 'T] ( 'F['T] -- i64 ) size ;
: main ( -- ) 5 Box sized . ;
```

```text
error: `size` of `Functor` in `sized` (line 9, col 43) expects `'F`, found `'F['T]`
```

Here `sized`'s signature has `'F['T]` as an input, so `unify_poly_input`'s App arm binds
`'F := Type::CtorImage(Box)` and `'T := i64` at the call — but the trait member `size`
declares a *bare* `'F` slot, and the member-bound-call path compares that Star-shaped
slot against the caller's App-shaped one. A member sig over the trait variable cannot
accept an App-shaped slot until member signatures carry applications (S2's member-sig
work). Contrast with p2, where `'F` is bare in the *caller's* sig too: dispatch works,
θ('F) is the concrete `Struct`, and no CtorImage ever arises.

### p8 — the R8i cross-call fence, exact text (O5)

```sth
import: intrinsics * ;

type: Box['T] v 'T ;
: inner['F 'T] ( 'F['T] -- 'F['T] ) ;
: outer['F 'T] ( 'F['T] -- 'F['T] ) inner ;
: main ( -- ) ;
```

```text
error: `outer` cannot call the polymorphic word `inner` (line 5, col 37)
  a higher-kinded application in a cross-called polymorphic word is not yet supported from a polymorphic body
  call `inner` from a monomorphic word instead
```

p8b (`outer` monomorphic, same call) compiles and runs. This fence (`poly_cross_match`,
`src/check/poly.rs:2673`) is what the W4 shared-bound dogfood (`twice`) will hit once
member sigs carry App; lifting it for the member-call shape is part of S2's scope
decision.

### p9 series — the Monad.bind question (O6/O10)

p9a (declared; body `call`):

```sth
import: intrinsics * ;

type: Box['T] v 'T ;
: use['F 'T 'U] ( 'F['T] [ 'T -- 'F['U] ] -- 'F['U] ) call ;
: main ( -- ) ;
```

```text
error: type mismatch in `use` (line 4)
  `call` expected `'T`, found `'F['T]`
  note: declared ( -- )
```

p9c (declaration only, empty body):

```text
error: stack effect mismatch in `bindq`
  body leaves `'F['T] [ 'T -- 'F['U] ]`, but the declared outputs are `'F['U]`
```

— the *declaration* with an App inside a quotation row parses and is representable
(p9c got all the way to body/effect checking); it is the body-level `call` on such a
row that cannot see through the App (p9a).

p9e (quotation-valued output):

```sth
: qpass['F 'T 'U] ( [ 'T -- 'F['U] ] -- [ 'T -- 'F['U] ] ) ;
```

```text
error: a quotation type `[ 'T -- 'F['U] ]` cannot appear as the output of `qpass`: a quotation is only legal as a direct parameter of a word this slice, and a runtime quotation value is slice 7
```

p9f (call-site inference into an App output):

```sth
: callq['F 'T 'U] ( [ 'T -- 'F['U] ] 'T -- 'F['U] ) swap call ;
: main ( -- ) ~[ 6 Box ] 5 callq Box> . ;
```

```text
error: `callq` in `main` (line 6) has output variable `'F` that no input binds
  note: supply it explicitly: `callq[SomeType SomeType SomeType]`
```

p9g (the advised explicit instantiation, bare ctor spelling) hits the p4 arity gate.

**Verdict for the phase-doc open question:** the quotation-effect machinery does *not*
handle `'F['U]` for free. Declarations represent it; `call` cannot see through it;
quotation-valued outputs are fenced at the S10 slice-7 boundary; call-site inference
does not reach App outputs. `Functor.map` never needs any of this (its quotation
parameter is `[ 'T -- 'U ]`, App-free); Monad.bind needs its own extension in a later
slice. S2 should keep App-inside-quotation-rows fenced and record this verdict.

### p11 — dedup and specificity for ctor targets (O8)

p11a: two `impl: Functor for Box[i64]` →
`error: duplicate`impl:` for `Box[i64]`(line 8, col 1); first declared at line 5, col 1`.

p11b: `Box['T]` + `Box['U]` (alpha variants) →
`error: duplicate`impl:` for `Box['U]`(line 8, col 1); first declared at line 5, col 1`.

p11c: `Box[i64]` + `Box['T]` coexist; the bounded call at `Box[i64]` dispatches to the
concrete impl (prints `1`). All S4 machinery working unchanged.

### p5j / p5k — the ctor-word wart (F10, pre-existing)

```sth
import: intrinsics * ;
import: hosted::show | . | ;

type: Box['T] v 'T ;
: main ( -- ) 5 Box Box> . ;
```

```text
error: unknown word `Box` in `main` (line 5)
```

The same constructor used inside a helper with a declared sig (`: mk ( i64 -- Box[i64] )
Box ;`) resolves and runs (p5k prints `5`; p2/p11c/m2 all rely on this shape). Bare
generic constructors as value words in a `main` body fail resolution — sharp, minimal,
pre-existing. It constrains how naturally the S2 dogfood program can be written (use
declared-sig helpers, as the S1 goldens already do) but does not block any exit
criterion.

## Body-pinning series (post-round, orchestrator) — F13/F14

The witness member bodies were pinned by compiling the standalone-poly-word equivalent
(the shape an impl member word will have). Working map-over-Option body, verified
end-to-end (`mapopt_poly`-shape at concrete types, `t6_noninline`, runs `5 / 4 / 999`):

```sth
: mapover ( Option['T] [ 'T -- 'U ] -- Option['U] )
  swap
  ~[ ( Some ) Some> swap call Some ]
  ~[ ( None ) drop drop None ]
  Option? ;
```

Idiom facts established by iteration (every failed spelling's error captured in
`/tmp/p7bs2-probes/logs/` and the fixture set):

- **Eliminator arms inherit the ambient stack below the scrutinee.** The quotation
  parameter rides the raw stack through the eliminator (`swap` before the arms);
  `Some> swap call Some` in the arm. Named captures (`| opt q |` then referencing
  `q` in an arm) double-push — five distinct checker errors pin the semantics.
- `call` pops the **quotation on top** and its input beneath (`terms.rs:350`:
  known literal → splice; abstract declared param → declared-effect check).
- `~[ ... ]` parameters require `inline` (located error); `[ ... ]` parameters work
  on non-inline words — and **non-inline is what dodges F13 below**.
- `&field` is blocked in generic bodies (already documented on `show.sth:8-10`);
  `Some> | x |` destructure is the arm-level alternative and is poly-OK for
  field-carrying variants.
- Ctors reject explicit instantiation (`` `Some` takes no type arguments ``); mono
  helpers with declared sigs are the ctor-in-`main` workaround (F12).

Two new pre-existing blockers surfaced (F13/F14 in the brief):

- **F13** — the second word in a file to call a field-carrying variant ctor after an
  `inline` word with a `~[` parameter fails with an identical-rendering mismatch
  (`` body leaves `Option[i64]` where the declaration requires `Option[i64]` ``);
  which word fails follows the second-`Some`-user, and reordering moves the error.
  Non-inline `[`-param words avoid it entirely.
- **F14** — a **zero-field variant ctor in a polymorphic arm does not unify with the
  ambient type variable**: the poly `mapover`'s None arm leaves `Option[i64]` (mono
  mint) against the Some arm's `Option['U]` — `` the arms of `Option?` ... disagree: a
  type variable is rigid across arms ``. Field-carrying ctors (`Some`) unify fine; so
  W3's Result arms (both field-carrying) dodge it, W2's None arm needs the fix.

## Mutation experiments

### m1 — member gates lifted (probe runner)

Diff (reverted; capture `logs/m1.diff`): `member_shape_is_supported` returns `true` for
`Quotation` and `App`; the member single-variable gate's `> 1` becomes `> 1000`.

Result — the HKT trait declaration gets past every parser gate and dies at a
**previously-unreachable checker gate**:

```text
error: trait member `map` of `Functor` (line 4, col 8) never takes `'T` (or `&'T`) directly as an input, so a call has nothing to dispatch on
  note: a variable nested inside a composite input (an array element, say) does not count
```

This is the trait-member dispatchability rule: the member must take the trait's variable
*directly* in an input so a call has a slot to dispatch on. For `map`, `'T` appears only
nested inside `'F['T]` and `'F` only as an application head — neither counts. S2 must
replace this rule with an HKT-aware one (an App-headed input whose head is the trait
variable is a dispatchable input). Note the gate names `'T`, the *member-local*
variable — the rule was written for single-var traits and reads the sig's variable
table, so it survives into a multi-var world in this shape.

### m2 — bare ctor target as desugar (probe runner)

Diff (reverted; capture `logs/m2.diff`): in the impl-target slot path, a bare generic
name (no following `[`) with `builder.forbid_bounds` set desugars to the constructor
applied to fresh pattern variables (`'ctor0`, …) — `for Box` ≡ `for Box['ctor0]`.

Result — `impl: Functor for Box` + the p2 bound-dispatch shape **compiles, runs, and
dispatches** (`m2_p4e` prints `9`; the applied-target control still prints `1`). Nothing
downstream breaks: dedup, specificity, orphan (trait-module rule via the `None`
wildcard), member-word synthesis, and dispatch all treat the desugared target exactly
like a hand-written applied-var target. The constructor-abstract target is therefore
*expressible today* as an S4 applied-var pattern with fresh variables; the only question
is sugar and naming (what the fresh variables are called in diagnostics).

Residual under m2: `m2_p4d` (dispatching from `main` via a bare `Box` value word rather
than a declared-sig helper) still dies on the F10 ctor-word wart — the desugar does not
affect word resolution.

### m3 — target-App fence deleted (orchestrator re-run)

Diff (reverted): the `if let Some(head) = poly_type_app_head(&pattern)` block in
`parse_impl_target` (`src/parser.rs:3457-3460`) deleted.

Fixture: `impl: Functor for 'F['T]` (an App-*headed* target — the impl abstracts over
the constructor itself) + the p2 call shape:

```text
error: cannot instantiate `'F` of `sized` with `Box[i64]` in `main` (line 11, col 20)
  `Box[i64]` does not satisfy `Functor`: no `( Box[i64] -- i64 )` found
```

The App-headed target parses, registers, passes orphan/dedup, and then never matches
anything: `match_impl_target_rec`'s `App => None` arm yields zero candidates and the
call site gets the ordinary unsatisfied-bound error, with the member sig rendered
correctly. No panic, no silent miscompile. Conclusions: the fence is a UX gate (a
focused message beats the generic unsatisfied-bound error), not a soundness fence; and
`impl: Functor for 'F['T]`-style *fully abstract* targets degrade safely even before S2
gives them meaning. (Worker A separately found that `PolyType::Concrete(CtorImage)`
bypasses this fence today and would be mis-classified as a value-concrete target — a
representation hazard S2's target representation must close, not an exploit path: the
parser never produces it.)

## Post-round tree state

`git status --porcelain` empty after every mutation's revert (m3 included); all probe
fixtures and captures under `/tmp/p7bs2-probes/`. HEAD `5443a0d` throughout. The
body-pinning series edits no source at all — fixtures only.
