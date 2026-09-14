# P7b.S15 paper tests — validated golden designs (Applicative.pure, SOO-30)

> **Slice number note:** 15 is provisional — the next free P7b number at writing
> time (`slice14-spec.md` is the highest existing P7b document). Renumber if the
> pipeline assigns differently.

Designed against the clean tree at HEAD `846f952` (worktree `soo-30`; the tree
carries only `probes/soo30*` additions, no `src/` or `lib/` changes). The probe
rounds are complete and verified: [slice15's ground truth is
`probes/soo30_findings.md`](../../probes/soo30_findings.md) (round 1) and
[`probes/soo30r2_findings.md`](../../probes/soo30r2_findings.md) (round 2),
with byte-exact stderr in `probes/soo30_baseline.md` and
`probes/soo30r2_baseline.md`. Format/voice: [slice13-paper-tests](./slice13-paper-tests.md).

**The post-slice world** these goldens describe is HEAD `846f952` plus exactly
four things: the S2-15.a gate relaxation (R-30.1), output-side grounding of
explicit instantiations at mono member calls (R-30.2), the bare-call remedy
correction (R-30.3), and the recorded-not-fixed zero-input defect (left as-is).
Round 2 ran at HEAD **plus the exact R-30.1 patch** (r2 findings, "The patch"),
so **every round-2 byte is a post-slice byte for any path R-30.2/R-30.3 do not
touch**. That equivalence is what makes most expectations below fully measured.
Round 2's `cargo test` inventory (r2f) measured the relaxation's blast radius:
exactly two flipped pins, nothing else.

Harness conventions from `tests/phase7b_slice2.rs`: `sooth_build` (:42),
`build_ok` (:50), `build_error` (:61; the caller asserts on the returned
stderr), `build_and_run` (:69), `single_file_hosted` (:92 — writes a temp
`sooth.pkg` depending on `core` + `hosted` and prepends
`import: intrinsics * ;` + `import: hosted::show | . | ;`), `single_file`
(:108 — prepends the intrinsics import only). Harness copies of fixtures drop
the fixture's own `intrinsics`/`show` import lines (duplicate imports collide;
the slice13 header note). Standalone fixture texts below are complete; the
placement section says which lines each harness copy drops.

## The mechanism the goldens pin (the four rulings)

- **R-30.1 (gate rule).** `member_binds_trait_var`
  (src/check/declarations.rs:408-418, error at :452-473) today refuses any
  member whose nonempty inputs lack a dispatchable head (`Var(0)` or
  `App { head: 0 }`, `dispatchable_head` at :402-405). The relaxation mirrors
  the same `dispatchable_head` check over the member's **outputs**: a member
  whose inputs don't mention the trait var is admitted IFF at least one output
  is trait-var-headed. Round 2 measured this exact patch: the true shape
  `pure ( 'A -- 'F['A] )` declares (r2a), all real-lib ctor impls check (r2c),
  and the full suite flips exactly the two gate pins (r2f).
