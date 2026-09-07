# Spec: P7b.S8b — the construction-wall fix, the pre-existing two-defect pair it exposes, and traitful `List` members (`map`, `append`)

**Status:** Landed (Phases 1-4).
**Merge note:** `docs/roadmap/ROADMAP.md`'s P7b row conflicts with `main` at merge time
(main's row reads S1–S10 landed, pending S8b/S8c, after the S6d retitle commits
`e11f088`/`2683505`/`df9d55f`); resolve as "S1–S10, S8b landed; pending: S8c; S6d —
slices as Iterator targets".
**Discovery:** [slice8b-brief](./slice8b-brief.md), [slice8b-probes](./slice8b-probes.md)
(P8b probe battery, segfault root-cause, spellings log, ledger — landed `b667916`).
**Roadmap entry:** [P7b-higher-kinded-types.md](../P7b-higher-kinded-types.md), "P7b.S8b".

Implementing commits (on docs base `a1b1276`):

- **Phase 1** `f57264f` — per-instantiation variant words + impl-target seed channel
  (`src/check/poly.rs`, `src/check/terms.rs`, `tests/phase7b_slice8b.rs`).
- **Phase 2** `3d868d3` — construction-wall `Generic` field arm + witness flip
  (`src/check/poly.rs`, `tests/phase7b_slice6.rs`, `tests/phase7b_slice8b.rs`).
- **Phase 3** `f0c9d4f` — traitful `List` surface goldens: `map`, `combine`, `empty`
  (`tests/phase7b_slice8b.rs`).
- **Phase 4** `eeeab3f` — roadmap entry, growth re-check, permitted comment fix
  (`docs/`, `src/ir/func_builder/mod.rs`).

## Problem Statement (why)

Two blockers, one shared root, plus a pair of pre-existing defects the first blocker's fix
exposes. S6 recorded a **construction wall**: any polymorphic body over `List['T]` that
*reconstructs* a `Cons` panicked in `poly_bind_construction_arg`, because the
self-reference field `^List['T]` arrives as a bare `PolyType::Generic` that no arm covered
— the catch-all fired `unreachable!`. This blocked exactly the members the roadmap wanted
next: per-impl traitful `map` (Functor) and `append` (Monoid) over the real `core::list`,
S6's two dropped goldens.

Lifting the wall *exposes* a pre-existing S6-era **two-defect bug**: `empty[List[i64]]`
over a generic-target impl seeded the member's θ positionally instead of through the
impl-target equation (mono returned `List[List[i64]]`), and the wrong mint's variant words
clobbered the lowering-side bare-name last-write-wins variant map, so **every** `Cons`/`Nil`
construction program-wide lowered with wrong field shapes — a silent 40-byte layout
corruption that SIGSEGVs any program containing both a prior construction and an
`empty[<inst>]` call. The crash reproduces with no self-reference field at all (`Opt`), so
it is not caused by the arm — but shipping the arm without the pair-fix turns a
compile-time panic into a silent program-global miscompile for exactly the programs S8b
exists to enable. S8b therefore delivered three things as one slice: the wall arm, the
two-defect fix (required, not optional), and the traitful `List` surface as goldens.

## Requirements (what landed)

- **R1.** Bind a bare `PolyType::Generic` construction field in `poly_bind_construction_arg`
  positionally against a `Generic` operand of the same ctor identity (`is_enum`, `idx`,
  `module`), recursing over field args vs operand args — both P8 shapes ground: an impl
  member body constructing `Cons`, and a plain generic word constructing `Cons`.
- **R2.** Reject, with a located `poly_rendered_type_mismatch_error`, any operand
  reaching the new arm that is not a `Generic` of the same identity (including a
  `Concrete` operand) — never a panic, never a silent bind. Mirrors the `App` arm.
  Byte-exact for the ctor-mismatch case (`badcons`, an integration pin); the
  `Concrete`-operand case is asserted at unit level only
  (`.contains("type mismatch")`, `src/check/poly.rs:15085`) — no live program reaches a
  `Concrete` operand today, so only the ctor-mismatch pin is a real-program golden.
