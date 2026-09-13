# P7b.S13 probes — cross-call App lift: pre-lift baseline and precedence (SOO-60)

Probe round for SOO-60 ("cross-call App lift"), run 260912 in the `soo-60`
worktree at `a3779f4` (build warm; no `src/` changes — the round writes
`probes/s13_*` fixtures, `probes/s13_baseline.md`, and this report only).
Fixtures: `probes/s13_*.sth`; frozen byte-exact stderr:
`probes/s13_baseline.md`. Invocation: `cargo run -q -- build probes/<name>.sth`
from the repo root (no manifest; fixtures import `intrinsics * ;` only). The
round captures TODAY'S bytes — the pre-lift baseline — and tests the recorded
precedence predictions; the post-lift goldens are the next round's paper
tests, except round E's fixture, written now as the primary golden candidate.
A prior prober attempt spent its whole budget on recon and wrote nothing, so
every citation below is re-measured at `a3779f4` with `grep -n`/`awk`
receipts, and the handed-down line numbers and one handed-down syntax "fact"
are corrected.

Provenance. The fence is S1-17.i (`poly_cross_match`,
`src/check/poly/crosscall.rs`): a poly cross-call with an `App` slot on either
side of an INPUT match is a located "unsupported" rejection ("S2 owns
constructor-keyed dispatch" — S2 shipped its member-dispatch route; the
non-member cross-call fence stayed, pinned by slice12's G8).
`poly_cross_output`'s wildcard rejects every compound OUTPUT the same way.
SOO-60 lifts both. The slice12-brief open questions recorded the prediction:
post-lift, App-vs-App matching binds the callee's head var to
`Image::CallerVar(caller head var)` and recurses on args; App/Generic output
rendering through the mapping needs no registry interning; the growth ban
stays. This round pins the bytes the lift moves and the arm precedence it must
preserve.

## Mechanism / drift-corrected citations (measured at `a3779f4`)

- `poly_cross_match` fn at `crosscall.rs:188`. Arm order: `(PolyType::Var(v),
  _)` first, then Concrete/Concrete, Array/Array, Ref/Ref, same-header
  Generic/Generic, then the **S1-17.i fence at :346-353** (comment :344-345,
  message :351), then the catch-all mismatch (:354). At d7559bb the fence was
  recorded at :346-352 — the pattern line is unchanged, the arm closes one
  line later.
- `poly_cross_output` fn at :365; the **compound-output wildcard at :398-405**
  (message :403). At d7559bb it was :387-396 — it moved +11 (S12's
  GenericVariant note above it). The handoff's "~:378-396" is stale.
- `poly_growing_cross_call_error` at :498 (message template :508) — unmoved.
- Both fences render through `poly_cross_call_unsupported_error`, whose tail
  is always "`<reason>` is not yet supported from a polymorphic body" /
  "call `<callee>` from a monomorphic word instead" — every block below shows
  it.

## The rounds

### Round A — fence baseline (re-run of `probes/s12_e_cross_call_app_fence.sth`)

```text
error: `outer` cannot call the polymorphic word `step` (line 13, col 3)
  a higher-kinded application in a cross-called polymorphic word is not yet supported from a polymorphic body
  call `step` from a monomorphic word instead
```

exit=1. **CONFIRMED** — byte-identical to the s12 capture; the fence is live at
`a3779f4` with the citations above.

### Round B — growth-vs-fence precedence

**B1** (`probes/s13_b_app_declared_helper.sth`): callee declares App-headed
input `'F['T]`, poly caller supplies `'It['T]`:

```text
error: `outer` cannot call the polymorphic word `step` (line 6, col 41)
  a higher-kinded application in a cross-called polymorphic word is not yet supported from a polymorphic body
  call `step` from a monomorphic word instead
```

exit=1. **CONFIRMED** — the S1-17.i fence bytes.

**B2** (`probes/s13_b_bare_var_growth.sth`): same caller shape, callee declares
a bare `'T`:

```text
error: `outer` cannot pass `'It['T]` to `'T` of the polymorphic word `step` (line 6, col 41)
  a polymorphic call site may pass a type variable only bare: wrapping it in `'It['T]` builds a larger type at every hop of a recursive call, which has no finite set of instantiations
  declare `step`'s parameter as `'It['T]` so the shape is matched structurally, or call it from a monomorphic word
```

exit=1. **CONFIRMED** — NOT the fence: the growth error. The `(PolyType::Var(v),
_)` arm fires before the App arm (crosscall.rs:345 vs :346), so an App operand
facing a bare-var declared input is GROWTH. The lift does not change growth
verdicts.

### Round C — output-side inventory (all callers' input sides clean)

**C1** (`probes/s13_c_app_output.sth`) — **NEW, three attempts; the specified
shape cannot reach the output arm.** As specified, the callee's OWN stack check fires first:

```text
error: stack effect mismatch in `mk`
  body leaves `'T`, but the declared outputs are `'F['T]`
```

exit=1. Two construction routes were tried and both are blocked. A member row
over a bare value input has no dispatch receiver:

```text
error: trait member `up` of `Lift` (line 9, col 5) has no input for a call to dispatch on (expected the trait's variable `'F` bare or heading an application like `'F['T]`)
```

exit=1. A head-bare dispatch row is a kind error:

```text
error: type variable `'F` at line 10, col 10 is used as a plain type but has kind `* -> *` (from an application of `'F` at line 9, col 13); a higher-kinded variable never appears bare
```

exit=1. Only member dispatch can produce an App value, and every dispatch
receiver reachable from a clean input is rejected. Conclusion: with a non-App
input side, an App-headed callee output appears UNREACHABLE in a poly→poly
call — the wildcard's App face has no known live shape (the input side of any
App-producing callee would itself be App/head-typed, i.e. fenced inward).

