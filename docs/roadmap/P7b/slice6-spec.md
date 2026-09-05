# P7b.S6 spec — container traits, linear merge, and `List['T]` in core

**Status: Implemented** — all six phases landed, reviewed, and committed on branch
`p7b-s6` (base `0becd89`, the brief/probes/spec commit): `2d1e309` (phase 1),
`a26c9ec` (phase 2), `b8f1cb0` (phase 3), `465937e` (phase 4), `e0f9ea1` (phase 5),
`3c8d215` (phase 6, which recorded the array-as-constructor deferral to S6b rather
than landing it — a well-formed outcome per R8.4's fence, below). Scope input was the
recon [brief](./slice6-brief.md) (findings F1–F7, open questions Q1–Q4) and the
verbatim [probe log](./slice6-probes.md); those two documents are the historical
record of the pre-implementation state and are preserved unchanged.

## What was done and why

The roadmap billed S6 as "the tier-1 library slice, unblocked by S4". That framing was
wrong: the recon round tested every container-trait member shape from a *monomorphic*
call site, where everything already worked, but the exit criterion asks for dispatch
*through shared bounds* — a polymorphic body — and two on-path cases **panicked the
compiler**. S6 turned out to be a compiler slice with a library payload, and its
substance is three compiler fixes with the library and goldens riding on top:

1. **A quotation-taking member, called through a shared bound, ICE'd** when the
   forwarded quotation parameter's effect was written *concrete* (`[ i64 -- i64 ]`).
   The operand folded to `PolyType::Concrete(Type::Quotation(..))` while the declared
   member input stayed `PolyType::Quotation(..)`; `unify_member_operand` had no arm
   bridging that representation gap, and the resulting mismatch diagnostic then indexed
   a member-local var off the end of the *caller's* var table and panicked. Two
   distinct defects: a diagnostic-render bug on top of a real rejection. The
   generic-effect spelling (`[ 'A -- 'A ]`) already worked at HEAD, and a written
   quotation *literal* already produced a clean located error — neither needed work.
2. **Giving core's `List['T]` any trait impl at all ICE'd.** `^List['T]` is a
   parser-accepted variant field, so a generic enum variant field *can* be
   `OwnedCell(Generic { is_enum: true, .. })`, but two twinned `unreachable!` arms (the
   destructure walk and the construction-bind walk) both asserted it never was.
3. **A nullary member (`empty`) could not ground from a mono body** even with an
   explicit `empty[i64]` instantiation, because the concrete-target guard rejected any
   explicit type list — meaningless for a normal member, but the *only* way to ground a
   zero-dispatchable-input member.

### The fixes, by commit

- **`2d1e309` (Phase 1, R2.a) — the diagnostic, ICE first.** Split
  `trait_member_operand_error` into `expected_sig`/`found_sig` and render each operand
  by provenance (raw member input against the member sig, caller slot against the
  caller sig), dropping the mixed-provenance `substitute_member_var` call so no index
  can run off either table. This turned the concrete-effect ICE into a located error,
  giving Phase 2 an error message to debug against instead of a panic.
- **`a26c9ec` (Phase 2, R2.b) — the cross-representation bridge.** One new
  `unify_member_operand` arm bridging a declared `PolyType::Quotation` against a found
  `PolyType::Concrete(Type::Quotation | InlineQuotation | OwningQuotation)`,
  reconstructing the concrete quotation's rows and unifying them structurally,
  honouring the inline/owning flavour. Both call sites (the erroring dispatch and the
  silent viability filter) are covered. Confined to trait-member operand unification;
  it does **not** relax "a quotation in a generic body is spliced where it is written"
  for combinators, and it does **not** import poly-site literal materialization (a
  written literal stays a located error by design).
- **`b8f1cb0` (Phase 3, R3/R7) — `List['T]` into core, and impls for it.** Added the
  missing `OwnedCell(payload)` arm to both twinned catch-alls by plain substitution
  (recurse into the payload and re-wrap), correcting the false-premise doc.
  `lib/core/list.sth` (`type: List['T] | Nil | Cons 'T rest ^List['T] ;` plus its
  exports) joins the `core` package's `module:` line, mirroring `option.sth`. `List`
  is deliberately **not** added to the prelude hub: a hub re-export carries words only,
  not type names, so it would be a partial and misleading surface. List trait members
  are **non-inline** (the natural default): they mint a real `IrFunc` and lower as
  ordinary calls, so there is no recursion wall — only `inline` combinators face
  splice-budget limits, and non-tail self-recursion in those is rejected at check time.
- **`465937e` (Phase 4, R4/R5) — mono `empty`, explicit instantiation only.** A
  zero-dispatchable-input branch in `resolve_mono_member_call` grounds the trait
  variable from an explicit `type_args` list and runs `find_bound_impl`, plus a narrow
  carve-out of the concrete-target guard (S2-16) for zero-dispatchable-input members
  only. Bare `empty` with no instantiation is a located error naming the `empty[i64]`
  remedy. Consuming-context grounding (deferred-slot inference) was ruled a **future**
  follow-on slice, not attempted-and-degraded here: sole-impl grounding and
  mirroring the nullary-variant-ctor path were both rejected (silently wrong on a
  second impl; separate mechanism), and full deferred-slot inference is out of a
  slice's budget. The scope fence holds: an operand-carrying member is byte-unchanged.
- **`e0f9ea1` (Phase 5, R1/R6/R6a/R9) — the surface, goldens, roadmap edit.** The
  container-trait surface (`Monoid`/`combine`/`empty`, `mconcat`) landed as golden
  programs in `tests/phase7b_slice6.rs`, not as shipped `lib` modules — only
  `list.sth` (Phase 3) ships. `mconcat` uses the bound-quotation-parameter spelling
  (`['F: Foldable 'T: Monoid] ( 'F['T] [ 'T 'T -- 'T ] -- 'T ) empty swap fold`),
  forwarding a generic-effect parameter through the already-working arm; it grounds
  over both `Option` and `List`. The exit-criterion dogfood maps and folds over
  `Option`/`Result`/`List` through shared bounds. R9's two rejecting linearity fixtures
  are attributed to the *general* mechanisms that fire (arm-shape parity, `call` arity
  underflow), **not** a Foldable-specific linearity rule — S6 adds none, and
  forgetting-via-`drop` stays legal per DESIGN.md. The phase doc's S6 paragraph was
  rewritten to current state (no history).

### Recorded walls (not fixed here)

- **`Monoid for List` / `Functor for List` — dropped from the goldens.** A real linear
  merge or `map` over `List` reconstructs a `Cons`, which panics in
  `poly_bind_construction_arg` on a bare `PolyType::Generic` field the Phase 3
  `OwnedCell` arm does not cover — distinct from M4's twinned arms, and *not* about
  recursion (a single non-recursive `Cons` inside any trait-member body over `List`
  reproduces it). `Foldable for List` (destructure-only, no reconstruction) is
  unaffected and landed in Phase 3. Fixing the construction-arm gap is a future slice's
  job. Witnessed by `monoid_for_list_append_construction_wall_is_recorded`.

### Rulings resolved as ruled

- **Q1 — mono `empty` via explicit instantiation only.** Ruled in R4/R5; landed as
  above.
- **Q2 — S6 owns the array-as-constructor widening, last and fenced.** Ruled in R8;
  the fence fired and the widening carved out to S6b (below).
- **R3's struct twin** — reachable via `^Self['T]`, but already covered pre-slice by
  `substitute_generic_field`'s `OwnedCell` arm and `instantiate_struct`'s
  memo-before-substitute; no new code (verified by the existing
  `instantiate_struct_pushes_memo_key_before_substituting_fields`).

## Phase 6 — array as a constructor (R8): carved out to S6b

R8 proposed making `array` a valid `impl:` target so it could become a
Functor/Foldable instance, in three pieces: `PolyType::App` gains `len_args`
(mirroring `PolyType::Generic`); a constructor identity for `array` spanning
`CtorImage`, `GenericId`, and `PolyType::Generic`'s `(is_enum, idx, module)` triple;
and `parse_impl_target` publishing real per-variable kinds instead of a hardcoded
`vec![Kind::Star]`. R8.4 fenced it to run **last**, after every exit-criterion phase
was green, precisely so the critical-path fixes (M3/M4) could not be starved by it.

**`3c8d215` recorded the carve-out; no `src/` file was edited.** The occurrence counts
re-verified clean against HEAD before any edit (`CtorImage` 127, `GenericId` 31,
`PolyType::App` 87, all within a point of the spec's sizing). The measured reason the
fence fired is the enduring finding here:

The `is_enum`-keyed sites split into **read-the-id** (mechanical: carry the triple
through unchanged — the majority, safe to widen) and **match-the-shape** (semantic:
use `is_enum` as a binary switch into a registry indexed by `idx`). The
match-the-shape sites route `is_enum: false` to `generics.structs[idx]`, a
`GenericStructDecl` that `instantiate_struct` walks to mint a concrete decl, and
`is_enum: true` to the enum twin. **`array` has no such declaration.** It is a built-in
`Type::Array` whose registry is *content-addressed* by `(element, count)`, not
header-indexed by `(idx, module)`. Making `array` a third `GenericId` case flowing
through the existing `is_enum`-binary sites would need a fourth registry
(`GenericArrayDecl`) plus an `instantiate_array`/`lookup_array` pair written from
scratch to bridge header-indexed identity onto the shape-indexed `ArrayDecl` registry
— **new machinery, not a widened match arm**, at exactly the `ir/driver.rs` sites R8.2
flagged as easy to miss. That is what R8.4's fence exists to catch before a partial
widening ships half-wired.

**Ruling: arrays do not become Functor/Foldable instances in S6.** The App-len-args
widening and the constructor-identity widening carve out to a future **S6b**, scoped as:
design the array-constructor registry bridge (`GenericArrayDecl` or equivalent, plus
`instantiate_array`/`lookup_array`) as its own piece before touching any
`is_enum`-binary call site. `App.len_args` could land independently but has no consumer
without the rest, so it carves out too rather than shipping as unmotivated churn. This
ruling satisfies the exit criterion's own wording, which named arrays only as a probe
with "or the kind story gets a ruling" as an accepted outcome.

## Exit criteria

- Container-trait members (map/fold) dispatch **through shared bounds** for both the
  generic- and concrete-effect quotation-parameter spellings; the written-literal shape
  is a located error, not a panic. **Met** (Phases 1–2).
- `List['T]` lives in `core`, takes a non-inline trait impl (`Foldable`), and drops
  linear payloads per instantiation, each witnessed by a golden. **Met** (Phase 3);
  `Monoid`/`Functor for List` recorded as a construction-arm wall, not landed.
- `empty[i64]` dispatches from a mono body; bare `empty` errors with a remedy;
  operand-carrying members are byte-unchanged (both S2-16 pins hold). **Met** (Phase 4).
- The dogfood maps and folds over `Option`/`Result`/`List` through shared bounds; no
  new Foldable-specific linearity rule. **Met** (Phase 5).
- `array` is a valid `impl:` target **or** the carve-out ruling to S6b is recorded with
  its measured reason. **Met via the ruling** (Phase 6).

New goldens live in `tests/phase7b_slice6.rs`.
