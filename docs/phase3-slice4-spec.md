# Phase 3 Slice 4 — Generalized recursive disposal (spec)

Design input: [the brief](./phase3-slice4-brief.md). Base: `main` @ `6f22576`, 679 tests
green. Revised after spec review round 1, which found the detection algorithm (R3) could
not actually find two of the three probe shapes it exists for, a real leak, a real
duplicate-block-label hazard, and two existing Slice 3 goldens that this slice must
invert rather than merely preserve. All three found by hand-testing against real
programs and hand-patched QBE, not by reading alone.

## Context: what is already true on the base commit

Verified by building and running programs, not by reading code.

- Three probe shapes all **compile and run correctly today**, producing balanced
  alloc/free traces: a wrapper-struct list (`type: Wrap v i64 next ^List ; type: List |
  Nil | Cons w Wrap ;`), a `^^Self` list (`type: L | Nil | Cons n i64 next ^^L ;`), and a
  mutual A/B chain (`type: A | ANil | ACons x i64 next ^B ; type: B y i64 z ^A ;`).
- All three **segfault identically to Slice 3's own pre-fix baseline**: exit 0 at
  N=50,000, SIGSEGV (139) at N=100,000, under the default 8 MB stack. None hits Slice 3's
  fused loop, because `recursive_loop_field`'s exact match (function at ir.rs:922,
  predicate `cells.payload[c.index()] == self_ty` at ir.rs:927) only recognizes a `^Self`
  field directly on the enclosing type.
- `begin_loop`/`finalize_loop` (ir.rs:1515, 1536) derive each phi's type from the value
  passed at the call site; nothing hardcodes one aggregate type. `FuncBuilder` has a
  single `header: Option<BlockId>` and flat `header_phis`/`back_edges` (ir.rs:1378-1397,
  the fields the `begin_loop`/`finalize_loop` doc comments describe) — **nested loops are
  not representable today**, which is why one loop iteration must be one full trip around
  a path, with no inner loop, rather than a design choice (see R7).
- `field_value` (ir.rs:2181) treats `Struct`/`Enum`/`Array` fields as an in-place
  aggregate projection (`field_aggregate_value`, ir.rs:2160, no free involved) and any
  other field type — including `OwnedCell`, not special-cased here — as a plain
  `FieldLoad` that reads the pointer itself, not its payload. `load_owned_payload`
  (ir.rs:1998) is the separate step that actually materializes a cell's payload and frees
  it, and it only allocates a reusable frame slot for an `Struct`/`Enum`/`Array` payload;
  a scalar payload — including a *nested* `OwnedCell`, the inner step of `^^Self` — is a
  plain `FieldLoad` with **no slot and no copyout hazard at that step** (ir.rs:1998-2016).
  `emit_drop` (ir.rs:2245) and `dispatch_on_tag` (ir.rs:2208) are generic over any
  value/enum, not tied to the entry type of a destructor. Together these confirm an
  intermediate struct or enum encountered partway along a longer recursive path is an
  ordinary in-frame value with no special representation.
- **Two existing Slice 3 goldens make claims this slice must invalidate, not merely
  preserve.** `mutually_recursive_types_dispose_on_recursive_path`
  (tests/phase0.rs:2852) asserts `assert_ne!(run_stack_bounded_golden("mutual-deep",
  …), Some(0))` for a 300,000-node mutual A/B chain, with the comment "the fallback
  fused-loop-less recursive path was proven to be taken" — this slice's whole point is to
  make that chain dispose in constant stack, so the assertion must become
  `assert_eq!(…, Some(0))`. `indirect_recursion_shapes_remain_depth_limited`
  (tests/phase0.rs:2906) asserts the same for a 1,000,000-node **wrapper-struct** list
  (`"wrapper-list"`) — that assertion must also invert. Its second assertion, for a
  **left-leaning tree** (branching, D1's territory, not this slice's), correctly stays as
  it is: it becomes the sole surviving proof that D1's one-edge narrowing was preserved.
- `synthesize_enum_destructor` (ir.rs:1008-1063) already contains the pattern that keeps
  the trailing `Ret` guard correct: it resets `b.terminated = false` immediately after
  `start_block` for **every** freshly-started variant block (ir.rs:1038), so the trailing
  `if !b.terminated { b.seal_block(Ret) }` (ir.rs:1059-1061) fires correctly whether or
  not that particular variant back-edged. `seal_block` itself (ir.rs:1494-1502) never
  touches `terminated` — the reset-then-check discipline is what makes it safe, and it is
  presently applied only once, at the entry dispatch. See R10.