**C2** (`probes/s13_c_generic_output.sth`) — **CONFIRMED after adjustment.**
The handed-down declaration syntax `type: Wrap['X] ( w: 'X ) ;` is a PARSE
ERROR ("expected a word, found LParen at line 7, col 16") — the real syntax is
variant-style, `type: Wrap['X] | Wrap 'X ;` (as at
`src/check/poly/tests.rs:3951`). With the ctor building the callee's output:

```text
error: `caller` cannot call the polymorphic word `mk` (line 11, col 37)
  returning the compound type `Wrap['T]` from a polymorphic word is not yet supported from a polymorphic body
  call `mk` from a monomorphic word instead
```

exit=1. The Generic-output face of the wildcard is REACHABLE and fenced. This
exact shape is already unit-pinned (`check_cross_call_unsupported_callee_shapes_name_themselves`,
tests.rs:3935, sub-fixture "returning the compound type `Box['U]` ...",
:3946).

**C3** (`probes/s13_c_ref_output.sth`) — **NEW: Ref outputs are banned at
declaration; the wildcard's Ref face is unreachable.** As specified
(`: mk ['T] ( 'T -- &'T ) ;`) and again as the error's own advice suggests
(borrow pass-through, `: mk ['T] ( &'T -- &'T ) ;`):

```text
error: a reference cannot be stored: `mk` declares the output `&'T` (line 7)
  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead
```

exit=1 both times. No poly word can declare a Ref output at all, so
`poly_cross_output` never sees one from source.

**C4** (`probes/s13_c_array_output.sth`) — **CONFIRMED after adjustment.** As
specified (`: mk ['T] ( 'T -- array['T 4] ) ;`, empty body) the callee's own
stack check fires first (no array ctor exists intrinsics-only); adjusted to
pass-through (`: mk ['T] ( array['T 4] -- array['T 4] ) ;`, caller `['U]
( array['U 4] -- array['U 4] ) mk` — the element var maps `'U`→`'T` on the
clean input side):

```text
error: `caller` cannot call the polymorphic word `mk` (line 9, col 46)
  returning the compound type `array['T 4]` from a polymorphic word is not yet supported from a polymorphic body
  call `mk` from a monomorphic word instead
```

exit=1. The Array-output face is REACHABLE and fenced by the same wildcard.

### Round D — App-vs-App near-misses (post-lift verdict candidates)

All four as first written died at the CALLEE's own check before any
cross-call: `: step ['F 'T] ( 'F['T] -- ) ;` with an empty body leaves the
declared input on the stack (stack-effect mismatch in `step`: the body leaves
`'F['T]`, but the declared outputs are empty). A cross-called callee must
be self-consistent first; adjusted to pass-through (`'F['T] -- 'F['T]`),
all four hit the input fence — **CONFIRMED ×4**. D1's bytes; the four differ
only in the reported line, col:

```text
error: `outer` cannot call the polymorphic word `step` (line 8, col 33)
  a higher-kinded application in a cross-called polymorphic word is not yet supported from a polymorphic body
  call `step` from a monomorphic word instead
