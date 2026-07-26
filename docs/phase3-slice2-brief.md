# Phase 3 Slice 2 — Heap + owning pointer + allocator (brief)

Slice 1 shipped the linear spine on a test-only primitive. This slice gives it a **real
resource**: a heap cell whose destructor actually frees memory. It is the first time
`drop` does something a leak-checker would care about.

Prerequisite state: Slice 1 is merged (`f8c48da`). 588 tests green.

## Locked decisions

- **D1 — The heap primitive is `Owned[T]`, a single heap cell** (Box-shaped), not a sized
  buffer. Slice 3's recursive data needs the *indirection*, and a growable buffer wants
  Phase 6's `alloc` layer (realloc, capacity, growth policy: a design of its own). A
  fixed-capacity heap buffer composes for free as `Owned[[u8 N]]`, with the size in the
  type. Runtime-sized allocation is **not** in this slice.
- **D2 — `Owned[T]` is a compiler-known type constructor, NOT generics.** One interned
  entry per concrete payload type, builtin words type-checked ad hoc at the call site,
  exactly as `[T N]` arrays work today (`ArrayDecl`/`intern_array_type` @ast.rs:171,
  `check_array_word` @check.rs:2039). There are **no type variables anywhere in the type
  system** today (verified: zero hits for `TypeVar`/`Type::Var`), and Phase 4 still owns
  introducing them.
  **Tripwire**: `Owned` is the *second* ad-hoc type constructor after arrays. If Slice 3
  wants a third, stop and reconsider: the special-casing has become the mechanism, and
  Phase 4's generics should subsume all of them rather than sit alongside.
- **D3 — `Owned[T]` is always linear**, whatever `T` is. It owns heap memory, so a Copy
  payload does not make the cell Copy. It composes into structs and enums for free via
  Slice 1's transitive propagation, and a struct containing one is linear.
- **D4 — A `Owned[T]` may hold a linear payload.** `Owned[__spy]` is legal and its
  disposal must drop the payload *then* free the cell, in that order (observable). This is
  deliberately unlike the linear-*array*-element case that Slice 1 rejected: an array
  needs an element-wise drop loop, whereas a cell has exactly one payload and the existing
  synthesized-destructor mechanism handles it directly.
- **D5 — Allocation is a single global allocator behind an interface in `core`**:
  `allocate(n)` / `free(ptr, n)`. Deliberately **not** parameterized per value. A swappable
  global (Rust `#[global_allocator]`-style) is cheap to retrofit later because it changes
  no value's representation; per-value allocators change all of them. Arenas/regions are a
  later question, most likely at Phase 8 (bare metal, where there is no libc at all).
- **D6 — The implementation is a compiler-emitted shim** wrapping `malloc`/`free`, exactly
  as the backend already emits the drop-spy's `printf` helper (`emit_spy_drop`
  @qbe.rs:543, `SPY_DROP_SYMBOL`). The language has no user-facing FFI yet and this slice
  does **not** add one. `core` sees an interface; the single implementation is a shim.
  **Rework expected**: once Phase 6 lands FFI-to-libc via safe wrappers, this shim should
  be re-expressed as ordinary bound foreign words and stop being a backend special case.
- **D7 — The destructor is compiler-known.** `drop` on an `Owned[T]` frees. User-definable
  destructor *bodies* were moved to Slice 6 (where `close` gives a second, dissimilar
  client to design against). `drop` already lowers to a `Call` to a per-type symbol
  (Slice 1 R16), so `emit_drop` @ir.rs:1890 gains an arm and the synthesized-destructor
  mechanism (`synthesize_aggregate_destructors` @ir.rs:793) gains a case.
