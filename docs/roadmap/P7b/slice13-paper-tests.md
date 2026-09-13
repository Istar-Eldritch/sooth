# P7b.S13 paper tests — validated golden designs (cross-call App lift, SOO-60)

Designed against the clean tree at HEAD `a3779f4` (worktree `soo-60`; suite
3528/0, measured this round: `cargo test` → 3528 passed, 0 failed). The probe
round is complete and verified: [slice13-probes](./slice13-probes.md) is
ground truth for today's bytes, reachability, and the moved-pin inventory;
frozen pre-fix stderr: `probes/s13_baseline.md` (byte-exact). Companion
docs: [slice12-brief](./slice12-brief.md) (the recorded prediction this
slice implements) and [slice12-paper-tests](./slice12-paper-tests.md)
(format/voice). Harness conventions from `tests/phase7b_slice8.rs`:
`single_file_hosted` (:54) / `build_run_keep` (:70; asserts build success and
run exit 0, returns stdout) / `build_ok` (:99) / `build_error_located`
(:119; asserts non-zero exit and no `panic`; the `error:`-in-stderr check is
caller-side — :119-135). No new diagnostic text is expected anywhere in this
slice except where a verdict MOVES (G5, G10.2): those bytes get pinned at
implementation time from the live binary, per house convention.

Fixture inventory (committed, byte-frozen): `probes/s13_b_app_declared_helper.sth`,
`s13_b_bare_var_growth.sth`, `s13_c_app_output.sth`, `s13_c_array_output.sth`,
`s13_c_generic_output.sth`, `s13_c_ref_output.sth`, `s13_d_arg_concrete.sth`,
`s13_d_arg_var_mismatch.sth`, `s13_d_bare_supplied.sth`,
`s13_d_generic_supplied.sth`, `s13_e_post_lift_green.sth`,
`s13_f_bare_and_applied.sth`, `s13_f_self_application.sth`, plus
`probes/s12_e_cross_call_app_fence.sth` (round A re-run). Goldens that need
NO new fixture text: G1, G2, G5 (committed fixtures verbatim) and all of
G10's unit retargets (inline strings, as today). G3/G4/G6/G7/G8 need edited
copies (texts below); their "today" bytes were re-measured this round on the
edited text via `cargo run -q -- build /tmp/<scratch>.sth` (exit codes
recorded per golden; no files added to the repo). The comment header line of
each edited fixture is cosmetic — re-spelling it shifts only the today line
number, not the verdict. Harness note: `single_file_hosted` prepends
`import: intrinsics * ;` + `import: hosted::show | . | ;`
(tests/phase7b_slice8.rs:54-69), so every harness copy of an s13 fixture
drops the fixture's own import line — a duplicate import collides in the
seen-map (collision errors at `declarations.rs:906`/`:912-918`; the slice12
G6 note).

## The mechanism the goldens validate

Four code facts decide every verdict below; all line numbers measured at
`a3779f4`.

