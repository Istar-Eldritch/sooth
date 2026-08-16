# Phase 3 Slice 3 — Recursive heap data + `isize` (brief)

Slice 2 gave the linear spine a real resource: a heap cell that allocates and frees. This
slice makes that cell **point back at its own type**, which is what turns it from a box
into a data structure. The headline work is not the type rule (see recon below, it already
works) but making **disposal of a recursive value run in constant stack**.

Prerequisite state: Slice 2 is merged (`e56342f`), registry bundling landed (`dfec9c3`,
`cc02ad8`). HEAD `cc02ad8`, 654 tests green.

## Recon: what already works today (measured, not assumed)

All four findings below were produced by building and running real programs on `cc02ad8`.
They are the reason this slice is narrower in design and sharper in engineering than the
ROADMAP entry suggests.

1. **Recursion through `^` is already legal.** `type: List | Nil | Cons v i64 next ^List ;`
   and `type: Node v i64 next ^Node ;` both compile and run today. The cycle detector's
   `type_node` (@check.rs:501) maps only `Struct`/`Enum`/`Array` to graph nodes and falls
   through to `None` for everything else, so a `^` edge is **already cut** from the
   value-containment graph. This is correct behaviour arrived at incidentally, and it is
   currently **untested**, so nothing stops a future refactor from re-breaking it.
2. **By-value recursion is still correctly rejected**: `type: Bad v i64 next Bad ;` gives
   `recursive struct definition (infinite size): Bad -> Bad`.
3. **Building and walking a list is already constant-stack.** A consuming walk
   (`| Cons | v nx | v + nx ^> sum`) is self-tail-recursive, so Slice 6's TCO already
   applies. A 1,000,000-node list builds fine.
4. **Disposal is the broken part, and its limit is measured.** The synthesized destructor
   recurses on the native stack. Same binary, list of 100,000 nodes:

   | stack | result |
   |---|---|
   | 8 MB (default) | **SIGSEGV**, exit 139 |
   | 64 MB | exit 0 |
   | 1 MB | **SIGSEGV**, exit 139 |

   50,000 nodes passes at the 8 MB default; 100,000 does not. The failure is purely
   destructor recursion depth, proven by the same binary passing under a larger `ulimit -s`.

So: the type rule needs **pinning, not building**. The destructor needs rewriting.

## Locked decisions

- **D1 — `isize` lands here, exactly as `usize` did.** A signed, target-width integer, no
  new semantics, no pointer arithmetic (no such words exist). Mechanical mirror of the
  existing `usize` paths through lexer/parser/check/ir/backend. It rides in as an early
  delivery phase rather than its own brief→spec→review cycle, because it has zero design
  content; giving it a full lifecycle would be process disproportionate to the change.
- **D2 — There is NO compiler-provided optional type.** Nullability is expressed by an
  ordinary user-authored two-variant enum, exactly as `MaybeInt`'s `None`/`Some` already
  does in `examples/shapes.sth`. **Reason**: a compiler-synthesized `Option`-shaped enum
  interned per payload type is precisely the throwaway machinery Phase 4's generics exist
  to delete; once `Option<T>` is a real generic stdlib enum with actual type variables, the
  synthesis code is dead weight. Hand-written enums are not superseded by generics, merely
  complemented. This also sidesteps Slice 2's third-ad-hoc-constructor tripwire entirely
  rather than spending it.
- **D3 — The recursion rule: a type cycle is legal iff every cycle passes through at least
  one `^`.** Struct fields, enum variant payloads and array elements are **by-value edges**
  and participate in cycle detection; `^` edges are cut. Per recon 1 this is already the
  implemented behaviour, so the work is to make it **intentional and tested** (and to keep
  the by-value diagnostic sharp), not to build it.
- **D4 — No positional restriction.** Struct fields and enum variants alike. The rule is
  about size finiteness, not idiom; carving out "enum variants only" would be a rule about
  taste enforced by the type checker.