- **D8 — Allocation failure traps and aborts**, reusing the bounds-check trap pattern
  (`emit_oob_trap` @qbe.rs:527). Optional/non-null pointers (Slice 3) and Result (Phase 5)
  do not exist yet, so there is nowhere to *return* a failure. **Explicitly revisited in
  Slice 3**, which is the slice that introduces somewhere for it to go. A silent NULL
  propagating into a deref is exactly the failure class this language claims to remove, so
  the trap is the honest placeholder, not laziness.
- **D9 — Testability: a test-only alloc/free counter in the runtime shim.** `free` is
  silent where the drop-spy printed, so without this the slice's exit criterion is
  untestable and we would be asserting the compiler emitted a call rather than that the
  program is correct, which this project's conventions reject (goldens, never IL-string
  assertions). Goldens assert the counts **balance** and are **exactly** the expected
  number, catching leaks and double-frees, the precise pair the linear spine claims to
  prevent. This is the drop-spy trick one level down; like the spy, it is convention-fenced
  test infrastructure, not user surface.
- **D10 — Access words mirror Slice 1's struct words**, so no new access idiom is invented:
  a constructor (move a value onto the heap), a **consuming unwrap** (hand the payload back,
  free the cell), and a **non-consuming Copy-only peek**. Peeking a *linear* payload is a
  compile error, consistent with `S|>fi` being Copy-only. Getting at contents by
  stack-threading is exactly why the ROADMAP put heap before refs; second-class refs stay
  deferred to Slice 4.

## Surface syntax (CONFIRMED)

The owning cell is spelled with a `^` sigil, in both type and term position:

| operation | spelling | effect |
|---|---|---|
| type | `^T` | `^i64`, `^Point`, `^[u8 1024]`, `^^i64` |
| construct | `^` | `( T -- ^T )`, moves the value onto the heap |
| unwrap (consuming) | `^>` | `( ^T -- T )`, frees the cell |
| peek (non-consuming) | `^\|>` | `( ^T -- ^T T )`, **Copy payload only** |
| dispose | `drop` | frees the cell, dropping a linear payload first |

Chosen over a `Owned[T]` word form, and over `Owned<T>`, for composition: `^` nests
*with* the leading-bracket array convention rather than against it, so owned-of-array and
array-of-owned read as mirrors (`^[u8 1024]` vs `[^i64 4]`), and `^^i64` stays legible where
`Owned[Owned[i64]]` does not. `Owned<T>` was rejected on lexing: `<`/`>` are not delimiters
while `[`/`]` are, so `Owned<[u8 1024]>` would tokenise into six pieces ending in a bare `>`
indistinguishable from the comparison operator, i.e. one syntax with two token shapes
depending on its payload. Making `<`/`>` delimiters is not available: it would break `<=`,
`>=`, `<>`, the `>usize` conversion prefix, and every `Point>x`/`Point<x` struct word.

Verified free and unambiguous against the current tree: `^` appears in no `.sth` source and
is not an operator (bitwise ops are `and`/`or`/`xor`/`shl`/`shr`); `^` is not a delimiter so
`^i64` scans as one word while `^[u8 1024]` splits at the bracket into `^` plus a type
expression; and `^|>` survives Slice 1's peek-glue rule (`|` joins when a word char precedes
and `>` follows) as a single token.

The cost accepted: a bare `^` is terse to the point of being easy to miss on a line, and the
sigil is less self-documenting than a word. Taken deliberately in exchange for the nesting
and buffer cases reading well, which is where most real use will land.

## Work by stage

- **lexer/parser**: `Owned[T]` in type position. Arrays already lex `[T N]`; the new shape
  is a name immediately followed by `[`. Slice 1's `S|>fi` lexing rule is the cautionary
  precedent (`|` was a hard delimiter and needed a targeted gluing rule): check whether
  `Owned[` needs similar care, and re-verify existing examples do not regress.
- **ast**: `Type::Owned(OwnedId, &'static str)` + an `OwnedDecl { payload: Type }` registry
  with `intern_owned_type`, mirroring `ArrayDecl`/`intern_array_type` @ast.rs:171 including
  dedup-by-shape.