**The input arm** (replaces the S1-17.i fence). Today the fence is the
two-sided pattern `(PolyType::App { .. }, _) | (_, PolyType::App { .. })` at
`crosscall.rs:346-353` (comment :344-345, message :351), ahead of the
catch-all `_ => Err(mismatch())` at :354 (the `mismatch` closure at
:198-205 renders through `poly_rendered_type_mismatch_error`,
`unify.rs:580-593`: "type mismatch in `{op}` (line N) / `{op}` expected
`{expected}`, found `{found}` / note: declared ..."). The lift replaces the
fence with a declared-App arm whose sub-dispatch on `supplied` is:

- `App { .. }`: arity guard (declared args len == supplied args len, else
  `mismatch()` — the same fall-through the same-header Generic/Generic
  guard's failure takes to reach :354), then bind the callee's head variable
  through the SAME consistency/conflict block the Var arm uses
  (`crosscall.rs:283-296`: find previous image, `poly_cross_var_conflict_error`
  on disagreement, else push), then recurse on args pairwise
  (`poly_cross_match`). D1/D2/E.
- `Generic { .. }`: the D4 ruling (next paragraph) — bind the head to a
  ctor-valued image, same arity guard, same recursion. A supplied Generic
  carrying `len_args` is the mono route's S1-7 rejection
  (`poly_app_len_domain_unsupported_error`; mono precedent
  `unify.rs:467-477`, re-guarded at grounding `unify.rs:878-899`).
- `Var(_)` (a supplied bare variable): `mismatch()` — G5's verdict.
- everything else (Concrete/Array/Ref/OwnedCell/QuotLit/Quotation/
  GenericVariant): falls to the catch-all `mismatch()` (:354).

The second fence pattern `(_, PolyType::App { .. })` is deleted: a supplied
App against a declared non-App slot becomes a mismatch — except a declared
bare Var, where the Var arm (`crosscall.rs:208`) precedes everything and
keeps GROWTH (round B2; G2). No pin exists on the supplied-App-vs-declared-
non-App face (round G's inventory has none), so the deletion is deliberate
(R-13.2 below).

**The D4 ruling (R-13.1)**: a concrete ctor supplied as an App head binds
the callee's head variable to `Image::Concrete(ctor_image_type(&generics,
gid-of-the-supplied-header))` where `generics` is the borrowed
`ctx.generics()` cell (`Some(cell) => cell.borrow()`; the `None` arm reuses
`poly_generic_not_yet_groundable_error`, the mono precedent).
`Image` (ast.rs:3007-3010) gains no variant: the ctor rides
`Image::Concrete(Type::CtorImage)` — "ground-flowing (it
lives in the same `ty` substitution map as every other binding) but **not a
concrete value type**: it exists only to be resolved as an `App` head
(S1-11)" (ast.rs:3280-3286) — exactly the binding the mono route inserts
(`unify.rs:488-497`, sole ctor `ctor_image_type` at ast.rs:3596). The Image
doc comment ("so no type constructor ever needs representing here",
ast.rs:2999-3005) is stale after the lift and must be rewritten
(comment-only; no ast.rs behavior changes). Rejected alternative and both
options' consequences: Open rulings.

**The output arms** (`poly_cross_output`, `crosscall.rs:365-405`). Today:
`Concrete` (:371-373), the Var arm through the mapping lookup (:375-391:
`Image::Concrete(t)` → `Concrete(t)`, `Image::CallerVar(w)` → `Var(w)`, no
entry → "an output type variable (`...`) that the callee's inputs do not
determine"), and the compound-output wildcard (:398-405, message :403). The
lift adds, before the wildcard: `Generic` (C2/G7), `Array` (C4/G8), and
`App` (G9) — each recurses through the Var-arm lookup per variable;
`len_args` and array lengths pass through concrete (`Len::Var` is fenced
upstream by `poly_cross_signature_supported` — "a length variable in the
callee's signature", `crosscall.rs:443-445`). The `App` arm's head renders
`Image::CallerVar(w)` → `PolyType::Var(w)` (D1/E: `'It['T]` in caller
space) and `Image::Concrete(CtorImage(gid, name))` →
`PolyType::Generic{gid's header, mapped args}` (D4: `Wrap['T]`) — symbolic,
no registry interning at walk time (the recorded prediction; the interning
happens at grounding). A non-CtorImage `Concrete` head is unreachable
(round F: a head slot only ever receives a caller var or a concrete ctor —
the kind checker rejects every other declaration, F1/F2) and guards to
`mismatch()`. Ref gains NO arm (C3: Ref outputs are banned at declaration —
"a reference cannot be stored", both spellings — so the wildcard's Ref face
is unreachable and stays as defense). `GenericVariant` stays in the
wildcard (the R3.4/R3.5 comment above it, :392-397: "declared output R3.5
never spells one, but a body-mint could in principle be cross-called
against").

**Grounding needs no new machinery.** Compose (`instantiate.rs:682`) folds
the mapping into θ_h (`Image::Concrete(t)` → `*t` at :696-698 — a CtorImage
head lands in `subst.ty` exactly as the mono route leaves it; `CallerVar(u)`
→ `caller.subst.ty_of(u)` at :700-702) and grounds the callee's outputs via
`apply_subst` (:704-712), whose mutable registries intern everything the new
arms can render (signature `unify.rs:598-607` — `arrays: &mut
Vec<ArrayDecl>, cells: &mut Vec<OwnedCellDecl>, refs: &mut Vec<RefDecl>`;
`intern_array_type` :657, `intern_ref_type` :693, `intern_owned_cell_type`
:700; the Generic/GenericVariant/App arms mint through `ctx.generics()` via
`MutRegistries`, :747-753 / :808-816 / :903-909; `Ctx::generics()` at
`engine.rs:1455`). `is_copy` already answers `CtorImage` = false
(`src/check/builtins.rs:560`; unit `is_copy_ctor_image_is_not_copy`,
:940), so the walk-time `Bound::Copy` discharge over a ctor-image head
(`crosscall.rs:76`'s guard arm declines a CtorImage — `type_is_registered`
answers true — so the plain Concrete arm at :87-88 fires) degrades to the
honest copy-bound rejection, never a
panic.

**Growth stays.** The Var arm at :208 precedes the deleted fence and
precedes the new arm, and its own supplied-match (:210-232) is untouched —
B2's bytes are pinned (baseline) and unit-pinned (tests.rs:3755,
phase7_slice3k.rs:195/:208). G2 is the must-not-move golden.

## G1 — `app_pass_through_cross_call_builds_and_runs_clean`

Fixture: `probes/s13_e_post_lift_green.sth`, verbatim (no edit). Today:
round E attempt 2's fence bytes — "error: `outer` cannot call the polymorphic
word `step` (line 11, col 41) / a higher-kinded application in a cross-called
polymorphic word is not yet supported from a polymorphic body / call `step`
from a monomorphic word instead", exit 1 (baseline). `main`'s grounding was
verified type-correct today by the probe's scratch variant with `outer`'s
body emptied (round E note). Post-fix: `build_run_keep` — exit 0, stdout
empty (the program is `42 Wrap outer drop`; nothing prints; the run path —
drop of a one-variant enum over i64 — was verified this round with the same
scratch trick: build ok, binary run exit 0, no output). `build_run_keep` and
not `build_ok` because the grounding in `main` is the point: it drives the
whole chain (mono App grounding → compose → apply_subst's App arm →
lowering's CtorImage expect). The harness copy drops the fixture's own
import line (header note).

## G2 — `bare_var_supplied_app_operand_still_grows_byte_identically`

Fixture: `probes/s13_b_bare_var_growth.sth`, verbatim. Today and post-fix:
the growth bytes, byte-identical — "error: `outer` cannot pass `'It['T]` to
`'T` of the polymorphic word `step` (line 6, col 41) / a polymorphic call
site may pass a type variable only bare: ... / declare `step`'s parameter as
`'It['T]` so the shape is matched structurally, or call it from a
monomorphic word", exit 1 (baseline B2). The pin rides a copy of slice12's
`build_error_bare` helper (precedent `tests/phase7b_slice12.rs:152`,
carried into `tests/phase7b_slice13.rs`) + the baseline byte-pin — NOT
`build_error_located`: the harness prepends the imports and drops the
fixture's own, shifting the diagnostic line 6→7, so the baseline bytes can
only be pinned from a byte-verbatim bare build. The Var arm precedes the
new App arm and is untouched, so
the lift cannot reach this verdict; if this golden moves, the slice has
broken precedence (probe fact 1).