- **R-30.2 (output-side grounding).** At a mono member call, an explicit
  instantiation unifies against the member's **output App** `'F['A]`:
  `5 pure[Option[i64]]` dissolves `'F:=Option`, `'A:=i64`, keys impl selection
  on the dissolved ctor head, and dispatches. Partially-applied ctor heads ride
  along as they already do on the operand path (P3b's 2-arity Result). The
  bare-ctor spelling `pure[Option]` STAYS parse-refused (p2a/P3 bytes).
- **R-30.3 (remedy correction).** The bare-call remedy error keeps its located
  two-line shape (r2b-2) but its example becomes achievable — rendered with
  knowledge of the dissolved member word's actual var count (today's example
  `pure[i64]` supplies one argument to a two-var dissolved word, which is
  exactly the arity wall r2b-1 measured).
- **R-30.4 (scope).** Plain-word output-only bound vars (the twin class) stay
  walled — DEFERRED, not in this slice. The shared-bound consumer golden (G5)
  uses an `'F`-in-input consumer shape instead, which is callable at mono sites
  by the P3b-proven operand-grounding pattern.

## G1 — `true_shape_member_declares_and_ctor_impl_checks`

Fixture: `probes/soo30r2_a_decl_impl.sth`, verbatim (fixture-local Box):

```text
import: intrinsics * ;
type: Box['A] | MkBox 'A ;
trait: Applicative['F: * -> *]
  : pure ( 'A -- 'F['A] ) ;
;
impl: Applicative for Box
  : pure MkBox ;
;
: main ( -- ) 42 drop ;
```

Today (HEAD, strict gate): the S2-15.a declaration error — `error: trait member
'pure' of 'Applicative' (line 4, col 5) has no input for a call to dispatch on
(expected the trait's variable ''F' bare or heading an application like
''F['T]')`, exit 1 (r1 baseline § p1 attempt 3). Post-slice: `build_ok` — build
clean, no stderr, exit 0.

Receipt chain: r2a measured **exactly these bytes** under the exact R-30.1
patch (build clean, exit 0) — the impl body `MkBox` unifies its `Box['A]`
output against the dissolved declared output through the input var, so the
P2e identical-renderings wall does not fire on the true shape (r2a; root-caused
r2e). Generalization receipt: the same shape's impls check on real lib
Option/Result/List (r2c, "all three ctor impls pass"). Fully measured.

## G2 — `option_ctor_constructs_through_the_output_app_instantiation`

Intent: the R-30.2 spelling on the real lib Option — the ticket's central
construction surface. New fixture text (show idiom verbatim from
`probes/soo30r2_c2_dispatch_operands.sth`):

```text
\ S15 golden G2 — Option through the R-30.2 spelling.
import: intrinsics * ;
import: core::option * ;
import: core::applicative | Applicative | ;
import: hosted::show | . | ;

: showopt ( Option[i64] -- ) ~[ ( Some ) Some> . ] ~[ ( None ) drop 0 . ] Option? ;

: main ( -- ) 5 pure[Option[i64]] showopt ;
```

Post-slice: `build_and_run` — build clean, run exit 0, stdout `5\n`
(`pure` dissolves to the Option impl `Some`, producing `Some(5)`; `showopt`
prints the inner `5`).

Receipt chain: (i) the spelling PARSES and resolves to the dissolved member
word — r2b-1/r2c measured the call reaching `pure;Applicative;4;Option['T0]`
(today it dies there on the 2-vars-1-arg arity wall); (ii) the Option impl
checks clean in the lib scaffold (r2c) and dispatches end-to-end when grounding
succeeds — r2c attempt 4 ran `5 Some pure[Option[i64] Option[i64]] showopt2`
printing `5`; (iii) R-30.2 supplies the new grounding itself.

**UNMEASURED-AT-RISK:** the R-30.2 spelling's success bytes. No probe has run a
successful 1-arg output-App instantiation; today's bytes for this exact main
are the r2c arity wall. Round 3 measures: build exit 0, stdout `5\n`. The
golden also presupposes the lib landing (lib/core/applicative.sth +
co-located `impl: Applicative for Option : pure Some ;`, `applicative` before
`option` in the pkg module list — the r2c scaffold shape, findings § R2c).

## G3 — `result_two_arity_ctor_constructs_with_partial_head`

Intent: the 2-param ctor through the same spelling — the
"partially-applied ctor heads ride along" case. New fixture text (show idiom
verbatim from the r2c fixture):

```text
\ S15 golden G3 — 2-arity Result through the R-30.2 spelling.
import: intrinsics * ;
import: core::result * ;
import: core::applicative | Applicative | ;
import: hosted::show | . | ;

: showres ( Result[i64 i64] -- ) ~[ ( Ok ) Ok> . ] ~[ ( Err ) drop 1 . ] Result? ;

: main ( -- ) 5 pure[Result[i64 i64]] showres ;
```

Post-slice: `build_and_run` — build clean, run exit 0, stdout `5\n` (the
Result impl `Ok` produces `Ok(5)`; `showres` prints the inner `5`).

Receipt chain: the Result impl (2-param target, body `Ok`) checks clean (r2c);
2-arity grounding works on the operand path — P3b measured operand `9 Err`
grounding `'F` with output `Ok 5`; the fully-positional instantiation route
ran end-to-end (r2c attempt 4: `5 Ok pure[Result[i64 i64] i64 Result[i64 i64]]
showres2`, three args, printing `5`). R-30.2's "partial heads ride along as on
the operand path" is the ruling that carries the 1-arg supply
(`Result[i64 i64]` fills `'F:=Result` with the impl's own vars carrying the
params).

**UNMEASURED-AT-RISK:** the 1-arg output-App spelling through a 2-param ctor.
The operand-path analogue is measured (P3b); the instantiation-path analogue is
the ruling. Round 3 measures: build exit 0, stdout `5\n`.

## G4 — `list_ctor_constructs_a_real_cons_cell`

Intent: the allocating ctor — `pure` must produce a genuine `Cons`-cell, not a
wrapper. New fixture text (show idiom verbatim from the r2c fixture):

```text
\ S15 golden G4 — List through the R-30.2 spelling.
import: intrinsics * ;
import: core::list * ;
import: core::applicative | Applicative | ;
import: hosted::show | . | ;

: showlist ( List[i64] -- )
  ~[ ( Nil ) drop ]
  ~[ ( Cons ) Cons> | v rest | v . rest ^> showlist ]
  List? ;

: main ( -- ) 5 pure[List[i64]] showlist ;
```

Post-slice: `build_and_run` — build clean, run exit 0, stdout `5\n` (the List
impl `Nil ^ Cons` produces `Cons(5, Nil)`; `showlist` walks the cell).

Receipt chain: the List impl body checks clean — r2c explicitly notes the
cell-boxing `^` interns an owned cell over the payload
(src/check/word_families.rs:1195; List's `rest` field is `^List['T]`) with no
impl mismatch; r2c attempt 4's List run (`5 single pure[List[i64] List[i64]]
showlist2`) printed `5` and P3b's `showlist` walked a real `Cons`-constructed
cell. Note G4's main needs NO `nile`/`single` helpers: those existed in r2c
only to construct List *operands* through the overloaded `Nil`; here `pure`
constructs the list, so `Nil`'s consumer is the impl body, whose dissolved
declared output pins it (r2c's impl-check receipt).

**UNMEASURED-AT-RISK:** the R-30.2 spelling's success bytes (as G2). Round 3
measures: build exit 0, stdout `5\n`.

## G5 — `shared_bound_consumer_dispatches_two_ctors_through_one_definition`

Intent: the ticket's "declared on a shared `Applicative` bound … dispatches per
constructor" surface — ONE poly definition, called at two mono sites with
different ctor operands. New fixture text (show + `nile` idioms verbatim from
the r2c fixture):

```text
\ S15 golden G5 — one shared-bound consumer, two constructors.
import: intrinsics * ;
import: core::option * ;
import: core::list * ;
import: core::applicative | Applicative | ;
import: hosted::show | . | ;

: showopt ( Option[i64] -- ) ~[ ( Some ) Some> . ] ~[ ( None ) drop 0 . ] Option? ;
: showlist ( List[i64] -- )
  ~[ ( Nil ) drop ]
  ~[ ( Cons ) Cons> | v rest | v . rest ^> showlist ]
  List? ;

\ pins Nil to List[i64] so the operand construction is unambiguous (r2c idiom)
: nile ( -- List[i64] ) Nil ;

: repure['F: Applicative 'A] ( 'F['A] 'A -- 'F['A] ) swap drop pure ;

: main ( -- )
  5 Some 7 repure showopt
  5 nile ^ Cons 7 repure showlist ;
```

Post-slice: `build_and_run` — build clean, run exit 0, stdout `7\n7\n`. The
`7` (not the operands' `5`) is what makes dispatch observable: each call drops
its ctor operand and re-pures the value through the SAME definition, and the
result carries the operand's constructor (Some(7) / Cons(7, Nil)).

Mechanism note (one correction to the ticket sketch): the sketched body
`swap pure` leaves the old container sitting under `pure`'s operand — with row
`( 'F['A] 'A -- 'F['A] )` the value is on TOP, so after `swap` the container is
on top and `pure` would consume it. The minimal fixing edit is
`swap drop pure`, which is exactly P3b's measured impl-body prefix
(`swap drop MkBox` / `swap drop Some` / `swap drop Ok` / `swap drop Nil ^
Cons`) with the ctor word replaced by the bound-supplied member call.

Receipt chain: (i) a poly body's bare `pure` is supplied by the word's own
`'F: Applicative` bound — r2d-a CONFIRMED this for twin's body (both errors
cited main, never the body; the mconcat precedent,
tests/phase7b_slice6.rs:347, is the shipped golden of a plain poly word with a
trait bound); (ii) the row shape `( 'F['A] 'A -- ... )` grounds `'F` from the
operand at mono sites — P3b's measured member row, and the mconcat golden is a
plain poly word with a bound called bare at a mono site dispatching over
core::option; (iii) the `swap drop` body prefix is measured (P3b/r2c impl
bodies). 'A:=i64 grounds consistently from both operands in each call.

**UNMEASURED-AT-RISK:** the whole fixture's run bytes — no probe ran a
successful `repure`-class call (r2d tested only the twin shapes, whose `'F` is
output-only). Round 3 measures: build exit 0, stdout `7\n7\n`. If round 3
exposes a wrinkle (e.g. the bound-supplied call's output not composing with the
operand-grounded `'F` at a plain-word site), that is discovery — goldens are
the discovery mechanism (slice13 posture).

## G6 — `bare_pure_mono_call_keeps_the_located_remedy_error_with_an_achievable_example`

Fixture: `probes/soo30r2_b2_bare.sth`, verbatim:

```text
import: intrinsics * ;
type: Box['A] | MkBox 'A ;
trait: Applicative['F: * -> *]
  : pure ( 'A -- 'F['A] ) ;
;
impl: Applicative for Box
  : pure MkBox ;
;
: main ( -- ) 42 pure drop ;
```

Post-slice expected (R-30.3-corrected second line):

```text
error: `pure` in `main` (line 9, col 18) is a trait member with no operand to dispatch on
  a monomorphic body cannot infer the trait's type here; write an explicit type argument, e.g. `pure[i64 i64]`
exit=1
```

Receipt chain: r2b-2 measured the shape at this exact fixture — line 1 is
byte-exact as shown (including col 18); line 2's prefix through `e.g.` is
byte-exact; today's example token is `pure[i64]`, which R-30.3 corrects: the
dissolved member word `pure;Applicative;0;Box['T0]` declares TWO vars
(`'ctor0`, `'A` — r2b-1's mechanism), so the corrected example supplies two
arguments (`pure[i64 i64]`, positional over the dissolved var list, target
params first — the order r2c attempt 4 exercised).

**UNMEASURED-AT-RISK:** the exact corrected example token. The ruling fixes
the requirement (var-count-correct, achievable) but not the rendering; the
plausible alternative is the output-App spelling `pure[Box[i64]]` (R-30.2's
golden spelling, G1's ctor). Round 3 pins the live remedy line. The golden
assertion should therefore pin line 1 byte-exact plus line 2's prefix through
`e.g. \``, and separately assert the example carries TWO type arguments.

## G7 — `bare_ctor_instantiation_argument_stays_parse_refused`

Intent: the R-30.2 landing must not weaken the S1-fence on bare ctor heads in
instantiation arguments (word-general; the `pure[Option]` spelling is refused
before checking). New fixture text (layout verified this round):

```text
import: intrinsics * ;
import: core::option * ;
import: core::applicative | Applicative | ;
import: hosted::show | . | ;
: main ( -- ) 5 pure[Option] drop ;
```

Post-slice expected (unchanged from today):

```text
error: generic type `Option` declares 1 type variable, but none were supplied at line 5, col 22 (apply it as `Option[T]`, one type argument per declared variable)
  note: a glued bracket is an explicit type instantiation; insert a space for a quotation or array literal
exit=1
```

Receipt chain: the message and remedy are r1-measured twice — p2a (fixture-local
`Box`, "line 9, col 23") and r1 P3 attempt 2 (lib `Option`, "line 19, col 10");
the column points at the generic NAME inside the brackets, not the word. The
line/col for THIS layout was measured this round via a borrowed-idiom run
(`cargo run -q -- build` on the same text with the applicative import line
replaced by a comment, `--manifest tests/fixtures/sooth.pkg`): the fence fired
at col 22 with both lines above, exit 1 — **today bytes, recorded as a layout
measurement only**; the verdict is expected byte-identical post-slice because
the slice does not touch the parse fence (it precedes module checking, so the
post-slice presence of core::applicative cannot change it; r1 P3 proved parse
precedes the declaration gate the same way). Fully measured.

## G8 — `wrong_shape_instantiation_still_fails_impl_selection`

Fixture: `probes/soo30r2_b1c_two_args.sth`, verbatim:

```text
import: intrinsics * ;
type: Box['A] | MkBox 'A ;
trait: Applicative['F: * -> *]
  : pure ( 'A -- 'F['A] ) ;
;
impl: Applicative for Box
  : pure MkBox ;
;
: main ( -- ) 42 pure[i64 i64] drop ;
```

Post-slice expected (conservative — unchanged from r2 bytes):

```text
error: `pure` in `main` (line 9, col 18) is a trait member of Applicative, but no `impl:` in this program dispatches on these operands
  the operand types here are `i64`; declare an impl of one of those traits for the operand's type, or import a word that claims this name
exit=1
```

Receipt chain: r2b-1c measured exactly these bytes under the R-30.1 patch: the
2-arg list satisfies the dissolved word's `('ctor0, 'A)` var list positionally,
then impl selection — which is operand-head-driven — fails because the `i64`
operand never mentions `Box`.

**UNMEASURED-AT-RISK (flagged by the task):** R-30.2 may change this verdict.
If the implementation keys impl selection on the ctor head of the
instantiation-grounded OUTPUT for positional supplies too, then `'ctor0:=i64,
'A:=i64` grounds the output to `Box[i64]`, the Box impl selects, and this
golden FLIPS to success (build clean; `42` boxed and dropped). The ruling text
keys selection on "the dissolved ctor head" only for the output-App unification
route (`pure[Option[i64]]`-class supplies). Round 3 measures which; if it
flips, retarget this golden to the success (build_and_run, stdout empty) and
rename accordingly — the flip is a ruling consequence, not a regression.

## G9 — `ap_declaration_stays_parse_fenced`

Intent: pins the companion scope — the shipped trait declares pure ONLY; `ap`
cannot even appear in a trait declaration. Fixture:
`probes/soo30_p5_ap_in_trait.sth`, verbatim:

```text
import: intrinsics * ;
type: Box['A] | MkBox 'A ;
trait: Applicative['F: * -> *]
  : pure ( 'A -- 'F['A] ) ;
  : ap ( 'F[ [ 'A -- 'B ] ] 'F['A] -- 'F['B] ) ;
;
impl: Applicative for Box
  : pure MkBox ;
;
: main ( -- ) 42 drop ;
```

Post-slice expected (unchanged):

```text
error: expected a type, found `[` at line 5, col 14 (a type application's arguments are types, not quotations)
exit=1
```

Receipt chain: r1 P5 measured exactly these bytes at this HEAD (the S1-6
fence, src/parser.rs:2882; ap's line 5 col 14 in this layout), and the slice
does not touch the fence — the impl-coverage-vs-quotation-audit question stays
moot behind it (r1 P5; the slice7 brief's audit fence,
src/check/audits.rs:431/:484, remains unreachable). Fully measured.

## G10 — `moved_gate_pins_retarget` (the two flipped pins)

r2f measured the relaxation's exact blast radius: two pins, both the same
`pick ( 'T -- 'F['T] )` shape, nothing else.

1. **Unit, src/check/declarations.rs:3932**
   `check_trait_decls_rejects_member_with_no_dispatchable_input` (helper
   `trait_check_src` at :3842; r2f: panics at :3942 on `unwrap_err()`). The
   member now DECLARES. Retarget: flip to a success assertion — same fixture
   string, `.unwrap()`, rename per the naming convention to
   `check_trait_decls_accepts_member_with_only_output_trait_var`, and rewrite
   the doc comment (:3927-3931) to record that R-30.1's output arm admits the
   shape (r2f measured exactly this flip). Keep a direct predicate unit beside
   it (precedent `member_binds_trait_var_accepts_any_receiver_position`,
   :3889): `member_binds_trait_var` returns true for a member whose inputs
   don't mention the trait var but whose output is `'F['T]`-headed.
2. **Golden, tests/phase7b_slice2.rs:144**
   `hkt_member_without_dispatchable_input_is_located_error` (helper
   `build_error` at :61). Same story: the fixture builds clean post-slice, so
   the verbatim S2-15.a text assert fails (r2f).
   **Pick: retarget IN PLACE to the R-30.1 POSITIVE shape** — same fixture
   (member `pick ( 'T -- 'F['T] )` under `Functor['F: * -> *]`), `build_ok`,
   rename e.g. `hkt_member_with_only_output_trait_var_declares`. Why the
   positive: r2f measured THIS EXACT fixture flipping to a clean build — the
   direct receipt; the fixture's line/col references in surrounding comments
   stay meaningful; and the S2-15.a error text keeps golden-level coverage via
   the new negative below plus the retained unit pin at :3964
   (`check_trait_decls_rejects_a_receiver_nested_in_an_array_input`).
   Retain the old test's distinguishing negative asserts inside the new
   negative sibling (next bullet), not here.
3. **New negative sibling (both levels).** R-30.1's refusal arm must stay
   observable: a member that mentions the trait var NOWHERE still refuses.
   Golden in phase7b_slice2.rs beside the retarget, fixture `pick ( 'T -- 'T )`
   (harness puts the member at line 4, col 5 — same span as the old golden's
   assert), expecting the S2-15.a text byte-for-byte and `!err.contains("note:")`
   (no nested-input note — the var appears nowhere near a composite). Unit twin
   in declarations.rs tests:
   `check_trait_decls_rejects_member_mentioning_the_trait_var_nowhere`.
   Receipt: DERIVED, not directly measured — the r2 patch's output arm requires
   a dispatchable output head and `dispatchable_head('T)` is false, and r2f's
   full-suite inventory ("flips EXACTLY those 2 pins, nothing else") confirms
   no other refusal moved. Round 3: run both negative fixtures under the gate
   patch (trivial).

## G11 — `twin_class_plain_word_walls_unchanged` (R-30.4's deferral, pinned)

All three r2d walls stay byte-identical post-slice; R-30.2 is member-call
grounding and must not leak into plain-word instantiation. Preamble for all
three (verbatim `probes/soo30r2_d_twin.sth` lines 1-9): the r2a decl+impl plus
`: twin['F: Applicative 'A] ( 'A 'A -- 'F['A] 'F['A] ) pure swap pure ;` at
line 9.

- **G11.a — arity wall** (`probes/soo30r2_d_twin.sth` verbatim, main
  `1 2 twin[Box[i64]] drop drop`): `error: \`twin\` (line 10) declares 2 type
  variables (\`'A\`, \`'F\`) but was given 1 type argument`, exit 1.
- **G11.b — the named output-only wall (primary pin)**
  (`probes/soo30r2_d2_twin_bare.sth` verbatim, main `1 2 twin drop drop`):

  ```text
  error: `twin` in `main` (line 10) has output variable `'F` that no input binds
    note: supply it explicitly: `twin[SomeType SomeType]`
  exit=1
  ```

- **G11.c — the remedy is unachievable for a `* -> *` var**
  (`probes/soo30r2_d3_twin_hkt_arg.sth` verbatim, main
  `1 2 twin[i64 Box] drop drop`): parse refusal — `error: generic type \`Box\`
  declares 1 type variable, but none were supplied at line 10, col 28 (apply it
  as \`Box[T]\`, one type argument per declared variable)`, exit 1.

Receipt chain: all three measured under the exact R-30.1 patch (r2 baseline §§
d, d2, d3); the preamble checks clean there (r2d-a), so the errors are
call-site errors at line 10. Post-slice these paths are untouched (R-30.4
defers plain-word output-only grounding; R-30.2 names member calls only) —
that non-leakage is precisely what this golden pins. Fully measured. If any of
the three moves, the implementation leaked plain-word grounding into the slice
— escalate, don't retarget silently.

## G12 — `zero_input_escape_hatch_still_fails_impl_check` (recorded-not-fixed)

Fixture: `probes/soo30_p2e_applied_instantiation_zero_input.sth`, verbatim:

```text
import: intrinsics * ;
type: Box['A] | MkBox 'A ;
trait: Applicative['F: * -> *]
  : pure ( -- 'F[i64] ) ;
;
impl: Applicative for Box
  : pure 42 MkBox ;
;
: main ( -- ) pure[Box[i64]] drop ;
```

Post-slice expected (unchanged):

```text
error: stack effect mismatch in `pure;Applicative;0;Box['T0]`
  body leaves `Box[i64]`, but the declared outputs are `Box[i64]`
exit=1
```

Receipt chain: r1 P2e measured exactly these bytes at this HEAD — the
zero-input shape bypasses `member_binds_trait_var`'s `inputs.is_empty()` arm
even pre-relaxation, so these are today-stable bytes, and the slice records the
defect without fixing it (r2e root cause: `check_poly_body`'s residual
comparison at src/check/poly.rs:905 is syntactic `PolyType` `PartialEq`; body
side mints `Concrete` via `poly_construct_generic`
(src/check/poly/construction.rs:587-613) vs declared `Generic` via
`ground_member_poly`'s App arm (src/ast.rs:2409-2477, splice :2464); the two
render identically through poly.rs:5912/:5980-5990 but compare unequal). The
true shape escapes because its output argument is a Var (r2e). Fully measured;
the identical-rendering collision is pinned AS-IS — fixing the render is a
separate defect ticket, not this slice.

## G13 — `double_wrap_operand_idiom_survives` (extension; the pre-slice working route)

Intent: the only measured-working mono dispatch route before R-30.2 (operand
carries the target head; instantiation args positional over the dissolved
word's vars) must either keep working or move consciously. Fixture:
`probes/soo30r2_b3b_dispatch_operand.sth`, verbatim:

```text
import: intrinsics * ;
import: hosted::show | . | ;
type: Box['A] | MkBox 'A ;
trait: Applicative['F: * -> *]
  : pure ( 'A -- 'F['A] ) ;
;
impl: Applicative for Box
  : pure MkBox ;
;
: showbox ( Box[i64] -- ) ~[ ( MkBox ) MkBox> . ] Box? ;
: showbox2 ( Box[Box[i64]] -- ) ~[ ( MkBox ) MkBox> showbox ] Box? ;
: main ( -- ) 42 MkBox pure[Box[i64] Box[i64]] showbox2 ;
```

Post-slice expected (conservative — unchanged): build clean, run exit 0,
stdout `42\n`.

Receipt chain: r2b-3b measured exactly this under the R-30.1 patch (build
clean, `42\n`, exit 0). **UNMEASURED-AT-RISK:** the same open bit as G8 — if
R-30.2's output-App unification intercepts ALL explicit instantiations, a
2-arg supply against the 1-arity output App could take a new arity/mismatch
error instead of the positional route. Round 3 measures; a flip here is a
ruling consequence to record, and G8's outcome predicts it (same mechanism
question).

## Test placement

**Unit level — src/check/declarations.rs `mod tests`** (beside the stage
function `member_binds_trait_var` :408; helper `trait_check_src` :3842):

- G10.1's flip: `check_trait_decls_accepts_member_with_only_output_trait_var`
  (success assertion) + the direct predicate unit for the new output arm.
- G10.3's unit twin:
  `check_trait_decls_rejects_member_mentioning_the_trait_var_nowhere`.
- Naming per CLAUDE.md (`thing_condition_expected`); happy path + error case
  for the new gate arm, error text asserted with the `contains` pairs the
  existing gate units use (:3932-3962 style, including the
  `!err.contains("note:")` conditional-note assert where applicable).

**Golden level — new tests/phase7b_slice15.rs**, harness helpers copied from
tests/phase7b_slice2.rs (`sooth_build` :42, `build_ok` :50, `build_error` :61,
`build_and_run` :69, `single_file_hosted` :92, `single_file` :108):

- `single_file` + `build_ok`: G1.
- `single_file` + `build_error` with byte-exact `contains` asserts (full-line,
  slice2 :144 style, including the distinguishing negative asserts): G6 (line 1
  and line-2 prefix byte-exact; example-token arity asserted separately per the
  at-risk note), G8, G11.a/b/c, G9, G12, G7.
- `single_file_hosted` + `build_and_run`: G2, G3, G4, G5 — the fixture src
  keeps its `core::*` import lines but DROPS its `import: intrinsics * ;` and
  `import: hosted::show | . | ;` lines (the harness prepends both; duplicate
  imports collide — slice13 header note). G7 uses `single_file_hosted` +
  `build_error` the same way.
- `single_file` + `build_and_run`: G13.
- Moved pins live in their EXISTING files: G10.1 in declarations.rs, G10.2/3 in
  tests/phase7b_slice2.rs. `tests/phase7b_slice15.rs` holds only new fixtures.
- Standalone/probe runs of core-importing fixtures need
  `--manifest tests/fixtures/sooth.pkg` (r1 P3 invocation receipt); the harness
  needs no flag (its temp `sooth.pkg` IS the manifest, slice2 :92-106).

## Facts the spec must carry

1. Round-2 bytes are post-slice bytes for every path R-30.2/R-30.3 don't
   touch — the r2 patch IS R-30.1, applied exactly (r2 findings, "The patch").
2. **Patch-vs-ruling delta on the output arm:** the r2 patch reuses
   `dispatchable_head` over outputs, which admits a BARE `Var(0)` output too
   (e.g. member `( 'T -- 'F )`), while R-30.1's text says "an App headed by the
   trait var". The spec must either tighten the predicate to App-only or widen
   the ruling text. No probe ran a bare-output member under the patch —
   UNMEASURED; round 3: one fixture. The goldens pin only the App-headed case
   (as ruled).
3. The two open mechanism bits R-30.2 leaves to round 3: the corrected remedy
   example's rendering (G6) and whether positional supplies keep the
   operand-driven selection path (G8/G13). G8's outcome predicts G13's.
4. G2-G4 presuppose the lib landing: lib/core/applicative.sth (true-shape
   trait + `export: Applicative ;`), co-located impls (`pure Some` /
   `pure Ok` / `pure Nil ^ Cons`), `applicative` before `option` in the pkg
   module list, each impl module headed with the applicative import — the
   r2c scaffold shape, all measured checking clean.
5. ap stays fenced (G9): the shipped trait declares pure only; the companion's
   separate gating is untouched.
6. Repure's body correction (G5 mechanism note): the ticket sketch's
   `swap pure` is wrong for the `( 'F['A] 'A -- 'F['A] )` row; `swap drop pure`
   is the minimal fixing edit and composes two measured receipts.
7. Diagnostics gaps recorded, out of scope: the declaration-gate error carries
   no file path when it fires inside an imported lib module (r1 P3min), and
   G12's mismatch renders two distinct types identically (r2e).
8. The relaxation fires at `trait:` time, before impl checking and before any
   call-site logic (r1 P1/P2) — so no G2-G8 expectation can exist today; every
   "today" line in this document is the pre-slice refusal these goldens replace.

## Unmeasured-at-risk register (what round 3 measures)

| Golden | Risk | Round-3 measurement |
| --- | --- | --- |
| G2, G3, G4 | R-30.2 spelling success bytes | build exit 0; stdout `5\n` each |
| G5 | repure run bytes (bound-supplied + operand-grounded composition) | build exit 0; stdout `7\n7\n` |
| G6 | corrected example token (positional vs output-App rendering) | live remedy line bytes |
| G8 | positional supplies: selection error vs output-keyed success | which side of the R-30.2 keying |
| G13 | double-wrap idiom survival under R-30.2 | build + run `42\n` or the new error |
| G10.3 | negative-shape refusals under the patch (derived, not run) | two trivial fixture builds |
