# Phase 3 Slice 3 — Recursive heap data + `isize` (spec)

Design input: [the brief](./phase3-slice3-brief.md). Base: `main` @ `be281ca`, 654 tests green.
Revised after spec review round 1, which found three blockers by hand-patching emitted QBE
and running it. Where this document previously justified a decision on grounds that review
refuted, the corrected justification is marked.

## Context: what is already true on the base commit

Verified by building and running programs, not by reading code.

- `type: List | Nil | Cons v i64 next ^List ;` and `type: Node v i64 next ^Node ;` both
  compile and run today. `type_node` (@check.rs:499-507) maps only `Struct`/`Enum`/`Array`
  into the cycle graph and returns `None` for `Type::OwnedCell`, so `^` edges are already
  cut. Correct, incidental, and **untested**.
- By-value recursion is still rejected:
  `recursive struct definition (infinite size): Bad -> Bad`, and `A -> B -> A` for mutual.
- Building and consuming-walking a list is already constant-stack (such walks are
  self-tail-recursive, so Slice 6's TCO applies).
- **Every recursive shape already builds and disposes correctly** with balanced traces:
  recursive enums, recursive structs, binary trees (`alloc 32` ×7 / `free 32` ×7),
  mutually recursive types, the wrapper-struct list, and `^^Self`. Nothing here needs
  building; phases 2 and 5 are largely **pinning existing behaviour**, and their risk is
  that phase 4 *regresses* one of these shapes.
- **Destructor stack depth is the only defect.** One binary, 100,000-node list: 8 MB stack
  → SIGSEGV (139); 64 MB → exit 0; 1 MB → SIGSEGV. 50,000 nodes passes at 8 MB.
- The fused loop is **proven implementable**: review hand-substituted it into the
  compiler's own `out.ssa` and disposed 1,000,000 nodes under `ulimit -s 1024`, exit 0.
  `begin_loop`/`finalize_loop` fit synthesized destructors, and QBE accepts a `phi` whose
  entry arm is an aggregate-typed param.

## Requirements

### `isize`

- **R1** — `isize` is a signed, target-width integer, mirroring `usize` including a
  distinct `IrType::Isize` variant, so the two sibling types are represented
  symmetrically. The work is **not** width (already parameterised via
  `scalar_size_align_ww` @ir.rs:355) but the coercion and diagnostic plumbing, which is
  `usize`-*named* rather than generic and must be either duplicated or parameterised by
  target type: `SlotMatch::{LiteralUsize, NeedsUsizeConversion}` (@check.rs:78-79),
  `PairMatch::NeedsUsizeConversion` (@check.rs:104), the `(Usize, I64)` literal-coercion
  pairs (@check.rs:113-115), and `usize_conversion_needed_error` (@check.rs:1535), whose
  message hardcodes `>usize`. Mixing `usize` and `isize` is a plain type mismatch.
  Printing routes to the signed format, not `$ufmt` (@qbe.rs:888). Adding an `IrType`
  variant gets exhaustiveness help from rustc for `match`es but **not** for the `_ =>`
  catch-alls at @qbe.rs:217, 254, 281, 300, 509, 720, 888; auditing those is the real risk.
- **R2** — `norm_scalar` (@qbe.rs:509) currently hardcodes `bits: 64` for `Usize`,
  contradicting its own word-width comment. **Correct it for both `usize` and `isize`** so
  each derives from the target word width. Mirroring without this would propagate a latent
  64-bit assumption into the new type, which is exactly what word-width-neutrality exists
  to prevent.

### The recursion rule (pinning existing behaviour)

- **R3** — A type cycle is legal **iff every cycle passes through at least one `^`**.
  Struct fields, enum variant payloads and array elements are by-value edges and
  participate in cycle detection; `^` edges are cut. This slice makes the existing
  behaviour intentional and tested.
- **R4** — No positional restriction: legal through a **struct field** as well as an enum
  variant payload. The rule is about size finiteness, not idiom.
- **R5** — No uninhabitedness detection. `type: Node v i64 next ^Node ;` compiles though it
  can never be constructed (`^` is non-null, so building one needs one first). Sound and
  self-correcting at the first constructor.
- **R6** — Arrays remain by-value edges; `[^T N]` is already rejected by Slice 2's
  linear-array-element rule.
- **R7** — By-value cycles keep their located diagnostic naming the full path, for self
  and mutual recursion alike.

### Disposal ordering

- **R8** — **Free ordering is reversed, globally**: for every owning cell the block is
  freed **before** its copied-out payload is dropped. **Justification is uniformity
  alone.** The earlier claim that the iterative form *requires* this is **false**: review
  built the fused loop with Slice 2's existing ordering untouched and disposed 1,000,000
  nodes under a 1 MB stack, exit 0. The loop's internal copyout-then-free is inherent to a
  loop and constrains nothing about non-recursive cell destructors. R8 therefore buys one
  ordering rule instead of two, plus lower peak disposal memory, at the cost of reversing a
  contract Slice 2 labelled contractual. **Consequence: phases 3 and 4 are independent and
  may land in either order.**
  Soundness is confirmed: `load_owned_payload` (@ir.rs:1906-1922) copies the payload out
  for every shape before the block is touched — `Blit` into a fresh frame slot for
  struct/enum/array, `FieldLoad` into a register for scalars **and for a nested
  `OwnedCell`**, so `^^T` copies the inner pointer out and freeing the outer block cannot
  touch the inner one. No lazy or by-reference read of the block exists after the copy.
- **R9** — R8's revision is **exactly** the following, and lands in the same commit.
  Verified exhaustively: these are the only two goldens in the suite whose expected stdout
  is ordering-sensitive across a drop/free or a free/free pair. Every REPL `drop N` golden
  is `__spy`-only with no cell, and the one REPL cell golden
  (`["alloc 8", "stack: <^i64>", "free 8"]`) has no linear payload, so none of them move.
  - `owned_linear_payload_drops_before_free` (tests/phase0.rs),
    `"alloc 8\ndrop 7\nfree 8\n"` → `"alloc 8\nfree 8\ndrop 7\n"`. **Both the test name and
    its comment ("the payload drops before the cell frees") become false and must be
    rewritten.**
  - `nested_owned_frees_inner_before_outer` (tests/phase0.rs),
    `"alloc 24\nalloc 8\nfree 24\nfree 8\n"` → `"alloc 24\nalloc 8\nfree 8\nfree 24\n"`.
    **The name inverts, and its comment explaining that the distinct 24/8 sizes prove the
    inner cell frees first now proves the opposite**; both must be rewritten. The distinct
    sizes remain valuable — they are what makes the new order falsifiable too.
  - Doc corrections in `docs/phase3-slice2-spec.md`: the R5 statement calling the ordering
    contractual, the word table, the phase log, and the criterion rows for the `^__spy`
    disposal and the `^^[u8 24]` nesting; plus the two corresponding passages in
    `docs/phase3-slice2-brief.md`.
- **R10** — **Disposal becomes pre-order**, explicitly and as a contract: a node's own
  fields are dropped and its cell freed *before* descending to the next node. Today's
  behaviour is post-order (the deepest node is disposed first). The fused loop forces
  pre-order for recursive types, and R8 already commits the non-recursive case to the same
  direction, so this is the consistent completion rather than a second rule. It matches
  Rust (own body first, then fields), it lowers peak disposal memory, and it is the
  ordering **Slice 6's user destructor bodies will inherit**, where it is the more useful
  choice because a node's children are still alive when its destructor runs.

### The iterative destructor

- **R11** — A type that is **directly** self-recursive (it has a variant field or struct
  field whose type is literally `^Self`) gets **one fused loop** over that cycle instead of
  mutually-recursive `cell_drop`/`enum_drop` functions. No general or mutual TCO is
  required. Reuses `begin_loop` (@ir.rs:1424) and `finalize_loop` (@ir.rs:1445), which
  review confirmed depend on nothing specific to user words. Shape:

  ```text
  drop_List(v):
    loop:
      match tag(v):
        Nil  -> ret
        Cons -> <drop every non-recursive linear field of v>
                p    = <the recursive ^ field of v>
                next = copyout(p)      # MUST be last: see R12
                free(p)
                v = next; continue
  ```

- **R12** — **The copyout-ordering invariant, which makes the loop correct.** The
  loop-carried value is a pointer to a frame slot, and `push_alloc` (@ir.rs:1386-1397)
  hoists that `Alloc` into the entry block so the loop does not grow the frame. There is
  therefore **one slot, reused every iteration**, and `copyout(p)` blits the next node into
  the very memory the current node occupies. **Every read of the current node — tag
  dispatch, field loads, non-recursive field drops, sibling recursions — must be emitted
  before the copyout.** This is a requirement, not an implementation detail: review
  produced live memory corruption by emitting in declaration order instead, yielding
  garbage `__spy` tags (`drop 23`, `drop 0`) while real nodes were never dropped, **with
  the alloc/free trace still perfectly balanced**, so no count-based or free-only golden
  can catch it. @ir.rs:1295-1311 already documents this aliasing hazard.
- **R13** — **Phase 4 builds its own recursion pass**; it does **not** reuse the checker's
  cycle information. The checker's graph deliberately deletes exactly the `^` edges phase 4
  needs (R3), `visit_recursion` (@check.rs:557+) returns `Result` and errors on the first
  back edge rather than producing components, and no SCC code exists in the repo. The pass
  belongs in `ir.rs` over `Registries`. For the direct case it needs no SCC at all: for
  enum `E`, a variant field of type `IrType::OwnedCell(c)` is a recursive edge iff
  `cells.payload[c.index()] == IrType::Enum(E)`. Note a `^`-inclusive graph genuinely has
  cycles, so any walk needs a visited set to terminate.
- **R14** — **The guarantee is narrow and must be stated as such.** Constant-stack
  disposal is guaranteed **only** for directly self-recursive types. These shapes keep
  O(depth) recursion and still overflow, all verified to compile and run today:
  - **indirect cycles** — the recursive `^` reached through an intervening struct
    (`type: Wrap v i64 n ^List ;` / `type: List | Nil | Cons w Wrap ;`): SIGSEGV at 200,000
    nodes, 8 MB;
  - **`^^Self`** (`type: L | Nil | Cons n ^^L ;`): the loop's `next = copyout(p)` yields
    `^L` not `L`, so the phi type does not match and the shape is excluded by construction;
  - **non-loop children** — a left-leaning tree under R17: SIGSEGV at 100,000 nodes, 8 MB.

  No user-facing note, README line or ROADMAP entry may claim "recursive heap data disposes
  in constant stack" without this qualification.
- **R15** — **Non-recursive destructors keep straight-line synthesis**, unchanged apart
  from R8's ordering. They are the overwhelming majority and must not acquire loop
  machinery.
- **R16** — `synthesize_struct_destructor` (@ir.rs:928) and `synthesize_enum_destructor`
  (@ir.rs:970) call `seal_block(Terminator::Ret(None))` unconditionally; `lower_word` guards
  with `if !b.terminated` (@ir.rs:1261) because a body ending in a back edge is already
  sealed. Phase 4 **must add that guard**, or a duplicate `BlockId` reaches the emitter.
  A self-recursive *struct* destructor is an exit-less loop and hits this immediately. The
  exit-less case is itself fine (QBE accepts a void function whose only path is a loop) and
  vacuously correct because such a type is uninhabited (R5), but it needs a
  "compiles, does not crash the emitter" test since R4+R5+R11 combine to generate it.
- **R17** — For a type with **several** recursive fields, the loop takes the **last
  recursive field in declaration order** and the others are recursed. A balanced tree is
  then log n deep; a left-leaning one is O(n) and is a documented limitation under R14, not
  a fixed case.
- **R18** — **Mutually recursive types** keep today's recursive destructor and its depth
  limit. A fused loop over a multi-type cycle needs a tagged loop over the whole component
  (the tier-2 SCC-contraction shape) and is out of scope. The fallback must be **tested**.

### Allocation failure

- **R19** — **The OOM revisit is closed, not deferred again.** Slice 2 deferred
  trap-vs-return here on the premise that optional pointers would be a compiler-known type
  to return. They are not: nullability is an ordinary user-authored two-variant enum (as
  `MaybeInt` already is in `examples/shapes.sth`), because a compiler-synthesized
  per-payload `Option` is exactly what Phase 4's generics delete. The allocator has nothing
  privileged to return, so **the trap stays**. No allocator, shim, trap or trace code
  changes in this slice.

### Test discipline (binding)

- **R20** — Every criterion is a runnable golden (native binary or REPL session), never an
  IL-string assertion, except where genuinely emitter-level. Trace-observing goldens assert
  the **full stdout** with `assert_eq!`, exact and ordered: never `contains`, never
  counting. Every negative golden asserts the diagnostic substring **and** the backticked
  type name, with an `"unexpected message: {err}"` context. Naming is
  `thing_condition_expected`.
- **R21** — Criterion 8 must be **verified to fail on the pre-change compiler** before the
  change lands. Already discharged: the base commit segfaults at 100,000 nodes at both 8 MB
  and 1 MB, so 1M at 1 MB cannot pass vacuously.

## Load-bearing invariants (must survive)

- Backend stays QBE; no LLVM. `Ptr[T]` stays opaque, never assumed to be a `u64`.
- The linear spine holds: exactly-once, no auto-drop, forgetting is a compile error.
- `core` stays `no_std`. No in-process JIT, no comptime interpreter.
- No user-facing FFI. The allocator shim stays a backend special case (R19).
- No new ad-hoc type constructor; `^` and `[T N]` remain the only two.

## Delivery phases

1. **`isize` (R1) and the `norm_scalar` word-width correction (R2).** Independent of
   everything else.
2. **Pin the recursion rule (R3–R7).** Production change expected to be zero or near-zero:
   making `type_node`'s exclusion of cell edges explicit rather than a fall-through.
3. **Reverse the free ordering (R8) and land the enumerated Slice 2 revision (R9)** in one
   commit, including both test renames. Independent of phase 4; may land before or after it.
4. **The iterative destructor (R11–R16)**, its own recursion pass, the copyout invariant,
   and the pre-order contract (R10). The centrepiece.
5. **Multi-child and mutual recursion (R17, R18), the documented limitation tests (R14),
   `examples/list.sth`, and the regression sweep.**

## Criterion → test map

Goldens live in `tests/phase0.rs` (native) or `tests/phase1.rs` (REPL), except criteria 2,
3 and 4, which are check-stage errors or accepts with no runtime and belong in unit tests
beside their stage.

| # | criterion | test |
|---|---|---|
| 1 | `isize` declares, computes, prints signed, converts to/from `i64`; mixing with `usize` is a mismatch | `isize_round_trips_arithmetic_and_conversion` |
| 1b | `usize` and `isize` widths derive from the word-width parameter, not a literal 64 (R2) | `scalar_widths_follow_word_width_for_both_size_types` |
| 2 | by-value self-recursion is a located error naming the cycle | `check_recursion_by_value_self_cycle_is_error` |
| 3 | by-value **mutual** recursion is a located error naming the full path | `check_recursion_by_value_mutual_cycle_is_error` |
| 4 | a `^` cycle through a **struct field** is accepted (R4) | `check_recursion_cell_cycle_in_struct_field_is_ok` |
| 5 | a recursive list with a `__spy` field builds, walks and disposes in the exact expected order — plain sizes alone do **not** discriminate the transform, since every cell is the same size | `recursive_list_disposes_in_expected_order` |
| 6 | a cell traces `free` **before** the payload's `drop` (R8) | `owned_cell_frees_before_dropping_payload` |
| 7 | the two revised Slice 2 goldens assert the new order, under corrected names (R9) | `owned_linear_payload_frees_before_dropping_payload`, `nested_owned_frees_outer_before_inner` |
| 8 | **1,000,000-node list disposes under `ulimit -s 1024`, exit 0** (R11) | `deep_list_disposes_in_constant_stack` |
| 9 | disposal is pre-order: a node's own `__spy` drops before the next node's (R10) | `recursive_disposal_is_pre_order` |
| 10 | **the copyout invariant**: a type whose recursive field is declared **first**, with two distinct `__spy` tags after it, disposes with correct tags and no garbage (R12) | `recursive_destructor_reads_node_before_overwriting_slot` |
| 11 | a non-recursive cell's destructor is behaviourally unchanged apart from R8 ordering (R15) | `non_recursive_cell_disposal_is_unchanged` |
| 12 | a binary tree builds and disposes correctly (R17) | `recursive_tree_builds_and_disposes` |
| 13 | the loop takes the last recursive field: distinct `__spy` tags per subtree make loop-vs-recurse order observable at runtime (R17, R20 forbids an IL assertion) | `multi_child_destructor_loops_on_last_recursive_field` |
| 14 | mutually recursive types with a base case in one of them build and dispose (R18) | `mutually_recursive_types_dispose_on_recursive_path` |
| 15 | a self-recursive **struct** (uninhabited, exit-less destructor) compiles and does not crash the emitter (R16) | `self_recursive_struct_destructor_compiles` |
| 16 | **documented limitations** (R14): the wrapper-struct list, `^^Self`, and a left-leaning tree each compile, run and dispose correctly at modest depth, with a comment recording that they remain O(depth) | `indirect_recursion_shapes_dispose_but_are_depth_limited` |
| 17 | `examples/list.sth` runs with exact stdout | `example_list_matches_golden` |
| 18 | REPL: a residual recursive value is disposed at `:quit` | `repl_quit_frees_residual_recursive_value` |
| 19 | no regression: 14 existing examples byte-identical, suite green | existing suite |

Criterion 13 must observe field choice **at runtime** via distinct `__spy` tags, not by
asserting emitted IL, which R20 forbids. Criterion 11 is behavioural for the same reason;
an IL baseline cannot be captured before phase 3, since phase 3 changes the IL.

## Explicitly out of scope

Fused loops for **indirect** recursion (the wrapper-struct list and `^^Self`) — the same
mechanism generalized to a projection path, and the natural **follow-on extension** to this
slice. Worklist-based disposal for **branching** structures (left-leaning trees), which
needs allocation during disposal and collides with R19's trap; deferred until a real client
needs it. Fused loops over multi-type cycles (R18, tier-2). Compiler-provided
`Option`/`Result` or any synthesized nullable-pointer type (Phase 4 generics). Returning
allocation failure rather than trapping (R19, Phase 4). Pointer arithmetic and pointer
differences. Zippers and compiler-generated one-hole types. Second-class refs and
`let`/`inout`/`sink`/`set` (Slice 4); reference counting (Slice 5); user-definable
destructor bodies (Slice 6); growable buffers and `Vec` (Phase 6 `alloc`).

## Phases

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "isize mirroring usize as a distinct IrType variant, and correcting norm_scalar's hardcoded 64-bit width for both size types",
      "difficulty": "standard",
      "changes": [
        "Add isize to the type name table and integer tower with a distinct IrType::Isize variant",
        "Duplicate or parameterise the usize-named coercion plumbing: SlotMatch::LiteralUsize, SlotMatch::NeedsUsizeConversion, PairMatch::NeedsUsizeConversion, the (Usize, I64) literal-coercion pairs, and usize_conversion_needed_error's hardcoded >usize message",
        "Mixing usize and isize is a plain type mismatch",
        "Route printing to the signed format rather than $ufmt",
        "Audit the seven backend catch-all match arms that rustc exhaustiveness will not flag",
        "Correct norm_scalar's hardcoded bits: 64 so both usize and isize derive from the target word width"
      ],
      "tests": [
        "isize_round_trips_arithmetic_and_conversion",
        "scalar_widths_follow_word_width_for_both_size_types"
      ],
      "exit": "isize behaves as usize does but signed; neither size type carries a literal 64; suite green and no example output changes"
    },
    {
      "phase": 2,
      "focus": "Pin the recursion rule with tests so the currently-incidental cut of cell edges becomes intentional",
      "difficulty": "standard",
      "changes": [
        "Make type_node's exclusion of OwnedCell edges explicit rather than a fall-through",
        "No relaxation of by-value cycle detection"
      ],
      "tests": [
        "check_recursion_by_value_self_cycle_is_error",
        "check_recursion_by_value_mutual_cycle_is_error",
        "check_recursion_cell_cycle_in_struct_field_is_ok"
      ],
      "exit": "Recursion through ^ is legal and tested in struct and enum position; by-value cycles still rejected with a path-naming diagnostic"
    },
    {
      "phase": 3,
      "focus": "Reverse the free ordering globally and land the enumerated Slice 2 golden and doc revision in the same commit",
      "difficulty": "standard",
      "changes": [
        "synthesize_cell_destructor frees the block before dropping the payload copy",
        "Rewrite tests/phase0.rs:2443 owned_linear_payload_drops_before_free, including its now-false name",
        "Rewrite tests/phase0.rs:2478 nested_owned_frees_inner_before_outer, including its inverted name and its comment asserting the opposite claim",
        "Correct docs/phase3-slice2-spec.md lines 11, 38, 66, 85 and 88, and docs/phase3-slice2-brief.md lines 80 and 134"
      ],
      "tests": [
        "owned_cell_frees_before_dropping_payload",
        "owned_linear_payload_frees_before_dropping_payload",
        "nested_owned_frees_outer_before_inner"
      ],
      "exit": "One ordering rule for every cell; no test, name, comment or document still asserts the old order. Independent of phase 4"
    },
    {
      "phase": 4,
      "focus": "Synthesize a fused iterative destructor for directly self-recursive types, with the copyout-ordering invariant and its own recursion pass",
      "difficulty": "hard",
      "changes": [
        "Build a recursion-detection pass in ir.rs over Registries; do not reuse the checker's graph, which deletes the cell edges this needs",
        "For the direct case test whether a variant or struct field of type OwnedCell(c) has cells.payload[c] equal to the enclosing type",
        "Emit one fused loop using begin_loop and finalize_loop",
        "Emit every read of the current node before the copyout that overwrites the reused frame slot",
        "Guard the trailing seal_block on b.terminated so an exit-less loop does not produce a duplicate BlockId",
        "Keep straight-line synthesis for every non-recursive type"
      ],
      "tests": [
        "deep_list_disposes_in_constant_stack",
        "recursive_destructor_reads_node_before_overwriting_slot",
        "recursive_disposal_is_pre_order",
        "recursive_list_disposes_in_expected_order",
        "non_recursive_cell_disposal_is_unchanged",
        "self_recursive_struct_destructor_compiles"
      ],
      "exit": "A 1,000,000-node list disposes under ulimit -s 1024 with exit 0, verified to fail on the pre-change compiler; no garbage tags from slot reuse; non-recursive destructors behaviourally unchanged apart from ordering"
    },
    {
      "phase": 5,
      "focus": "Multi-child and mutually recursive types, the documented depth limitations, the list dogfood, and the regression sweep",
      "difficulty": "standard",
      "changes": [
        "Loop on the last recursive field in declaration order, recurse the others",
        "Leave mutually recursive types on the recursive destructor path",
        "Record the wrapper-struct, double-cell and left-leaning-tree shapes as depth-limited, in tests and in any user-facing note",
        "Add examples/list.sth"
      ],
      "tests": [
        "recursive_tree_builds_and_disposes",
        "multi_child_destructor_loops_on_last_recursive_field",
        "mutually_recursive_types_dispose_on_recursive_path",
        "indirect_recursion_shapes_dispose_but_are_depth_limited",
        "example_list_matches_golden",
        "repl_quit_frees_residual_recursive_value"
      ],
      "exit": "Trees and mutual recursion dispose correctly, the narrow guarantee is stated honestly and its exclusions are tested rather than silent, all 14 prior examples byte-identical and the suite green"
    }
  ]
}
```
