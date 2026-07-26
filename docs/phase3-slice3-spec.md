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

**Why `isize` lands here with no in-slice consumer.** `ROADMAP.md` names pointer
differences as `isize`'s only motivation, and this slice explicitly puts pointer
arithmetic out of scope, so R1/R2 add a type nothing in this slice calls — exactly the
kind of speculative addition CLAUDE.md warns against. The justification is narrower than
"future-proofing": `isize` is the ROADMAP's own promised counterpart to `usize` (both were
introduced together in the Slice 5 entry as "`usize` and likely `isize`"), it closes a
legible, already-named gap in the integer tower rather than opening a new one, and its
cost is genuinely small — mirroring a type that already exists, not designing one. If this
reasoning does not hold, phase 1 should move to whichever slice first calls it.

- **R1** — `isize` is a signed, target-width integer, mirroring `usize` including a
  distinct `IrType::Isize` variant, so the two sibling types are represented
  symmetrically. The work is **not** width (already parameterised via
  `scalar_size_align_ww` @ir.rs:355) but the coercion and diagnostic plumbing, which is
  `usize`-*named* rather than generic and must be either duplicated or parameterised by
  target type: `SlotMatch::{LiteralUsize, NeedsUsizeConversion}` (@check.rs:78-79),
  `PairMatch::NeedsUsizeConversion` (@check.rs:104), the `(Usize, I64)` literal-coercion
  pairs (@check.rs:113-115), and `usize_conversion_needed_error` (@check.rs:1535), whose
  message hardcodes `>usize`. Mixing `usize` and `isize` is a plain type mismatch.
  Printing routes to the signed format, not `$ufmt` (@qbe.rs:888).
  **Correction after review**: an earlier draft warned that backend `_ =>` catch-alls would
  hide missed sites. That is false — the `IrType` matches at @qbe.rs:217, 254, 281, 509,
  720 and 888 are all **exhaustive**, so rustc flags every one when the variant is added
  (the `_ =>` at @qbe.rs:300 belongs to `alloc_op`, which matches on a `u32` alignment, not
  on `IrType`). Adding the variant is therefore *safer* than described. The one genuine
  catch-all is `norm_scalar`'s `other => other`, which would silently pass `Isize` through
  unnormalized — and that is exactly the function R2 rewrites.
- **R2** — `norm_scalar` (@qbe.rs:508) hardcodes `bits: 64` for `Usize`, contradicting its
  own word-width comment, and it **takes no width parameter**, so "correct it" is not
  expressible without a signature change. It gains a `word_width` parameter following the
  established `_ww` convention (`scalar_size_align_ww` @ir.rs:350, `build_registries_ww`
  @ir.rs:430), and both `Usize` and `Isize` derive their width from it. Mirroring without
  this would propagate a latent 64-bit assumption into the new type, which is what
  word-width-neutrality exists to prevent. The change is a behavioural no-op on the only
  existing target, so it is pinned by a unit test that passes an explicit flipped width —
  exactly how `word_width_parameter_sizes_usize_not_a_literal_eight` (@ir.rs:2711-2726)
  already pins the equivalent claim for layout.

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
- **R7** — By-value cycles keep their existing diagnostic naming the full path, for self
  and mutual recursion alike. **Correction after review**: an earlier draft called this
  diagnostic "located." It is not — `visit_recursion` (@check.rs:559) returns a bare
  `String` with no span and unbackticked type names (`Bad -> Bad`, not `` `Bad` ``), unlike
  an ordinary checker error (`` unknown word `nosuch` in `main` (line 1) ``). R20's
  backticked-type-name rule is carved out for this one message: it never had backticks and
  adding a span is a real, unscoped piece of work, not the "near-zero" change phase 2
  claims. This slice **keeps** the message as-is; it does not add location.

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
- **R9** — R8's revision is **exactly** the following, and lands in the same commit. An
  earlier draft claimed only two stdout goldens were affected and called that exhaustive;
  review found that search was scoped to `assert_eq!(stdout, ...)` in `tests/` and **missed
  a unit test and three prose sites**. The corrected list is below. Confirmed *not*
  affected: every REPL `drop N` golden is `__spy`-only with no cell; the one REPL cell
  golden (`["alloc 8", "stack: <^i64>", "free 8"]`) has no linear payload; and
  @ir.rs:1941's comment about `^>` materialising before freeing stays true, because unwrap
  already copies out then frees and has no payload *drop* to reorder.
  - `owned_linear_payload_drops_before_free` (tests/phase0.rs),
    `"alloc 8\ndrop 7\nfree 8\n"` → `"alloc 8\nfree 8\ndrop 7\n"`. **Both the test name and
    its comment ("the payload drops before the cell frees") become false and must be
    rewritten.**
  - `nested_owned_frees_inner_before_outer` (tests/phase0.rs),
    `"alloc 24\nalloc 8\nfree 24\nfree 8\n"` → `"alloc 24\nalloc 8\nfree 8\nfree 24\n"`.
    **The name inverts, and its comment explaining that the distinct 24/8 sizes prove the
    inner cell frees first now proves the opposite**; both must be rewritten. The distinct
    sizes remain valuable — they are what makes the new order falsifiable too.
  - **`synthesized_cell_destructor_copies_out_a_linear_aggregate_payload_before_freeing`
    (a unit test in `src/ir.rs`)** — asserts the call sequence
    `vec!["sooth_struct_drop_0", FREE_SYMBOL]`, which **inverts** to
    `vec![FREE_SYMBOL, "sooth_struct_drop_0"]`. Its name, its doc comment ("both precede
    the free") and its assertion message ("the payload's own destructor runs, then the cell
    frees") all become false. Its second assertion, `blit_at < calls[0].0`, stays true in
    substance — the copy-out must still precede everything — but its message needs
    rewording since `calls[0]` becomes the free.
  - **`src/ir.rs:985`** — `synthesize_cell_destructor`'s doc comment, "Drop the payload
    first if it is linear, then free the cell", becomes the opposite.
  - **`ROADMAP.md:73`** — "able to hold a linear payload (dropped before the cell is
    freed)" becomes false.
  - Doc corrections in `docs/phase3-slice2-spec.md`: the R5 statement calling the ordering
    contractual, the word table, the phase log, and the criterion rows for the `^__spy`
    disposal and the `^^[u8 24]` nesting; plus `docs/phase3-slice2-brief.md:151`
    ("payload first, then free. Pin it in a golden").