```

exit=1 each:

- **D1** `s13_d_arg_var_mismatch.sth` — `'It['U]` vs `'F['T]` (arg vars
  differ): fence, line 8, col 33.
- **D2** `s13_d_arg_concrete.sth` — `'It[i64]` vs `'F['T]` (concrete arg):
  fence, line 7, col 31.
- **D3** `s13_d_bare_supplied.sth` — bare `'F` vs declared `'F['T]`: fence,
  line 8, col 24. Two sub-findings: the first write's unused bound `'T` was
  rejected before anything else ("type variable `'T` declared in the bound
  bracket of `outer` ... never appears in the effect"); and the caller's bare
  unbounded `'F` (kind inferred `*`) produces NO kind error — the fence's
  declared-App arm fires. The C1 kind error only fires for a var BOUND
  higher-kinded (`['F: Lift]`) and then used bare.
- **D4** `s13_d_generic_supplied.sth` — `Wrap['T]` vs declared `'F['T]`
  (concrete ctor head): fence, line 9, col 30.

All four are the byte-exact pre-lift goldens for the post-lift round. Note for
the spec: D4's supplied head is a concrete CTOR (`Generic`), so post-lift the
callee head var must bind to something ctor-valued there — `Image::CallerVar`
cannot represent it (see Facts).

### Round E — the post-lift green golden candidate

`probes/s13_e_post_lift_green.sth`: `type: Wrap['X] | Wrap 'X ;`,
`: step ['F 'T] ( 'F['T] -- 'F['T] ) ;`,
`: outer ['It 'T] ( 'It['T] -- 'It['T] ) step ;`,
`: main ( -- ) 42 Wrap outer drop ;`. Today:

```text
error: `outer` cannot call the polymorphic word `step` (line 11, col 41)
  a higher-kinded application in a cross-called polymorphic word is not yet supported from a polymorphic body
  call `step` from a monomorphic word instead
```

exit=1. **CONFIRMED** — the fence fires. The grounding in main was verified
type-correct TODAY by a scratch variant with `outer`'s body emptied (no
cross-call): exit 0 and a linked ELF. So only the fence stands between this
program and a green build; it is the primary post-lift golden. (Incidental:
the mono route already grounds an App-declared poly word over a concrete ctor
head — `'It:=Wrap 'T:=i64` at a mono call site checks clean today.)

### Round F — declaration kind-consistency (CallerVar-only heads)

- **F1** `s13_f_bare_and_applied.sth` — one var bare AND App-headed:

```text
error: type variable `'F` at line 6, col 21 is applied like a type constructor but has kind `*` (bound bare at line 6, col 15); only a higher-kinded variable can head `'F[...]`
```

- **F2** `s13_f_self_application.sth` — self-application:

```text
error: type variable `'F` at line 6, col 12 is applied like a type constructor but has kind `*` (bound bare at line 6, col 12); only a higher-kinded variable can head `'F[...]`
```

