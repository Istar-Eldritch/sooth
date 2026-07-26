# Phase 3 Slice 4 — Generalized recursive disposal (spec)

Design input: [the brief](./phase3-slice4-brief.md). Base: `main` @ `6f22576`, 679 tests
green.

## Context: what is already true on the base commit

Verified by building and running programs, not by reading code.

- Three probe shapes all **compile and run correctly today**, producing balanced
  alloc/free traces: a wrapper-struct list (`type: Wrap v i64 next ^List ; type: List |
  Nil | Cons w Wrap ;`), a `^^Self` list (`type: L | Nil | Cons n i64 next ^^L ;`), and a
  mutual A/B chain (`type: A | ANil | ACons x i64 next ^B ; type: B y i64 z ^A ;`).
- All three **segfault identically to Slice 3's own pre-fix baseline**: exit 0 at
  N=50,000, SIGSEGV (139) at N=100,000, under the default 8 MB stack. None hits Slice 3's
  fused loop, because `recursive_loop_field`'s exact match
  (`cells.payload[c] == self_ty`, ir.rs:922) only recognizes a `^Self` field directly on
  the enclosing type.
- `begin_loop`/`finalize_loop` (ir.rs:1515, 1536) derive each phi's type from the value
  passed at the call site; nothing hardcodes one aggregate type. A loop body can do more
  than one unwrap-step per physical iteration.
- `field_value` (ir.rs:2181) already treats `Struct`/`Enum`/`Array` fields as an in-place
  aggregate projection (`field_aggregate_value`, no free involved) and any other field
  type — including `OwnedCell`, which is not special-cased here — as a plain
  `FieldLoad` that reads the pointer itself, not its payload. `load_owned_payload`
  (ir.rs:1998) is the separate step that actually materializes a cell's payload and frees
  it. `emit_drop` (ir.rs:2245) and `dispatch_on_tag` (ir.rs:2208) are generic over any
  value/enum, not tied to the entry type of a destructor. Together these confirm an
  intermediate struct or enum encountered partway along a longer recursive path is an
  ordinary in-frame value with no special representation, and the existing primitives
  (`field_value`, `emit_drop`, `dispatch_on_tag`, `load_owned_payload`) are sufficient to
  walk it — no new IR value kind or backend change is needed, only a longer sequence of
  calls to primitives that already exist.

## Requirements

### Detection: generalizing "the recursive field" to a path

