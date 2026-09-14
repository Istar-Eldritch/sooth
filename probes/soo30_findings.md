# SOO-30 probe findings — Applicative.pure grounding (pre-spec measurement)

Per-probe verdicts. Format: PREDICTION / RESULT (CONFIRMED|REFUTED|NEW) / exact
bytes / citations with grep -n receipts. HEAD `846f952`, worktree clean at start.

## P1 — decl+impl check, fixture-local (`probes/soo30_p1_decl_impl.sth`)

PREDICTION: trait decl + ctor-keyed impl both check (callee-side App dissolution + S8b construction arm on a bare-var ctor field).

RESULT: **REFUTED at (a); (b) masked, then CONFIRMED via supplementary P1b.**

(a) The trait declaration does NOT check as written. `pure ( 'A -- 'F['A] )` is a
declaration-time located error, byte-identical to the pinned S2-15.a golden:

```text
error: trait member `pure` of `Applicative` (line 4, col 5) has no input for a call to dispatch on (expected the trait's variable `'F` bare or heading an application like `'F['T]`)
exit=1
```

Mechanism: `member_binds_trait_var` (src/check/declarations.rs:412-418) — a member
whose inputs are nonempty must mention the trait var in a *dispatchable input head*
(`Var(0)` or `App { head: 0 }`); only zero-input members bypass the gate
(`empty ( -- 'T )` passes that way). Error text: src/check/declarations.rs:463.
Pre-existing pinned golden of the identical shape: `hkt_member_without_dispatchable_input_is_located_error`,
tests/phase7b_slice2.rs:145 (member `pick ( 'T -- 'F['T] )` under `Functor['F: * -> *]`).

(b) The impl question is masked by (a) — the gate fires at `trait:` time, before
`impl:` is reached. Measured independently in `probes/soo30_p1b_impl_body_ctor_word.sth`
(supplementary, dummy dispatchable input `: pure ( 'F['A] 'A -- 'F['A] ) ;`, body
`swap drop MkBox ;`): **builds clean, exit=0**. The member word synthesizes as
`pure;Applicative;0;Box['T0]`, and the checker renders the declared output
dissolved to `Box['ctor0]` (seen in attempt 1's mismatch bytes) — so ground_member_poly's
App dissolution on the output plus the ctor-word body unification both work today.
The dispatchability gate is the ONLY blocker for the ticket shape.

Fixture-shape receipts for the record: single-line `impl: ... : member ... ;` is not
grammar (member bodies are word-defs `: name body ;`, src/parser.rs:4256-4297), and a
`trait:` member list closes with one final `;` (src/parser.rs:4089-4091, loop breaks on
`;` then expects it).

## P1b — supplementary (recorded above)

The synthesized member word name for a ctor-keyed impl is
`pure;Applicative;0;Box['T0]` — matches the documented
`member;Trait;module;Type` scheme (lib/core/iterator.sth's export note,
grep -n "synth_member_word_name" src/parser.rs).

## P2 — mono-site grounding of 'F (the central question)

PREDICTION (p2a): `pure[Box]` parses (bare ctor as instantiation argument), type-checks, dispatches.
PREDICTION (p2b): empty-style located error citing a `pure[...]` remedy.
PREDICTION (p2c): kind error ('F is *->*, Box[i64] is *).
PREDICTION (p2d): refusal, no consuming-context inference (empty precedent).

RESULT: **all four REFUTED-BLOCKED — the P1 declaration gate fires before any call-site
grounding logic can run.** The central question is unreachable via the ticket shape today.

- p2a (`42 pure[Box] drop`) — NEW, parse-level: the bare-ctor instantiation hypothesis is
  REFUTED with its own named remedy, before checking even starts (parse precedes the
  declaration gate):

```text
error: generic type `Box` declares 1 type variable, but none were supplied at line 9, col 23 (apply it as `Box[T]`, one type argument per declared variable)
  note: a glued bracket is an explicit type instantiation; insert a space for a quotation or array literal
exit=1
```

  So an instantiation argument must be an *applied* type; a bare constructor head is not
  admitted (contrast the Monoid precedent, where the argument is a concrete type `i64`).

- p2b/p2c/p2d — byte-identical to P1(a)'s gate error (see P1). `pure[Box[i64]]` PARSES
  (no p2a-style parse error), so applied instantiation arguments are grammatical; the
  declaration gate then fires. p2d's consuming context is likewise never reached.
- Empty-precedent comparison: p2b's error is NOT the empty-style remedy error
  (`bare_nullary_member_without_instantiation_is_located_error`, tests/phase7b_slice6.rs:269-290
  cites `empty[i64]`) — the S2-15.a declaration gate shadows it entirely. The
  `pure[...]`-remedy error shape predicted for the spec does not exist today.

Supplementary P2e (`probes/soo30_p2e_applied_instantiation_zero_input.sth`): zero-input
members bypass `member_binds_trait_var` (src/check/declarations.rs:412-418,
`inputs.is_empty()` arm), so `: pure ( -- 'F[i64] ) ;` declares — but the impl body
check then fails with **identical renderings on both sides**:

```text
error: stack effect mismatch in `pure;Applicative;0;Box['T0]`
  body leaves `Box[i64]`, but the declared outputs are `Box[i64]`
exit=1
```

Two types that render as `Box[i64]` are unequal (the dissolved declared output vs the
ctor word's output differ structurally — a rendering-collision in the mismatch path).
So even the gate-bypassing zero-input shape has NO working impl today. Net: **there is
currently no declarable, implementable member shape for output-only-'F construction**;
the SOO-30 blocker is the S2-15.a declaration gate plus this impl-side mismatch, not
mono-call-site grounding (which nothing can reach).

## P3 — real lib types (temporary lib/ scaffolding, REVERTED)

PREDICTION: per-ctor impls check; `5 pure[Option]`-style mono calls ground/dispatch end-to-end.

RESULT: **REFUTED as specified; per-ctor impls + end-to-end dispatch CONFIRMED on the
P1b-proven dummy-input shape (supplementary P3b).**

- As specified (ticket shape in lib/core/applicative.sth): the fixture dies in PARSE at
  the first `pure[Option]` (line 19, col 10) with the same p2a-class error — bare ctor
  as instantiation argument refused, remedy `Option[T]`. Parse precedes module checking,
  so the lib-side gate is masked. `probes/soo30_p3min_libgate.sth` (clean-parse main)
  exposes it: byte-identical S2-15.a gate error citing `line 4, col 5` — but NOTE: no
  file path for lib/core/applicative.sth appears in the message (the import error in the
  same run DID cite its file). A cross-module declaration error with no module citation
  is a diagnostics gap worth flagging to the spec.
- Invocation receipt: core-importing probes need
  `--manifest tests/fixtures/sooth.pkg` (recorded in s12_baseline.md's s12_h entry;
  without it: the anonymous-package import error, byte-recorded).
- Supplementary P3b (`probes/soo30_p3b_dispatch_operands.sth`, lib member
  `: pure ( 'F['A] 'A -- 'F['A] ) ;`, impl bodies `swap drop Some` / `swap drop Ok` /
  `swap drop Nil ^ Cons`): **builds clean (exit=0) and RUNS: stdout `5\n5\n5\n`, exit=0.**
  - Each per-ctor impl checks, co-located in the target type's own module
    (option.sth/result.sth/list.sth) with the trait imported from applicative.sth — the
    orphan rule's target-module arm (src/check/declarations.rs:468+ comment) admits this.
  - End-to-end dispatch confirmed per constructor: Option output dispatches Some,
    Result output dispatches Ok, List output is a real `Cons`-constructed cell
    (`showlist` walked it).
  - **2-arity Result grounds**: operand `9 Err` (Result[i64 i64]) grounds 'F at the
    mono call site and the output is `Ok 5` — partially-applied ctor heads work on the
    operand-carrying path.
  - Bare `pure` (no `[...]` instantiation) grounds at these mono call sites: all member
    vars ground from operands ('F from the dummy operand, 'A from the value operand).
    Contrast map's golden which instantiates `map[i64 i64]` — needed only because a
    quotation's vars cannot ground from operands (tests/phase7b_slice6.rs:498).
  - Scaffolding receipts: lib modules need their own `import: intrinsics | swap drop | ;`
    for impl bodies (option.sth et al. import nothing by default); pkg entry added as
    `applicative` before `option` in lib/core/sooth.pkg's module list. All reverted
    (`git checkout -- lib/ && rm lib/core/applicative.sth`); final tree shows probes/
    additions only.

## P4a — shared-bound consumer

PREDICTION: (a) bare `pure` inside the poly body checks (mconcat/empty precedent); (b) mono site `twin[Box]` grounds (ctor-instantiation one level up).

RESULT: **(b) REFUTED at parse; (a) unreachable (masked twice over).**

- `twin[Box]` is REFUTED by the same p2a-class parse rejection, now for a plain word
  (not a trait member): "generic type `Box` declares 1 type variable, but none were
  supplied at line 10, col 24 (apply it as `Box[T]` ...)" — so the bare-ctor
  instantiation-argument refusal is word-general, not member-specific.
- The bound-bracket predicate syntax itself parses: `: twin['F: Applicative 'A] ( ... ) ;`
  passed parse (the error is line 10), so `'F: Applicative` word bounds are grammatical
  today.
- (a) bare `pure` in the poly body is unreachable: the ticket-shaped member can't
  declare (P1 gate), and the P1b-proven dummy-input member needs an 'F operand the
  specified `twin` row doesn't carry — so the mconcat precedent (bare member call
  supplied by the word's own bound, tests/phase7b_slice6.rs:347) cannot be exercised
  for `pure`'s shape today.

## P5 — ap scope pin

PREDICTION: does the trait decl parse; does the pure-only impl fail member coverage or does the quotation audit fire at impl declaration?

RESULT: **REFUTED earlier than both — ap's DECLARATION is parse-fenced.**

```text
error: expected a type, found `[` at line 5, col 14 (a type application's arguments are types, not quotations)
exit=1
```

The fence is the S1-6 parse-time rejection of a quotation-shaped application argument
(src/parser.rs:2882; slice1-brief.md:124 "fences a quotation-shaped argument inside an
application"). So the coverage-vs-audit ordering question is unanswerable and moot:
`ap ( 'F[ [ 'A -- 'B ] ] 'F['A] -- 'F['B] )` cannot appear in any trait declaration
today. This independently forces the slice's "shipped trait declares pure ONLY"
decision — and combined with P1, today NEITHER Applicative member is declarable in
ticket shape (pure: S2-15.a declaration gate; ap: S1-6 parse fence). The slice7 brief's
final ruling already recorded the impl-side audit fence (audit_poly_input_quotation's
Generic arm at src/check/audits.rs:431 into reject_poly_quotation_anywhere at :484)
for the lifted-fence variant; that audit is unreachable behind the parse fence as
spelled today.

## Net answer for SOO-30 (what works TODAY, measured)

1. `pure ( 'A -- 'F['A] )` is UNDECLARABLE: S2-15.a declaration gate
   (`member_binds_trait_var`, src/check/declarations.rs:412-418/:463; pinned golden
   tests/phase7b_slice2.rs:145). Only zero-input members or members with an
   'F-headed input pass.
2. `pure[Box]` (bare ctor instantiation argument) is UNPARSEABLE, word-general
   (members P2a line 19/P3 line 19; plain words P4a line 10), remedy text names
   `Box[T]`-style applied arguments. `pure[Box[i64]]` parses.
3. The callee side WORKS: a gate-passing member with an 'F-headed input checks its
   ctor-word body against the dissolved App output with 'F:=Box (P1b exit=0), and on
   real lib types all three ctor impls (Option/Result/List) check and dispatch
   end-to-end with operand-carrying mono calls (P3b: build exit=0, run `5/5/5`,
   exit=0) — including 2-arity Result grounding 'F from a `Result[i64 i64]` operand,
   and bare (uninstantiated) mono member calls when operands ground every var.
4. The impl-side mismatch for the zero-input escape hatch (P2e: identical-rendering
   `Box[i64]` vs `Box[i64]` rejected) is a real checker inconsistency blocking the
   only gate-bypassing pure-like shape.
5. `ap` is UNDECLARABLE at parse (S1-6 fence, src/parser.rs:2882), so the shipped
   trait declares pure only by necessity; the coverage-vs-audit ordering question is
   moot.
6. Spec-relevant diagnostics gaps observed: the declaration-gate error carries no
   file path when it fires inside an imported lib module (P3min), and P2e's mismatch
   renders two distinct types identically.

Handed-down citation re-measurements (grep -n receipts):
tests/phase7b_slice6.rs:269 (bare-empty remedy comment), :347 (mconcat poly-body
bare member precedent), src/check/audits.rs:431 and :484 (quotation audit arms) —
all confirmed at HEAD 846f952.
