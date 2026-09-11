# P7b.S6d — Slices as Iterator targets (the built-in-view lift)

Status: Implemented (branch `soo-38`, tracking **SOO-38**; base `b777cbb`).
Reference documentation. The full delivery-plan spec is preserved in git
history before commit `adfe676`.

## What was done and why

`Slice['T]`/`!Slice['T]` are runtime-length views (`Type::Slice`, interned
per `(element, mutable)` with no count) that have existed since P7.S3c, but
they could not dispatch through the `Iterator` protocol. `impl: Iterator for
Slice[i64]` parsed as an impl target, yet `next`'s App-headed member row
(`'It['T] -- Step['T 'It['T]]`) hit the S2-6 concrete-target fence
(`member_app_concrete_target_error`, `src/ast.rs`): a built-in slice carries
no ctor header for `ImplTarget::is_mono_ctor_app` to recognize, so the P7b.S8
Range lift (which keys on a `GenericId` into the struct/enum registries) did
not transfer. `Type::Slice` comes from `intern_slice_type`, keyed structurally
with no header. The generic spelling `Slice['T]` fails even earlier
(`unknown type 'T`) and is not a workaround.

The earlier PREREQ slice closed the deeper wall (a `Step[i64 Slice[i64]]`
monomorph lays out as a two-word slot; storage/return bans are located; the
in-frame deriv/alias propagation sites are live). This slice is the residual
checker/target-grammar work to make slices dispatch. **No `src/ir/` change.**
Delivered size was **M** (the roadmap's original `S-M` estimate was corrected).

Key decisions, as shipped:

- **Shared `Slice[i64]` only.** The mutable `!Slice[i64]` target stays out:
  Ruling A's enum-payload sweep fires first at the `Step` declaration span.
  `Slice['T]` generic-element targets are an unreachable spelling, not a
  deferral. Deferral pointers: SOO-45 (mutable), SOO-48 (generic-element).

- **Fence lift = a two-half sentinel patch, not a new `PolyType` variant.**
  The parser wraps the slice element in a *fake* `PolyType::Generic` carrying
  a reserved sentinel index (`SLICE_SENTINEL_IDX = u32::MAX`), grounds the
  App-headed member row against it, then rewrites it back to
  `Concrete(Type::Slice(..))` before any code reads its bogus header. This is
  a momentary, deliberately documented violation of `PolyType::Generic`'s
  invariant, carrying mutual pointers between the construction site and the
  invariant doc. The dispatch half reads the already-grounded member word in
  `resolve_mono_member_call`. The recorded (unbuilt) fallback if the invariant
  exception were rejected was a dedicated `PolyType::SliceApp` variant —
  bigger and touching every exhaustive match; not taken.

- **Per-impl `inline` keyword.** Impl members gained an optional `inline`
  spelling; `declares_inline` is the impl's when present, else inherited from
  the trait member. This is load-bearing, with no fallback: the member-output
  ban forces the slice `next` to be inline (a `Step` carrying a view may not
  cross a non-inline member's return boundary), and marking the *trait* member
  inline is unacceptable because it re-routes 6 List/Range goldens through the
  inline-poly-member nullary-ctor mis-resolution. Those 6 canaries pin that
  trait-level behaviour is unchanged.

- **A pre-existing Step-monomorph clobber fix (candidate visibility, not
  span-keying).** Co-locating the slice impl in `lib/core/iterator.sth` mints
  a `Step[i64 Slice[i64]]` monomorph for every importer beside the Range
  impl's `Step[i64 List[i64]]`. The bare-name candidate lookup's env-hit arm
  returned only the single parse-time-minted monomorph and never consulted
  `mint_fallback_candidates` (which ran only on the sibling env-miss arm), so a
  second check-time-minted monomorph of the same generated enum was invisible
  and re-typed List consumers' `More>` resolution. The fix unions the env-hit
  candidates with the live check-time mints, keyed per-monomorph so each
  bare-name site still resolves to its own. `select_overload_fallback_sourced`
  was exonerated (it operand-filters first, so a wrong-shaped monomorph cannot
  survive its match) and left untouched.

- **Borrow-join union + `back_edge_outs` deriv forwarding, together.** A
  rootless one-sided join asymmetry now unions instead of refusing, at BOTH
  join sites (`check_branch_join` and `merge_arm_output_slot`), conditioned on
  `owned_root.is_none()` — the same predicate the back-edge guard and
  `carried_borrow` already use. `back_edge_outs` forwards `deriv` alongside
  `surviving`. The two ship together for soundness: the join-union alone would
  turn `while`'s loud back-edge error into a silent laundering channel for any
  shape that cannot recover the deriv from a sibling arm. Rooted one-sided and
  two-root asymmetries stay refused byte-identically. The disjointness
  argument: naming does not always mint a rootless reborrow —
  `Provenance::reborrow` inherits an existing root; naming is rootless only
  when the binding carries no held deriv (a parameter-seeded slice slot is
  `Slot::computed`, deriv-free). A rooted deriv survives a reborrow, so no
  known spelling can launder a rooted asymmetry into the union's rootless case.

- **The library impl, mono consumers only.** `impl: Iterator for Slice[i64]`
  ships in `lib/core/iterator.sth` with `: next inline ...` over
  `len`/`&>`/`subslice`/`More`/`Done`. `for_each`/`fold` and any bound-generic
  consumer stay closed at the `'It['T]` slot unification (no ctor head; SOO-60
  territory). Mono consumers work: self-tail recursive drain, non-tail
  recursive drain, unrolled bounded stepping, and the while-threaded drain
  (enabled by the join/back-edge pair).

### Load-bearing invariants (unchanged)

No `Type::Slice` carve-out in `contains_reference`; no second `Iterator`
protocol — the sentinel is a grounding-path device, not a predicate
relaxation. No `src/ir/` change; `Ptr[T]` stays opaque and the two-word slice
slot is PREREQ-shipped. Every intermediate commit is at least as safe as the
base: the existing bans and refusals are byte-identical throughout.

### P5 discovery: the consumer-type tie-break

Shipping the library impl made two parse-time mints of one generated-enum
header a permanent library fact (the slice impl's `Step[i64 Slice[i64]]`
beside the Range impl's), making nullary-ctor sites with two same-header
candidates reachable — the one shape the operand filter cannot discriminate.
The shipped answer is a strictly-narrowing consumer-type tie-break
(`generated_enum_consumer_type_pick`, `src/check/terms.rs`), firing only where
the operand-filtered set still holds 2+ candidates of one generated-enum
header AND a unique `consumer_expected_type` match exists; no unique match
keeps today's exact Ambiguous bytes. This deliberately retired the slice8b
fence golden per that fence's own doc prescription, renamed
`two_instantiations_ground_a_bare_nullary_variant_from_its_consumer_type`
(`tests/phase7b_slice8b.rs:342`).

## Implementation

- **Per-impl `inline` desugar (REQ-3)**: `adfe676` — `src/parser.rs` makes
  `declares_inline` impl-spelling-first at the member-inheritance point.

- **Two-half sentinel fence lift (REQ-2)**: `a43f160` —
  `SLICE_SENTINEL_IDX`/`slice_sentinel`/`rewrite_slice_sentinel`
  (`src/parser.rs:880-930`) plus the slice branch in `parse_impl_member_body`
  preempting both the `is_mono_ctor_app` and `is_concrete` arms; the
  `PolyType::Generic` invariant-exception doc (`src/ast.rs`); the dispatch arm
  in `resolve_mono_member_call` (`src/check/poly/ground.rs`).

- **Step-monomorph clobber fix (REQ-4)**: `d4e55a2` — the env-hit candidate
  arm in `src/check/terms.rs` (~`:990-1012`) unions with
  `mint_fallback_candidates`'s check-time mints, keyed per-monomorph.

- **Borrow-join union + back-edge deriv forwarding (REQ-5)**: `08382e6` — the
  `owned_root.is_none()` union at `src/check/terms.rs:3795` and the
  eliminator-merge site in `src/check.rs`; `back_edge_outs`
  (`src/check/terms.rs:3963`) forwards `deriv`. The PREREQ white-box guard
  `back_edge_outs_forwards_surviving_set_along_index_map` stays green.

- **Library impl + consumer goldens + P5 tie-break (REQ-6)**: `6c3f7ba` —
  `impl: Iterator for Slice[i64]` in `lib/core/iterator.sth:80`;
  `generated_enum_consumer_type_pick` (`src/check/terms.rs:3160`, units at
  `:6218`/`:6254`); the slice8b fence retirement
  (`tests/phase7b_slice8b.rs`).

- **Docs corrections**: `969586c` — `docs/roadmap/P7b-higher-kinded-types.md`
  (admissions line, `&>` read, `Size` `S-M`→`M`, sentinel-patch mechanism) and
  `docs/roadmap/ROADMAP.md:56`.

### Tests

The golden set lives in `tests/phase7b_slice6d.rs` (27 tests, added across
`a43f160`/`d4e55a2`/`08382e6`/`6c3f7ba`), including the exit criterion
`list_and_slice_impls_drain_through_one_imported_protocol` (`:694`) and the
while-drain `while_threaded_slice_drain_runs_once_back_edge_outs_forward_deriv`
(`:628`). Two pre-existing suite canaries pin the clobber fix:
`two_asymmetric_instantiations_eliminate_independently_in_one_word`
(`tests/phase6_slice3b.rs:208`) and
`a_two_parameter_generic_enum_is_eliminated_at_swapped_monomorphs`
(`tests/phase7_slice12.rs:681`). G12 (the `ast.rs` unreachable-panic negative
control) was a manual implementation-time spike, not a suite golden. The
slice8b fence golden was retired to the consumer-type tie-break test named
above.