exit=1 both. **CONFIRMED** — the kind checker already rejects both
declarations. Within any checkable signature a head var is only ever
App-headed (never a bare value, never self-applied), so it can never be
instantiated to a concrete monomorphic TYPE; a head slot can only receive a
caller variable (B2/D1 shape → `Image::CallerVar`) or a concrete CTOR (D4
shape). `Image::Concrete` holds a value-kind `Type`, so the recorded
prediction's `Image::CallerVar` covers D1 but not D4 — the ctor-valued head
image is the spec's open bit.

### Round G — pin inventory (greps only, no builds)

**Moves when the fence/output wildcard lifts — must be rewritten:**

- `src/check/poly/tests.rs:608` `non_member_app_cross_call_still_rejects_with_p8_fence_text`
  (P7b.S2 S2-10) — expects the fence error for an App-slot cross-call
  (`inner`/`outer`, `( 'G['T] -- ) drop`); asserts "cannot call the
  polymorphic word `inner`" (:617) and the higher-kinded sentence (:622).
- `src/check/poly/tests.rs:1782` `poly_cross_match_app_slot_is_unsupported_not_a_panic`
  (S1-17.i unit) — calls `poly_cross_match` directly with `(App{head:0,
  args:[Var(1)]}, Var(0))`, expects err containing "supported"; its expected
  verdict is exactly what the lift redefines.
- `src/check/poly/tests.rs:3935` `check_cross_call_unsupported_callee_shapes_name_themselves`
  — sub-fixture 2 (:3946, "returning the compound type `Box['U]` ...") pins
  the Generic-output wildcard and moves when it lifts; sub-fixtures 1 (length
  var in callee sig, :3940) and 3 (row-polymorphic call, :3955) are different
  fences and stay.
- `tests/phase7b_slice12.rs:451` `cross_call_app_fence_stays_byte_identical`
  (slice12 G8) — byte-exact `assert_eq!` on the s12_e stderr (:469), identical
  to round A's capture. Its name says "stays byte identical"; the lift breaks
  it by design and the test must be retargeted.