## G3 — `app_vs_app_head_and_arg_vars_both_bind`

Fixture: edited copy of `probes/s13_d_arg_var_mismatch.sth` (D1) — the
callee already pass-through; outer gains its output, and the fixture gains
the Wrap type + a grounding main:

```text
\ S13 golden G3 — D1 post-lift green: App-vs-App, arg vars differ ('T callee, 'U caller).
import: intrinsics * ;

type: Wrap['X] | Wrap 'X ;

: step ['F 'T] ( 'F['T] -- 'F['T] ) ;
: outer ['It 'U] ( 'It['U] -- 'It['U] ) step ;
: main ( -- ) 42 Wrap outer drop ;
```

(harness copy drops the import line). Today, measured this round on this
text: the S1-17.i fence — "error: `outer` cannot call the polymorphic word
`step` (line 7, col 41)" + the two fence lines, exit 1. Post-fix:
`build_run_keep`, exit 0, stdout empty. Mechanism: input App-vs-App — head
binds F→`Image::CallerVar(It)`, arg recurses to the Var arm binding
T→`CallerVar(U)`; the output App arm renders `'It['U]` through the same
lookup, matching outer's declared output; main's mono grounding binds
It→CtorImage(Wrap), U→i64; compose θ_h = {F→CtorImage(Wrap), T→i64}. The
fixture edits are forced: without outer's output the caller's own stack
check fails (the body leaves `'It['U]` against declared outputs empty —
probe fact 5), and without `main` the link step fails by design (the
slice12 G1 convention; here the grounding main is spelled out instead).

