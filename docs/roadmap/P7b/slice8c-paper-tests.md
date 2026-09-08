# P7b.S8c paper tests — validated golden designs (recon round)

Designed and executed against the clean tree at HEAD `445a74e` by the probe
round; fixtures lived under `/tmp/s8c-probe/` (ephemeral, sources preserved
verbatim in [slice8c-probes](./slice8c-probes.md)). Companion docs:
[slice8c-brief](./slice8c-brief.md),
[slice8c-probes](./slice8c-probes.md). Harness conventions from
`tests/phase7b_slice8.rs`: `single_file_hosted`/`build_ok`/`build_error_located`
(the latter two asserting `!stderr.contains("panicked")` — the
admission-safety sweep's standing rule), `thing_condition_expected` naming.

Every fixture below was built at HEAD `445a74e` and its measured "today"
behaviour is recorded. The "post-fix" column is the *expected* golden; where
the exact diagnostic text depends on the fix direction the spec picks, the
design pins the invariant (located, non-panic, names the site) and the exact
bytes get pinned at implementation time from the live binary, per house
convention.

## G1 — `bound_dispatched_member_with_free_input_local_is_located_not_panicked`

The S8-review repro twin, panicked→located (roadmap S8c exit item). Fixture:
`plainslot.sth` (probes doc, P1). Today: `sooth build` panics at
`src/ir/driver.rs:579:14`, exit 101. Post-fix: `build_error_located` — non-zero
exit, `error:` stderr, no `panicked`. The message must name the member and the
unbound variable; the spec picks the exact text (fence-at-declaration vs
fence-at-call-site wording differ — see the brief's D1).

- Pre-fix red/green character: red as a **panic** (exit 101, `panicked` in
  stderr), so the golden asserts `build_error_located` succeeds post-fix.
- If the spec instead picks the binding-rule direction (D1's option 2), this
  fixture becomes a **positive** golden (G1r below) — the two directions are
  mutually exclusive and G1/G1r are written as alternatives, exactly one
  surviving. The post-fix monomorph to assert is
  `sooth_mono_odd_Odd_0_Box__T0___m0__t0_i64_t1_i64` (w3r's measured-
  ingredient prediction: member word registered as `odd;Odd;0;Box['T0]__m0`,
  per-site θ binding the target var and the local, both to `i64`). One
  spelling constraint the fixture must respect (w3r Q4a): the consumer's row
  must spell its slots in the member row's order — a mismatched spelling
  fails located at the consumer's own operand check before any dispatch, and
  is a different (pre-existing, healthy) fence.

## G1r — `bound_dispatch_grounds_member_locals_from_the_call_site` (alternative to G1)

Only if D1's option 2 (per-site binding rule) is chosen: the same fixture
builds, runs, exits 0. There is no output to observe (`drop drop`) — the pin
is build+run exit 0, mirroring `build_run_keep`'s assert. A type-observing
twin (member body printing the local) is deliberately **not** designed here:
it would pin Route B's grounding semantics before the spec has chosen them;
if option 2 is picked, add it then.

## G2 — `mono_call_site_member_local_stays_working` (positive pin)

Fixture: `mono.sth` (probes doc, P2). Today: builds, runs, exit 0. Post-fix:
must stay build+run exit 0 — whichever direction the spec picks, the mono
route's call-site grounding is load-bearing (it is Route D's contract, and
`impl: Ord for i64`-style members plus every existing mono member call ride
it). Golden: `build_run_keep`-style assert (exit 0; stdout empty).

## G3 — `quotation_row_member_local_behaves_as_the_spec_decides`

Fixture family: `qrow.sth` (P4) and `qrow3.sth` (P5). Today: P4 located at
the call site by the caller-side rule; P5 (explicit instantiation) panics at
`driver.rs:579`. The pre-existing caller-side golden for P4's message already
exists in spirit (`tests/phase7b_slice2.rs`'s unbound-output pins); the new
pin is P5: post-fix it must be located, not panicked (G3a, fence direction) or
build+run clean (G3b, binding-rule direction). Written as the same
alternatives pair as G1/G1r; exactly one survives. P4's own behaviour is
unchanged by either direction and gets no new golden (the caller rule is not
this slice's code).

## G4 — `app_headed_member_local_keeps_grounding_per_site` (regression pin)

Fixture: `mixed3.sth` (probes doc, P8) — HKT trait, App-arg local plus
plain-slot local, two differently-typed operands, bound-dispatched from a mono
caller. Today: builds, runs, exit 0; the monomorph symbol
`sooth_mono_w2_W2_0_Box__T0___m0__t0_i64_t1_e0_Bool` shows both locals bound
independently. Post-fix: must stay build+run exit 0 **and** the monomorph
symbol must stay byte-identical — this is the fence-direction safety proof:
if the spec picks a declaration-time fence, this fixture is the canary
proving the fence does not outlaw Route A's working plain-slot local (`'V`).
Assert style: build+run exit 0 plus a symbol assertion against
`driver::emit_ssa_with_manifest`-style capture (the S8 REQ-11 pin's
mechanism, `tests/phase7b_slice8.rs`'s
`consuming_loop_over_range_is_one_frame_with_a_back_edge_and_next_is_a_real_frame`).

## G5 — `shipped_lib_trait_dispatch_symbols_unchanged` (regression net, design only)

Whichever direction is picked, the fix touches the member-dispatch path that
mints every S6/S7/S8 dogfood instantiation. The existing suite already pins
this hard (the S6/S7/S8 byte-exact goldens and the REQ-11 IL pin); this
design adds no new fixture — it is a **checklist item for the phase gate**:
`cargo test` green with zero retouched baseline ILs, and any retouch gets
justified in the phase commit message the way `d2e8123` did for S9. Rationale
for recording it here: Route B's fix (if option 2) reorders θ construction,
and θ order feeds `instantiation_symbol` — the sort invariants (P7.S3t,
`theta.ty.sort_by_key`) are the reason no churn is *expected*, and this item
is where that expectation is measured rather than assumed.

## G6 — `concrete_target_member_dispatch_checks_site_slots` (candidate, scope decision pending)

Fixture: `concrete4.sth` (probes doc, P12) — concrete impl target, member
local, a `List[i64]` flowed through the local's slot into a member typed
`(i64, i64)`. Today: builds, runs, prints — silent type hole. Post-fix
(only if the spec folds the concrete-winner arm into this slice's scope —
brief D2): `build_error_located` naming the member and the mismatched slot.
If the spec carves it out, this design moves to the carve-out slice's own
paper doc verbatim. Not written as a positive pin of today's behaviour — the
house rule (admission-safety sweep) forbids pinning a silent wrong-typing as
a golden.

## Round-2 validation (worker-measured at HEAD `62a928d`, post-S8b-merge)

A five-worker `prober` round re-measured every fixture at the S8b base. All
six "Today" claims above are **CONFIRMED byte-identical** (exit codes, panic
bytes, error texts, monomorph symbols — w1/w5; no behavioural delta from the
S8b merge, which left `src/ir/driver.rs`, `src/ast.rs`, `src/parser.rs`, and
`lib/` diff-empty). Round-2 additions to the designs:

- **G4 expected symbol** (exact assert target): the member monomorph
  `sooth_mono_w2_W2_0_Box__T0___m0__t0_i64_t1_e0_Bool`; the caller twin,
  optional, `sooth_mono_consume4__m0__t0_c0m0_Box_t1_i64_t2_e0_Bool`.
- **G4 mechanism CONFIRMED**: `sooth::driver::emit_ssa_with_manifest`
  (`src/driver.rs:897`, called by `sooth build` at `:891`) is the whole-closure
  capture the REQ-11 pin uses (`tests/phase7b_slice8.rs:742`), and committed
  tests already filter `sooth_mono_*` symbols from that capture
  (`tests/phase7b_slice8b.rs:177`, `:565`) — the exact-symbol assert is
  writable today.
- **G5 named guards** (the checklist item's existing net):
  `consuming_loop_over_range_is_one_frame_with_a_back_edge_and_next_is_a_real_frame`
  (`tests/phase7b_slice8.rs:742`),
  `functor_for_list_map_lowers_as_one_non_inline_frame`
  (`tests/phase7b_slice8b.rs:531`), and the nullary-member mint pins
  (`tests/phase7b_slice8b.rs:137/152/237`).
- **G6 observability**: the wrongness is unambiguously observable today
  (w2's exploit: a member body computing on the local slot prints a `Bool`'s
  discriminant `+1` or a `List[i64]`'s head word `+1`) — the post-fix
  `build_error_located` golden has a concrete wrong-behaviour it retires, and
  Route D's `trait_member_operand_error` text is the ready diagnostic
  template if D2 folds the check in.
- **New-cells note for the implementing phase**: round 2 also measured an
  **array-element** member local (`array['U 2]`) and the **bare-var catch-all**
  target (`impl: Odd for 'T`) — both panic at `driver.rs:579` today. G1's
  panicked→located pin should be written to cover the cheapest of these
  twins, and the fence/binding code must handle both shapes whichever
  direction D1 picks (they are the same unbound-union-var mechanism,
  `build_member_var_union`, `parser.rs:768/817-827`).

## Cross-cutting invariants for the implementing phase

- Every new golden lives beside its stage per CLAUDE.md: the fence/binding
  code is `src/check/`, so its unit tests go in the changed checker file's
  `#[cfg(test)] mod tests` (message bytes, variable naming, both dispatch
  routes), and the end-to-end goldens above go in `tests/phase7b_slice8.rs`
  (S8c is an S8 follow-up; the file's header comment gains an S8c line).
- The repro twins G1/G1r and G3a/G3b are the "S8-review repro twins
  panicked→located" roadmap exit item — exactly one of each pair survives,
  and the phase commit message names which direction won.
- No `src/ir/` change (roadmap scope): `driver.rs:579`'s `expect` stays as
  the backstop. G2/G4 double as the proof the backstop never fires on the
  routes that were already healthy.