- **check**: `is_copy` @check.rs:144 gains an `Owned` arm returning `false` unconditionally
  (D3). Ad-hoc checking for the three words (D10), including the Copy-only peek
  restriction. Constructor/unwrap/peek effects computed from the concrete payload type.
- **ir**: an `IrType` variant for the cell so drop dispatch can tell it from a plain
  pointer (Slice 1 needed `IrType::Spy` for exactly this reason; do not dispatch off
  `Ptr`). `emit_drop` @ir.rs:1890 gains an arm; the synthesized per-type destructor drops
  the payload then calls free. `Ptr[T]` stays **opaque**: never assumed to be a `u64`.
- **backend/qbe**: emit the alloc/free shims and the alloc/free counter; map the cell's
  `IrType` to the pointer width. Trap-and-abort on a NULL from `malloc`.
- **repl**: an `Owned` left on the residual stack must be freed at `:quit` via the existing
  `dispose_residual` @repl.rs:477 path. Expected to work unchanged; needs a golden.

## Success criteria (each a runnable golden, native or REPL)

1. Allocate then explicitly `drop`: program runs, alloc count == free count == 1.
2. Forgetting to dispose an `Owned` is a compile error (the Slice 1 unconsumed/surplus check).
3. `dup` of an `Owned` is a compile error naming the type.
4. Use-after-move of an `Owned` local is a compile error (so a double-free is unreachable).
5. Unwrap hands the payload back and frees exactly once (counts balance, value correct).
6. Peek reads a Copy payload without consuming the cell; peeking twice then disposing
   yields exactly one free.
7. Peek of a **linear** payload is a compile error.
8. `Owned[__spy]`: disposal drops the payload **then** frees, order asserted via the spy's
   printed tag interleaved with the counter.
9. An `Owned` inside a struct makes the struct linear; dropping the struct frees the cell.
10. `Owned[[u8 N]]` as a fixed-capacity heap buffer: allocate, write, read back, free once.
11. REPL `:quit` frees a residual `Owned` (counts balance at session end).
12. No regression: all 14 examples byte-identical, all 588 existing tests still pass.

## Risks and watch-items

- **The registry-parameter wart reaches its breaking point.** `is_copy` is already
  `is_copy(ty, structs, enums, arrays)` (@check.rs:144) after Slice 1. Adding an `Owned`
  registry makes it **five** parameters threaded through ~15 call sites. This is the
  moment to fold the registries into a single `Registry` borrow rather than thread a fifth,
  which would leave the signature *smaller* than it is today. Flagged in Slice 1's review
  as the right follow-up; Slice 2 is where it stops being optional.
- **Counter must not perturb the program.** Decide whether the counter is always emitted
  (and merely unread outside tests) or gated; always-emitted is simpler and keeps one code
  path, but must not change observable stdout for existing goldens.
- **Drop ordering with a linear payload** (D4) is observable and therefore a contract:
  free first, then the payload (reversed in Phase 3 Slice 3, R8, for uniformity across every
  cell). Pin it in a golden, do not let it fall out of implementation order.
- **`Owned` of a zero-sized payload**, if such a type is constructible, needs a defined
  answer rather than a `malloc(0)` accident.
- **OOM is hard to test** without exhausting memory. A huge single allocation is the
  cheapest probe; if it proves flaky, assert the trap path exists by other means rather
  than shipping a flaky golden.

## Explicitly out of scope

User-facing FFI (Phase 6); growable/runtime-sized buffers and `Vec` (Phase 6 `alloc`);
optional / non-null pointers and recursive heap data (Slice 3); second-class refs,
`let`/`inout`/`sink`/`set` and escape checking (Slice 4); reference counting (Slice 5);
user-definable destructor bodies and fds (Slice 6); generics and polymorphic `drop`
(Phase 4); arenas / per-value allocators (revisit at Phase 8).
