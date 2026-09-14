# Non-regression baseline: SOO-30 probe round (Applicative.pure grounding)

Captured 260914, HEAD `846f952` (branch `soo-30`; worktree clean at start — this
probe writes `probes/soo30_*` fixtures, this baseline, the findings file, and
temporary `lib/` scaffolding in P3 which is reverted before finishing; no `src/`
changes). Byte-exact `cargo run -q -- build probes/<name>.sth` stderr + exit code
per fixture, run from the repo root. Fixtures that need core modules copy the
import lines from `probes/s12_a_drain_fold_option_row.sth`; the rest start with
`intrinsics * ;`. Where the produced binary is also executed, its stdout/exit is
recorded under a `run:` line.

### probes/soo30_p1_decl_impl.sth (two fixture-shape attempts, then the measured bytes)

attempt 1 — single-line `impl: Applicative for Box : pure MkBox ;`:

error: trait `Applicative` declares member `impl:` without a leading `:` at line 4, col 1
  note: a trait member is declared `: impl: ( ... ) ;`, the same form as a word definition
exit=1

attempt 2 — multi-line impl, trait still on one line (member list never closed):

error: trait `Applicative` declares member `impl:` without a leading `:` at line 4, col 1
  note: a trait member is declared `: impl: ( ... ) ;`, the same form as a word definition
exit=1

attempt 3 (final fixture) — trait members each `: name ( sig ) ;` plus closing `;`:

error: trait member `pure` of `Applicative` (line 4, col 5) has no input for a call to dispatch on (expected the trait's variable `'F` bare or heading an application like `'F['T]`)
exit=1

### probes/soo30_p1b_impl_body_ctor_word.sth (supplementary — impl question (b) with a gate-passing dummy input)

attempt 1 — body `drop MkBox` (dropped the wrong operand):

error: stack effect mismatch in `pure;Applicative;0;Box['T0]`
  body leaves `Box[Box['ctor0]]`, but the declared outputs are `Box['ctor0]`
exit=1

attempt 2 (final fixture) — body `swap drop MkBox`:

(build clean; no stderr)
exit=0

### probes/soo30_p2a_ctor_instantiation.sth (main = `42 pure[Box] drop`)

error: generic type `Box` declares 1 type variable, but none were supplied at line 9, col 23 (apply it as `Box[T]`, one type argument per declared variable)
  note: a glued bracket is an explicit type instantiation; insert a space for a quotation or array literal
exit=1

### probes/soo30_p2b_bare.sth (main = `42 pure drop`)

error: trait member `pure` of `Applicative` (line 4, col 5) has no input for a call to dispatch on (expected the trait's variable `'F` bare or heading an application like `'F['T]`)
exit=1

### probes/soo30_p2c_applied_instantiation.sth (main = `42 pure[Box[i64]] drop`)

error: trait member `pure` of `Applicative` (line 4, col 5) has no input for a call to dispatch on (expected the trait's variable `'F` bare or heading an application like `'F['T]`)
exit=1

### probes/soo30_p2d_consuming.sth (main = `: main ( -- Box[i64] ) 42 pure ;`)

error: trait member `pure` of `Applicative` (line 4, col 5) has no input for a call to dispatch on (expected the trait's variable `'F` bare or heading an application like `'F['T]`)
exit=1

### probes/soo30_p2e_applied_instantiation_zero_input.sth (supplementary — p2c's question on a gate-passing member)

error: stack effect mismatch in `pure;Applicative;0;Box['T0]`
  body leaves `Box[i64]`, but the declared outputs are `Box[i64]`
exit=1

### probes/soo30_p3_dispatch.sth (real lib types; Invocation: `cargo run -q -- build probes/soo30_p3_dispatch.sth --manifest tests/fixtures/sooth.pkg` — core-importing probes need the manifest, see s12_baseline.md)

attempt 1 — without the manifest flag:

error: import `core::option` at line 5, col 1 in /root/code/ordfruma/sooth-worktrees/soo-30/probes/soo30_p3_dispatch.sth:
  /root/code/ordfruma/sooth-worktrees/soo-30/probes/soo30_p3_dispatch.sth has no ancestor `sooth.pkg` and no user-level manifest, so it is an implicit anonymous package that can only import `intrinsics` and its own quoted-path siblings
  `core` cannot be resolved; write $XDG_CONFIG_HOME/sooth/global_sooth.pkg with a `depends:` entry, add an ancestor `sooth.pkg`, or pass `--manifest <path>`
exit=1

attempt 2 (final fixture) — with `--manifest tests/fixtures/sooth.pkg`:

error: generic type `Option` declares 1 type variable, but none were supplied at line 19, col 10 (apply it as `Option[T]`, one type argument per declared variable)
  note: a glued bracket is an explicit type instantiation; insert a space for a quotation or array literal
exit=1

### probes/soo30_p3min_libgate.sth (same scaffolding, clean-parse main; same manifest invocation)

error: trait member `pure` of `Applicative` (line 4, col 5) has no input for a call to dispatch on (expected the trait's variable `'F` bare or heading an application like `'F['T]`)
exit=1

### probes/soo30_p3b_dispatch_operands.sth (supplementary — lib impls on the P1b-proven dummy-input shape; same manifest invocation)

(build clean; no stderr)
exit=0
run: ./probes/soo30_p3b_dispatch_operands → stdout `5\n5\n5\n`, exit=0

### probes/soo30_p4a_poly_body_bare.sth (`twin` + `twin[Box]` mono call)

error: generic type `Box` declares 1 type variable, but none were supplied at line 10, col 24 (apply it as `Box[T]`, one type argument per declared variable)
  note: a glued bracket is an explicit type instantiation; insert a space for a quotation or array literal
exit=1

### probes/soo30_p5_ap_in_trait.sth (trait declaring BOTH pure and ap, impl covering only pure)

error: expected a type, found `[` at line 5, col 14 (a type application's arguments are types, not quotations)
exit=1