- **R1 — One pass, not two.** `recursive_loop_field` is replaced by a general path-finding
  function (working name `recursive_disposal_path`, ir.rs, beside where
  `recursive_loop_field` lives today) that subsumes it entirely. Direct self-recursion
  (today's only detected shape) becomes the length-1 special case of the general
  algorithm, not a separately maintained fast path. There is exactly one detection
  mechanism after this slice, not two competing ones.
- **R2 — The path is a sequence of typed steps, not a field index.** A discovered path is
  an ordered list where each element is one of:
  - **a byval projection**: extract a `Struct`/`Enum` field from the current aggregate
    (`field_value`, no free; if the current aggregate is an enum, this step is preceded by
    a tag dispatch, see R5), or
  - **a cell unwrap**: materialize a `^T` field's payload and free the cell
    (`load_owned_payload` + `free`, exactly today's single step, generalized to occur at
    any position in the path, not only the last).

  A direct `^Self` field is a path of one cell-unwrap step. The wrapper-struct case is one
  byval-projection step (into `Wrap`) followed by one cell-unwrap step. `^^Self` is two
  consecutive cell-unwrap steps with no byval projection between them (a cell's payload
  being another cell has no other fields to project). The mutual A/B case, from `B`'s
  side, is one cell-unwrap (`z: ^A`) followed by a tag dispatch on the unwrapped `A` (see
  R5) then, in the continuing variant, one more cell-unwrap (`next: ^B`).
- **R3 — The walk algorithm, and why it terminates.** For a candidate field of type
  `^T` on the enclosing type `Self`, walk from `T`:
  - If `T` is `Self`: done, path found (today's case, generalized to length 1).
  - If `T` is a `Struct`: for its fields, in declaration order, prefer the **last** field
    whose type is `Self`, an `OwnedCell` continuing toward `Self`, or another
    `Struct`/`Enum` worth recursing into — mirroring `recursive_loop_field`'s existing
    `next_back()` tie-break, generalized to every level of the walk, not only the entry
    type. Recurse the walk into that field.
  - If `T` is an `Enum`: for **each** variant, apply the same rule to its fields. A variant
    with no continuing field is a base case for this path (R5); at most one variant may
    contain a continuing field, checked by the walk (see below).
  - Maintain a **visited set of types** seen so far in the current walk. If the walk
    revisits a type without having reached `Self`, this candidate field's path fails
    (dead end into an unrelated cycle, e.g. some other type's independent
    self-recursion) and the field falls back to today's ordinary recursive `emit_drop`
    call, exactly as an unrecognized field already does. The visited set is what makes
    the walk terminate: the *byval* subgraph is acyclic by construction (R3–R7 of Slice
    3), but a walk that crosses cell boundaries re-enters a graph that can contain
    cycles unrelated to `Self`, and only a visited set — not "the byval part is acyclic"
    — proves termination once cells are crossed.
  - If more than one variant of an intermediate enum contains a continuing field toward
    `Self` (a branching cycle, not a simple one), the walk fails for this candidate and
    falls back to recursion: D1 restricts this slice to simple (non-branching) cycles.
- **R4 — The walk is detection-time only, over the static type graph; it is not a
  runtime concept.** No visited set, no cycle guard, and no double-free check exist at
  disposal time (see D2/R9). R3's visited set exists purely to make the *search* for a
  path terminate; once a path is found, it is a fixed, statically-known sequence, walked
  once per loop iteration with no re-checking.

### Codegen: one fused loop per participating type, walking the whole path

- **R5 — Base cases can occur anywhere along the path, not only at the entry type.**
  Today, `synthesize_enum_destructor` dispatches on the entry enum's own tag once, and
  non-recursive variants terminate (`ret`). This generalizes unchanged in kind, only in
  position: whenever the path passes through an enum (the entry type or an intermediate
  one reached via a cell unwrap), a tag dispatch (`dispatch_on_tag`, reused as-is) is
  emitted at that point, every non-continuing variant drops its own fields via the
  ordinary `emit_drop` and terminates the loop (`ret`), and the one continuing variant's
  fields are dropped in declaration order before the walk proceeds to the next path step.
  For the mutual A/B case, `B`'s destructor loop dispatches on `A`'s tag mid-loop, not at
  entry, since `B` is a plain struct with no tag of its own.
- **R6 — Every type on a cycle gets its own fused loop, entered from its own shape; no
  synthesized destructor calls another synthesized destructor to traverse the same
  cycle.** For the mutual A/B case, `drop_A` and `drop_B` are two independent loops, each
  the same cyclic path rotated to start its own tag dispatch (or lack of one, for a
  struct) first. This is a considered rejection of the simpler-looking alternative,
  "`drop_B` just calls the already-synthesized `drop_A` on the unwrapped payload": that
  alternative is a plain function call between two synthesized functions forming a
  **mutual tail-call cycle**, and Slice 6's tail-call-to-loop lowering explicitly does
  not perform SCC contraction for mutual tail recursion (`ROADMAP.md`, Slice 6 entry:
  "tier-2 SCC contraction stays deferred"; user-level mutual tail recursion is a located
  compile error there for the same reason). Calling across the cycle would use an
  ordinary native `Call`, growing the Rust-compiled call stack by one frame pair per
  logical node and reproducing exactly the defect this slice exists to fix. Inlining the
  whole rotated path into each participating type's own loop is what actually achieves
  constant stack.
- **R7 — One loop iteration is one full trip around the path, however many steps it
  has; there is no separate "hops per iteration" concept and no inner loop.** The
  path's length is fixed and known at codegen time (R2/R3 produce it once, statically);
  the loop body is simply the path's steps emitted in order — byval projections, field
  drops, tag dispatches, cell unwraps — ending with the back-edge feeding the final
  cell-unwrap's result to the header phi, exactly as today's single-step case does. This
  resolves the brief's open question about per-shape unroll counts: there is no unroll
  factor to choose, only a path to emit linearly.
- **R8 — The copyout-ordering invariant (Slice 3's R12) generalizes to every cell-unwrap
  step in the path, not only the last.** The loop-carried value is a pointer to one
  entry-hoisted frame slot (`push_alloc`, ir.rs:1386), reused every iteration. Every read
  of data at a given path position — a byval projection, a field drop, a tag dispatch —
  must be emitted before the cell-unwrap step that overwrites that data. This applies
  independently at **each** cell boundary the path crosses (e.g. `^^Self`'s two
  consecutive unwraps each have their own ordering requirement), not just once at the
  end. Verification must use distinct `__spy` tags at every level of the path, not only
  the outermost, since a violation at an inner step corrupts data while leaving the
  alloc/free trace balanced, exactly as Slice 3's R12 found for the single-step case.
- **R9 — No aliasing or double-free guard exists anywhere in this mechanism, and none is
  needed.** `^T` ownership is exclusive and struct/enum setters are whole-value
  functional transforms (`S<fi : ( S Ti -- S )`), so no value this slice's types can
  build has an actual runtime cycle: every type-level cycle these three shapes legalize
  still produces a value-level tree. The generalized detection and loop codegen never
  need a visited set, a "seen this pointer" check, or any per-node bookkeeping at
  disposal time — only at detection time, over the static type graph (R3/R4).
- **R10 — The `terminated` guard (Slice 3's R16) applies uniformly and its trigger
  condition is now path-dependent, not entry-type-dependent.** `seal_block` must still
  check `b.terminated` before sealing the trailing `Ret`, for every synthesized
  destructor with a loop. Whether a given loop is exit-less (no base case reachable)
  no longer depends solely on whether the *entry* type is a struct (Slice 3's case): a
  struct entry whose path passes through an intermediate enum with a genuine
  terminating variant (e.g. `B`'s path through `A`/`ANil`) **does** terminate normally.
  Exit-less loops remain possible (an all-struct cycle with no enum anywhere on the
  path, mirroring Slice 3's uninhabited self-recursive struct) and must not crash the
  emitter.
- **R11 — Arrays remain by-value edges, unchanged.** `[^T N]` is still rejected by
  Slice 2's linear-array-element rule (R6 of Slice 3), so an array cannot launder any of
  this slice's indirection either. No new work; stated for completeness since the
  detection walk must not attempt to cross an array element as if it were a struct
  field.

### Test discipline (binding, carried from Slice 3's R20/R21)

- **R12** — Every criterion is a runnable golden, never an IL-string assertion. Every
  path-observing golden uses **distinct `__spy` tags at every level of the path**
  (R8) and asserts the full ordered stdout with `assert_eq!`.
- **R13** — Every new constant-stack criterion must be verified to fail on the
  pre-change compiler (the base commit) before the change lands. Already discharged by
  this spec's recon: all three shapes SIGSEGV at 100,000 nodes under the default 8 MB
  stack on `6f22576`.

## Load-bearing invariants (must survive)

- Backend stays QBE; no LLVM. `Ptr[T]` stays opaque, never assumed to be a `u64`.
- The linear spine holds: exactly-once, no auto-drop, forgetting is a compile error.
- `core` stays `no_std`. No in-process JIT, no comptime interpreter.
- No new ad-hoc type constructor; `^` and `[T N]` remain the only two.
- D1 from the brief: the loop still descends exactly one recursive edge per node
  (`recursive_disposal_path` picks one path per candidate field, per R3's tie-break);
  branching stays out of scope, moved to Phase 6.

## Delivery phases

1. **Generalize detection (R1–R4).** Replace `recursive_loop_field` with
   `recursive_disposal_path`, returning `Option<Vec<PathStep>>` (or equivalent), unit
   tested directly against all three probe shapes plus near-miss shapes, with no codegen
   change yet — the existing single-step loop codegen keeps working unchanged for direct
   self-recursion, since a length-1 path is exactly today's case.
2. **Generalize loop codegen to walk a path (R5–R10).** Extend the loop-body emission
   (today's single-step `emit_recursive_step`) to walk an arbitrary `Vec<PathStep>`,
   handling byval projections, mid-path tag dispatch and its per-variant termination,
   and multiple cell-unwrap steps with the ordering invariant applying at each. Wire
   `synthesize_struct_destructor`/`synthesize_enum_destructor` to call the general path
   function. The wrapper-struct list and `^^Self` list now use the fused loop.
3. **Mutual types, both directions, and the regression sweep (R6 verification, R11–R13).**
   Both `drop_A` and `drop_B` get their own independent loop; verify each separately.
   Add near-miss regression coverage (a dead-end wrapper struct that doesn't actually
   cycle back stays recursive; a `^^OtherType` cell-of-cell stays recursive, distinguishing
   it from true `^^Self`; two unrelated independently-self-recursive types aren't
   cross-wired). Add the three 1,000,000-node constant-stack goldens (wrapper-struct list,
   `^^Self` list, mutual chain from both `A` and `B`), each verified to fail on the base
   commit. Extend or add example dogfood. Update `ROADMAP.md`'s Slice 4 entry from "not
   yet locked" to done, with a brief summary matching this slice's actual mechanism.

## Criterion → test map

Goldens live in `tests/phase0.rs`, except criteria 1 and 8, which have no observable
runtime behaviour and belong in unit tests beside `ir.rs`.

| # | criterion | test |
|---|---|---|
| 1 | `recursive_disposal_path` finds the correct path for all three probe shapes (wrapper-struct, `^^Self`, mutual A/B from both directions) and returns `None` for a plain non-recursive struct | `recursive_disposal_path_finds_indirect_nested_and_mutual_cycles` |
| 2 | wrapper-struct list builds, disposes with distinct `__spy` tags per node in the correct order, at small N | `wrapper_struct_recursive_list_disposes_in_expected_order` |
| 3 | `^^Self` list builds, disposes with distinct `__spy` tags per node, verifying both cell-unwrap steps read before their respective copyouts (R8) | `double_cell_recursive_list_disposes_in_expected_order` |
| 4 | mutual A/B chain disposes correctly from **both** `drop_A` and a `drop_B`-rooted golden, with distinct tags per node | `mutual_recursive_chain_disposes_from_both_directions` |
| 5 | **1,000,000-node wrapper-struct list disposes under `ulimit -s 1024`, exit 0**, verified to SIGSEGV on the pre-change compiler (R13) | `deep_wrapper_struct_list_disposes_in_constant_stack` |
| 6 | **1,000,000-node `^^Self` list disposes under `ulimit -s 1024`, exit 0**, verified to SIGSEGV on the pre-change compiler | `deep_double_cell_list_disposes_in_constant_stack` |
| 7 | **1,000,000-node mutual A/B chain disposes under `ulimit -s 1024`, exit 0, from both directions**, verified to SIGSEGV on the pre-change compiler | `deep_mutual_chain_disposes_in_constant_stack` |
| 8 | the exit-less loop case still applies where the whole path is struct-only (no enum, no base case anywhere), and does not crash the emitter (R10) | `all_struct_recursive_cycle_destructor_compiles` |
| 9 | near-miss regression: a wrapper struct pointing to an unrelated type (no cycle) stays on the recursive path; a `^^OtherType` (inner payload not `Self`) stays recursive; two unrelated independently-self-recursive types are not cross-wired | `non_cyclic_indirect_and_nested_shapes_are_not_treated_as_recursive` |
| 10 | no regression: all prior examples and REPL goldens byte-identical, full suite green | existing suite |

## Explicitly out of scope

Worklist-based disposal for branching structures (moved to Phase 6; needs a growable
heap structure and a fallible-push story this slice's simple-cycle shapes don't need).
Multiple recursive edges per node of any kind, direct or generalized (D1; still Phase
6's territory). Compiler-provided `Option`/`Result` or any synthesized nullable-pointer
type (Phase 4 generics). Pointer arithmetic and pointer differences. Second-class refs
and `let`/`inout`/`sink`/`set` (Slice 5); reference counting (Slice 6); user-definable
destructor bodies (Slice 7); growable buffers and `Vec` (Phase 6 `alloc`).

## Phases

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Generalize recursive-edge detection into a path-finding pass over Registries, subsuming recursive_loop_field",
      "difficulty": "hard",
      "changes": [
        "Replace recursive_loop_field with recursive_disposal_path returning an ordered path of typed steps (byval projection into a struct/enum field, or cell unwrap), with direct self-recursion as the length-1 case",
        "Walk struct fields and enum variants in declaration order, preferring the last continuing field at every level, generalizing today's next_back() tie-break to every step of the walk, not only the entry type",
        "Track a visited set of types during the walk; revisiting a type without reaching Self fails this candidate and falls back to ordinary recursive emit_drop, since the byval subgraph is acyclic by construction but a walk crossing cell boundaries re-enters a graph that can cycle without involving Self",
        "An intermediate enum with more than one variant containing a continuing field is a branching cycle and fails this candidate (D1 restricts this slice to simple cycles)",
        "Keep the loop codegen unchanged in this phase; wire nothing new into synthesize_struct_destructor/synthesize_enum_destructor yet"
      ],
      "tests": [
        "recursive_disposal_path_finds_indirect_nested_and_mutual_cycles"
      ],
      "exit": "recursive_disposal_path correctly identifies the path for the wrapper-struct, ^^Self, and mutual A/B (both directions) probe shapes, and correctly returns None for ordinary non-recursive types and for the branching/ambiguous near-miss cases; existing direct self-recursion tests from Slice 3 stay green unchanged since a length-1 path behaves identically to today"
    },
    {
      "phase": 2,
      "focus": "Generalize the fused loop's codegen to walk an arbitrary path instead of a single step",
      "difficulty": "hard",
      "changes": [
        "Extend the loop-body emission to walk a Vec<PathStep>: byval field projections via field_value (no free), field drops of non-continuing fields via emit_drop, mid-path tag dispatch via dispatch_on_tag with every non-continuing variant dropping its own fields and terminating the loop, and cell unwraps via load_owned_payload plus free",
        "Enforce the copyout-ordering invariant independently at every cell-unwrap step in the path, not only the last",
        "Guard the trailing seal_block on b.terminated for every synthesized destructor with a loop, since whether a loop is exit-less is now path-dependent rather than determined solely by the entry type's own kind",
        "Wire synthesize_struct_destructor and synthesize_enum_destructor to call recursive_disposal_path instead of recursive_loop_field"
      ],
      "tests": [
        "wrapper_struct_recursive_list_disposes_in_expected_order",
        "double_cell_recursive_list_disposes_in_expected_order",
        "all_struct_recursive_cycle_destructor_compiles"
      ],
      "exit": "The wrapper-struct list and the ^^Self list both dispose via the fused loop with correctly ordered __spy traces at every path level; an all-struct cycle with no base case anywhere on its path still compiles without a duplicate block label"
    },
    {
      "phase": 3,
      "focus": "Mutual types from both directions, near-miss regression coverage, constant-stack proofs, and the regression sweep",
      "difficulty": "standard",
      "changes": [
        "Confirm drop_A and drop_B each get their own independent fused loop, the same cyclic path rotated to start from each type's own shape, with no synthesized destructor calling another across the cycle",
        "Add near-miss regression tests: a dead-end wrapper struct pointing to an unrelated type, a ^^OtherType cell-of-cell distinct from true ^^Self, and two unrelated independently-self-recursive types",
        "Add the three 1,000,000-node constant-stack goldens under ulimit -s 1024, each confirmed to SIGSEGV on the pre-change compiler",
        "Extend example dogfood if useful",
        "Update ROADMAP.md's Slice 4 entry from not-yet-locked to done, describing the actual path-based mechanism"
      ],
      "tests": [
        "mutual_recursive_chain_disposes_from_both_directions",
        "non_cyclic_indirect_and_nested_shapes_are_not_treated_as_recursive",
        "deep_wrapper_struct_list_disposes_in_constant_stack",
        "deep_double_cell_list_disposes_in_constant_stack",
        "deep_mutual_chain_disposes_in_constant_stack"
      ],
      "exit": "All three generalized shapes dispose in verified constant stack at 1,000,000 nodes; the mutual case is proven correct from both directions; near-miss shapes are proven not to false-fire; all 14+ prior examples byte-identical and the full suite green"
    }
  ]
}
```