## G4 — `app_vs_app_concrete_arg_grounds_the_element`

Fixture: edited copy of `probes/s13_d_arg_concrete.sth` (D2), same edit
shape as G3: outer becomes `: outer ['It] ( 'It[i64] -- 'It[i64] ) step ;`,
the Wrap type is added, and the grounding main
`: main ( -- ) 42 Wrap outer drop ;` is appended:

```text
\ S13 golden G4 — D2 post-lift green: App-vs-App, concrete arg ('It[i64] vs 'F['T]).
import: intrinsics * ;

type: Wrap['X] | Wrap 'X ;

: step ['F 'T] ( 'F['T] -- 'F['T] ) ;
: outer ['It] ( 'It[i64] -- 'It[i64] ) step ;
: main ( -- ) 42 Wrap outer drop ;
```

Today, measured this round: the fence at "(line 7, col 40)", exit 1. Post-
fix: `build_run_keep`, exit 0, stdout empty. Mechanism: the arg binds
T→`Image::Concrete(i64)` directly (the Var arm's Concrete case,
`crosscall.rs:210`); the output App arm renders `'It[i64]`; main grounds
It→CtorImage(Wrap) and compose θ_h = {F→CtorImage(Wrap), T→i64} — the
element's concreteness travels through the mapping, not around it.

## G5 — `bare_var_supplied_where_app_declared_is_a_rendered_mismatch`

Fixture: `probes/s13_d_bare_supplied.sth` (D3), verbatim. Today: D3's fence
bytes — "error: `outer` cannot call the polymorphic word `step` (line 8,
col 24)" + the two fence lines, exit 1 (baseline). Post-fix:
`build_error_located` — the verdict MOVES from the fence to the rendered
mismatch (the (App, Var) sub-dispatch routes to `mismatch()`): the shape is
"type mismatch in `step` (line 8, col 24) / `step` expected `'F['T]`, found
`'F` / note: declared ..." — same span as today's capture, exact bytes
pinned at implementation time from the live binary (the `note: declared`
tail renders the caller's effect via `effect_str`). The renderings are
verified in code: the declared App renders `'F['T]` — "the applied
variable's own surface spelling, then its arguments" (poly.rs:5973-5981,
the S1-14 comment; unit-pinned today by
`poly_type_str_renders_a_variable_application`, tests.rs:4549) — and the
supplied bare var renders `'F` (the Var arm, poly.rs:5908-5911 via
`foreign_var_str`). Rationale: a supplied bare variable always has kind `*`
— unbounded vars infer `*` (D3 sub-finding), and a `* -> *` variable cannot
be USED bare (the kind checker rejects it: C1 attempt 3, F1/F2) — so it can
never fill an applied head; binding it would type a value-kind variable as
a constructor.

## G6 — `concrete_ctor_supplied_as_head_binds_the_ctor_image`

Fixture: edited copy of `probes/s13_d_generic_supplied.sth` (D4) — outer
gains its output, plus the grounding main:

```text
\ S13 golden G6 — D4 post-lift green (ruling R-13.1: ctor-image head bind).
import: intrinsics * ;

type: Wrap['X] | Wrap 'X ;

: step ['F 'T] ( 'F['T] -- 'F['T] ) ;
: outer ['T] ( Wrap['T] -- Wrap['T] ) step ;
: main ( -- ) 42 Wrap outer drop ;
```

Today, measured this round: the fence at "(line 7, col 39)", exit 1 (D4's
declared-App face: the operand `Wrap['T]` is Generic-headed, the declared
input is App). Post-fix per ruling R-13.1 (bind, not reject):
`build_run_keep`, exit 0, stdout empty. Mechanism: the (App, Generic)
sub-arm binds F→`Image::Concrete(CtorImage(Wrap))` — arity 1=1; the
Generic's `(is_enum, idx, module)` header names the ctor — and recurses the
arg T→`CallerVar(T)`; the output App arm renders the head image to
`PolyType::Generic{Wrap, [Var(T)]}`, matching outer's declared `Wrap['T]`;
main grounds T→i64; compose θ_h = {F→CtorImage(Wrap), T→i64}; apply_subst's
App arm (unify.rs:843-911) resolves step's `'F['T]` → `Wrap[i64]`. This is
the golden that exercises R-13.1's load-bearing claim end to end (the
head-image grounding, not just the walk-time match).

## G7 — `generic_output_renders_through_the_mapping`

Fixture: edited copy of `probes/s13_c_generic_output.sth` (C2). The
committed C2 caller asks for a DIFFERENT var than it passes
(`['T 'U] ( 'T -- Wrap['U] ) mk ;`): post-lift that fails the CALLER's own
body check, because the render is faithful to the mapping — it returns
Wrap[caller's 'T], not the asked-for Wrap['U] (note below). The golden uses
the aligned spelling:

```text
\ S13 golden G7 — C2 post-lift green: Generic output rendered through the mapping.
import: intrinsics * ;

