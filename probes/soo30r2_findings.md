# SOO-30 probe round 2 — relaxed-gate world (Applicative.pure)

Per-probe verdicts under the locally-patched (temporary) S2-15.a gate. Format:
PREDICTION / RESULT (CONFIRMED|REFUTED|NEW) / exact bytes / grep -n receipts.
Round 1 findings: `probes/soo30_findings.md`; round-1 bytes: `probes/soo30_baseline.md`.
Round-2 bytes: `probes/soo30r2_baseline.md`. HEAD `846f952` + the patch below.

## The patch (exact diff, also saved at /tmp/soo30_r2_gate.patch)

`member_binds_trait_var` (src/check/declarations.rs:408): mirrored the input arm's
`dispatchable_head` check over the member's outputs, reusing the same predicate
(including the `&`-unwrapping courtesy). Nothing else touched.

```diff
diff --git a/src/check/declarations.rs b/src/check/declarations.rs
index a4704ca..90ed7eb 100644
--- a/src/check/declarations.rs
+++ b/src/check/declarations.rs
@@ -411,6 +411,10 @@ fn member_binds_trait_var(member: &TraitMember) -> bool {
             PolyType::Ref(referent, _) => dispatchable_head(referent),
             other => dispatchable_head(other),
         })
+        || member.sig.outputs.iter().any(|output| match output {
+            PolyType::Ref(referent, _) => dispatchable_head(referent),
+            other => dispatchable_head(other),
+        })
 }
```

