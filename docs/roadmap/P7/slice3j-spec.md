# P7.S3j — A shape-changing combinator parameter declaring a slot above its row

**Status: implemented.** A row-typed inline combinator whose quotation parameter declares
fixed slots *above* its output row (`~[ ..a -- ..b i64 ]`, e.g. a hand-written `pick`)
grounds when called from a non-inline generic body. Previously only the monomorphic splice
path handled the declared trailing suffix; the polymorphic path read the produced row
straight off an arm's exit and rejected the parameter as un-groundable.

## What shipped

The monomorphic fixed point (`check_literal_against_declared_effect`'s shape-changing
branch, `src/check.rs:2124`) is now ported into `src/check/poly.rs`:

- **`poly_declared_arm`** no longer rejects a non-empty `outs` in the shape-changing arm
  (`(Some(a), Some(b)) if a != b`); the grounded suffix rides through on `DeclaredArm::outs`.
  A variable-carrying suffix still fails the upstream `ground(...)` and keeps the
  pre-existing `poly_combinator_abstract_signature_error` ("cannot ground") rejection.
- **`ArmRule::Row(u32, Vec<Type>, String)`** carries the declared suffix types and the
  rendered declaration alongside the output row id.
- **`poly_combinator_call`'s walk closure** splits `suffix.len()` slots off each arm's exit,
  checks length *and* per-slot type against the declared suffix, and feeds only the stripped
  region to `shape_baseline` cross-arm agreement. The call's exit row is the stripped `..b`.
- **Pre-walk sibling guard** (`src/check/poly.rs:2385`): arms sharing an output row id must
  declare one common suffix. Rejected before any arm walks, for `ArmRule::Fixed`'s reason —
  no arm-against-sibling rule holds a lone arm, and stripping each parameter's own suffix
  off its own arm would otherwise hide the difference from the cross-arm rule, leaving a slot
  the exit row has no account of. Reuses the "cannot ground" wording.

### Deviations from the original plan

- **A new diagnostic was needed** (the plan said none): `poly_arm_declared_suffix_mismatch_error`
  (`src/check/poly.rs:4458`), the `ArmRule::Row` twin of `poly_arm_declared_effect_mismatch_error`.
  The latter's second line ("a non-shape-changing quotation parameter carries one row…")
  is false for this case.
- **R3's regression did not pass unchanged.** `a_slot_declared_above_a_produced_row_is_located`
  keeps its source byte-for-byte, but its asserted message changed: the malformed `bad`
  (`-- 'T i64`, the un-stripped row) now *grounds* at the `pick` call — nothing at that call
  site can tell which of the two enclosing declarations is wrong, since `bad` declares no row
  for `..b` to answer to. It is caught one step later by the ordinary declared-vs-actual check
  (`stack effect mismatch in \`bad\``). The backend row-length panic in
  `src/ir/func_builder/quotation.rs` stays unreachable: `bad` never typechecks.

## Tests (`tests/phase7_slice3b_follow.rs`)

| Test | Guards |
| --- | --- |
| `a_slot_declared_above_a_produced_row_is_stripped_and_grounds` | R1/R2 golden: builds and runs, stdout `"5\n"` |
| `a_slot_declared_above_a_produced_row_is_located` | R3: malformed enclosing effect still rejected, now as an output mismatch |
| `a_suffix_slot_disagreeing_with_the_declared_type_is_error` | the per-slot type half of `suffix_matches` |
| `a_suffix_shorter_than_the_declared_row_is_error` | the length half — `zip` truncation would otherwise let an empty tail vacuously agree, ICEing in `ir/func_builder/calls.rs` |
| `arms_sharing_a_row_with_different_declared_suffix_types_is_error` | sibling guard; without it, `qbe: invalid type for operand … in phi` |
| `arms_sharing_a_row_with_different_declared_suffix_lengths_is_error` | sibling guard, the worse half: silently miscompiles without it |
| `an_abstract_declared_suffix_is_still_the_cannot_ground_rejection` | a `'X` suffix keeps the pre-existing rejection |

Each guard was mutation-tested: deleting what it guards leaves the rest of the suite green.

## Untouched

The monomorphic path, the eliminator, the `if`/`times`/`unless` family, and the
`inline`-splice route (`inline_generic_body_still_splices_a_row_combinator` pins
it). `poly_combinator_abstract_signature_error`'s *rejection* stays for a
variable-carrying suffix, but its *wording* was narrowed (dropped the
now-invalid "slot above a row" cause) since that ground is no longer a
rejection reason -- a necessary correction, not an untouched surface.