- **R10** — **Disposal becomes pre-order**, explicitly and as a contract: a node's own
  fields are dropped and its cell freed *before* descending to the next node. Today's
  behaviour is post-order (the deepest node is disposed first). The fused loop forces
  pre-order for recursive types, and R8 already commits the non-recursive case to the same
  direction, so this is the consistent completion rather than a second rule. It matches
  Rust (own body first, then fields), it lowers peak disposal memory, and it is the
  ordering **Slice 6's user destructor bodies will inherit**, where it is the more useful
  choice because a node's children are still alive when its destructor runs.

### The iterative destructor

- **R10b** — R10's ordering claim is "parent before children." An earlier draft glossed
  it as matching Rust's `Drop::drop`-before-fields; that is Slice 6's still-open question
  (whether a user destructor body runs before or instead of synthesized field glue,
  `ROADMAP.md`), not this one, and citing it risks pre-judging that decision. Drop the
  analogy; keep the claim.
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
  belongs in `ir.rs` over `Registries`. For the direct case it needs no SCC at all: a
  variant field **or struct field** of type `IrType::OwnedCell(c)` is a recursive edge iff
  `cells.payload[c.index()]` is `IrType::Enum(E)` or `IrType::Struct(S)` for the enclosing
  type — the struct half is what triggers R16's exit-less-destructor case. Note a
  `^`-inclusive graph genuinely has cycles, so any walk needs a visited set to terminate.
- **R14** — **The guarantee is narrow and must be stated as such.** Constant-stack
  disposal is guaranteed **only** for directly self-recursive types. The following shapes,
  **among others**, keep O(depth) recursion and still overflow, all verified to compile and
  run today:
  - **indirect cycles** — the recursive `^` reached through an intervening struct
    (`type: Wrap v i64 n ^List ;` / `type: List | Nil | Cons w Wrap ;`): SIGSEGV at 200,000
    nodes, 8 MB;
  - **`^^Self`** (`type: L | Nil | Cons n ^^L ;`): the loop's `next = copyout(p)` yields
    `^L` not `L`, so the phi type does not match and the shape is excluded by construction;
  - **non-loop children** — a left-leaning tree under R17: SIGSEGV at 100,000 nodes, 8 MB;
  - **mutually recursive types** (R18): still on the recursive destructor path by
    definition, so still O(depth);
  - **mixed types**, both directly and indirectly recursive: the loop takes the direct
    field, but disposal through the indirect field stays O(depth). The loop's presence
    does not make the type's *other* cycles constant-stack.

  Recursion through an array is correctly absent from this list: `[^T N]` is rejected by
  Slice 2's linear-array-element rule, so it cannot launder indirection at all (R6).
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

