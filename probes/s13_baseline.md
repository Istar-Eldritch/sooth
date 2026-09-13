# Non-regression baseline: SOO-60 probe round (P7b.S13 cross-call App lift)

Captured 260912, HEAD `a3779f4` (branch `soo-60`; worktree clean — this probe
writes `probes/s13_*` fixtures, this baseline, and the slice13 report only, no
`src/` changes). Byte-exact `cargo run -q -- build probes/<name>.sth` stderr +
exit code per fixture. Invocation: run from the repo root, no manifest (these
fixtures import `intrinsics * ;` only). Fixtures whose first written shape
died before the target arm record every attempt; the final on-disk fixture is
the last attempt.

### probes/s12_e_cross_call_app_fence.sth — re-run (round A, fence baseline)

error: `outer` cannot call the polymorphic word `step` (line 13, col 3)
  a higher-kinded application in a cross-called polymorphic word is not yet supported from a polymorphic body
  call `step` from a monomorphic word instead
exit=1

### probes/s13_b_app_declared_helper.sth (round B1 — App-declared callee input)

error: `outer` cannot call the polymorphic word `step` (line 6, col 41)
  a higher-kinded application in a cross-called polymorphic word is not yet supported from a polymorphic body
  call `step` from a monomorphic word instead
exit=1

### probes/s13_b_bare_var_growth.sth (round B2 — bare-var callee, App supplied)