- **D5 — No uninhabitedness detection.** `type: Node v i64 next ^Node ;` is finite in size
  but impossible to construct, since `^` is non-null and always-present. Accept it: it is
  sound, it self-corrects the moment someone tries to write a constructor, and detecting it
  means a real inhabitedness analysis. Rust behaves identically with `Box`.
- **D6 — Arrays stay by-value edges.** They cannot launder indirection anyway: Slice 2
  rejects linear array elements and `^T` is linear, so `[^T N]` is already an error.
- **D7 — Synthesized destructors become iterative for single-type recursion.** For a type
  that is recursive with itself, emit **one fused loop** over the recursive cycle rather
  than mutually-recursive `cell_drop`/`enum_drop` functions. This needs no general TCO: the
  compiler owns these functions and can emit a loop directly. The recursive SCC is the same
  cycle information D3 computes, and `FuncBuilder` already has loop/phi/back-edge support
  (`begin_loop` @ir.rs:1424, `finalize_loop` @ir.rs:1445) from Slice 6.

  ```
  drop_List(v):
    loop:
      match tag(v):
        Nil  -> ret
        Cons -> drop v.field0          # i64, no-op
                p    = v.field1        # ^List
                next = copyout(p)      # payload copy, as ^> already does
                free(p)
                v = next; continue
  ```

- **D8 — Free ordering is reversed, globally.** The cell is freed **before** its
  copied-out payload is dropped, for *every* cell, not only recursive ones. The iterative
  form requires it (deferring the free means remembering pointers, which is a stack again),
  and one ordering rule beats an ordering that depends on whether a type happens to be
  recursive. **Sound** because Slice 2 already copies the payload out of the block before
  touching it (`load_owned_payload` @ir.rs:1907), so the copy does not alias the freed
  memory. Bonus: peak memory during disposal drops, since blocks are released while
  descending instead of held until unwind. **This revises shipped Slice 2 behaviour**:
  goldens asserting `alloc 8 / drop 7 / free 8` become `alloc 8 / free 8 / drop 7`, and the
  spec text stating payload-drop-precedes-free must be corrected rather than quietly
  contradicted.
- **D9 — Multi-child recursive types loop on the last recursive field in declaration
  order** and recurse on the others. A balanced tree is then log n deep, which is fine at
  any realistic size. The choice of *which* child is arbitrary but must be written down and
  tested, or it becomes an accident of synthesis order.
- **D10 — Mutually recursive *types* (`A` holds `^B`, `B` holds `^A`) keep the recursive
  destructor** and its depth limit. A fused loop over a multi-type SCC needs a tagged loop
  over the whole component, which is the tier-2 SCC-contraction shape the ROADMAP already
  describes for mutual TCO. Out of scope here, but the fallback must be **tested**, not
  merely asserted, so the scope line is real.
- **D11 — The OOM revisit is CLOSED, not deferred again.** Slice 2 deferred the
  trap-vs-return decision to this slice on the premise that optional pointers would exist
  as a compiler-known type to return. Per D2 they do not: `Option` is user code, so the
  allocator has nothing privileged to return, and inventing one is the synthesis we
  rejected. **Keep the trap.** Revisit in Phase 4, where real generics give an honest
  `Option<T>`/`Result<T,E>` for a fallible allocation word.

## Work by stage

- **lexer/parser**: `isize` as a type name, alongside `usize`. Nothing else; `^` and enum
  syntax are unchanged, and recursive declarations already parse.
- **check**: `isize` in the integer tower (literal defaults, conversions, homogeneous-op
  rules) mirroring `usize`. Add tests pinning D3/D4/D6 so the currently-incidental
  cell-edge cut becomes intentional. Confirm the by-value cycle diagnostic still names the
  path (`Bad -> Bad`).
- **ir**: the centrepiece. `synthesize_aggregate_destructors` (@ir.rs:862) and
  `synthesize_cell_destructor` (@ir.rs:987) gain SCC awareness: a self-recursive type gets
  a fused loop (D7), everything else keeps today's straight-line synthesis. Apply the D8
  reorder in `synthesize_cell_destructor`. `emit_drop` (@ir.rs:2128) dispatch is unchanged.
