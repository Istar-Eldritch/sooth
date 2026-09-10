# P7b.S12 probes — poly-body App-dispatch output rendering (SOO-39)

Probe round for SOO-39 ("Poly body: App-dispatch call loses its outputs"),
run 260911 in the `soo-39` worktree at `d7559bb` (main tip: S13's poly.rs
partition included; S6d-PREREQ landed). Fixtures: `probes/s12_*.sth`; frozen
byte-exact stderr: `probes/s12_baseline.md`. Invocation:
`cargo run -q -- build probes/<name>.sth` from the repo root (no manifest;
fixtures import `intrinsics * ;` only). An independent verification prober
re-ran the battery on a scratch `git archive HEAD` extract; its verdicts are
recorded at the bottom.

Provenance. The defect was first recorded by probe round S6d-4 (260907,
[slice6d-probes](./slice6d-probes.md)): "A `['It: Cursor]` draining fold
fails at check with a stack-effect mismatch (`add` needs 2 values, but the
stack holds 0): the poly-body stack checker loses an App-headed dispatch
call's outputs … Recorded, not chased." S6d-PREREQ shipped (260909) and
deferred it as its out-of-scope item (3); Linear holds it as SOO-39. This
round chases it.

## Mechanism (established by probes B/G, then read out of the code)

A bound-generic body's member dispatch (`poly_trait_member_call`,
`src/check/poly/ground.rs`) renders the member row's outputs into the
caller's variable space through `render_member_decl`. That function handles
`PolyType::App` (the abstract-headed remainder `'It['T]`) correctly — head
and args both render through the unification bindings — but has **no
`PolyType::Generic` arm**: a concrete-ctor application (`Option['T]`,
`Step['T 'It['T]]` — how a generic ctor applied to variables is spelled in
`PolyType` space) falls into `other => other.clone()`, so its **args stay in
member-variable space**. The member row's element variable (member id 1; the
trait header var is id 0) lands verbatim in the **caller's** id space, where
id 1 is whatever the caller's effect mentioned second — or nothing at all.

Three faces of the same leak, all reproduced:

- **Out-of-range id** (caller has 1 variable): renders `'?1` — a garbage
  type every downstream concrete-shaped operation chokes on. The operator
  path (`poly_delegate_op`) extracts the *maximal concrete suffix* of the
  operand run and reports the operator as underflowing: this is the
  S6d-4 "loses its outputs" symptom. The outputs are on the stack; they are
  just non-concrete, so the concrete check sees nothing usable.
- **In-range, wrong id**: renders a *different* caller variable — silently
  mis-typed at check time (a `* -> *` constructor variable as an element).