`cargo build`: Finished dev profile, no warnings. (Fixture shapes reuse round 1's
known-good grammar — trait member lists are multi-line `: name ( sig ) ;` blocks
closed by a final `;`; the task sketch's single-line form is not grammar.)

## R2a — decl+impl under relaxed gate (`probes/soo30r2_a_decl_impl.sth`)

PREDICTION: trait decl passes the relaxed gate; impl check is the open risk
(P2e-class identical-renderings mismatch possible).

RESULT: **CONFIRMED, and the P2e wall does NOT fire on the true shape.**

- The trait decl `pure ( 'A -- 'F['A] )` now passes the gate (round 1: the
  S2-15.a error at decl time, `probes/soo30_baseline.md` P1).
- The ctor-keyed impl `: pure MkBox ;` **checks clean — build exit=0, no
  stderr**. No `body leaves ... but the declared outputs are ...` mismatch:
  with a nonempty input ('A), the body's `MkBox` output `Box['A]` unifies
  against the dissolved declared output `Box[<impl var>]` via the input var,
  the same path that made P1b pass. The P2e mismatch is specific to the
  zero-input shape (arg coming from the sig literal, no operand to unify
  through) — diagnosed structurally in R2e below.

Bytes: `probes/soo30r2_baseline.md` § soo30r2_a_decl_impl.sth.

## R2b — mono call-site grounding (fixture-local; preamble = R2a's decl+impl)

### R2b-1 `42 pure[Box[i64]] drop` (`probes/soo30r2_b1_applied_instantiation.sth`)

PREDICTION (under test): the applied instantiation argument unifies against the
member's output App (`'F['A] := Box[i64]` → `'F:=Box`, `'A:=i64`) and the call
type-checks.

RESULT: **REFUTED — NEW arity wall on the dissolved member word.**

```text
error: `pure;Applicative;0;Box['T0]` (line 9) declares 2 type variables (`'ctor0`, `'A`) but was given 1 type argument
exit=1
```

Mechanism: the call resolves to the per-impl member word
`pure;Applicative;0;Box['T0]`, whose signature is the DISSOLVED
`( 'A -- Box['ctor0] )` — 'F is gone (baked into the word as the impl target
head), leaving the word generic over the impl target's element var `'ctor0` AND
the member input var `'A`. The instantiation argument list is checked
positionally against THAT var list, so `Box[i64]` (1 arg) under-supplies a
2-var word. There is no `'F['A] := Box[i64]` unification path at the call; the
trait var never survives to the call site.

### R2b-2 `42 pure drop` (`probes/soo30r2_b2_bare.sth`)

PREDICTION (under test): the empty-style remedy error naming a `pure[...]` fix.

RESULT: **CONFIRMED — a NEW error shape (not the round-1 gate error), the
no-dispatch-operand remedy error.**

```text
error: `pure` in `main` (line 9, col 18) is a trait member with no operand to dispatch on
  a monomorphic body cannot infer the trait's type here; write an explicit type argument, e.g. `pure[i64]`
exit=1
```

The operand `42 : i64` grounds `'A` but nothing grounds `'F` (the true shape has
no 'F-carrying input), so impl selection fails. The remedy text names
`pure[i64]` — a ONE-arg example. Tension with R2b-1: a 1-arg instantiation on
this member word hits the 2-vars-1-arg arity error, so the remedy as rendered is
unachievable for this member shape (follow-up R2b-1c tests the 2-arg form).

Bytes: `probes/soo30r2_baseline.md` §§ b1, b2.

### R2b-1c follow-up `42 pure[i64 i64] drop` (`probes/soo30r2_b1c_two_args.sth`)

RESULT: **NEW — past the arity wall, into impl selection.** The 2-arg list
satisfies the dissolved word's `('ctor0, 'A)` var list positionally, then impl
selection fails operand-driven:

```text
error: `pure` in `main` (line 9, col 18) is a trait member of Applicative, but no `impl:` in this program dispatches on these operands
  the operand types here are `i64`; declare an impl of one of those traits for the operand's type, or import a word that claims this name
exit=1
```

So instantiation args ground the member word's residual vars, but impl SELECTION
stays operand-driven: with no 'F-carrying input, the operand (`i64`) never
mentions the target (`Box`), and no instantiation form fixes that — `Box[i64]`
is the wrong arity (R2b-1), and `i64` names no impl (this probe).

### R2b-3 dispatch (`probes/soo30r2_b3_dispatch.sth` specified; `probes/soo30r2_b3b_dispatch_operand.sth` adapted)

PREDICTION (under test): `pure[Box[i64]]` dispatches to the Box impl.

RESULT: **REFUTED as specified; CONFIRMED via the operand-carrying adaptation.**

- Specified form (`42 pure[Box[i64]] showbox`): byte-identical R2b-1 arity error
  (line 11 with the show import). The specified call shape can never dispatch.
- Adapted form — operand carries the target head (double-wrap idiom):
  `: main ( -- ) 42 MkBox pure[Box[i64] Box[i64]] showbox2 ;` (instantiation
  `'ctor0:=Box[i64]`, `'A:=Box[i64]`, operand `Box[i64]` dispatches on its head)
  — **builds clean and RUNS: stdout `42\n`, exit=0.** The Box impl is selected,
  and the output survives two `MkBox>` extractions through the variant idiom.

Mechanism net for the spec: under the relaxed gate the true shape is declarable
and implementable, and it dispatches end-to-end — but ONLY through an operand
whose head names the target (for pure's shape that means the "value" operand is
itself an 'F value, i.e. double wrapping). The natural mono call
`42 pure[Box[i64]]` is unreachable: 1-arg instantiation under-supplies the
dissolved 2-var member word, and there is no 'F-grounding from the output
context. Bare `pure` gets the no-dispatch-operand remedy error whose printed
example (`pure[i64]`) is itself unachievable (arity wall, R2b-1).

Bytes: `probes/soo30r2_baseline.md` §§ b1c, b3, b3b.

## R2c — real lib types end-to-end under relaxed gate (temporary lib/ scaffolding, REVERTED)

PREDICTION (under test): build exit=0, run prints a marker per ctor; watch for
the P2e-class impl mismatch.

RESULT: **SPLIT — all three TRUE-shape ctor impls CHECK (the P2e wall does NOT
reproduce); the SPECIFIED mono calls hit the R2b-1 arity wall; the
operand-carrying adaptation builds and dispatches end-to-end (`5/5/5`, exit=0).**

- Scaffolding (all reverted): `lib/core/applicative.sth` with the true-shape
  pure-only trait + `export: Applicative ;`; `applicative` inserted before
  `option` in lib/core/sooth.pkg's module list; true-shape impls co-located —
  option.sth `impl: Applicative for Option : pure Some ;`, result.sth
  `: pure Ok ;`, list.sth `: pure Nil ^ Cons ;` (p3b's body minus the
  dummy-operand prefix), each module headed with
  `import: self::applicative | Applicative | ;`.
- **Impl check: all three ctor impls pass.** C1's only error is at the probe's
  main (line 19, the first call) — the checker had already checked the lib
  impls, including the 2-param Result (body `Ok`) and List's cell-boxing body
  (`^` interns an owned cell over the payload, src/check/word_families.rs:1195;
  List's `rest` field is `^List['T]`). No identical-renderings mismatch
  anywhere on the true shape. Round-1's P2e wall is confirmed specific to the
  zero-input member (see R2e).
- Specified calls `5 pure[Option[i64]]` etc.: byte-identical R2b-1 arity wall —
  `error: \`pure;Applicative;4;Option['T0]\` (line 19) declares 2 type variables
  (\`'ctor0\`, \`'A\`) but was given 1 type argument`, exit=1.
- Adapted calls (operand carries the target head; instantiation args are
  positional over the dissolved word's vars — target params first, then 'A):
  `5 Some pure[Option[i64] Option[i64]] showopt2`,
  `5 Ok pure[Result[i64 i64] i64 Result[i64 i64]] showres2` (note the THREE-arg
  instantiation for 2-param Result: `'ctor0`, `'T1`, `'A`),
  `5 single pure[List[i64] List[i64]] showlist2` — **build clean, run prints
  `5\n5\n5\n`, exit=0.** The Box-class finding generalizes to all real lib
  constructors: Option output dispatches Some>, Result output dispatches Ok>,
  List output is a real Cons cell walked by showlist2.
- Scaffolding side-findings (recorded for the spec's diagnostics inventory):
  a nullary ctor (`Nil`) is overload-ambiguous when its immediate consumer is
  generic (`^`) — the declared output of an enclosing word does not reach
  through, and `Nil[List[i64]]` explicit instantiation is refused (`Nil takes no
  type arguments; only a call to a polymorphic word may be explicitly
  instantiated`); the working idiom is a helper word with a declared output
  (`: nile ( -- List[i64] ) Nil ;`) consumed before `^`.
- Invocation receipt re-confirmed: core-importing probes need
  `--manifest tests/fixtures/sooth.pkg`.

Bytes: `probes/soo30r2_baseline.md` §§ c, c2 (four attempts).

## R2d — shared-bound consumer (`probes/soo30r2_d_twin.sth`, `probes/soo30r2_d2_twin_bare.sth`, `probes/soo30r2_d3_twin_hkt_arg.sth`)

PREDICTION (under test): (a) bare `pure` inside the poly body checks (mconcat
precedent needs twin itself to check first); (b) `twin[Box[i64]]` grounds twin's
own 'F at the mono site (the applied arg would have to unify against the OUTPUT
row, not substitute 'F directly).

RESULT: **(a) CONFIRMED — the HKT mconcat precedent holds. (b) REFUTED — and
stronger: NO instantiation form can ground twin's 'F today.**

- (a) `twin['F: Applicative 'A] ( 'A 'A -- 'F['A] 'F['A] ) pure swap pure ;`
  CHECKS: both errors cite line 10 (main), never the body — the word's own
  trait-bound 'F supplies the bare `pure` calls inside the body, exactly the
  mconcat precedent (tests/phase7b_slice6.rs:347), now proven for a `* -> *`
  bound.
- (b1) `1 2 twin[Box[i64]] drop drop`: the SAME arity wall as R2b-1, now for a
  plain word — `error: \`twin\` (line 10) declares 2 type variables (\`'A\`,
  \`'F\`) but was given 1 type argument`, exit=1. The applied argument is checked
  positionally against the word's full var list; there is no
  unify-against-the-output-row path.
- (b2) bare `1 2 twin drop drop`: a NEW error shape —
  `error: \`twin\` in \`main\` (line 10) has output variable \`'F\` that no input
  binds` / `note: supply it explicitly: \`twin[SomeType SomeType]\``, exit=1.
  This is the output-only-var diagnostic (contrast the member-specific
  no-dispatch-operand error of R2b-2), and its remedy correctly shows the
  TWO-arg form the word's arity demands.
- (b3) the remedy is nonetheless UNACHIEVABLE for an HKT var: `1 2 twin[i64 Box]
  drop drop` dies at PARSE with the p2a-class refusal — `error: generic type
  \`Box\` declares 1 type variable, but none were supplied at line 10, col 28
  (apply it as \`Box[T] ...\`), exit=1. A `* -> *` var position can only be named
  by a bare constructor head, and bare constructor heads are refused as
  instantiation arguments (round 1 P2a/P4a), kind-unaware. So a word whose 'F
  appears only in outputs (twin, and any Applicative-consuming combinator with
  pure-only shape) declares and body-checks but can NEVER be called at a mono
  site: the output-row grounding mechanism the spec needs does not exist yet.

Bytes: `probes/soo30r2_baseline.md` §§ d, d2, d3.

## R2e — P2e mismatch diagnosis (source reading; round-1 bytes: `probes/soo30_baseline.md` § p2e)

**Root cause: the impl-body residual comparison is syntactic `PartialEq` on
`PolyType`, and the two sides of the zero-input shape are DIFFERENT VARIANTS
that render identically — `Concrete(interned instantiation)` on the body side
vs `Generic{...}` on the declared side.**

- The check: `check_poly_body` ends with `if residual_pt != sig.outputs` at
  src/check/poly.rs:905 (error built by `poly_output_mismatch_error`,
  src/check/poly.rs:5646-5660), and `PolyType` derives plain structural
  `PartialEq, Eq` (src/ast.rs:2651) — no unification, no normalization.
- Declared side: the impl member's output `'F[i64]` grounds through
  `ground_member_poly`'s App-dissolve arm (src/ast.rs:2409-2477), which for a
  trait-var-headed application ALWAYS returns `PolyType::Generic { name: "Box",
  args: [Concrete(i64)], .. }` (the splice at src/ast.rs:2464).
- Body side: the ctor call `MkBox` goes through `poly_construct_generic`
  (src/check/poly/construction.rs:443). When every construction argument is
  CONCRETE it takes the minting path — `PolyType::Concrete(instantiate_struct(..))`
  (src/check/poly/construction.rs:587-613), and `instantiate_struct`
  (src/ast.rs:1368) interns the instantiation with a cached display name
  `Box[i64]` (`type_instantiation_name`, src/ast.rs:836). When any argument is
  still a variable it takes the SYMBOLIC path — `PolyType::Generic { args:
  resolved, .. }` (same match, else arm).
- Renderings collide: `poly_type_str` renders the Concrete arm via
  `t.name()` (src/check/poly.rs:5912 — the cached "Box[i64]") and the Generic
  arm via `format!("{name}[{}]", ..)` (src/check/poly.rs:5980-5990) — both
  "Box[i64]", while `Concrete(Type::Struct(..)) != Generic{..}` as variants.
- Why the TRUE shape (R2a/R2c) escapes: its output argument is the member's own
  input var `'A` — identified with the impl target's slot variable in the union
  id space (S2-5), so at impl-check time it is a `Var`, not `Concrete`. The ctor
  call therefore takes the SYMBOLIC path and produces `Generic{Box, [Var]}`,
  which equals the declared `Generic{Box, [Var]}` exactly. The mismatch fires
  only when the output argument is a sig-literal concrete type with NO input var
  to unify through — i.e. the zero-input escape-hatch shape alone.

Verdict for the spec: the relaxed-gate impl check on the true shape does NOT hit
this wall (R2a/R2c bytes confirm). The P2e mismatch is a PRE-EXISTING defect in
`check_poly_body`'s residual comparison (normalize a fully-concrete
`Generic` to its interned instantiation — or compare kind-aware renderings —
before the `!=`), reachable today by any zero-input member whose declared
output names a concrete instantiation of the impl target. Fixing it is
independent of the gate relaxation; leaving it recorded is defensible since the
spec's pure shape has a nonempty input.

## R2f — pinned-test inventory under the patch (before revert)

`cargo test --no-fail-fast` (all targets): **exactly TWO failures, both pins of
the same gate rule in the same `pick ( 'T -- 'F['T] )` shape — nothing else
flipped.** Prediction CONFIRMED (the slice2 golden flips); no other moves.

1. `check::declarations::tests::check_trait_decls_rejects_member_with_no_dispatchable_input`
   (unit, src/check/declarations.rs:3937): the gate's own unit pin. Panics at
   src/check/declarations.rs:3942 — `unwrap_err()` on an `Ok(())`: the member
   `pick ( 'T -- 'F['T] )` under `Functor['F: * -> *]` now DECLARES, so the
   expected "has no input for a call to dispatch on" error never fires.
2. `hkt_member_without_dispatchable_input_is_located_error`
   (tests/phase7b_slice2.rs:145; assertion macro at :63): the round-1-pinned
   golden. The fixture (same `pick` shape, member at line 4 col 5) builds
   clean, so the verbatim S2-15.a error-text assert fails. This is precisely
   the pin the spec must MOVE (or re-word as a fixture under a lifted gate).

Everything else passes: unittests 2052 passed / 1 failed, src/main 13 ok,
all tests/ targets ok (phase0 202, phase7b_slice2 16 passed/1 failed, and every
other phase suite green). The relaxation's blast radius is exactly the two
gate-pins — no downstream pins (impl checks, dispatch, diagnostics) moved.

## Round-2 net answer for the spec

1. The relaxed gate (output-dispatchable arm) admits the true `pure
   ( 'A -- 'F['A] )` shape; the ctor-keyed impl checks — including all real
   lib ctors (Some/Ok/`Nil ^ Cons`), 2-param Result included. The P2e
   identical-renderings mismatch does NOT fire on the true shape (R2a/R2c;
   root-caused in R2e as a pre-existing `Concrete`-vs-`Generic` variant
   inequality in check_poly_body's residual comparison, poly.rs:905).
2. Mono-site grounding is the REAL blocker now, on two independent walls:
   (i) the dissolved member word carries the target's vars + 'A, so
   `pure[Box[i64]]` (1 arg) under-supplies a 2-var word — arity wall
   (R2b-1, generalized in R2c); (ii) impl selection is operand-head-driven,
   so with no 'F-carrying input no instantiation form selects the impl
   (R2b-1c). The ONLY working mono call is the double-wrap idiom: an operand
   that itself carries the target head (`42 MkBox pure[Box[i64] Box[i64]]`),
   which type-checks, dispatches, and runs (R2b-3b: `42`; R2c: `5/5/5`).
3. Bare `pure` at a mono site produces the no-dispatch-operand remedy error
   (`write an explicit type argument, e.g. \`pure[i64]\``) whose example is
   unachievable for this shape — a remedy-text defect to spec (R2b-2).
4. Shared-bound consumers work at DECLARATION (twin's HKT mconcat precedent
   holds: the word's own `'F: Applicative` bound supplies bare `pure` in the
   body) but are un-callable at mono sites: the plain-word arity wall
   (R2d-1), the new "output variable that no input binds" remedy error
   (R2d-2), and the parse-level refusal of bare ctor heads in instantiation
   arguments (R2d-3) close every route. Output-only-'F grounding does not
   exist at call sites in any form today.
5. The relaxation flips exactly the two gate-pins (unit + slice2 golden);
   no other pins move (R2f).