type: Wrap['X] | Wrap 'X ;

: mk ['T] ( 'T -- Wrap['T] ) Wrap ;
: caller ['U] ( 'U -- Wrap['U] ) mk ;
: main ( -- ) 42 Wrap caller drop ;
```

(the callee is tests.rs:3944's pattern — `: box ( 'U -- Box['U] ) Box ;` —
with the caller aligned to what the mapping can carry). Today, measured
this round: the output wildcard — "error: `caller` cannot call the
polymorphic word `mk` (line 7, col 34) / returning the compound type
`Wrap['T]` from a polymorphic word is not yet supported from a polymorphic
body / call `mk` from a monomorphic word instead", exit 1 (C2's arm; the
committed C2 capture is the same arm at line 11, col 37). Post-fix:
`build_run_keep`, exit 0, stdout empty. Mechanism: input T→`CallerVar(U)`
(Var arm, clean); the output Generic arm recurses Wrap's arg through the
Var-arm lookup (`crosscall.rs:375-391`) → `PolyType::Generic{Wrap,
[Var(U)]}` — the "Wrap['U-rendered]" of the brief — matching caller's
declared output; main grounds U→i64; compose grounds mk's `Wrap['T]` →
`Wrap[i64]` through the Generic mint arm (unify.rs:704-752).

Note (deliberate): the committed C2 spelling post-lift dies in the caller's
own body check — "body leaves `Wrap['T]`, but the declared outputs are
`Wrap['U]`" — which is the honest render, not a defect. The golden pins the
aligned spelling; this note records why the fixture text differs from the
committed probe.

## G8 — `array_output_renders_through_the_mapping`

Fixture: edited copy of `probes/s13_c_array_output.sth` (C4) — the only
edit is appending `: main ( -- ) ;` (below the error site):

```text
\ S13 golden G8 — C4 post-lift green: Array output rendered through the mapping.
import: intrinsics * ;

: mk ['T] ( array['T 4] -- array['T 4] ) ;
: caller ['U] ( array['U 4] -- array['U 4] ) mk ;
: main ( -- ) ;
```

Today, measured this round: the output wildcard — "error: `caller` cannot
call the polymorphic word `mk` (line 5, col 46) / returning the compound
type `array['T 4]` from a polymorphic word is not yet supported from a
polymorphic body / call `mk` from a monomorphic word instead", exit 1 (same
arm and column as C4's capture, whose error sits at line 9 col 46 — only
the line differs, because the committed fixture carries five comment lines
this copy drops). Post-fix: `build_ok` ONLY — no array constructor exists
intrinsics-only (C4 attempt 1), so no grounding main is spellable and
compose's array interning is not exercised by a golden; the claim is the
walk-time render: the Array arm recurses elem T→`CallerVar(U)` →
`array['U 4]`, matching caller's declared output. (apply_subst's array
interning is existing machinery — `intern_array_type`, unify.rs:657.)

## G9 — `the_e_shape_exercises_both_lifted_arms_end_to_end`

Fixture: G1's (`probes/s13_e_post_lift_green.sth`, verbatim) — this section
is the mechanism walkthrough G1 asserts, spelling out which arms fire, in
order:

1. `main` (mono) calls `outer`: `unify_poly_input`'s App arm
   (unify.rs:424-513) recovers Wrap's header from the `Wrap[i64]` slot and
   binds It→CtorImage(Wrap), T→i64 — works today (round E incidental: the
   mono route already grounds an App-declared word over a concrete ctor
   head).
2. `outer`'s body walk hits `step`: the NEW input arm (App, App) — arity
   1=1; head F→`CallerVar(It)` through the Var-arm consistency block
   (`crosscall.rs:283-296`); arg T→`CallerVar(T)` through the Var arm
   (:208).
3. The NEW output App arm (`poly_cross_output`): head lookup →
   `CallerVar(It)` → `PolyType::Var(It)`; arg lookup → `Var(T)`; renders
   `'It['T]`, matching outer's declared output — the body stack check
   passes.
4. At `outer`'s grounding: compose (`instantiate.rs:682`) builds
   θ_h = {F→CtorImage(Wrap), T→i64} from the mapping (:696-702); apply_subst
   resolves step's output `'F['T]` → `Wrap[i64]` via the App arm
   (unify.rs:843-911) with the registries interning.
5. Lowering: the App head resolves through `subst.ty` to the CtorImage —
   "checked: unification bound the App head to a CtorImage; any other
   binding is rejected before lowering (S1-15.g)" (src/ir/driver.rs:708-711).

No new fixture; no harness run beyond G1's. This is the shape that makes
"the lift touches both fences" observable: the input arm at step 2, the
output arm at step 3, and the recorded prediction's two claims (head binds
`CallerVar`; rendering needs no walk-time interning) at steps 2-4.

## G10 — `moved_pins_retarget` (the four moving pins)

1. `src/check/poly/tests.rs:608`
   `non_member_app_cross_call_still_rejects_with_p8_fence_text` — fixture
   `inner['G 'T] ( 'G['T] -- ) drop ;` / `outer['G 'T] ( 'G['T] -- ) inner ;`
   / empty main; asserts "cannot call the polymorphic word `inner`" (:617)
   and the higher-kinded sentence (:622). Post-lift the shape checks clean:
   input (App, App) binds G→`CallerVar(G)`, T→`CallerVar(T)`; no outputs on
   either side; no bounds. Retarget: flip to a green assertion (the fixture
   checks Ok), rename e.g.
   `non_member_app_cross_call_checks_clean_after_the_lift`, delete the two
   fence-text asserts, and rewrite the stale doc comment above it
   (:602-606, "the cross-call fence is NOT lifted").
2. `src/check/poly/tests.rs:1782`
   `poly_cross_match_app_slot_is_unsupported_not_a_panic` — direct
   `poly_cross_match` call, declared `App{head:0, args:[Var(1)]}` vs
   supplied `Var(0)`: exactly G5's shape. Retarget: expect the mismatch —
   assert "type mismatch" and the two renderings (`'F['T]` expected, `'F`
   found); rename per G5 (e.g.
   `poly_cross_match_app_slot_vs_bare_var_is_a_rendered_mismatch`); the
   not-a-panic property is what the harness's no-panic checks assert
   elsewhere.
3. `src/check/poly/tests.rs:3935`
   `check_cross_call_unsupported_callee_shapes_name_themselves`,
   sub-fixture 2 (:3942-3947, "returning the compound type `Box['U]` ..."):
   post-lift green — box's `Box['U]` renders Box[g's 'T] through the
   mapping and `drop` consumes it, g declares no outputs. Retarget:
   RETARGET IN PLACE (phase 2) — keep the tuple, flip its expectation to
   the post-lift clean check (same source, green), rename/comment
   accordingly; it is the poly-body-drop twin of G7 (`: g ( 'T -- ) box
   drop ;` is a shape no other golden covers). Sub-fixtures 1 (:3940,
   length var in the callee sig) and 3 (:3948-3952, row-polymorphic call)
   are different fences and stay.
4. `tests/phase7b_slice12.rs:450` `cross_call_app_fence_stays_byte_identical`
   (fn :450; the byte `assert_eq!` at :467-470, expected-string line :469) —
   slice12's G8. The pinned shape (Cursor-bounded `i64 'It[i64] -- i64`
   pass-through pair) post-lift: input App-vs-App binds It→`CallerVar(It)`,
   arg i64/i64 Concrete; the `Bound::User(Cursor)` discharges symbolically
   (the caller declares `['It: Cursor]` too — `sig.has_bound`,
   `crosscall.rs:56`); outputs i64 concrete. Checks clean. Retarget:
   replace the byte-pin with a green assertion — same source plus
   `: main ( -- ) ;` through the harness's `build_ok`; name e.g.
   `cross_call_app_slot_checks_clean_after_the_hkt_lift`; rewrite the doc
   comment (:441-448, "the poly-body cross-call fence ... is a follow-up
   slice (SOO-60) and does not move") to record that SOO-60 is that slice
   and the byte-pin moved by design. A grounding main for this shape would
   need an `Opt` value spelled in `main` (its typing unverified here); the
   empty-main `build_ok` is the retarget, and the grounding twin is
   REQUIRED golden G12 (spec, phase 3 — user-ratified): its fixture text
   and bytes are finalized and pinned at implementation time.

## G11 — `existing_suite_green` (regression canaries)

`cargo test` at the fix commit: 3528+ passed, 0 failed (3528 at base plus
this slice's new/retargeted tests). Named canaries, all expected untouched —
the lift's diff surface is `src/check/poly/crosscall.rs`, its tests, and the
ast.rs Image doc comment:

- slice12's G9 byte-stability canaries: `tests/phase7b_slice8.rs:425`
  `for_each_drains_a_list_through_the_iterator_bound`, :446
  `fold_sums_a_list_through_the_iterator_bound`, :535
  `for_each_and_fold_drain_a_range_through_the_iterator_bound`.
- Growth: `tests.rs:3755` (`check_growing_cross_call_is_error`, asserts
  "builds a larger type at every hop" :3765), `tests.rs:3779`
  (`check_growing_cross_call_concrete_reference_is_unsupported_not_growth`,
  the R6 concrete-compound arm), `tests/phase7_slice3k.rs:195`/`:208`
  (`a_cross_call_growing_the_type_is_a_located_rejection`).
- Quotation: `tests.rs`, `poly_quotlit_against_legal_inline_quotation_param_rejects_at_the_cross_call`
  (expect at :6051; asserts "passing a quotation to a polymorphic word"
  :6053-6057) — the Var-arm rejection, untouched by the lift.
- Inline routing: `tests.rs:4157`/`:4176` (the transitive-discovery pair;
  same first line as the fence, different reason).
- Ambiguity: `tests/phase7_slice3k.rs:326`
  (`a_cross_call_through_an_overloaded_generic_word_is_a_located_rejection`;
  the probe cites :325, the fn sits at :326 at `a3779f4`).
- slice12's render goldens: the untouched tests in
  `tests/phase7b_slice12.rs` stay green; the only one that moves is G10.4.

## Facts the spec must carry

1. Precedence: the `(PolyType::Var(v), _)` arm (`crosscall.rs:208`) fires
   before the lifted App arm — an App operand facing a BARE-var declared
   input stays the GROWTH error (:498; B2 bytes; unit pins tests.rs:3755,
   phase7_slice3k.rs:195/:208). The lift must not touch the Var arm's
   supplied-match (:210-232).
2. The lifted input arm's sub-dispatch: supplied App → arity guard + head
   bind + pairwise recursion (existing consistency/conflict machinery,
   `crosscall.rs:283-296`); supplied Generic → the R-13.1 ctor-image bind
   (a supplied Generic with `len_args` takes the mono route's S1-7
   rejection, `unify.rs:467-477` precedent); supplied Var → rendered
   mismatch (G5); everything else → the :354 catch-all. The second fence
   pattern `(_, PolyType::App { .. })` is deleted; supplied-App against a
   declared non-App slot becomes a mismatch (R-13.2 — deliberate, unpinned
   today).
3. Output arms (`poly_cross_output`, :365-405): Generic, Array, and App
   render through the mapping (each recursing the Var-arm lookup at
   :375-391); `len_args`/array lengths pass through concrete (`Len::Var`
   fenced upstream at :443-445); the App head renders `CallerVar` →
   `PolyType::Var(w)` and `Concrete(CtorImage)` → `PolyType::Generic`;
   Ref gains no arm (C3 — banned at declaration); GenericVariant stays in
   the wildcard (:392-397, R3.5 convention).
4. The ctor-image head rides `Image::Concrete(Type::CtorImage)` — no new
   Image variant; `ctor_image_type` (ast.rs:3596) stays the sole
   constructor and the cross-call arm mints through `ctx.generics()`
   (engine.rs:1455). The Image doc comment (ast.rs:2999-3005, "no type
   constructor ever needs representing here") is stale after the lift and
   must be rewritten (comment-only).
5. Grounding is existing machinery: compose (`instantiate.rs:682`) folds
   the mapping into θ_h (:696-702) and grounds outputs via `apply_subst`
   (unify.rs:598-607) whose registries intern Generic/Array/Ref/cell shapes
   (:657/:693/:700) and mint Generic/GenericVariant/App through
   `ctx.generics()` (mint calls :756/:758; :808-816/:903-909); lowering
   asserts the App-head-CtorImage invariant (src/ir/driver.rs:708-711).
   `is_copy(CtorImage)` = false (src/check/builtins.rs:560) keeps the
   walk-time Copy discharge honest.
6. Cross-called callees must be self-consistent first: every green golden
   uses a pass-through or constructing body; an empty body only checks when
   declared outputs equal inputs (probe fact 5 — the D/C attempts that died
   at the callee's own stack check).
7. Harness copies of s13 fixtures drop the fixture's own
   `import: intrinsics * ;` — `single_file_hosted` prepends it, and a
   duplicate import collides (the import seen-map; collision errors at
   declarations.rs:906/:912-918).
8. Measured locations at `a3779f4`: Var arm :208, image match :210-211,
   consistency block :283-296 (conflict at :287), S1-17.i fence :346-353
   (comment :344-345), catch-all :354, `poly_cross_output` :365 (Var arm
   :375, wildcard :398-405, message :403), signature gate :429-453 (len
   fence :443-445),
   growth error :498, unsupported error :564. The handed-down d7559bb
   numbers are one line stale (fence arm closes :353, wildcard :398-405) —
   see the probe report's drift note.
9. Moved pins (round G) and their retargets are G10: tests.rs:608 (flips
   green), :1782 (retargets to the G5 mismatch), :3935 sub-fixture 2
   (retargeted in place — green, the poly-body-drop twin of G7),
   phase7b_slice12.rs:450 (byte-pin →
   green). Pins that stay: growth (:3755, phase7_slice3k.rs:195/:208),
   concrete-compound (:3779), quotation (:6051), inline-routing (:4157/
   :4176), ambiguity (phase7_slice3k.rs:326).

## Open rulings

- **R-13.1 (the D4 head image — the slice's one open design bit).**
  CHOSEN: bind the callee's head variable to
  `Image::Concrete(Type::CtorImage)` minted via `ctor_image_type` over the
  supplied Generic's header. REJECTED: the rendered mismatch ("you cannot
  pass a concrete ctor where a higher-kinded variable is declared").
  Consequences if the rejection were taken instead: the cross-call route
  becomes strictly weaker than the mono route for the identical shape — a
  mono call site grounds `'It:=Wrap` over an App-declared word clean today
  (round E incidental; `unify_poly_input`'s App arm, unify.rs:424-513) —
  and D4's twin where the caller spells its input `'It['T]` instead of
  `Wrap['T]` (the E shape) goes green while the `Wrap['T]` spelling stays
  red, with no kind-level difference between the two programs. The chosen
  ruling reuses established App-head semantics end to end: bind
  (unify.rs:488-497), ground (unify.rs:843-911), lower
  (src/ir/driver.rs:708-711),
  symbol-key on `GenericId` (ast.rs:3055-3060, S2-12). If the spec
  overrides to the rejection: G6 flips to `build_error_located` with the
  mismatch bytes pinned live; G10.4's retarget is unchanged; nothing else
  in this document moves. Recorded here in the PB-5 pattern for the spec's
  open-questions section.
- **R-13.2 (supplied App vs declared non-App).** Post-lift: rendered
  mismatch. Today that face is the fence's second pattern; no fixture or
  pin exercises it (round G's inventory has none), so deleting the pattern
  without a replacement arm hands the face to the :354 catch-all --
  deliberate, recorded so the deletion is not read as an oversight. If the
  spec prefers keeping a named rejection there, it is a new diagnostic (a
  third message) and out of this slice's no-new-text posture.
- **R-13.3 (bound discharge over a ctor-image head — user-ratified: required
  G12).** The lift adds no machinery. The walk-time `Bound::Copy` arm
  degrades honestly (`is_copy(CtorImage)` = false, src/check/builtins.rs:560,
  so the copy-bound rejection fires -- crosscall.rs:87-88). A `Bound::User`
  on a ctor-image head defers to compose's `resolve_user_bound` loop
  (crosscall.rs:104 defers; instantiate.rs:726-750 resolves) with
  ty = CtorImage. Golden G12 (spec, phase 3) now VERIFIES that existing
  path end to end: the G10.4 source plus a spelled grounding main that
  instantiates the bounded consumer, its fixture text and bytes finalized
  and pinned at implementation time. If G12 exposes a
  resolve_user_bound-over-CtorImage defect, that is an implementation-time
  P0 to fix or scope explicitly — goldens are the discovery mechanism.