## Requirements

### Detection: generalizing "the recursive field" to a path

- **R1 — One pass, not two.** `recursive_loop_field` is replaced by a general path-finding
  function (working name `recursive_disposal_path`, ir.rs, beside where
  `recursive_loop_field` lives today) that subsumes it entirely. Direct self-recursion
  (today's only detected shape) becomes the length-1 special case of the general
  algorithm, not a separately maintained fast path.
- **R2 — The path is a sequence of typed steps.** A discovered path is an ordered list
  where each element is one of:
  - **a byval projection**: extract a `Struct`/`Enum` field from the current aggregate
    (`field_value`, no free; entering an enum this way requires a tag dispatch at that
    point, exactly as R5 describes for a cell-reached enum), or
  - **a cell unwrap**: materialize a `^T` field's payload and free the cell
    (`load_owned_payload` + `free`, at any position in the path, not only the last).

  A direct `^Self` field is a path of one cell-unwrap step. The wrapper-struct case is one
  byval-projection step (into `Wrap`) followed by one cell-unwrap step. `^^Self` is two
  consecutive cell-unwrap steps with no byval projection between them. The mutual A/B
  case, from `B`'s side, is one cell-unwrap (`z: ^A`) followed by a tag dispatch on the
  unwrapped `A` (R5), then, in the continuing variant, one more cell-unwrap (`next: ^B`).
- **R3 — The walk is an explicit backtracking search, starting from every field of the
  entry type, not only its `^T` fields.** This corrects two independent gaps a first
  reading of an earlier draft had: an algorithm that only considered `^T`-typed candidate
  fields can never find the wrapper-struct shape (`List`'s `Cons` variant has no `^`
  field at all — the cell is one struct hop away, inside `Wrap`), and an algorithm with
  no case for "the current type is itself a cell" can never find `^^Self` (its second
  step's source type is `OwnedCell`, not `Self`, `Struct`, or `Enum`).

  Define `find_path(current: IrType, target: IrType, visited: &mut Vec<IrType>) ->
  Option<Vec<PathStep>>`, called initially with `current = Self` (the enclosing type)
  and `target = Self`:
  - If `visited` already contains `current` **and this is not the first call**: prune
    this branch (`None`) — a revisit means the walk wandered into some other type's own
    cycle, unrelated to `Self`. `visited` is scoped **per path attempt**: pushed on
    entry to `find_path`, popped on return, so an abandoned branch can never poison a
    sibling branch still being tried. This is what makes the search terminate: the type
    registries are finite, and a walk that never revisits a type has length at most the
    number of declared types.
  - Push `current` onto `visited`.
  - If `current` is a `Struct`: for its fields, in **reverse declaration order**, try
    each candidate in turn:
    - a field of type `^T`: if `cells.payload[c] == target`, this step alone completes
      the path (`Some(vec![Unwrap(c)])`); otherwise recurse
      `find_path(cells.payload[c], target, visited)` and, on `Some(rest)`, prepend
      `Unwrap(c)`;
    - a field of type `Struct`/`Enum`: recurse `find_path(field.ty, target, visited)`
      and, on `Some(rest)`, prepend `Project(field)`.

    Return the **first candidate (tried last-to-first) that yields a complete path** —
    a backtracking choice determined *after* a sub-walk succeeds, not a pre-walk
    syntactic guess at which field looks "worth" following. This generalizes
    `recursive_loop_field`'s existing `next_back()` tie-break (which already filters to
    *actually* recursive fields before taking the last one) to every level of the walk,
    rather than picking a field that merely looks plausible. If **no** candidate field
    of a struct yields a path, the whole struct-level candidacy fails and the caller's
    own backtracking continues to its next candidate. Non-chosen fields at this level
    are not specially marked as excluded from anything; they simply are not the
    continuing edge, and are dropped like any other field (R5).
  - If `current` is an `Enum`: for **each** variant independently, apply the struct rule
    above to its fields. **At most one variant may yield a path**; if more than one
    does, this is a branching cycle through an enum, out of scope under D1, and the
    whole candidacy at the level that reached this enum fails (the caller backtracks).
    A struct with two fields that both independently reach `target` is **not** the same
    situation and is not rejected: R3's last-tried-first-success rule already picks one
    deterministically (the same "loop the last, recurse the rest" shape Slice 3 already
    uses at the entry level, generalized to any struct level, direct or intermediate).
    The asymmetry — enum branching rejected outright, struct branching resolved by
    picking one — exists because an enum's non-chosen branch would need a second,
    divergent continuation emitted *inside* the same tag dispatch, which is real added
    codegen this slice does not need, while a struct's non-chosen field is already
    handled today as an ordinary dropped field, no new mechanism required.
  - If `current` is `OwnedCell(c)`: another cell layer; if `cells.payload[c] == target`
    this closes the path (`Some(vec![Unwrap(c)])`, `^^Self`'s exact shape), otherwise
    recurse into `cells.payload[c]` and prepend `Unwrap(c)`.
  - Any other `current` (scalar, `Bool`, `Array`, …): dead end, `None`.
- **R4 — The walk is detection-time only, over the static type graph; it is not a
  runtime concept.** No visited set, no cycle guard, and no double-free check exist at
  disposal time (D2/R9). R3's visited set exists purely to make the *search* terminate
  and to prevent one dead branch from hiding a path a sibling branch would have found;
  once a path is found it is fixed and walked once per loop iteration with no
  re-checking.

### Codegen: one fused loop per participating type, walking the whole path

- **R5 — Every level of the path drops its own non-continuing fields, and base cases
  can occur anywhere along the path, not only at the entry type.** At **every** struct
  level or enum-variant level the path visits — the entry type and any intermediate one
  reached via a cell unwrap or byval projection alike — every field that is not the
  path's continuing field is dropped via the ordinary `emit_drop`, in declaration order,
  exactly mirroring `synthesize_enum_destructor`'s existing per-variant loop
  (ir.rs:1042-1050: iterate all fields, skip the looped one, drop the rest), generalized
  to run once per path level rather than once at the entry. Whenever the path passes
  through an enum (entry or intermediate), a tag dispatch (`dispatch_on_tag`, reused
  as-is) is emitted at that point; every non-continuing variant drops its own fields and
  terminates the loop (`ret`), and the continuing variant's non-path fields drop before
  the walk proceeds to the next step. For the mutual A/B case, `B`'s destructor loop
  dispatches on `A`'s tag mid-loop, not at entry, since `B` is a plain struct with no tag
  of its own.
- **R6 — Every type on a cycle gets its own fused loop, entered from its own shape; no
  synthesized destructor calls another synthesized destructor to traverse the same
  cycle.** For the mutual A/B case, `drop_A` and `drop_B` are two independent loops,
  each the same cyclic path rotated to start its own tag dispatch (or lack of one, for a
  struct) first — each produced by calling the **same** generalized detection-and-codegen
  machinery on that type's own fields; the mutual case needs no special-casing beyond
  the mechanism already being general. This is a considered rejection of the
  simpler-looking alternative, "`drop_B` just calls the already-synthesized `drop_A` on
  the unwrapped payload": destructor synthesis **bypasses the tail-call machinery
  entirely** — the back-edge transform in `lower_call` is gated on `name ==
  self.cur_word_name` (ir.rs:1767), every synthesized destructor is built with an empty
  `cur_word_name` (`FuncBuilder::new(env, resolve, regs, String::new())`, ir.rs:957,
  1021, 1089, 1153), and disposal goes through `emit_drop` (ir.rs:2245), which emits an
  unconditional `Instr::Call`, never through `lower_call` at all. So a `drop_B`-calls-
  `drop_A` design is always an ordinary native `Call`, and verified in the emitted IL for
  the mutual probe: the cycle `enum_drop_0 → cell_drop_0 → struct_drop_0 → cell_drop_1 →
  enum_drop_0` is four plain `call`s, with `cell_drop` additionally allocating its own
  frame per hop — reproducing, and slightly worsening, exactly the O(depth) defect this
  slice exists to fix. (This is *not* a case of Phase 2 Slice 6's tail-call-to-loop
  lowering missing a tier-2 SCC-contraction case — that lowering never sees these
  functions in the first place, so the fix cannot arrive by that mechanism ever landing.)
  Inlining the whole rotated path into each participating type's own loop is what
  actually achieves constant stack.
- **R7 — One loop iteration is one full trip around the path, however many steps it
  has; there is no separate "hops per iteration" concept and no inner loop.** This is
  forced, not a preference: `FuncBuilder` has one `header` and flat `header_phis`/
  `back_edges` (ir.rs:1378-1397), so nested loops are not representable. The path's
  length is fixed and known at codegen time (R3 produces it once, statically); the loop
  body is the path's steps emitted in order — byval projections, field drops, tag
  dispatches, cell unwraps — ending with the back-edge feeding the final cell-unwrap's
  result to the header phi, exactly as today's single-step case does.
- **R8 — The copyout-ordering invariant generalizes to every *aggregate* cell-unwrap
  step in the path, independently.** Each distinct aggregate type (`Struct`/`Enum`)
  visited within one loop iteration gets its own hoisted frame slot via `push_alloc`
  (ir.rs:1480), reused every iteration for that type; the loop-carried phi's own type is
  only one such slot when the path touches more than one aggregate type (the mutual A/B
  case hoists two: one sized for `A`, one for `B`). Every read of data held in a given
  slot — a byval projection out of it, a field drop, a tag dispatch on it — must be
  emitted before the cell-unwrap step that overwrites that slot, checked independently
  per slot. **`^^Self` has exactly one such hazardous step, not two**: its inner
  cell-unwrap (`^^Self` → `^Self`) reads a scalar pointer via a plain `FieldLoad`
  (ir.rs:1998-2016, the non-aggregate branch of `load_owned_payload`) with no frame slot
  and therefore no copyout hazard; only the **outer** unwrap (materializing the
  aggregate `Self` value itself) uses a hoisted slot. Verification must use distinct
  `__spy` tags at every level that *does* hold an aggregate slot, since a violation
  there corrupts data while leaving the alloc/free trace balanced, exactly as Slice 3
  found for the single-step case.
- **R9 — No aliasing or double-free guard exists anywhere in this mechanism, and none is
  needed.** `^T` ownership is exclusive and struct/enum setters are whole-value
  functional transforms (`S<fi : ( S Ti -- S )`), so no value this slice's types can
  build has an actual runtime cycle: every type-level cycle these three shapes legalize
  still produces a value-level tree. `load_owned_payload` always `Blit`s an aggregate
  payload into a fresh frame slot rather than aliasing the cell, so there is no
  interior-pointer-into-freed-block hazard even transiently. The generalized detection
  and loop codegen never need a visited set, a "seen this pointer" check, or any
  per-node bookkeeping at disposal time — only at detection time, over the static type
  graph (R3/R4).
- **R10 — The reset-then-check discipline that already guards the trailing `Ret` must
  apply at every freshly-started block along the generalized path, not only at the
  entry dispatch.** `synthesize_enum_destructor` already resets `b.terminated = false`
  immediately after `start_block` for every entry-level variant block (ir.rs:1038), so
  the trailing `if !b.terminated { b.seal_block(Ret) }` (ir.rs:1059-1061) correctly
  seals a base-case variant with `Ret` and skips sealing a variant that already
  back-edged. Once the path can introduce an **intermediate** tag dispatch (R5, the
  mutual case: `B`'s destructor dispatches on `A`'s tag mid-loop), each of *that*
  dispatch's freshly-started variant blocks needs the identical reset-then-check
  treatment, not just the outer one. Concretely: `type: P q ^Q ; type: Q r ^P ;` compiles
  today (an all-struct pair, no enum, no base case — Slice 3's exit-less shape,
  generalized to a two-type cycle) and must still compile to one exit-less loop per
  type without a duplicate block label; separately, `type: A | ANil | ACons x i64 next
  ^B ; type: B y i64 z ^A ;`, where `A`'s **terminating** variant (`ANil`) happens to be
  declared **before** its continuing variant, must still terminate `B`'s loop correctly
  — this ordering is buildable today and is exactly the shape where forgetting the
  reset-then-check discipline at the intermediate dispatch produces a duplicate
  `BlockId`, which `qbe` rejects outright (`multiple definitions of block @start`,
  verified against a hand-constructed case). Whether a given generalized loop is
  exit-less now depends on whether *any* enum anywhere on the discovered path has a
  reachable terminating variant, not on the entry type's own kind alone.
- **R11 — Arrays remain by-value edges, unchanged.** `[^T N]` is still rejected by
  Slice 2's linear-array-element rule, so an array cannot launder any of this slice's
  indirection either. No new work; the detection walk (R3) has no `Array` case for
  exactly this reason — it is a correct omission, not an oversight.

### Test discipline (binding)

- **R12** — Every criterion is a runnable golden, never an IL-string assertion. Every
  path-observing golden uses **distinct `__spy` tags at every level of the path that
  holds an aggregate frame slot** (R8) and asserts the full ordered stdout with
  `assert_eq!`. At least one golden per generalized shape must additionally declare its
  recursive/continuing field **before** its own `__spy` fields (mirroring Slice 3's own
  ordering-trap test), since a declaration-order-only golden cannot distinguish correct
  emission from an R8 violation when the cell field happens to already be declared
  last.
- **R13** — Every new constant-stack criterion must be verified to fail on the
  pre-change compiler (the base commit) before the change lands, using the **same
  program the criterion actually runs** (1,000,000 nodes, `ulimit -s 1024`), not an
  inference from a different N or a different stack bound. Already partially discharged
  by this spec's recon at N=100,000/8 MB; the 1,000,000-node/1 MB programs themselves
  must also be run against the base commit during implementation and confirmed to
  SIGSEGV before the goldens are trusted.
- **R14** — Every deep constant-stack golden's builder must itself be constant-stack
  (a self-tail-recursive `build`, as Slice 3's own deep goldens use), so the criterion
  measures disposal, not construction. At least one deep golden runs under a
  `ulimit -v`-bounded memory cap in addition to a stack-bounded one, so "exits 0 in
  constant stack" cannot be satisfied by leaking instead of freeing.

## Load-bearing invariants (must survive)

- Backend stays QBE; no LLVM. `Ptr[T]` stays opaque, never assumed to be a `u64`.
- The linear spine holds: exactly-once, no auto-drop, forgetting is a compile error.
- `core` stays `no_std`. No in-process JIT, no comptime interpreter.
- No new ad-hoc type constructor; `^` and `[T N]` remain the only two.
- D1 from the brief: the loop still descends exactly one recursive edge per node at any
  given struct level (R3's last-tried-first-success rule), and rejects branching through
  an enum outright rather than resolving it; genuine multi-child branching disposal
  stays Phase 6's.

## Delivery phases

1. **Generalize detection (R1–R4) and wire it in behind a length gate.** Replace
   `recursive_loop_field` with `recursive_disposal_path` (the backtracking walk), unit
   tested directly against all three probe shapes, their near-misses, and the
   composition case (a wrapper struct inside a 2-type cycle: `type: P q ^W ; type: W m
   i64 next ^Q ; type: Q r ^P ;`, verified buildable). Wire the new function into
   `synthesize_struct_destructor`/`synthesize_enum_destructor` in this same phase (not
   left unwired — an unwired private function fails `cargo clippy -- -D warnings`'
   dead-code lint under the project's definition of green): use the discovered path
   when its length is 1 (identical to today's single-step behaviour, so no existing
   goldens move), and fall back to ordinary recursive `emit_drop` for any longer path
   until phase 2 lands the general loop-body codegen.
2. **Generalize loop codegen to walk a path of any length (R5–R10), applied uniformly to
   every synthesized destructor.** Extend the loop-body emission to walk an arbitrary
   `Vec<PathStep>`: byval field projections via `field_value`, non-continuing field
   drops via `emit_drop` at every level (R5), mid-path tag dispatch via
   `dispatch_on_tag` with the reset-then-check discipline applied at every freshly
   started block it introduces (R10), and cell unwraps via `load_owned_payload` plus
   `free`, with the copyout-ordering invariant enforced independently per aggregate slot
   (R8). This mechanism is written once and applies to whichever type's own destructor
   calls it — `drop_A` and `drop_B` both call the same generalized path-finder and
   path-walker on their own fields, so the mutual case (R6) requires no phase-3
   special-casing, only its own verification. The wrapper-struct list, the `^^Self`
   list, and both directions of the mutual A/B pair all use the fused loop after this
   phase.
3. **Constant-stack proofs, near-miss and composition regression, the two Slice 3 golden
   inversions, dogfood, and the ROADMAP correction.**
   - Invert `mutually_recursive_types_dispose_on_recursive_path`'s deep assertion
     (tests/phase0.rs:2852) from `assert_ne!(…, Some(0))` to `assert_eq!(…, Some(0))`,
     rewriting its comment; the small-chain trace in the same test may also need its
     expected order updated once the fused loop changes disposal order for that shape —
     verify against the actual implementation rather than assuming the old order
     survives.
   - Invert `indirect_recursion_shapes_remain_depth_limited`'s wrapper-list assertion
     (tests/phase0.rs:2906) the same way; **keep its left-leaning-tree assertion
     unchanged** — that is the surviving proof D1's one-edge narrowing held.
   - Add the three 1,000,000-node constant-stack goldens (wrapper-struct list, `^^Self`
     list, mutual chain from **both** `drop_A` and `drop_B`, as two separate assertions
     since the `drop_B`-rooted one is R6's only proof), each verified against the base
     commit per R13, each built via a constant-stack builder per R14, plus one
     memory-bounded variant.
   - Add near-miss regression, extending the existing
     `non_recursive_cell_shapes_are_not_treated_as_recursive` (tests/phase0.rs:2716)
     rather than duplicating it: a dead-end wrapper struct whose *last* cell field
     points to an unrelated type while an *earlier* field is the genuine edge (proving
     the backtracking search, not a greedy one, is what's implemented — a positive
     case, not a near-miss, and the one most likely to silently fail under a
     naively-greedy implementation); a `^^OtherType` where the inner payload does not
     reach `Self`; two unrelated independently-self-recursive types.
   - Extend example dogfood: at minimum one program demonstrating a generalized shape,
     matching the brief's "one program per shape, at a depth that would segfault today"
     ask; if fewer are added, state plainly that the 1,000,000-node goldens already
     discharge the constant-stack claim and the byte-identical example sweep is not
     worth slowing down further, rather than leaving it as "if useful".
   - Rewrite `ROADMAP.md`'s Phase 3 Slice 4 entry from "not yet locked" to done,
     **and** correct the Slice 3 completion blurb's now-false claim that "indirect
     cycles, `^^Self`, and mutually recursive types keep the recursive path and its
     depth limit," and the Next-action line that still lists worklist-based branching
     disposal as part of Slice 4 (superseded by the `6f22576` move to Phase 6).

## Criterion → test map

Goldens live in `tests/phase0.rs`, except criterion 1, which has no observable runtime
behaviour and belongs in a unit test beside `ir.rs`. Criterion 8 stays a `tests/phase0.rs`
runtime golden (build-succeeds, per Slice 3's own precedent for its exit-less-loop case),
not a unit test, since the property being proved is that `qbe` accepts the emitted
module, which a unit test over IR values cannot show.

Criteria 2, 3, and 4 (small-N traced goldens for the three probe shapes) are
**correctness-preservation** tests, not mechanism proofs: all three shapes already
produce these traces on the unmodified base compiler (see Context), so passing them alone
does not show a fused loop exists — only the constant-stack goldens (5, 6, 7) do that,
mirroring Slice 3's own note that a small balanced-tree golden "would also pass on a
compiler that never builds a multi-child loop at all."

| # | criterion | test |
|---|---|---|
| 1 | `recursive_disposal_path` finds the correct path for all three probe shapes, the composition shape (wrapper struct inside a 2-type cycle), and correctly returns `None` for a plain non-recursive struct, a dead-end wrapper whose last field misleads a greedy search, a `^^OtherType`, and two unrelated self-recursive types | `recursive_disposal_path_finds_indirect_nested_mutual_and_composed_cycles`, `recursive_disposal_path_rejects_non_cyclic_and_misleading_shapes` |
| 2 | wrapper-struct list builds, disposes with distinct `__spy` tags per node in the correct order, at small N — correctness-preserving, not a mechanism proof | `wrapper_struct_recursive_list_disposes_in_expected_order` |
| 3 | `^^Self` list builds, disposes with distinct `__spy` tags per node, verifying the **single** hazardous (outer) cell-unwrap step reads before its copyout (R8) — correctness-preserving, not a mechanism proof | `double_cell_recursive_list_disposes_in_expected_order` |
| 4 | mutual A/B chain disposes correctly from **both** `drop_A` and a `drop_B`-rooted golden, with distinct tags per node, and with the recursive field declared **before** the spy fields at at least one level to trap declaration-order emission (R12) — correctness-preserving, not a mechanism proof | `mutual_recursive_chain_disposes_from_both_directions` |
| 5 | **1,000,000-node wrapper-struct list disposes under `ulimit -s 1024`, exit 0**, via a self-tail-recursive builder (R14), verified to SIGSEGV on the pre-change compiler on this exact program (R13) | `deep_wrapper_struct_list_disposes_in_constant_stack` |
| 6 | **1,000,000-node `^^Self` list disposes under `ulimit -s 1024`, exit 0**, same conditions | `deep_double_cell_list_disposes_in_constant_stack` |
| 7 | **1,000,000-node mutual A/B chain disposes under `ulimit -s 1024`, exit 0, from both `drop_A` and `drop_B` as two separate assertions** (the `drop_B`-rooted one is R6's sole proof that the two loops are independent, not a call across the cycle), same conditions, plus one memory-bounded (`ulimit -v`) variant for any one of the three shapes (R14) | `deep_mutual_chain_disposes_in_constant_stack_from_a`, `deep_mutual_chain_disposes_in_constant_stack_from_b`, `deep_recursive_chain_disposes_within_bounded_memory` |
| 8 | the exit-less loop case extends to an all-struct **two-type** cycle (`type: P q ^Q ; type: Q r ^P ;`) and does not crash the emitter (R10); a second sub-shape adds a byval wrapper hop into the all-struct cycle (`type: P q ^Q ; type: Q w W ; type: W p ^P ;`), stressing the reset-then-check discipline at a byval-reached level too | `all_struct_recursive_cycle_destructor_compiles`, `all_struct_cycle_with_wrapper_hop_destructor_compiles` |
| 9 | a base-case variant declared **before** its continuing sibling (`type: A | ANil | ACons x i64 next ^B ; type: B y i64 z ^A ;`,`ANil` first) still terminates `B`'s mid-loop dispatch correctly — the concrete shape that reproduces R10's duplicate-block hazard if the reset-then-check discipline is missed at the intermediate dispatch | `intermediate_dispatch_with_base_case_declared_first_terminates_correctly` |
| 10 | no regression: all prior examples and REPL goldens byte-identical, full suite green, **except** the two Slice 3 boundary assertions this slice deliberately inverts (`mutually_recursive_types_dispose_on_recursive_path`'s deep case, `indirect_recursion_shapes_remain_depth_limited`'s wrapper-list case) | existing suite, with those two updated in place |

## Explicitly out of scope

Worklist-based disposal for branching structures (moved to Phase 6; needs a growable
heap structure and a fallible-push story this slice's simple-cycle shapes don't need).
Branching through an intermediate **enum** with more than one continuing variant (R3;
struct-level branching is resolved by the existing last-field rule, not rejected).
Compiler-provided `Option`/`Result` or any synthesized nullable-pointer type (Phase 4
generics). Pointer arithmetic and pointer differences. Second-class refs and
`let`/`inout`/`sink`/`set` (Phase 3 Slice 5); reference counting (Phase 3 Slice 6);
user-definable destructor bodies (Phase 3 Slice 7); growable buffers and `Vec` (Phase 6
`alloc`).

## Phases

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Generalize recursive-edge detection into a backtracking path-finding pass over Registries, subsuming recursive_loop_field, wired in behind a path-length gate",
      "difficulty": "hard",
      "changes": [
        "Replace recursive_loop_field with recursive_disposal_path: a backtracking find_path(current, target, visited) starting from every field of the entry type (not only ^T fields), with cases for Struct (try fields in reverse declaration order, recurse, first successful complete path wins), Enum (apply the struct rule per variant; at most one variant may yield a path or the candidacy fails), and OwnedCell (close the path if the payload is the target, else recurse into the payload and prepend an Unwrap step)",
        "Scope the visited set per path attempt (pushed on entry, popped on return) so an abandoned branch cannot poison a sibling branch still being tried",
        "Wire recursive_disposal_path into synthesize_struct_destructor and synthesize_enum_destructor in this same phase: use the path when its length is 1 (identical to today), fall back to ordinary recursive emit_drop for any longer path until phase 2, so the function is never dead code under -D warnings and no existing golden's behaviour changes"
      ],
      "tests": [
        "recursive_disposal_path_finds_indirect_nested_mutual_and_composed_cycles",
        "recursive_disposal_path_rejects_non_cyclic_and_misleading_shapes"
      ],
      "exit": "recursive_disposal_path correctly identifies the path for the wrapper-struct, ^^Self, mutual A/B (both directions), and composed probe shapes, correctly returns None for non-recursive types and the misleading/near-miss cases including a dead-end-last-field wrapper that a greedy (non-backtracking) search would fail, and is wired into both destructor synthesis functions with no behavioural change yet for any length > 1 path; existing Slice 3 tests stay green unchanged; suite (including clippy -D warnings) green"
    },
    {
      "phase": 2,
      "focus": "Generalize the fused loop's codegen to walk a path of any length, applied uniformly to every synthesized destructor",
      "difficulty": "hard",
      "changes": [
        "Extend the loop-body emission to walk a Vec<PathStep> of any length: byval field projections via field_value (no free), non-continuing field drops via emit_drop at every path level (entry and intermediate alike), mid-path tag dispatch via dispatch_on_tag with every non-continuing variant dropping its own fields and terminating the loop",
        "Apply the reset-then-check terminated discipline (b.terminated = false immediately after every freshly-started block, checked before the trailing seal_block(Ret)) at every tag dispatch the path introduces, not only the entry dispatch",
        "Give each distinct aggregate type visited within one iteration its own hoisted frame slot via push_alloc, and enforce the copyout-ordering invariant independently per slot; note ^^Self's inner cell-unwrap has no slot and no ordering hazard since load_owned_payload's scalar branch is a plain FieldLoad",
        "Remove phase 1's length-1 gate: recursive_disposal_path's result is used at whatever length it returns, for every synthesized destructor uniformly, so drop_A and drop_B each independently discover and emit their own rotated loop with no special-casing for the mutual shape"
      ],
      "tests": [
        "wrapper_struct_recursive_list_disposes_in_expected_order",
        "double_cell_recursive_list_disposes_in_expected_order",
        "all_struct_recursive_cycle_destructor_compiles",
        "all_struct_cycle_with_wrapper_hop_destructor_compiles",
        "intermediate_dispatch_with_base_case_declared_first_terminates_correctly"
      ],
      "exit": "The wrapper-struct list and the ^^Self list both dispose via the fused loop with correctly ordered __spy traces; an all-struct two-type cycle, with and without a byval wrapper hop, compiles without a duplicate block label; a base-case variant declared before its continuing sibling still terminates a mid-loop dispatch correctly"
    },
    {
      "phase": 3,
      "focus": "Constant-stack proofs, near-miss and composition regression, the two Slice 3 golden inversions, dogfood, and the ROADMAP correction",
      "difficulty": "standard",
      "changes": [
        "Verify drop_A and drop_B each get their own independent fused loop (falls out of phase 2's uniformity; add a dedicated golden proving the drop_B direction specifically, since it is R6's sole discriminator against the rejected cross-call alternative)",
        "Invert mutually_recursive_types_dispose_on_recursive_path's deep assertion (tests/phase0.rs:2852) to assert_eq!(..., Some(0)); check whether its small-chain trace's expected order also changes and update if so",
        "Invert indirect_recursion_shapes_remain_depth_limited's wrapper-list assertion (tests/phase0.rs:2906) the same way; leave its left-leaning-tree assertion untouched",
        "Add the three 1,000,000-node constant-stack goldens (wrapper-struct, ^^Self, mutual from both directions) plus one memory-bounded variant, each with a constant-stack builder and each verified to fail on the base commit",
        "Extend non_recursive_cell_shapes_are_not_treated_as_recursive with the dead-end-last-field-misleads-a-greedy-search case, a ^^OtherType-does-not-reach-Self case, and two unrelated self-recursive types",
        "Add example dogfood for at least one generalized shape, or state explicitly why the constant-stack goldens already discharge the claim",
        "Rewrite ROADMAP.md's Phase 3 Slice 4 entry from not-yet-locked to done, correct the Slice 3 completion blurb's now-false depth-limit claim, and correct the Next-action line that still lists worklist-based branching disposal under Slice 4"
      ],
      "tests": [
        "deep_wrapper_struct_list_disposes_in_constant_stack",
        "deep_double_cell_list_disposes_in_constant_stack",
        "deep_mutual_chain_disposes_in_constant_stack_from_a",
        "deep_mutual_chain_disposes_in_constant_stack_from_b",
        "deep_recursive_chain_disposes_within_bounded_memory",
        "mutually_recursive_types_dispose_on_recursive_path",
        "indirect_recursion_shapes_remain_depth_limited"
      ],
      "exit": "All three generalized shapes dispose in verified constant stack at 1,000,000 nodes, the mutual case proven correct from both directions; near-miss and composition shapes proven not to false-fire; both Slice 3 boundary goldens correctly inverted with no other regression; all prior examples byte-identical and the full suite green"
    }
  ]
}
```
