# Non-regression baseline: SOO-30 probe round 2 (relaxed gate)

Byte-exact `cargo run -q -- build probes/<name>.sth` stderr + exit per fixture,
run from the repo root at HEAD `846f952` + the R2 gate patch (see
`probes/soo30r2_findings.md`). Format follows `probes/soo30_baseline.md`. Core
imports use `--manifest tests/fixtures/sooth.pkg` (round-1 receipt).

### probes/soo30r2_a_decl_impl.sth (true shape `pure ( 'A -- 'F['A] )`, impl body `MkBox`)

(build clean; no stderr)
exit=0

### probes/soo30r2_b1_applied_instantiation.sth (main = `42 pure[Box[i64]] drop`)

error: `pure;Applicative;0;Box['T0]` (line 9) declares 2 type variables (`'ctor0`, `'A`) but was given 1 type argument
exit=1

### probes/soo30r2_b2_bare.sth (main = `42 pure drop`)

error: `pure` in `main` (line 9, col 18) is a trait member with no operand to dispatch on
  a monomorphic body cannot infer the trait's type here; write an explicit type argument, e.g. `pure[i64]`
exit=1

### probes/soo30r2_b1c_two_args.sth (main = `42 pure[i64 i64] drop` — 2-arg positional follow-up)

error: `pure` in `main` (line 9, col 18) is a trait member of Applicative, but no `impl:` in this program dispatches on these operands
  the operand types here are `i64`; declare an impl of one of those traits for the operand's type, or import a word that claims this name
exit=1

### probes/soo30r2_b3_dispatch.sth (specified dispatch attempt, main = `42 pure[Box[i64]] showbox`; invocation: `cargo run -q -- build probes/soo30r2_b3_dispatch.sth --manifest tests/fixtures/sooth.pkg`)

error: `pure;Applicative;0;Box['T0]` (line 11) declares 2 type variables (`'ctor0`, `'A`) but was given 1 type argument
exit=1

### probes/soo30r2_b3b_dispatch_operand.sth (adapted: operand carries the target head, main = `42 MkBox pure[Box[i64] Box[i64]] showbox2`; same manifest invocation)

(build clean; no stderr)
exit=0
run: ./probes/soo30r2_b3b_dispatch_operand → stdout `42\n`, exit=0

### probes/soo30r2_c_dispatch.sth (specified mono calls `5 pure[Option[i64]]` etc.; scaffolding: lib/core/applicative.sth true-shape trait + impls Some/Ok/`Nil ^ Cons` co-located in option/result/list, `applicative` added to sooth.pkg before option; invocation: `cargo run -q -- build probes/soo30r2_c_dispatch.sth --manifest tests/fixtures/sooth.pkg`)

error: `pure;Applicative;4;Option['T0]` (line 19) declares 2 type variables (`'ctor0`, `'A`) but was given 1 type argument
exit=1

### probes/soo30r2_c2_dispatch_operands.sth (adapted operand-carrying calls; same manifest invocation; intermediate attempts recorded below)

attempt 1 — List operand inline `5 Nil ^ Cons pure[List[i64] List[i64]] showlist2`:

error: no overload of `Nil` in `main` (line 27) accepts these operands
  candidate: no operands -> `List[List[i64]]`
  candidate: no operands -> `List[i64]`
  note: every candidate takes no operands, so only the consumer's declared type picks one; name a concrete instantiation at the consumer to resolve it
exit=1

attempt 2 — helper `: single ( i64 -- List[i64] ) Nil ^ Cons ;` (declared output still does not reach Nil through the generic `^`):

error: no overload of `Nil` in `single` (line 25) accepts these operands
  candidate: no operands -> `List[List[i64]]`
  candidate: no operands -> `List[i64]`
  note: every candidate takes no operands, so only the consumer's declared type picks one; name a concrete instantiation at the consumer to resolve it
exit=1

attempt 3 — `Nil[List[i64]]` explicit instantiation:

error: `Nil` (line 25) takes no type arguments; only a call to a polymorphic word may be explicitly instantiated
exit=1

attempt 4 (final fixture) — split so Nil's consumer is a word-end with declared output: `: nile ( -- List[i64] ) Nil ;` then `: single ( i64 -- List[i64] ) nile ^ Cons ;`; mains `5 Some pure[Option[i64] Option[i64]] showopt2` / `5 Ok pure[Result[i64 i64] i64 Result[i64 i64]] showres2` / `5 single pure[List[i64] List[i64]] showlist2`:

(build clean; no stderr)
exit=0
run: ./probes/soo30r2_c2_dispatch_operands → stdout `5\n5\n5\n`, exit=0

### lib scaffolding revert receipt

`git checkout -- lib/ && rm lib/core/applicative.sth` — run after R2c (see
findings § R2c); `git status --short` re-checked at the end of the round.

### probes/soo30r2_d_twin.sth (`: twin['F: Applicative 'A] ( 'A 'A -- 'F['A] 'F['A] ) pure swap pure ;` + main `1 2 twin[Box[i64]] drop drop`)

error: `twin` (line 10) declares 2 type variables (`'A`, `'F`) but was given 1 type argument
exit=1

### probes/soo30r2_d2_twin_bare.sth (same preamble; main = `1 2 twin drop drop`)

error: `twin` in `main` (line 10) has output variable `'F` that no input binds
  note: supply it explicitly: `twin[SomeType SomeType]`
exit=1

### probes/soo30r2_d3_twin_hkt_arg.sth (same preamble; main = `1 2 twin[i64 Box] drop drop` — can a *->* var position be named?)

error: generic type `Box` declares 1 type variable, but none were supplied at line 10, col 28 (apply it as `Box[T]`, one type argument per declared variable)
exit=1