- **R19** — **The trap-vs-return decision is closed, not deferred again.** Slice 2 deferred
  it on the premise that optional pointers would be a compiler-known type to return. They
  are not: nullability is an ordinary user-authored two-variant enum (as `MaybeInt` already
  is in `examples/shapes.sth`), because a compiler-synthesized per-payload `Option` is
  exactly what Phase 4's generics delete. The allocator has nothing privileged to return, so
  **the trap stays**. Revisit in Phase 4 with real generics. No allocator, shim, trap or
  trace code changes in this slice. **What stays open**: Slice 2 also recorded a known-good
  technique for a runtime OOM golden (`LD_PRELOAD`-interposing `malloc` to return NULL for
  small sizes, sound where `ulimit`/`RLIMIT_AS` are not) and today's only coverage is the
  IL-level `emitted_alloc_shim_has_null_trap`. That golden is **not** written by this slice
  either — the trap-vs-return *decision* is closed; the trap-vs-return *test* stays
  deferred, and the spec says so rather than implying it is now covered.
- **R19b** — **The ROADMAP no longer matches this slice's scope and must be corrected in
  the same delivery.** `ROADMAP.md:80` names "optional / non-null pointers" as this
  slice's job, and the slice-3 entry frames the OOM revisit as introducing them
  ("this is the slice that introduces optional / non-null pointers, so it is the first
  point at which a failed allocation can be *returned*"). R19 overturns exactly that
  premise: there is no compiler-known optional/non-null pointer type, in this slice or at
  all. The ROADMAP entry must be rewritten to describe what this slice actually does
  (recursion through `^`, the iterative destructor, `isize`), rather than a phase 4 planning
  document silently disagreeing with a shipped slice.

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
   **Whichever phase lands second must root its own trace goldens at a bare enum value, not
   a `^`-wrapped one**: a cell-rooted trace's outermost `free` comes from
   `sooth_cell_drop_*` and moves under R8, so a golden written before both phases land could
   assert an ordering that depends on which landed first. Rooting at the enum value itself
   sidesteps the dependency entirely.
4. **The iterative destructor (R11–R16)**, its own recursion pass, the copyout invariant,
   and the pre-order contract (R10). The centrepiece.
5. **Multi-child and mutual recursion (R17, R18), the documented limitation tests (R14),
   `examples/list.sth`, the ROADMAP correction (R19b), and the regression sweep.**
   **`examples/list.sth` must both walk and dispose**, not dispose alone: a purely
   consuming walk exercises Slice 6's TCO, which already works today, not this slice's
   destructor. `ROADMAP.md`'s own dogfood description ("builds, walks, and is explicitly
   freed") already asks for both; the example should build, sum via a non-consuming or
   consuming walk, then `drop` what remains, so it demonstrates something this slice adds.

## Criterion → test map

Goldens live in `tests/phase0.rs` (native) or `tests/phase1.rs` (REPL), except criteria 1b,
2, 3, 4 and 4b, which have no observable runtime behaviour and belong in unit tests beside
their stage.

Every `__spy`-observing golden below uses **distinct tags per node** and asserts the **full
stdout**. This is not stylistic: R12 records that the copyout bug corrupted tags while
leaving the alloc/free trace perfectly balanced, and criterion 5 notes that uniform cell
sizes make a size-only transcript non-discriminating. Where a transcript is the proof, the
spec states the expected transcript so implementation cannot ratify whatever it emits.

| # | criterion | test |
|---|---|---|
| 1 | `isize` declares, computes, prints signed, converts to/from `i64` | `isize_round_trips_arithmetic_and_conversion` |
| 1b | `norm_scalar(Usize, 4)` is `Int { bits: 32, signed: false }` and `norm_scalar(Isize, 4)` is `Int { bits: 32, signed: true }`, plus the 8-byte cases (R2). Unit test: no non-64-bit target exists, so no program behaviour can distinguish this | `norm_scalar_follows_word_width_for_both_size_types` |
| 1c | mixing `usize` and `isize` is a located error naming both backticked types (R1) | `check_isize_mixed_with_usize_is_error` |
| 1d | an `isize`-declared output needs an explicit conversion, with the message naming the `isize` form rather than the hardcoded `>usize` (R1) | `check_isize_declared_output_needs_conversion_is_error` |
| 2 | by-value self-recursion is rejected naming the cycle (R7's carve-out: unbackticked, unlocated, unchanged from today) | `check_recursion_by_value_self_cycle_is_error` |
| 3 | by-value **mutual** recursion is rejected naming the full path `A -> B -> A` (the genuinely uncovered case; self-recursion is already partly covered) | `check_recursion_by_value_mutual_cycle_is_error` |
| 4 | a `^` cycle through a **struct field** is accepted (R4) | `check_recursion_cell_cycle_in_struct_field_is_ok` |
| 4b | `[^T N]` stays rejected and an array element remains a by-value cycle edge (R6) | `check_recursion_array_element_is_a_value_edge` plus the existing `check_value_recursion_through_array_element_is_error` |
| 5 | a recursive list with a `__spy` per node builds, walks and disposes in the exact expected order | `recursive_list_disposes_in_expected_order` |
| 6 | R8 ordering for an **aggregate** payload containing a linear field — the `Blit`-into-a-frame-slot arm of `load_owned_payload`, where freeing early is most likely to bite, and the one shape criterion 7 does not reach | `owned_aggregate_payload_frees_before_dropping_fields` |
| 7 | the three revised Slice 2 assertions carry the new order under corrected names (R9) | `owned_linear_payload_frees_before_dropping_payload`, `nested_owned_frees_outer_before_inner`, `synthesized_cell_destructor_frees_before_dropping_a_linear_aggregate_payload` |
| 8 | **1,000,000-node list disposes under `ulimit -s 1024`, exit 0** (R11) | `deep_list_disposes_in_constant_stack` |
| 9 | disposal is pre-order: a node's own `__spy` drops before the next node's (R10) | `recursive_disposal_is_pre_order` |
| 10 | **the copyout invariant**: recursive field declared **first**, two distinct `__spy` tags after it. A violation loses the deeper node's tags or repeats a node's, while the alloc/free trace stays balanced (R12) | `recursive_destructor_reads_node_before_overwriting_slot` |
| 11 | **the detection pass does not false-positive** (R13, R15): three near-miss shapes stay straight-line — a cell whose payload is a *different* aggregate, `^^T` whose inner payload is not the enclosing type, and a struct holding a `^` to an unrelated enum | `non_recursive_cell_shapes_are_not_treated_as_recursive` |
| 12 | a binary tree builds and disposes with distinct per-node tags in the stated order (R17) | `recursive_tree_builds_and_disposes` |
| 13a | tag order shows the loop descends the **last** recursive field: looping the first would print `1, 3, 2` where looping the last prints `1, 2, 3` (R17) | `multi_child_destructor_loops_on_last_recursive_field` |
| 13b | **a 1,000,000-node right-leaning tree disposes under `ulimit -s 1024`, exit 0.** 13a alone passes on a compiler that never builds a multi-child loop at all, since it only distinguishes left-from-right traversal; depth is what proves the loop exists | `deep_right_leaning_tree_disposes_in_constant_stack` |
| 14 | mutually recursive types (base case in one of them; the all-struct pair is uninhabited per R5) dispose correctly, **and** a mutual chain deep enough to overflow at 1 MB while the direct list passes, which is what proves the R18 fallback was taken | `mutually_recursive_types_dispose_on_recursive_path` |
| 15 | a self-recursive **struct** (uninhabited, exit-less destructor) compiles and does not crash the emitter (R16). Non-vacuous: destructors are synthesized for every declared type, so the declaration alone exercises the path | `self_recursive_struct_destructor_compiles` |
| 16 | **the limitation boundary** (R14): at 1M nodes under `ulimit -s 1024` the direct list exits 0 while the wrapper-struct list and the left-leaning tree do not. Asserting the *asymmetry* is the point; "disposes at modest depth" is already true on the base commit and would pass a compiler where phase 4 was never written | `indirect_recursion_shapes_remain_depth_limited` |
| 17 | `examples/list.sth` runs with exact stdout | `example_list_matches_golden` |
| 18 | REPL: a residual recursive value is disposed at `:quit` | `repl_quit_frees_residual_recursive_value` |
| 19 | no regression: 14 existing examples byte-identical, suite green | existing suite |

**Harness notes.** `run_binary` (@tests/phase0.rs:20-37) does
`status.code().expect("process should exit normally, not die by signal")`, so a SIGSEGV
fails the test but reports the wrong thing. Criteria 8, 13b and 16 need a helper returning
the exit status rather than unwrapping it, asserting `Some(0)` with a message naming stack
overflow, and for 16 asserting the negative case explicitly. The `ulimit` precedent is
`run_owned_memory_bounded_golden` (@tests/phase0.rs:2380-2404); a `ulimit -s` sibling is a
two-line change, and the signal propagates through `sh` because `exec` replaces it.
Measured: building 1M nodes is tail-recursive and needs no stack (build plus a consuming
walk runs at `ulimit -s 256` in 0.15 s), so these criteria are cheap. The base compiler
SIGSEGVs at 1 MB, 8 MB **and** 64 MB, surviving only at 256 MB, so R21 holds with a wide
margin.

**Coverage gaps, stated rather than implied.** R13 has no direct criterion because the pass
is structurally invisible at runtime; criterion 11 is its proxy. R19 has none because the
slice changes no allocator code, and there is **no OOM-trap test anywhere in the suite**, so
"unchanged" rests on criterion 19's example sweep. Criteria 2 and 3 partly duplicate
existing coverage (`check_value_recursion_through_array_element_is_error` @src/check.rs:2960
and the REPL goldens at @tests/phase1.rs:299 and :544); phase 2 should extend rather than
clone them, and the genuinely untested parts are the mutual path naming and the `^`-cycle
accept.

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
        "Give norm_scalar a word_width parameter following the established _ww convention, since it currently takes none, and derive both Usize and Isize widths from it rather than the hardcoded 64",
        "Note its other => other arm is the one genuine catch-all that would silently pass Isize through unnormalized; every other backend IrType match is exhaustive and rustc will flag it"
      ],
      "tests": [
        "isize_round_trips_arithmetic_and_conversion",
        "norm_scalar_follows_word_width_for_both_size_types",
        "check_isize_mixed_with_usize_is_error",
        "check_isize_declared_output_needs_conversion_is_error"
      ],
      "exit": "isize behaves as usize does but signed; neither size type carries a literal 64, pinned by a unit test passing an explicit flipped width; suite green and no example output changes"
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
        "check_recursion_cell_cycle_in_struct_field_is_ok",
        "check_recursion_array_element_is_a_value_edge"
      ],
      "exit": "Recursion through ^ is legal and tested in struct and enum position; by-value cycles still rejected with a path-naming diagnostic"
    },
    {
      "phase": 3,
      "focus": "Reverse the free ordering globally and land the enumerated Slice 2 golden and doc revision in the same commit",
      "difficulty": "standard",
      "changes": [
        "synthesize_cell_destructor frees the block before dropping the payload copy",
        "Rewrite the owned_linear_payload_drops_before_free golden including its now-false name and comment",
        "Rewrite the nested_owned_frees_inner_before_outer golden including its inverted name and its comment asserting the opposite claim",
        "Rewrite the src/ir.rs unit test synthesized_cell_destructor_copies_out_a_linear_aggregate_payload_before_freeing, whose asserted call sequence inverts and whose name, doc comment and assertion message all become false",
        "Correct synthesize_cell_destructor's doc comment at src/ir.rs:985 and the Slice 2 completion line at ROADMAP.md:73",
        "Correct the drop-ordering passages in docs/phase3-slice2-spec.md and docs/phase3-slice2-brief.md"
      ],
      "tests": [
        "owned_linear_payload_frees_before_dropping_payload",
        "nested_owned_frees_outer_before_inner",
        "synthesized_cell_destructor_frees_before_dropping_a_linear_aggregate_payload",
        "owned_aggregate_payload_frees_before_dropping_fields"
      ],
      "exit": "One ordering rule for every cell; no test, name, comment or document still asserts the old order; whichever of phases 3/4 lands second roots its own trace goldens at a bare enum value so the golden text has no dependency on landing order. Independent of phase 4"
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
        "non_recursive_cell_shapes_are_not_treated_as_recursive",
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
        "Record the wrapper-struct, double-cell, left-leaning-tree and mutually-recursive shapes as depth-limited, in tests and in any user-facing note",
        "Add examples/list.sth: build, walk (sum), then drop the remainder, not disposal alone",
        "Rewrite ROADMAP.md:80 and the slice-3 entry, which still name optional/non-null pointers as this slice's job and frame the OOM revisit as introducing them"
      ],
      "tests": [
        "recursive_tree_builds_and_disposes",
        "multi_child_destructor_loops_on_last_recursive_field",
        "deep_right_leaning_tree_disposes_in_constant_stack",
        "mutually_recursive_types_dispose_on_recursive_path",
        "indirect_recursion_shapes_remain_depth_limited",
        "example_list_matches_golden",
        "repl_quit_frees_residual_recursive_value"
      ],
      "exit": "Trees and mutual recursion dispose correctly, a 1M-node right-leaning tree disposes under ulimit -s 1024 so the multi-child loop is proven to exist rather than merely traversed in the right order, the limitation boundary is asserted as an asymmetry, all 14 prior examples byte-identical and the suite green"
    }
  ]
}
```
