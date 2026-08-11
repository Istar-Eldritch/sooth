# Phase 4 — row grounding at the check sites

Slice 10a, phase 4 of 7. Delivers **R9** and **R10's message half**.

## What landed

The whole phase is one mechanism plumbed through five callers. A row-bearing
declared quotation parameter (`~[ ..s i64 -- ..s ]`) grounds its row `..s` to
the concrete caller-stack region below the combinator's fixed inputs. The
grounding happens **at the callee side**, in `check_literal_against_declared_
effect`, never in `apply_subst`.

- **`apply_subst` is left untouched.** Splicing a caller region into its
  *interned* `&'static QuotEffect` would mint an effect no literal and no
  forwarded parameter could ever equal again, breaking the abstract comparison
  at the poly forward arm. Phase 3's note already said this; nothing here
  contradicts it. `git diff src/check.rs` shows no change to `apply_subst`.
- **`check_literal_against_declared_effect` grows a `row: &[Type]` parameter.**
  The region is prepended below the declared inputs when seeding the `fresh`
  sub-stack, and required back unchanged on the exit row. Concretely:
  - `fresh = row.map(Slot::computed) ++ eff.inputs.map(Slot::computed)`.
  - the exit check compares `result` against `row ++ eff.outputs` (the carried
    region is a fixed point: N=0 leaves it untouched, N≥2 feeds one iteration's
    output into the next).
- **The prepend is type-only** (`Slot::computed`, so `deriv`/`surviving`/`quot`
  are dropped, R16). Prepending the caller's *real* slots would make the
  existing exit-row borrow guard flag a caller borrow riding untouched in the
  row as `quotation borrows place` — a false positive on correct code. Taking
  the region as `&[Type]` rather than `&[Slot]` makes that regression a type
  error, not a silent behaviour change. Pinned by
  `grounded_row_region_is_type_only_so_a_caller_borrow_is_not_flagged`.
- **The region is stripped before rendering** (R9/R10). The mismatch diagnostic
  builds `actual` from `result`, which now contains the prepended region; the
  strip (`result.iter().skip(row.len())`) keeps the caller's concrete stack out
  of the printed effect, so `declared`/`actual` show only the quotation's own
  fixed slots. Pinned by exact text in
  `row_grounding_mismatch_strips_the_caller_region`.

### The five callers, enumerated (R9)

Only the poly literal path passes a non-empty region; the other four pass `&[]`
because their `eff` is a `QuotEffect`, which carries no row:

| Caller | `check_literal_...` site | Region |
| --- | --- | --- |
| Poly literal splice (`check_poly_combinator_args`) | context 1 | `stack[..base]` when `pin` declares a row (`PolyType::Quotation(.., Some(_), _)`), else empty |
| Mono declared parameter (`inline_combinator`) | context 4, unreachable for `~` | `&[]` |
| `materialize_quotation_at_boundary` | erasure boundary | `&[]` |
| `if`-join, arm `a` | join erasure | `&[]` |
| `if`-join, arm `b` | join erasure | `&[]` |

### The four R9 contexts

1. **Known-literal splice** — the poly literal path. The row region is
   `stack[..base]`; per R4 the quotation's row is the signature's own top-level
   row, so it grounds to the same region the top-level row does.
2. **Abstract pass-down** — the poly forward arm. Unchanged: the forwarded
   parameter is compared as a row-free `QuotEffect` against the grounded
   `concrete` (both drop the row via `apply_subst`), so the comparison still
   works and the spliced callee body grounds the row itself.
3. **Definition-site, no caller** — the standalone body check grounds the row
   to the **empty** region. A row-bearing `~` combinator's own `f call` reaches
   `check_abstract_quotation_call` with a row-free `eff`, so the empty region is
   implicit and no code change was needed. Pinned by
   `row_bearing_combinator_checks_standalone_with_empty_region`.
4. **Mono declared parameter** — unreachable for a `~`. `inline_combinator`
   branches on `word.poly.is_some()`; a `~`-bearing signature (row-bearing or
   not) is poly-forced (phase 1/2), so it always routes to
   `check_poly_combinator_args` and never to the monomorphic path. The test
   asserts the **routing** (`WordDef.poly.is_some()`), not the absence of an
   error.

`check_abstract_quotation_times` was **not** generalised (spec instruction): the
intrinsic keeps its own row-preserving check, untouched.

## R10 — the message half

Phases 1–3 already made the declaration-site renderers row-aware (`Type::name`,
`poly_type_str`, `poly_quotation_type_mismatch_error`, and the `unify_poly_input`
let-else). Phase 4's only message work is the **strip**: the grounding mismatch
renders the quotation's fixed effect with the caller's row region removed, so
the caller's stack never leaks into a printed type. The declared/actual types
are the row-free `quotation_type(eff.inputs, ...)` on both sides — consistent,
and with the row provably absent (the anti-leak assertion is pinned to exact
text, not a substring that survives the row vanishing).

## Tests

Six integration goldens in `tests/phase4_slice10a_inline_quotation.rs`, all
built around `apply-with ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s )` — `times`'s
signature shape minus the back-edge (phase 5), so they exercise R9 without
depending on the self-tail rewrite:

- `row_bearing_inline_quotation_grounds_and_runs` (context 1): `10 5 [ + ]
  apply-with` folds the row's top (`10`) with the fixed input (`5`) → `15`.
- `row_grounding_mismatch_strips_the_caller_region` (R9/R10): `[ dup ]` is a
  located mismatch rendering `[ i64 -- ]` / `[ i64 -- i64 i64 ]`, with the
  caller row provably stripped.
- `abstract_row_bearing_quotation_passes_down` (context 2): `outer` forwards its
  own row-bearing `~` parameter into `apply-with` → `15`.
- `row_bearing_combinator_checks_standalone_with_empty_region` (context 3):
  `apply-with` checks standalone, row grounded to empty, no call site present.
- `row_bearing_inline_quotation_routes_to_the_poly_path` (context 4): a
  row-bearing `~` signature sets `WordDef.poly`.
- `grounded_row_region_is_type_only_so_a_caller_borrow_is_not_flagged`: a live
  borrow `&v` riding untouched in the caller row is not flagged.

## R20 — mutation evidence

Each guard reverted individually against a scratch copy of `src/check.rs`,
confirmed to flip its test, then restored (verified byte-identical via `diff`):

| Guard | Reverted to | Flips |
| --- | --- | --- |
| R9 row prepend | `fresh` from `eff.inputs` only, ignoring `row` | `row_bearing_inline_quotation_grounds_and_runs`, `abstract_row_bearing_quotation_passes_down` (and, transitively, the mismatch and borrow tests); standalone stays green, correctly |
| R10 render strip | `actual` from full `result`, no `skip(row.len())` | `row_grounding_mismatch_strips_the_caller_region` (prints `[ i64 -- i64 i64 i64 ]`) |
| R9 type-only prepend | `row: &[Slot]`, prepend the caller's real slots | `grounded_row_region_is_type_only_so_a_caller_borrow_is_not_flagged` (false `borrows the enclosing place \`v\` (D3)`) |

## Green

`cargo fmt --check && cargo clippy -- -D warnings && cargo test`: 779 lib tests,
full integration suite including `qbe_baseline` (byte-identical) and the 22
`phase4_slice10a_inline_quotation` goldens (16 → 22). `git diff --stat` touches
only `src/check.rs` and `tests/phase4_slice10a_inline_quotation.rs`;
`lib/combinators.sth` is byte-unchanged and `apply_subst` is untouched.
