# Phase 3 Slice 3 — Recursive heap data + `isize` (spec)

Design input: [the brief](./phase3-slice3-brief.md). Base: `main` @ `e01f045`, 654 tests green.

## Context: what is already true on the base commit

Verified by building and running programs, not by reading code. These findings set the
scope: the type rule needs **pinning**, the destructor needs **rewriting**.

- `type: List | Nil | Cons v i64 next ^List ;` compiles and runs today. `type_node`
  (@check.rs:501) maps only `Struct`/`Enum`/`Array` into the cycle graph and returns `None`
  otherwise, so `^` edges are already cut. Correct, incidental, and **untested**.
- `type: Bad v i64 next Bad ;` is still rejected:
  `recursive struct definition (infinite size): Bad -> Bad`.
- Building and consuming-walking a list is already constant-stack, because such walks are
  self-tail-recursive and Slice 6's TCO applies.
- The synthesized destructor recurses on the native stack. One binary, 100,000 nodes:
  8 MB stack → SIGSEGV (139); 64 MB → exit 0; 1 MB → SIGSEGV. 50,000 nodes passes at 8 MB.

## Requirements

### `isize`

- **R1** — `isize` is a signed, target-width integer, the exact mirror of `usize` through
  lexer, parser, check, ir and backend. Width comes from the same word-width parameter
  `usize` uses; no literal `8` is introduced. No pointer arithmetic and no pointer-difference
  words are added, because no consumer exists.

### The recursion rule

- **R2** — A type cycle is legal **iff every cycle passes through at least one `^`**.
  Struct fields, enum variant payloads and array elements are by-value edges and participate
  in cycle detection; `^` edges are cut. This is the existing behaviour; this slice makes it
  intentional and tested rather than incidental.
- **R3** — No positional restriction: a `^` cycle is legal through a **struct field** as
  well as an enum variant payload. The rule is about size finiteness, not idiom.
- **R4** — No uninhabitedness detection. `type: Node v i64 next ^Node ;` compiles even
  though it can never be constructed (`^` is non-null, so building one needs one first).
  Sound, self-correcting at the first constructor, and detecting it would need a real
  inhabitedness analysis.
- **R5** — Arrays remain by-value edges. `[^T N]` is already rejected by Slice 2's
  linear-array-element rule, so arrays cannot launder indirection regardless.
- **R6** — By-value cycles keep their located diagnostic naming the cycle path, for both
  self-recursion (`Bad -> Bad`) and mutual recursion (`A -> B -> A`).

### Disposal

- **R7** — **Free ordering is reversed, globally.** For every owning cell, the block is
  freed **before** its copied-out payload is dropped. Applies to all cells, not only
  recursive ones: one ordering rule, not an ordering that depends on whether a type happens
  to be recursive. Sound because `load_owned_payload` (@ir.rs:1907) already copies the
  payload out of the block before it is touched, so the copy does not alias freed memory.
- **R8** — R7 **revises shipped Slice 2 behaviour**. Every golden asserting
  `alloc N / drop T / free N` becomes `alloc N / free N / drop T`, and the Slice 2 spec
  sentence stating that the payload's destructor precedes the free must be corrected. The
  revision lands in the **same phase** as R7; a state where tests and docs disagree is not
  an acceptable intermediate.
- **R9** — **Synthesized destructors are iterative for self-recursive types.** A type that
  is recursive with itself gets **one fused loop** over the recursive cycle, not
  mutually-recursive `cell_drop`/`enum_drop` functions. No general or mutual TCO is
  required: the compiler owns these functions and emits the loop directly, reusing
  `FuncBuilder`'s existing loop support (`begin_loop` @ir.rs:1424, `finalize_loop`
  @ir.rs:1445). Shape:

  ```text
  drop_List(v):
    loop:
      match tag(v):
        Nil  -> ret
        Cons -> drop v.field0          # non-recursive fields, in declaration order
                p    = v.field1        # the recursive ^ field
                next = copyout(p)
                free(p)                # R7 ordering
                v = next; continue
  ```

- **R10** — For a recursive type with **several** recursive fields, the loop takes the
  **last recursive field in declaration order** and the others are recursed. A balanced
  tree is then log n deep. The choice is arbitrary but must be written down and tested, or
  it becomes an accident of synthesis order.
- **R11** — **Mutually recursive types** (`A` holds `^B`, `B` holds `^A`) keep today's
  recursive destructor and its depth limit. A fused loop over a multi-type SCC needs a
  tagged loop over the whole component (the tier-2 SCC-contraction shape) and is out of
  scope. The fallback must be **tested**, not merely asserted, so the scope line is real.