error: `outer` cannot pass `'It['T]` to `'T` of the polymorphic word `step` (line 6, col 41)
  a polymorphic call site may pass a type variable only bare: wrapping it in `'It['T]` builds a larger type at every hop of a recursive call, which has no finite set of instantiations
  declare `step`'s parameter as `'It['T]` so the shape is matched structurally, or call it from a monomorphic word
exit=1

### probes/s13_c_app_output.sth (round C1 — App callee output; three attempts)

attempt 1 — as specified (`: mk ['F 'T] ( 'T -- 'F['T] ) ;`, empty body):

error: stack effect mismatch in `mk`
  body leaves `'T`, but the declared outputs are `'F['T]`
exit=1

attempt 2 — Lift member row over a bare value input (`: up ( 'T -- 'F['T] ) ;`):

error: trait member `up` of `Lift` (line 9, col 5) has no input for a call to dispatch on (expected the trait's variable `'F` bare or heading an application like `'F['T]`)
exit=1

attempt 3 (final fixture) — head-bare dispatch row (`: up ( 'F -- 'F['T] ) ;`):

error: type variable `'F` at line 10, col 10 is used as a plain type but has kind `* -> *` (from an application of `'F` at line 9, col 13); a higher-kinded variable never appears bare
exit=1

### probes/s13_c_generic_output.sth (round C2 — Generic callee output; two attempts)

attempt 1 — handed-down type syntax `type: Wrap['X] ( w: 'X ) ;`:

error: parse error: expected a word, found LParen at line 7, col 16
exit=1

attempt 2 (final fixture) — variant syntax `type: Wrap['X] | Wrap 'X ;`, ctor body:

error: `caller` cannot call the polymorphic word `mk` (line 11, col 37)
  returning the compound type `Wrap['T]` from a polymorphic word is not yet supported from a polymorphic body
  call `mk` from a monomorphic word instead
exit=1

### probes/s13_c_ref_output.sth (round C3 — Ref callee output; two attempts)

attempt 1 — as specified (`: mk ['T] ( 'T -- &'T ) ;`):

error: a reference cannot be stored: `mk` declares the output `&'T` (line 7)
  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead
exit=1

attempt 2 (final fixture) — borrow pass-through per the error's advice (`: mk ['T] ( &'T -- &'T ) ;`):

error: a reference cannot be stored: `mk` declares the output `&'T` (line 7)
  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead
exit=1

### probes/s13_c_array_output.sth (round C4 — Array callee output; two attempts)

attempt 1 — as specified (`: mk ['T] ( 'T -- array['T 4] ) ;`, empty body):

error: stack effect mismatch in `mk`
  body leaves `'T`, but the declared outputs are `array['T 4]`
exit=1

attempt 2 (final fixture) — pass-through (`: mk ['T] ( array['T 4] -- array['T 4] ) ;`):

error: `caller` cannot call the polymorphic word `mk` (line 9, col 46)
  returning the compound type `array['T 4]` from a polymorphic word is not yet supported from a polymorphic body
  call `mk` from a monomorphic word instead
exit=1

### probes/s13_d_arg_var_mismatch.sth (round D1 — App-vs-App, arg vars differ)

attempt 1 — callee `( 'F['T] -- )`, empty body:

error: stack effect mismatch in `step`
  body leaves `'F['T]`, but the declared outputs are ``
exit=1

attempt 2 (final fixture) — callee pass-through:

error: `outer` cannot call the polymorphic word `step` (line 8, col 33)
  a higher-kinded application in a cross-called polymorphic word is not yet supported from a polymorphic body
  call `step` from a monomorphic word instead
exit=1

### probes/s13_d_arg_concrete.sth (round D2 — App-vs-App, concrete arg)

attempt 1 — callee `( 'F['T] -- )`, empty body: same bytes as D1 attempt 1
(stack-effect mismatch in `step`: the body leaves `'F['T]`, but the declared
outputs are empty), exit=1.

attempt 2 (final fixture) — callee pass-through:

error: `outer` cannot call the polymorphic word `step` (line 7, col 31)
  a higher-kinded application in a cross-called polymorphic word is not yet supported from a polymorphic body
  call `step` from a monomorphic word instead
exit=1

### probes/s13_d_bare_supplied.sth (round D3 — declared App, supplied bare var)

attempt 1 — outer bound `['F 'T]` with 'T unused:

error: type variable `'T` declared in the bound bracket of `outer` at line 7, col 13 never appears in the effect
exit=1

attempt 2 (final fixture) — binds list only 'F; callee pass-through:

error: `outer` cannot call the polymorphic word `step` (line 8, col 24)
  a higher-kinded application in a cross-called polymorphic word is not yet supported from a polymorphic body
  call `step` from a monomorphic word instead
exit=1

### probes/s13_d_generic_supplied.sth (round D4 — Generic supplied, App declared)

attempt 1 — callee `( 'F['T] -- )`, empty body: same bytes as D1 attempt 1
(stack-effect mismatch in `step`: the body leaves `'F['T]`, but the declared
outputs are empty), exit=1.

attempt 2 (final fixture) — callee pass-through:

error: `outer` cannot call the polymorphic word `step` (line 9, col 30)
  a higher-kinded application in a cross-called polymorphic word is not yet supported from a polymorphic body
  call `step` from a monomorphic word instead
exit=1

### probes/s13_e_post_lift_green.sth (round E — post-lift golden candidate)

attempt 1 — `: main ( -- ) ;` (empty main):

error: `outer` cannot call the polymorphic word `step` (line 8, col 41)
  a higher-kinded application in a cross-called polymorphic word is not yet supported from a polymorphic body
  call `step` from a monomorphic word instead
exit=1

attempt 2 (final fixture) — concrete grounding in main (`type: Wrap['X] | Wrap 'X ;`, `: main ( -- ) 42 Wrap outer drop ;`):

error: `outer` cannot call the polymorphic word `step` (line 11, col 41)
  a higher-kinded application in a cross-called polymorphic word is not yet supported from a polymorphic body
  call `step` from a monomorphic word instead
exit=1

Note: main's grounding was verified type-correct TODAY by a scratch variant
under /tmp with `outer`'s body emptied (no cross-call): exit 0 and a linked
ELF. So the grounding parses, `42` literals work intrinsics-only, the variant
ctor builds, and a MONO call site already grounds an App-declared poly word
over a concrete ctor head (`'It:=Wrap 'T:=i64`) — only the fence stands
between this program and a green build.

### probes/s13_f_bare_and_applied.sth (round F1 — one var bare and App-headed)

error: type variable `'F` at line 6, col 21 is applied like a type constructor but has kind `*` (bound bare at line 6, col 15); only a higher-kinded variable can head `'F[...]`
exit=1

### probes/s13_f_self_application.sth (round F2 — self-application)

error: type variable `'F` at line 6, col 12 is applied like a type constructor but has kind `*` (bound bare at line 6, col 12); only a higher-kinded variable can head `'F[...]`
exit=1
