# Non-regression baseline: SOO-30 probe round 3 (measure-then-pin, pre-Phase-2-goldens)

Byte-exact `cargo run -q -- build probes/<name>.sth` stderr + exit per fixture,
run from the repo root at HEAD `bc702fb` (Phase 1 gate landed) + the round-3
temporary output-App-route patch (see `probes/soo30r3_findings.md`; the patch is
reverted after the round). Format follows `probes/soo30r2_baseline.md`. All
probes take `--manifest tests/fixtures/sooth.pkg` (round-2 receipt; the
fixture-local probes import `hosted::show`, which needs a manifest too).

### probes/soo30r3_g14.sth (R3-G14, THE primary target: main = `42 pure[Box[i64]] showbox`, fixture-local Box)

(build clean; no stderr)
exit=0
run: ./probes/soo30r3_g14 → stdout `42\n`, exit=0

### probes/soo30r3_g6.sth (R3-G6: main = `42 pure drop` — remedy with the corrected example)

error: `pure` in `main` (line 13, col 18) is a trait member with no operand to dispatch on
  a monomorphic body cannot infer the trait's type here; write an explicit type argument, e.g. `pure[Box[i64]]`
exit=1

(r2b-2 line-1 template: unchanged — same wording, span located at this fixture's
own line/col; r2b-2's example `pure[i64]` is replaced by the achievable
output-App spelling `pure[Box[i64]]`, which R3-G14 proves builds and dispatches.)

### probes/soo30r3_g8.sth (R3-G8: main = `42 pure[i64 i64] drop` — 2-arg positional)

error: `pure` in `main` (line 13, col 18) is a trait member of Applicative, but no `impl:` in this program dispatches on these operands
  the operand types here are `i64`; declare an impl of one of those traits for the operand's type, or import a word that claims this name
exit=1

(r2b-1c template byte-identical — same wording; span located at this fixture's
own line/col. No ESCALATE: the conservative prediction held.)

### probes/soo30r3_g13.sth (R3-G13: main = `42 MkBox pure[Box[i64] Box[i64]] showbox2` — double-wrap idiom)

(build clean; no stderr)
exit=0
run: ./probes/soo30r3_g13 → stdout `42\n`, exit=0

(r2b-3b bytes identical: build clean, run `42\n` exit=0. No ESCALATE.)

### probes/soo30r3_g2.sth (R3-G2: main = `5 pure[Option[i64]] showopt`; scaffolding per r2c receipt)

(build clean; no stderr)
exit=0
run: ./probes/soo30r3_g2 → stdout `5\n`, exit=0

### probes/soo30r3_g3.sth (R3-G3: main = `5 pure[Result[i64 i64]] showres` — 2-param Result, 1-arg spelling)

(build clean; no stderr)
exit=0
run: ./probes/soo30r3_g3 → stdout `5\n`, exit=0

(2-param Result grounds through the 1-arg output-App spelling: the seed binds
both ctor params (`'ctor0 := i64`, `'ctor1 := i64`) from the instantiation's
ctor arguments and the residual `'A` (union id 2) from the output App's single
argument. No ESCALATE.)

### probes/soo30r3_g4.sth (R3-G4: main = `5 pure[List[i64]] showlist` — real Cons cell)

(build clean; no stderr)
exit=0
run: ./probes/soo30r3_g4 → stdout `5\n`, exit=0

### probes/soo30r3_g5.sth (R3-G5: `: repure['F: Applicative 'A] ( 'F['A] 'A -- 'F['A] ) swap drop pure ;`, mains `5 Some 7 repure showopt nile 7 repure showlist`)

(build clean; no stderr)
exit=0
run: ./probes/soo30r3_g5 → stdout `7\n7\n`, exit=0

(Intermediate attempt recorded honestly: the first G5 fixture led each site
with an unconsumed `5` literal before `nile` — `5 nile 7 repure showlist`
failed with `stack effect mismatch in \`main\` (line 11) / body leaves 1
values, but ( … ) declares 0 outputs / note: declared ( -- )`, exit=1. That
was a probe-authoring bug (nile takes no operands; the literal was never
consumed), not a checker wall: Option alone (`5 Some 7 repure showopt`) built
and printed`7\n` with the identical preamble. The corrected fixture is the
one above. repure is called BARE — both vars operand-grounded, no explicit
instantiation — exercising the existing poly member-call path with 'F in an
input, exactly the R-30.4 consumer shape.)
