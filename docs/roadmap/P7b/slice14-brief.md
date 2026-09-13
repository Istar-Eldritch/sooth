# P7b.S14 discovery brief — strictify the S9 pre-guard's scope-derived ctor params

Source: SOO-58 (P7b.S11 open ruling question), decided via interview 2026-09-13.
See the Linear comment on SOO-58 for the full interview record.

## Ruling (decided, not open)

`bare_generated_word_own_module_grounding` (the S9 pre-guard, `src/check/terms.rs:1879`)
fires on a bare ctor call that resolves to the caller's *own* struct/enum header, but
where the only same-named candidate available in scope (`mint_fallback_candidates`,
`terms.rs:2433`) is a foreign module's eager mint. Today it borrows that foreign
candidate's type-argument list unconditionally to instantiate the caller's own header —
no reachability check at all.

`foreign_single_candidate_grounding` (the S10 ladder function, `terms.rs:2086`) handles
the sibling shape (caller has no own header) and *does* gate this: it requires the
foreign candidate's declaring module to be reachable through the caller's own import
set (plain imports, selective imports, or a resolved hub re-export chain) before
borrowing its instantiation, else it errors (`ambiguous_generic_headers_error` when
two or more reachable declaring headers exist, `unreachable_declaring_module_error`
when the sole candidate's declaring module isn't reachable at all).

**Decision: apply the same reachability gate, verbatim, to the S9 pre-guard's
foreign-arg borrow.** Not a weaker or own-header-specific variant — the identical
`foreign_single_candidate_grounding` reachability logic, reused (not reimplemented) for
this second call site.

## Known blast radius (established during the interview, not to be rediscovered)

`mint_fallback_candidates` scans the whole-program live generic-mint registry — every
struct/enum/variant mint made so far anywhere in the compiled program — not anything
scoped by the caller's import graph. This is confirmed directly by the existing golden
fixtures: in `tests/phase7b_slice9.rs`'s G1 family, `a.sth` mints its *own* `Widget`
header using `b.sth`'s already-minted `Widget[i64]` argument list, and `a.sth` never
imports `b` — only `main.sth` imports both `self::a` and `self::b`.

Because of this, applying S10's reachability gate verbatim will flip **every** G1-family
golden in `tests/phase7b_slice9.rs` from "mint own header using foreign args, silently"
to "unreachable declaring module" errors, unless each fixture's caller module gains an
explicit import of the foreign minter module (or the call site gains explicit type
args, which is the pre-existing escape hatch `poly_call_takes_type_args`'s R-6 category
already supports — see `slice11-spec.md` §Explicit-args category).

Affected tests (confirmed by reading `tests/phase7b_slice9.rs`), all currently
asserting the *unguarded* borrow succeeds or a specific own-header mismatch error:

- `cross_module_same_shaped_impls_dispatch_each_callers_own_impl` (G1)
- `every_bare_ctor_site_in_one_module_grounds_at_the_callers_own_header` (G1a)
- `field_projection_reads_the_caller_grounded_mints_own_field` (G1b)
- `bare_ctor_arity_mismatch_with_the_callers_own_header_is_a_located_error` (G1c)
- `bare_ctor_kind_mismatch_with_the_callers_own_header_is_a_located_error` (G1d)
- `same_named_headers_of_differing_shapes_destructure_each_modules_own_layout` (G1e)
- `same_named_headers_of_differing_shapes_pack_each_modules_own_field_values` (G1f)
- `cross_module_same_shaped_impls_eager_minter_wins_regardless_of_caller` (G2r)

`G2` (`cross_module_same_shaped_impls_via_named_instantiation_dispatch_each_callers_own_impl`)
already has each caller name its own instantiation explicitly (`mk`), so it never
reaches the pre-guard's foreign-arg-borrow path at all and should be unaffected — verify
during implementation rather than assuming.

The unit tests directly exercising `bare_generated_word_own_module_grounding` (via
`ground_in_module_3`, `terms.rs` `#[cfg(test)] mod tests`, ~line 5541 onward) construct
`ctx.modules()` as `None` in the existing harness (per the S9/S10 test-layout convention
documented at `terms.rs` around the S10 unit block: "the default harness passes `None`,
which never reaches the new check at all"). Reusing `foreign_single_candidate_grounding`
means the pre-guard now needs `ctx.modules()` data at this call site too, so this test
harness assumption needs revisiting for the new gated units.

## What decides whether a fixture needs an import add vs. is expected to newly error

For each currently-green G-series test: if the interview's intent is "preserve the
existing behavior wherever the reachability rule would already permit it," add the
missing import (or hub re-export) to the fixture's caller module so it's an intentional,
in-scope rewrite — not a silent behavior loss. Where a golden's whole *point* is
`a` not needing to know about `b` (if any exist — audit before assuming), that is a
genuine behavior change to call out explicitly in the spec's non-functional requirements,
not something to route around by weakening the gate.

## Scope

- `src/check/terms.rs`: `bare_generated_word_own_module_grounding`'s foreign-candidate
  branch (~terms.rs:1930-1945, the `foreign_single_candidate_grounding` call site is
  the pattern to replicate/reuse — do not reimplement the reachability logic a second
  time; consider extracting a shared helper if the two call sites would otherwise
  duplicate the walk).
- `tests/phase7b_slice9.rs`: audit and, where required, rewrite the G1-family fixtures
  per the blast-radius list above.
- `src/check/terms.rs` unit tests (`ground_in_module_3` harness): extend to pass a
  non-`None` `ctx.modules()` where the new gate needs it.
- `docs/roadmap/P7b/slice11-spec.md` §Blast radius: the "S9 pre-guard exemption stands"
  bullet is now stale once this lands — supersede it, don't leave it contradicting the
  new behavior.
- `docs/roadmap/P7b-higher-kinded-types.md`: mark this ruling closed and record the
  slice landing, per project convention (`ROADMAP/DESIGN: no history` — current state
  only).

## Out of scope

- No change to `foreign_single_candidate_grounding` itself beyond whatever extraction
  is needed to share its logic with the pre-guard.
- No change to the S10 headerless-caller shape's own goldens (`tests/phase7b_slice10.rs`).
- No revisiting of the `dp_g`/`dp_g2`/`dp_g3` declaration-order-first ambiguity gap
  (`slice11-probes.md`) — that's a separate, still-undecided finding, not this ruling.

## Open questions for spec-writer to carry forward (not user decisions — verify in code)

- Whether `G2` truly never reaches the pre-guard's foreign-borrow path (stated above as
  a working assumption) — confirm by reading the fixture against `mk`'s explicit
  instantiation before writing the phase plan.
- Whether any other test file beyond `phase7b_slice9.rs` exercises this pre-guard's
  foreign-arg-borrow success path (grep beyond slice9/slice10 before finalizing the
  fixture list).