- **R3.** Reject, as a located error (dedicated `poly_generic_field_len_unbound_error`), a
  `Generic` field carrying a length **variable**: construction infers no lengths, so there
  is no slot to bind one into. The dedicated message earns its keep on quality -- a field's
  length-variable id lives in its own header's declaration space, which the caller's
  `len_var_names` cannot name, so a rendered mismatch would print a placeholder where a
  dedicated message says what is unbindable. The stored `^List['T]` is `len_args: []` today,
  but the shape is spellable now (`Ring['T 'N: Len] head 'T next ^Ring['T 'N]`).
  - **Amended (round-1 review):** the fence originally fired on any non-empty `len_args`,
    which rejected a **concrete**-length field (`Ring['T 3]`) with a message naming a
    length variable it does not have. A concrete length names no variable and needs no
    binding, so it now grounds when both sides agree and is an ordinary
    `poly_rendered_type_mismatch_error` when they do not -- honest there, because
    `poly_type_str` does render length arguments (`Ring[i64 3]` against `Ring[i64 5]`).
    Witnessed by `generic_field_with_a_concrete_length_grounds` and the two
    `poly_bind_construction_arg_generic_*_concrete_len_*` unit tests.
  - **Amended again (round-3 review, P0):** routing the concrete-unequal case to
    `poly_rendered_type_mismatch_error` sent shapes to `poly_type_str` that the old
    all-lengths fence had pre-empted, exposing a **pre-existing panic class**: the renderer
    indexed the caller's variable tables raw, so any variable id from another declaration
    space (a construction field carries *its own header's*) was an index-out-of-bounds ICE
    on the error path. It was reachable before this slice through a length-variable-carrying
    operand meeting a differently-headed field, and the amendment additionally made
    `Holder['T] r Ring['T 3]` -- a clean located error at `5d20aac` -- ICE, which is how the
    review found it. Fixed at the root by making `poly_type_str` total (`foreign_var_str`):
    an id its signature cannot name renders as `'?0` / `'?len0` / `'?row0` instead of
    panicking, and in-range rendering is byte-identical. The fence ordering is unchanged;
    it is now justified by message quality rather than ICE-safety, since the type-variable
    route into the same panic was never something this fence could cover. Witnessed by
    three goldens (`*_renders_a_foreign_field_var_as_a_placeholder`) and
    `poly_type_str_renders_a_var_past_the_sig_tables_as_a_placeholder`.
- **R4.** A nullary trait member called with an explicit type argument over a **generic,
  single-type-variable** impl target instantiates through the impl-target equation:
  `empty[List[i64]]` over `impl: Monoid for List` seeds the element var by matching the
  target `List['E]` against the call-site type (`'E := i64`), minting a monomorph returning
  `List[i64]` (24-byte Nil), never `List[List[i64]]`. The concrete-target nullary path
  (S6's `empty[i64]`) stays byte-unchanged.
  - **Fence (amended by the round-1 review):** for a **multi-variable** impl target
    (`Monoid for Pair['A 'B]`) the arity gate catches only the *short* spelling:
    `check_poly_call` compares the call site's `type_args.len()` against the member sig's
    *full* variable count (`build_member_var_union`), so one-argument `empty[Pair[i64 str]]`
    hits `instantiation_arity_error` before the seed channel runs. The **full-length**
    spelling does not: `empty[Pair[i64 i64] str]` carries two arguments for two declared
    variables, passes the gate, and reaches a seed that grounds *both* variables from the
    dispatch type alone — so the second written argument was read by nothing.
    `empty[Pair[i64 i64] str]` and `empty[Pair[i64 i64] i64]` minted the identical
    monomorph (`..._t0_i64_t1_i64`, confirmed by `nm`). Now a located conflict: past
    position 0 (which on this path names the *dispatch type*, not variable #0's value) a
    written argument must agree with what the target determined, and a variable the seed
    does not reach binds from it. Witnessed by
    `nullary_member_type_argument_disagreeing_with_its_impl_target_is_an_error` and its
    agreeing twin. The seed channel still carries types only (`Subst.len` is not threaded),
    so a length-carrying generic target cannot ground a nullary member this way (same
    family as R3).
  - **Asymmetry:** on the nullary seed-channel path a written type argument means "the
    dispatch type" (matched against the impl-target pattern); on the operand-dispatched
    generic-call path (S3t) the same written type argument still means "variable #0".
  - **Defensive:** the seed's `Some(empty)` positional-restore fallback has no spelled
    fixture (only an all-concrete target pattern reaches it; mutating its discriminator to
    `is_none()` survives the suite). Accepted as defensive coverage, recorded not
    fixture-built.
- **R5.** A checker-resolved enum construction/destructure site lowers with its own
  resolved instantiation's field shapes even when another instantiation of the same header
  is minted later. Per-instantiation resolution applies wherever the checker resolved an
  instantiation differing from the bare-name default. The bare-name last-write-wins
  variant-word map still correctly decides every site with no such resolution (all
  non-generic enums: recorded symbol IS the bare surface name). Eliminator call sites are
  out of scope (see Fences).
- **R6.** The Opt-shaped base repro (`Opt['T] | None | Some 'T`, `impl: Monoid for Opt`,
  no self-reference field, a prior `Some` construction, then `empty[Opt[i64]]`) builds and
  runs exit 0 — it SIGSEGVs at the base — pinned as a golden independent of the List wall.
- **R7.** The S6 recorded-wall witness flips from `..._construction_wall_is_recorded` to a
  positive golden `monoid_for_list_append_construction_builds_and_runs_clean` (build, run,
  drop clean); the pre-S8b panic text is preserved in the doc comment.
- **R8.** `impl: Functor for List` with the S6 signature (`map ( 'F['T] [ 'T -- 'U ] --
  'F['U] )`) grounds end-to-end: a consumer dispatching through a shared `Functor` bound
  with the `'U := 'T` specialization maps a real `List[i64]`, the member lowers as one
  non-inline real frame, and dropping the mapped list disposes the `Cons` chain per
  instantiation with constant stack (S6 destructor goldens do not regress).
- **R9.** `append` grounds through the ruled host trait `Monoid for List` (PB-5: `combine`
  = append, `empty` = `Nil`): `combine` through a shared `Monoid` bound appends two
  `List[i64]` spines (prints `1 2 3 5 3`); `empty[List[i64]]` grounds explicitly in a mono
  main. Bound-directed `empty` resolving at the `List` impl itself is **unverified** and a
  future item — constrained by the single-`List`-instantiation-per-program spelling fence
  (no nested `List[List[i64]]` helpers); the S6 `mconcat_over_list_dispatches` golden
  dispatches `Monoid for i64`'s `empty` and stays byte-identical.
- **R10.** Linearity teeth: an undropped `map` or `append` result is a compile error.
- **R11.** `dup` of a `List['T]` operand stays fenced byte-exact (`poly_copy_gate`,
  `Generic`/`App` arms; `poly_copy_generic_error`).
- **R12.** Spelling fences stay byte-identical: a `map` producing a distinct `'U` through a
  shared bound remains unspellable (inference does not bind output-only vars; quotation
  types rejected against plain-var slots; poly→poly quotation passing fenced). Verified
  substitutes (`'U := 'T` specialization, composition, mono middleman) recorded in the
  roadmap entry. Recorded fence, not a deliverable.
- **R13.** Ship surface (ruled): the traitful `List` surface lands as golden fixtures under
  `tests/` (S6 convention) — trait decls, impls, consumers live in the golden programs;
  only the already-shipped `lib/core/list.sth` and the unchanged `lib/core/sooth.pkg`
  ship. `src/ir/` stays behaviourally diff-empty for the whole slice; the sole permitted
  `src/ir/` edit is the comment-only correction at `src/ir/func_builder/mod.rs` (Phase 1
  falsified its "Empty on every corpus/test path" claim).
- **R14.** Non-functional gate: `cargo fmt --check`, `cargo clippy -- -D warnings`, full
  `cargo test` green; every pre-existing golden byte-unchanged (`mklist`, `Range`'s
  `next`, S8's 24 slice tests, S6's suite); unit tests beside every changed stage
  function; every new error pinned byte-exact.

## Ruled decisions

- **PB-5 (host trait for `append`, 2026-09-07 user decision interview):** `Monoid for
  List` (`combine` = append, `empty` = `Nil`). S8b closes S6's *whole* recorded wall —
  both dropped goldens (`Monoid for List`, `Functor for List`) land. User-authorized.
- **Ship surface:** S6 convention (golden fixtures), not an S8-style `core::iterator` lib
  module (R13).
- **Two-defect fix in scope:** required, as its own phase — the wall-lift is unusable
  until the pair is fixed; the fallback (keep the wall) trades a compile-time panic for a
  silent program-global miscompile.
- **Witness (R7):** flip to a positive golden, not retire.

## Fences recorded (pre-existing, no S8b action)

- **Second-instantiation nullary construction:** a bare nullary variant ctor of a generic
  header in a mono body grounds at the single first candidate regardless of expected
  output; the sig check catches it as a located mismatch. Adjacent to S6 R4's future
  consuming-context-grounding slice. (This is why `mconcat`-over-`List`-Monoid goldens
  must stay single-instantiation: `p8b-monoid-list-mconcat.sth` failed only from its
  second helper `: mkemptyof ( -- List[List[i64]] ) Nil ;`, not the `mkempty` spelling the
  S6 golden uses byte-unchanged.)
- **Missing-impl vs explicit instantiation:** an explicit type argument does not rescue a
  missing impl — `empty[i64]` with only `impl: Monoid for List` in scope fails as a
  missing-impl error. Distinct from S6 R4's consuming-context limit. Every `empty` golden
  must call `empty[...]` at a type with a dispatching impl present in that program.
- **Mono-body eliminator sites** (`List?` in `showlist`): route to
  `check_eliminator_call` ahead of the fix surface and record a resolution only under a
  splice — a mono-body eliminator never reaches the fix. Still correct whenever all
  instantiations agree; a miscompile risk only when two instantiations coexist with an
  eliminator call in a mono body. Same pre-existing class.
- **Struct-word twin + cross-module variant names:** two same-module instantiations of a
  generic *struct* sharing a bare ctor name in unrecorded mono sites (same last-write-wins
  class); cross-module same-named variant names in the flat `enums.words` map. No known
  miscompile repro; future slice.

## New-in-slice caveat (recorded, pinned)

- **Ordinary-word-walk double-record:** created by Phase 1's recording arm, not
  pre-existing -- a mono `inline` (combinator) word's body checked as an ordinary mono
  word records the construction span in `builtin_overloads`, which `calls.rs`'s read wins
  over any splice-keyed one. Benign at one θ per mono body (the flat map is not
  `span.module`-keyed, so cross-module same-named variants fall to the pre-existing
  bare-key hazard). Deliberate scope stop; pinned by
  `mono_inline_combinator_variant_construction_builds_and_runs`.

Also out of scope (brief + probes): iterator adaptors (`zip`/`take`/`rev`), lazy/streaming
iteration, a bound-generic `map` producing `'It['U]` (recorded impossible, P8-5b/5c), the
distinct-`'U` map spelling (R12), the D5 borrow gate, numeric traits, phantom parameters,
new trait-declaration syntax, the array-as-`'F` kind story (S6b), S8's per-`next`-call
frame question.

## Design notes (as landed)

- **Wall arm (Phase 2):** `poly_bind_construction_arg`'s catch-all gains a
  `PolyType::Generic` field arm — the `App` arm's twin with the header already concrete.
  Identity is the `GenericId` triple (compared as `match_impl_target_rec`'s `Generic` arm
  does); anything else takes the located mismatch. With identity matched the bind recurses
  positionally over field args vs operand args; grounding stays `apply_subst`'s job.
- **θ seeding (Phase 1, defect a):** `resolve_mono_member_call`'s nullary branch dispatched
  via `find_bound_impl` correctly but then handed the raw call-site `type_args` to
  `check_poly_call`, whose S3t seeding binds positionally (variable #0 = the impl header's
  element var, so `empty[List[i64]]` seeded `'E := List[i64]`). The fix derives the member
  θ from the impl-target equation (`match_impl_target`) and seeds it on a **separate
  channel** from `type_args` (an impl-target-derived seed is partial); the positional
  contract stays untouched for every other caller, and the explicit-`type_args` arity gate
  is unchanged.
- **Variant words (Phase 1, defect b):** a mono-body construction checked while only one
  instantiation exists takes the single-candidate arm of the term chooser, which recorded
  nothing for a non-splice generated enum word. The fix adds the resolved-symbol record as
  a further `else` branch (using `is_generated_enum_word`, `splice_enum_site`'s non-splice
  twin). Lowering needs **no change** — its `builtin_overloads` read already precedes the
  struct/enum arms and dispatches mangled enum keys — keeping `src/ir/` diff-empty and
  retiring the last-write-wins hazard for every checker-resolved site.
  - **Stop-and-record invariant honoured:** a checker-recorded symbol missing `enums.words`
    would ICE in `lower_resolved_word_call` (no bare-key fall-back). Guarded by the two
    lowering-side enum-dispatch tests plus the suite; the fix is checker-side-only by
    construction.
- **Traitful surface (Phase 3):** S6-convention goldens. Member bodies follow shipped
  non-inline patterns (`Foldable for List`'s recursive destructure, the witness's
  reconstructing append, the `Functor for Option` map body).

## Growth-structure verdict (Phase 4)

Re-check over every file touched (`src/check/poly.rs`, `src/check/terms.rs`,
`tests/phase7b_slice6.rs`, `tests/phase7b_slice8b.rs`): the new arm and its dedicated
error constructor sit beside existing same-kind binding/error-rendering arms; the
θ-seeding change extends the nullary rescue branch in place; `terms.rs`'s new
`is_generated_enum_word` sits beside its twin `splice_enum_site` and the single-candidate
arm's new `else if` mirrors the multi-candidate arm's existing `builtin_overloads` insert.
No import divergence, no function added that never calls its neighbors.
`tests/phase7b_slice8b.rs` is a new file but follows the established one-file-per-slice
golden pattern. `poly.rs` and `terms.rs` remain large (each one compiler-stage module) —
a standing size fact about the check stage, not a signal this slice introduced. **No split
warranted from this slice's diffs.**