- **Id coincidence**: when the caller's effect mentions its element variable
  second (id 1) — `lib/core/iterator.sth`'s `fold`/`for_each` do — the
  leaked member id reads as the right caller variable and the program
  checks. **This coincidence is the only reason the shipped Iterator
  consumers work.** (`fold`'s effect `( 'It['T] 'A [...] -- 'A )` mentions
  `'It` first, so 'It=0, 'T=1; the member row's element is also id 1.)

Variable-id ordering fact the analysis depends on (probe G): a poly word's
`ty_var_names` are interned in the **effect's mention order**, not the bound
list's written order — `['It: Cursor 'T] ( 'T 'It['T] -- … )` has 'T=0,
'It=1. (Probe F's residual is what exposed this: the leaked member id 1
displayed as `'It`, not `'T`.)

The diagnostics twin `substitute_member_var` (same file) has the same
missing Generic arm; its callers render error text only, so it garbles
messages but moves no stack slots.

## The rounds

### Round A — the S6d-4-faithful drain fold (`probes/s12_a_drain_fold_option_row.sth`)

Option-row `Cursor` trait (`next ( 'It['T] -- Option['T] 'It['T] )`), a
bound-generic drain over `'It[i64]`, self-tail recursion in the `Some` arm:

```sth
: drain ['It: Cursor] ( i64 'It[i64] -- i64 )
  next swap
  ~[ ( None ) drop drop ]
  ~[ ( Some ) Some> rot add swap drain ]
  Option? ;
```

```text
error: stack effect mismatch in `drain` (line 18)
  `add` needs 2 values, but the stack holds 1
  note: declared ( -- )
```

Same failure family as the S6d-4 record. The `Some` payload is the leaked
`'?1`; `add`'s operands after `rot` are (payload `'?1`, acc `i64`); the
maximal-concrete suffix is `[acc]` alone → "holds 1".

### Round A2 — byte-exact S6d-4 message (`probes/s12_a2_add_holds_zero.sth`)

Round A with `add` directly on the payload (no `rot`): `add`'s two operands
are the App-typed remainder and the leaked payload — both non-concrete, so
the suffix extraction stops immediately:

```text
error: stack effect mismatch in `drain` (line 18)
  `add` needs 2 values, but the stack holds 0
  note: declared ( -- )
```

This is the S6d-4 record's exact message shape ("holds 0").

### Round B — what the dispatch actually leaves (`probes/s12_b_member_output_residual.sth`)

The drain's body reduced to `next swap` with the residual exposed in the
declared outputs:

```text
error: stack effect mismatch in `drainB`
  body leaves `i64 'It[i64] Option['?1]`, but the declared outputs are `i64 Option[i64]`
```

Decisive. All three values are present — nothing is *lost*. The remainder
`'It[i64]` renders **correctly** (the `App` arm works); the `Option` output
carries the leaked member element id, out of range in a one-variable
caller: `'?1`. The "outputs lost" framing is the downstream suffix-extraction
symptom, not the mechanism.

### Round C — in-range mis-typing (`probes/s12_c_arm_payload_mistype.sth`)

Element variable written first (`['E 'It: Cursor]`, so 'E=0, 'It=1); the
`Some` payload is destructured and returned:

```text
error: the arms of `Option?` in `drainC` (line 18) disagree: an earlier one leaves `'E`, this one leaves `'It`
  a type variable is rigid across arms: it is never bound to the other arm's type
```

The `Some` payload — necessarily `'E` — is typed `'It`: the `* -> *`
constructor variable. A silent mis-typing at check time, caught here only
because the arms disagreed; a body shaped to agree would carry the wrong
type onward.

### Round D — control: self-call with an App operand (`probes/s12_d_self_call_app_operand.sth`)

A body that only self-calls with an App-headed operand (`looper`) **checks
clean** (the link step then fails for lack of `main` — fixture artifact, not
a checker verdict). The self-call arm is a structural pointwise match; no
rendering, no leak. So recursion itself needs no fix.

### Round E — control: the cross-call App fence (`probes/s12_e_cross_call_app_fence.sth`)

A poly body calling a *different* poly word whose declared input is
App-headed:

```text
error: `outer` cannot call the polymorphic word `step` (line 13, col 3)
  a higher-kinded application in a cross-called polymorphic word is not yet supported from a polymorphic body
  call `step` from a monomorphic word instead
```

The S1-17.i fence (`poly_cross_match`, `src/check/poly/crosscall.rs`) — a
deliberate, located rejection, not a loss. This is the second, independent
facet of the roadmap's "poly bodies calling poly words over compound
receivers" characterization. `poly_cross_output`'s compound-output rejection
(the `_ =>` arm) fences the output side the same way. Neither is exercised
by the drain-fold shape (member dispatch + self-call only), so neither
blocks S6d's `fold`/`for_each`-style consumers.

### Round F — the id-space control (`probes/s12_f_id_coincidence_space.sth`)

Same dispatch as round B with the element kept polymorphic and the effect
mentioning `'T` first ('T=0, 'It=1):

```text
error: stack effect mismatch in `drainF`
  body leaves `'T 'It['T] Option['It]`, but the declared outputs are `'T 'It['T] Option['T]`
```

The in-range face: the `Option`'s element reads `'It` (the constructor
variable). Round F initially read as a refutation of the clone theory (the
coincidence guess predicted `Option['T]`); resolving it produced the
effect-mention-order fact above and made all eight rounds consistent under
the single missing-Generic-arm mechanism.

### Round G — member sig introspection (`probes/s12_g_member_sig_introspect.sth`)

An operand that unifies but leaves the word's outputs empty exposes the
rendered residual against a one-variable caller:

```text
error: stack effect mismatch in `drainG`
  body leaves `Option['?1] 'It[str]`, but the declared outputs are ``
```

Confirms the App arm renders a *concretely-argued* operand correctly
(`'It[str]`) while the Generic output leaks (`Option['?1]`) — same fixture
family as round B, different binding, identical split behaviour.

## Round-level verdict

- The S6d-4 defect is **live at `d7559bb`** and is one root cause:
  `render_member_decl`'s missing `PolyType::Generic` arm (plus the same gap
  in `substitute_member_var` for diagnostics). "Loses its outputs" was the
  suffix-extraction symptom; the outputs are present but member-space-typed.
- The defect is masked today exactly when the caller's effect mentions its
  element variable second — the accident `lib/core/iterator.sth`'s
  consumers stand on. Any consumer with a concrete element, an extra leading
  variable, or a differently-ordered effect breaks.
- The cross-call App fence (S1-17.i) is a separate, located facet of the
  same roadmap sentence; it does not gate the S6d consumer shapes.
- Base health at `d7559bb`: full suite green (3463 passed / 0 failed,
  worktree run); the verification prober re-confirmed on a scratch extract.
- Observed alongside (cosmetic, non-blocking): underflow/type-mismatch
  errors inside a poly body print `note: declared ( -- )` — `ctx.effect()`
  is the poly word's empty placeholder effect (`word.effect`), not its
  `PolySig`. Diagnostics are behaviour (CLAUDE.md); worth fixing in this
  slice if cheap, else recorded.

## Independent verification prober

A `prober` subagent re-ran the battery on a scratch `git archive HEAD`
extract at `d7559bb` (worktree untouched). Verdicts: **C1/C2** (root cause +
diagnostics twin, render-site uniqueness) **CONFIRMED**; **C3** probe
battery **8/8 byte-identical** against `probes/s12_baseline.md`; **C4** id
ordering **confirmed in full** (parser.rs:2043-2049 first-mention ids;
bracket attached post-effect, "so ids stay effect-derived",
parser.rs:3344-3346; member header pre-interned at 0,
parser.rs:4024-4046) with two added Step-row empirics: a fold-layout caller
renders `Step['T 'It['T]]` (the coincidence, proven at the Step row), a
swapped caller leaks verbatim `Step['It 'T['It]]` — nested App included;
**C5** confinement confirmed, with the mechanism correction that
`resolve_mono_member_call` grounds via `ast::ground_member_type` +
concrete-effect fallback and routes generic-impl winners through
`check_poly_call` (not `apply_subst` directly — confinement unaffected);
**C6** hand-trace predicts probe F green under the proposed Generic arm;
**C7** scratch `cargo test` 3463 passed / 0 failed. One process finding: a
superseded draft of probe F still on disk without a baseline entry was
deleted; the canonical fixture is `s12_f_id_coincidence_space.sth`.
Verdicts are folded into the brief's recon addendum.