- **R12** — **Non-recursive destructors are unchanged** apart from the R7 reordering. They
  are the overwhelming majority and must not acquire loop machinery.

### Allocation failure

- **R13** — **The OOM revisit is closed, not deferred again.** Slice 2 deferred
  trap-vs-return to this slice on the premise that optional pointers would exist as a
  compiler-known type to return. They do not: nullability is an ordinary user-authored
  two-variant enum (as `MaybeInt` already is in `examples/shapes.sth`), because a
  compiler-synthesized per-payload `Option` is exactly the machinery Phase 4's generics
  delete. The allocator has nothing privileged to return, so **the trap stays**. Revisit in
  Phase 4 with real generics. No allocator, shim, trap or trace code changes in this slice.

### Test discipline (binding)

- **R14** — Every criterion is a runnable golden (native binary or REPL session), never an
  IL-string assertion, except where genuinely emitter-level. Trace-observing goldens assert
  the **full stdout** with `assert_eq!`, exact and ordered: never `contains`, never counting.
  Every negative golden asserts the diagnostic substring **and** the backticked type name,
  with an `"unexpected message: {err}"` context. Naming is `thing_condition_expected`.
- **R15** — The depth criterion (criterion 8) must be **verified to fail on the
  pre-change compiler** before the change lands, so it is known to discriminate rather than
  passing vacuously.

## Load-bearing invariants (must survive)

- Backend stays QBE; no LLVM. `Ptr[T]` stays opaque and is never assumed to be a `u64`.
- The linear spine holds: exactly-once, no auto-drop, forgetting is a compile error.
- `core` stays `no_std`. No in-process JIT, no comptime interpreter.
- No user-facing FFI is added. The allocator shim stays a backend special case (R13).
- No new ad-hoc type constructor is introduced (R13 rationale); `^` and `[T N]` remain the
  only two.

## Delivery phases

1. **`isize`.** Fully independent of everything else; lands first and green on its own.
2. **Pin the recursion rule.** Tests for R2–R6. Production change expected to be zero or
   near-zero; if `type_node` needs an explicit cell arm to make the cut intentional rather
   than a fall-through, that is the change.
3. **Reverse the free ordering (R7) and revise the Slice 2 goldens and spec text (R8)**
   in one commit. No loop yet. Independently meaningful: it lowers peak disposal memory on
   its own, and it is the precondition the loop needs.
4. **The iterative destructor (R9, R12)** plus the constant-stack criterion. This is the
   centrepiece and the only phase that touches destructor synthesis structurally.
5. **Multi-child and mutual recursion (R10, R11), the `examples/list.sth` dogfood, and the
   regression sweep.**

## Criterion → test map

Every row names the test that proves it. Goldens live in `tests/phase0.rs` (native) or
`tests/phase1.rs` (REPL), except criteria 2, 3, 4 and 12, which are parse/check errors with
no runtime and belong in unit tests beside their stage.

| # | criterion | test |
|---|---|---|
| 1 | `isize` declares, computes, prints, converts to/from `i64` | `isize_round_trips_arithmetic_and_conversion` |
| 2 | by-value self-recursion is a located error naming the cycle | `check_recursion_by_value_self_cycle_is_error` |
| 3 | by-value **mutual** recursion is a located error naming the full path | `check_recursion_by_value_mutual_cycle_is_error` |
| 4 | a `^` cycle through a **struct field** is accepted (R3) | `check_recursion_cell_cycle_in_struct_field_is_ok` |
| 5 | a recursive enum builds, walks and disposes with a balanced trace | `recursive_list_builds_walks_and_disposes` |
| 6 | disposal traces `free` **before** the payload's `drop` (R7) | `owned_cell_frees_before_dropping_payload` |
| 7 | revised Slice 2 goldens assert the new order exactly (R8) | existing Slice 2 trace goldens, updated in place |
| 8 | **1,000,000-node list disposes under `ulimit -s 1024`, exit 0** (R9) | `deep_list_disposes_in_constant_stack` |
| 9 | a non-recursive cell's destructor is unchanged apart from ordering (R12) | `non_recursive_cell_destructor_is_unchanged` |
| 10 | a binary tree builds and disposes correctly (R10) | `recursive_tree_builds_and_disposes` |
| 11 | a balanced tree of substantial size disposes under a bounded stack (R10) | `balanced_tree_disposes_within_log_depth` |
| 12 | the loop takes the last recursive field in declaration order (R10) | `multi_child_destructor_loops_on_last_recursive_field` |
| 13 | mutually recursive types build and dispose at modest depth (R11) | `mutually_recursive_types_dispose_on_recursive_path` |
| 14 | `examples/list.sth` runs with exact stdout | `example_list_matches_golden` |
| 15 | REPL: a residual recursive value is disposed at `:quit` | `repl_quit_frees_residual_recursive_value` |
| 16 | no regression: 14 existing examples byte-identical, 654 tests pass | existing suite |