- **backend/qbe**: `isize` width/signedness mapping. No allocator changes: the shim,
  trap and trace are untouched (D11).
- **repl**: recursive types in a session, and a residual recursive value disposed at
  `:quit` through the existing `dispose_residual` path.
- **docs**: correct the Slice 2 spec's drop-ordering statement (D8) rather than leaving two
  documents disagreeing.

## Success criteria (each a runnable golden, native or REPL)

1. `isize` round-trips: declare, arithmetic, print, convert to/from `i64`, mirroring the
   existing `usize` goldens.
2. `type: List | Nil | Cons v i64 next ^List ;` compiles, builds, walks and disposes with a
   balanced allocation trace.
3. By-value self-recursion stays a located error naming the cycle path (`Bad -> Bad`).
4. By-value *mutual* recursion (`A` holds `B` holds `A`) stays a located error naming the
   full path.
5. A cycle through `^` in a **struct** field is accepted (D4), not just an enum variant.
6. **The headline criterion**: build a 1,000,000-node list and `drop` it under
   `ulimit -s 1024` (1 MB). Must exit 0. Today's binary segfaults at 100,000 nodes with
   8 MB, so this fails loudly before the change and passes after. Reuses the `sh -c "ulimit
   ... && exec"` harness pattern Slice 2 established for its `RLIMIT_AS` leak bound.
7. Disposal trace ordering after D8: a cell holding a `__spy` traces `free` **before** the
   payload's `drop`, and the revised Slice 2 goldens assert the new order exactly.
8. A multi-child recursive type (a binary tree) builds and disposes correctly; a balanced
   tree of substantial size disposes under a bounded stack (D9).
9. Mutually recursive types `A`↔`B` build and dispose correctly at modest depth, pinning
   the D10 fallback path.
10. `examples/list.sth` ships as the dogfood (build, walk/sum, dispose) with an exact-stdout
    golden, and is genuinely idiomatic user code, unlike Slices 1 and 2 which correctly
    shipped no example.
11. No regression: all 14 existing examples byte-identical, all 654 existing tests pass
    (modulo the Slice 2 ordering goldens deliberately revised under D8).

## Risks and watch-items

- **D8 is a behaviour change to a shipped slice.** The risk is not soundness (the payload
  is already copied out) but bookkeeping: every Slice 2 golden and spec sentence asserting
  payload-drop-before-free must be found and revised together. A half-applied reorder that
  leaves docs and tests disagreeing is the likely failure, not a crash.
- **SCC detection must not silently mis-classify.** A type wrongly judged non-recursive
  keeps the recursive destructor and merely stays depth-limited (benign); a type wrongly
  judged recursive gets a loop that must still be *correct* for the non-recursive case.
  Prefer erring toward the recursive path and test both classifications explicitly.
- **The loop must not regress the simple case.** Non-recursive cells are the overwhelming
  majority; their destructors should come out unchanged (byte-identical IL is a reasonable
  check, given the emitter tests already assert IL text).
- **Depth criterion needs a real margin.** 1 MB stack with 1M nodes is ~100x today's
  failure point, so it cannot pass by accident. Verify it fails on `cc02ad8` before
  implementing, so the test is known to discriminate.
- **`isize` may surface latent word-width assumptions.** `usize` already proved the paths
  are width-parameterised, but signedness interacts with conversions and comparisons.

## Explicitly out of scope

Compiler-provided `Option`/`Result` and any synthesized nullable-pointer type (D2, Phase 4
generics); pointer arithmetic and pointer differences (no consumer yet); fused destructor
loops over **multi-type** SCCs (D10, tier-2 shape); returning allocation failure rather than
trapping (D11, Phase 4); a zipper and any compiler-generated one-hole types (a design
exploration, kept as a stretch, never an exit criterion); second-class refs and
`let`/`inout`/`sink`/`set` (Slice 4); reference counting (Slice 5); user-definable
destructor bodies (Slice 6); growable buffers and `Vec` (Phase 6 `alloc`).
