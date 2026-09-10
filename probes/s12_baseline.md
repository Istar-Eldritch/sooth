# Non-regression baseline: SOO-39 probe round (S12 recon)

Captured 260911, HEAD `d7559bb` (main tip; worktree `soo-39` fast-forwarded
to it, no `src/` changes). Byte-exact `cargo run -q -- build probes/<name>.sth`
stderr + exit code per fixture. Invocation: run from the repo root, no
manifest (these fixtures import `intrinsics * ;` only).

### probes/s12_a_drain_fold_option_row.sth

error: stack effect mismatch in `drain` (line 18)
  `add` needs 2 values, but the stack holds 1
  note: declared ( -- )
exit=1

### probes/s12_a2_add_holds_zero.sth

error: stack effect mismatch in `drain` (line 18)
  `add` needs 2 values, but the stack holds 0
  note: declared ( -- )
exit=1

### probes/s12_b_member_output_residual.sth

error: stack effect mismatch in `drainB`
  body leaves `i64 'It[i64] Option['?1]`, but the declared outputs are `i64 Option[i64]`
exit=1

### probes/s12_c_arm_payload_mistype.sth

error: the arms of `Option?` in `drainC` (line 18) disagree: an earlier one leaves `'E`, this one leaves `'It`
  a type variable is rigid across arms: it is never bound to the other arm's type
exit=1

### probes/s12_d_self_call_app_operand.sth

error: stack effect mismatch in `main` (line 12)
  body leaves 1 values, but ( … ) declares 0 outputs
  note: declared ( -- )
exit=1

### probes/s12_e_cross_call_app_fence.sth

error: `outer` cannot call the polymorphic word `step` (line 13, col 3)
  a higher-kinded application in a cross-called polymorphic word is not yet supported from a polymorphic body
  call `step` from a monomorphic word instead
exit=1

### probes/s12_f_id_coincidence_space.sth

error: stack effect mismatch in `drainF`
  body leaves `'T 'It['T] Option['It]`, but the declared outputs are `'T 'It['T] Option['T]`
exit=1

### probes/s12_g_member_sig_introspect.sth

error: stack effect mismatch in `drainG`
  body leaves `Option['?1] 'It[str]`, but the declared outputs are ``
exit=1

### probes/s12_h_end_to_end_list_drain.sth

Invocation: `cargo run -q -- build probes/s12_h_end_to_end_list_drain.sth --manifest tests/fixtures/sooth.pkg`

```text
error: stack effect mismatch in `drain` (line 27)
  `add` needs 2 values, but the stack holds 1
  note: declared ( -- )
```

exit=1 (pre-fix). The `impl:` member checks clean (mono route); the error is the
bound-generic consumer — the slice's exact target.