**Stays (verified against this round's evidence):**

- `src/check/poly/tests.rs:3755` `check_growing_cross_call_is_error` (asserts
  "builds a larger type at every hop", :3765) and
  `tests/phase7_slice3k.rs:208` — the growth ban stays (B2 re-confirmed the
  bytes today); expected to survive the lift.
- `src/check/poly/tests.rs:3779`
  `check_growing_cross_call_concrete_reference_is_unsupported_not_growth`
  (:3788) — the R6 concrete-compound arm ("passing the concrete compound
  value"), a different wildcard sibling; untouched by an App lift.
- `src/check/poly/tests.rs:6053` — "passing a quotation to a polymorphic
  word" (Var-arm Quotation rejection), separate reason text.
- `src/check/poly/tests.rs:4157` / `:4176` — inline-callee transitive
  rejections; they share the "cannot call the polymorphic word `h`" first line
  but their reason is inline routing, not App slots.
- `tests/phase7_slice3k.rs:325` — cross-call ambiguity ("names more than one
  polymorphic word"), unrelated.

## Verdict table

| round | fixture | today's bytes | verdict |
|---|---|---|---|
| A | `s12_e_cross_call_app_fence.sth` (re-run) | S1-17.i fence | CONFIRMED |
| B1 | `s13_b_app_declared_helper.sth` | S1-17.i fence | CONFIRMED |
| B2 | `s13_b_bare_var_growth.sth` | growth error (`only bare`, :498) | CONFIRMED |
| C1 | `s13_c_app_output.sth` | callee stack check → no-dispatch-receiver → kind error (3 attempts) | NEW (App output unreachable from a clean input) |
| C2 | `s13_c_generic_output.sth` | parse error → compound-output wildcard | CONFIRMED after adjustment (+ NEW: handed-down type syntax wrong) |
| C3 | `s13_c_ref_output.sth` | "a reference cannot be stored" (2 attempts) | NEW (Ref output banned at declaration; wildcard face unreachable) |
| C4 | `s13_c_array_output.sth` | callee stack check → compound-output wildcard | CONFIRMED after adjustment |
| D1 | `s13_d_arg_var_mismatch.sth` | callee stack check → S1-17.i fence | CONFIRMED after adjustment |
| D2 | `s13_d_arg_concrete.sth` | callee stack check → S1-17.i fence | CONFIRMED after adjustment |
| D3 | `s13_d_bare_supplied.sth` | unused-bind error → S1-17.i fence | CONFIRMED after adjustment |
| D4 | `s13_d_generic_supplied.sth` | callee stack check → S1-17.i fence | CONFIRMED after adjustment |
| E | `s13_e_post_lift_green.sth` | S1-17.i fence (grounding scratch-verified clean) | CONFIRMED |
| F1 | `s13_f_bare_and_applied.sth` | kind error (`only a higher-kinded variable can head`) | CONFIRMED |
| F2 | `s13_f_self_application.sth` | kind error (same rule) | CONFIRMED |

## Facts the spec must carry

1. **Precedence**: the `(PolyType::Var(v), _)` arm fires before the App fence —
   an App operand facing a BARE-var declared input is the GROWTH error
   (crosscall.rs:498; B2 bytes), not the fence. The lift must not change
   growth verdicts; the growth bytes are pinned (B2) and unit-pinned at
   tests.rs:3755 / phase7_slice3k.rs:208.
2. **Both output faces that are reachable are fenced by the same wildcard**:
   Generic ctor outputs (C2, ctor-built body required) and fixed-length Array
   outputs (C4, pass-through shape; the element var maps on the clean input
   side) — one message, "returning the compound type `...` from a polymorphic
   word is not yet supported from a polymorphic body".
3. **Unreachable wildcard faces**: Ref outputs are banned at DECLARATION ("a
   reference cannot be stored", even borrow pass-through — C3, 2 attempts);
   App outputs have no known route from a clean input side (C1, 3 attempts:
   callee stack check, no dispatch receiver, higher-kinded-never-bare). The
   lift's output mapping only has to handle shapes that can actually arrive:
   Generic and Array (and, per the recorded prediction, App — but no poly→poly
   source shape is known to produce one today).
4. **Kind consistency already enforced**: a var is bare (`*`) or App-headed,
   never both (F1), and never self-applied (F2); unbound vars default to `*`
   and "only a higher-kinded variable can head `'F[...]`". Consequence: a head
   slot can only receive a caller var (`Image::CallerVar`, D1/B2) or a
   concrete ctor (D4) — never a concrete monomorphic type. D4's ctor-valued
   head image is not covered by the recorded prediction.
5. **Cross-called callees must be self-consistent first**: an empty body only
   checks when declared outputs equal declared inputs (pass-through); any
   `( 'F['T] -- )`-style declaration with an empty body dies at the callee's
   own stack check before cross-call semantics (D attempts 1). Post-lift
   goldens must use pass-through or constructing bodies.
6. **Syntax facts**: `type:` declarations are variant-style
   (`type: Wrap['X] | Wrap 'X ;`); the handed-down `( w: 'X )` field-list form
   is a parse error. Bound brackets reject unused vars ("never appears in the
   effect"). `42` literals, variant ctors, and `drop` of a user type all work
   intrinsics-only (E grounding scratch).
7. **Moved-pin inventory** (round G): tests.rs:608, :1782, :3935 (sub-fixture
   2), phase7b_slice12.rs:451/:469 must be rewritten by the lift; growth,
   concrete-compound, quotation, inline-routing, and ambiguity pins stay.
8. **Line drift vs d7559bb**: fence arm :346-352 → :346-353 (pattern line
   unchanged); output wildcard :387-396 → :398-405; growth error :498
   unchanged. The handoff's "now" numbers (:344-352, ~:378-396) were stale;
   all citations here are `awk`/`grep -n`-measured at `a3779f4`.