Criterion 8 is the headline. It must fail on the pre-change compiler (R15): today's binary
segfaults at 100,000 nodes with 8 MB, so 1,000,000 nodes at 1 MB is ~100x past the failure
point and cannot pass by accident.

## Explicitly out of scope

Compiler-provided `Option`/`Result` or any synthesized nullable-pointer type (Phase 4
generics); pointer arithmetic and pointer differences; fused destructor loops over
multi-type SCCs (R11, tier-2); returning allocation failure rather than trapping (R13,
Phase 4); zippers and compiler-generated one-hole types (a design exploration, a stretch,
never an exit criterion); second-class refs and `let`/`inout`/`sink`/`set` (Slice 4);
reference counting (Slice 5); user-definable destructor bodies (Slice 6); growable buffers
and `Vec` (Phase 6 `alloc`).

## Phases

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "isize as a target-width signed integer, the exact mirror of usize through lexer, parser, check, ir and backend",
      "difficulty": "standard",
      "changes": [
        "Add isize alongside usize in the type name table and integer tower",
        "Width from the existing word-width parameter; introduce no literal 8",
        "Conversions and homogeneous-op rules mirroring usize",
        "Backend width and signedness mapping"
      ],
      "tests": [
        "isize_round_trips_arithmetic_and_conversion"
      ],
      "exit": "isize behaves as usize does but signed; full suite green; no example output changes"
    },
    {
      "phase": 2,
      "focus": "Pin the recursion rule with tests so the currently-incidental cut of ^ edges becomes intentional",
      "difficulty": "standard",
      "changes": [
        "Make type_node's exclusion of cell edges explicit rather than a fall-through, if needed",
        "No relaxation of by-value cycle detection"
      ],
      "tests": [
        "check_recursion_by_value_self_cycle_is_error",
        "check_recursion_by_value_mutual_cycle_is_error",
        "check_recursion_cell_cycle_in_struct_field_is_ok",
        "recursive_list_builds_walks_and_disposes"
      ],
      "exit": "Recursion through ^ is legal and tested in both struct and enum position; by-value cycles still rejected with a path-naming diagnostic"
    },
    {
      "phase": 3,
      "focus": "Reverse the free ordering globally so a cell is freed before its copied-out payload is dropped, revising the Slice 2 goldens and spec text in the same commit",
      "difficulty": "standard",
      "changes": [
        "synthesize_cell_destructor frees the block before dropping the payload copy",
        "Update every Slice 2 trace golden from alloc/drop/free to alloc/free/drop",
        "Correct the Slice 2 spec sentence asserting payload-drop-precedes-free"
      ],
      "tests": [
        "owned_cell_frees_before_dropping_payload"
      ],
      "exit": "One ordering rule for every cell; no test or document still asserts the old order"
    },
    {
      "phase": 4,
      "focus": "Synthesize an iterative fused-loop destructor for self-recursive types, leaving non-recursive destructors structurally unchanged",
      "difficulty": "hard",
      "changes": [
        "Detect self-recursive types from the same cycle information the checker computes",
        "Emit one fused loop over the recursive cycle using FuncBuilder's existing loop support",
        "Keep straight-line synthesis for every non-recursive type"
      ],
      "tests": [
        "deep_list_disposes_in_constant_stack",
        "non_recursive_cell_destructor_is_unchanged"
      ],
      "exit": "A 1,000,000-node list disposes under ulimit -s 1024 with exit 0; verified to fail on the pre-change compiler; non-recursive destructors unchanged apart from ordering"
    },
    {
      "phase": 5,
      "focus": "Multi-child and mutually recursive types, the list dogfood example, and the regression sweep",
      "difficulty": "standard",
      "changes": [
        "Loop on the last recursive field in declaration order, recurse the others",
        "Leave mutually recursive types on the recursive destructor path",
        "Add examples/list.sth"
      ],
      "tests": [
        "recursive_tree_builds_and_disposes",
        "balanced_tree_disposes_within_log_depth",
        "multi_child_destructor_loops_on_last_recursive_field",
        "mutually_recursive_types_dispose_on_recursive_path",
        "example_list_matches_golden",
        "repl_quit_frees_residual_recursive_value"
      ],
      "exit": "Trees and mutual recursion both dispose correctly, the documented fallback is tested rather than asserted, all 14 prior examples are byte-identical and the suite is green"
    }
  ]
}
```
