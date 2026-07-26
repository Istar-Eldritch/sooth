# Phase 3 Slice 2 — Heap + owning pointer + allocator (spec)

Derived from [`phase3-slice2-brief.md`](./phase3-slice2-brief.md); its decisions D1-D10 are
locked and are not reopened here. Base: `main` at Slice 1 merged, 588 tests green.

This slice **changes the compiler** (`src/**`). It gives the linear spine its first real
resource: a heap cell whose destructor actually frees memory.

## Requirements (traceable)

- **R1 (D1)** — The heap primitive is `Owned[T]`: exactly one heap-allocated cell holding
  one value of the concrete payload type `T`. No sized/growable buffer, no runtime-sized
  allocation. A fixed-capacity heap buffer is expressed as `Owned[[u8 N]]`, size in the type.
- **R2 (D2)** — `Owned[T]` is a **compiler-known type constructor, not generics**: one
  interned registry entry per concrete payload type, deduplicated by shape, mirroring
  `ArrayDecl`/`intern_array_type` (@ast.rs:171). Builtin words are type-checked ad hoc at
  the call site from the concrete payload, mirroring `check_array_word` (@check.rs:2039).
  **No type variables are introduced anywhere**; Phase 4 still owns that.
- **R3 (D2 tripwire)** — `Owned` is the second ad-hoc type constructor after arrays. This
  spec does not add a third and does not build a general mechanism. If Slice 3 needs one,
  that is the signal to reconsider in favour of Phase 4's generics.
- **R4 (D3)** — `Owned[T]` is **always linear**, regardless of whether `T` is `Copy`.
  `is_copy` returns `false` for it unconditionally. Linearity propagates transitively via
  Slice 1's existing rules, so a struct or enum containing an `Owned` is linear with
  synthesized drop glue, at no additional cost in this slice.
- **R5 (D4)** — An `Owned[T]` **may hold a linear payload** (`Owned[__spy]` is legal).
  Disposal drops the payload **first**, then frees the cell. This ordering is observable and
  is therefore a contract, not an implementation detail.
- **R6 (D5)** — Allocation goes through a **single global allocator** behind an interface
  conceptually in `core`: `allocate(n) -> ptr` and `free(ptr, n)`. It is **not**
  parameterized per value; no value carries an allocator. Arenas/regions are out of scope.
- **R7 (D6)** — The one implementation is a **compiler-emitted shim** wrapping
  `malloc`/`free`, following the precedent of the drop-spy's emitted `printf` helper
  (`emit_spy_drop` @qbe.rs:543, symbol constant `SPY_DROP_SYMBOL`). **No user-facing FFI is
  added.** This shim is expected to be re-expressed as ordinary bound foreign words once
  Phase 6 lands FFI-to-libc.
- **R8 (D7)** — The destructor is **compiler-known**: `drop` on an `Owned[T]` frees the
  cell. User-definable destructor *bodies* are Slice 6. `drop` continues to lower to a
  `Call` to a per-type symbol (Slice 1 R16), so `emit_drop` (@ir.rs:1890) gains an arm and
  `synthesize_aggregate_destructors` (@ir.rs:793) gains a case. **No new `Instr`/
  `Terminator` variant.**
- **R9 (D8)** — A failed allocation (`malloc` returning NULL) **traps and aborts**, reusing
  the bounds-check trap pattern (`emit_oob_trap` @qbe.rs:527). No NULL may propagate into a
  dereference. Revisited in Slice 3, which introduces optional/non-null pointers.
- **R10 (D9)** — A **test-only alloc/free counter** lives in the runtime shim so disposal is
  golden-observable (`free` is silent where the drop-spy printed). It is convention-fenced
  test infrastructure, not user surface, exactly like `__spy`. Goldens assert **exact**
  counts, not merely that they balance.
- **R11 (D10)** — Access words mirror Slice 1's struct-word family: a constructor, a
  consuming unwrap, and a non-consuming **Copy-only** peek. Peeking a linear payload is a
  compile error, consistent with `S|>fi`. No reference machinery is introduced; contents are
  reached by stack-threading, which is why heap precedes refs (Slice 4).
- **R12 (D10)** — Surface spellings, mirroring the struct-word family:
  `Owned` is `( T -- Owned[T] )`; `Owned>` is `( Owned[T] -- T )` and frees the cell;
  `Owned|>` is `( Owned[T] -- Owned[T] T )` for a Copy payload only; `drop` disposes.
  Type spelling is `Owned[i64]`, `Owned[Point]`, `Owned[[u8 1024]]`.
- **R13 (brief risk 1)** — The three (now four) type registries threaded as separate
  parameters are consolidated into a **single `Registry` borrow** before `Owned` is added.
  `is_copy(ty, structs, enums, arrays)` (@check.rs:144) is already four parameters across
  ~15 call sites; threading a fifth is strictly worse than consolidating once. After
  consolidation the signature must be **smaller** than it is today, not larger.
- **R14 (brief risk 2)** — The counter must not change observable stdout for any existing
  golden. Prefer one always-emitted code path that tests read explicitly over conditional
  compilation, but it must be invisible unless asked for.
- **R15 (brief risk 4)** — `Owned` of a zero-sized payload, if such a type is
  constructible, has a defined answer rather than a `malloc(0)` accident. **Open for the
  implementer**: determine whether a zero-sized type is constructible at all today; if it
  is not, state that and add no machinery.

## Load-bearing invariants (must not break)

- Backend stays **QBE**; no LLVM.
- IR `Ptr[T]` stays **opaque**: never assumed to be a `u64` (a future WASM lowering depends
  on this).
- Drop dispatch must **not** key off a bare pointer type. Slice 1 needed a distinct
  `IrType::Spy` for precisely this reason; the cell needs its own `IrType` variant so
  `emit_drop` can tell it from an arbitrary pointer.
- `core` stays `no_std`. The language sees an allocator *interface*; libc appears only in
  the emitted shim.
- No new `Instr`/`Terminator` variant (R8). No user-facing FFI (R7).
- Existing behaviour: all 14 examples byte-identical, all 588 existing tests pass.

## Delivery phases

### Phase 1 — Registry consolidation (prerequisite refactor)

- Fold the separate `structs`/`enums`/`arrays` parameters into a single `Registry` borrow
  threaded through the checker and lowerer (R13). No behaviour change whatsoever.
- **Changes**: `src/check.rs`, `src/ir.rs`, `src/repl.rs`, `src/ast.rs`.
- **Exit**: no signature takes more than the consolidated borrow where it previously took
  three or four; 588 tests still pass unchanged; fmt/clippy clean; zero behaviour diff.

### Phase 2 — The `Owned[T]` type: interning, classification, surface spelling

- `Type::Owned` + an owned-cell registry with dedup-by-shape interning (R1, R2).
- `Owned[T]` in type position through parser/ast (R12). **No lexer change is needed, and
  this was verified against the current lexer, not assumed**: `[` is already a hard
  delimiter (@lexer.rs:22, emitted at :84), so `Owned[i64]` scans as `Word("Owned")`,
  `LBracket`, `Word("i64")`, `RBracket`, making the bracket purely a parser concern; and
  Slice 1's peek glue (@lexer.rs:96-111) joins `|` into the current word when a word char
  precedes it and `>` follows, so `Owned|>` already scans as a single word. Re-verify
  existing examples regardless.
- `is_copy` returns `false` for `Owned` unconditionally (R4); transitive propagation through
  structs/enums follows from Slice 1 at no extra cost.
- **Changes**: `src/parser.rs`, `src/ast.rs`, `src/check.rs`, `tests/phase0.rs`.
- **Exit**: criteria 3 and 9 pass; green; no regression.

### Phase 3 — Allocation: the shim, the counter, the OOM trap  *(hard)*

- Emit the `allocate`/`free` shim wrapping `malloc`/`free` (R6, R7), the alloc/free counter
  (R10, R14), and the NULL-return trap (R9).
- Map the cell's `IrType` to the pointer width, keeping `Ptr[T]` opaque.
- **Changes**: `src/backend/qbe.rs`, `src/ir.rs`, `tests/phase0.rs`.
- **Exit**: criterion 1 passes (exact counts); the trap path exists and is exercised as far
  as is non-flaky; green; no regression.

### Phase 4 — The three access words

- Constructor, consuming unwrap, Copy-only peek, with ad-hoc checking from the concrete
  payload type (R11, R12), including the compile error for peeking a linear payload.
- **Changes**: `src/check.rs`, `src/ir.rs`, `tests/phase0.rs`.
- **Exit**: criteria 5, 6, 7, 10 pass; green; no regression.

### Phase 5 — Drop glue and payload ordering

- `emit_drop` arm + synthesized per-type destructor: drop the payload, **then** free (R5, R8).
- **Changes**: `src/ir.rs`, `src/backend/qbe.rs`, `tests/phase0.rs`.
- **Exit**: criteria 2, 4, 8 pass, with criterion 8's ordering pinned by interleaved output;
  green; no regression.

### Phase 6 — REPL residual disposal and regression sweep

- Confirm an `Owned` on the residual REPL stack is freed at `:quit` via the existing
  `dispose_residual` path (@repl.rs:477). Expected to need no production change.
- Full regression sweep.
- **Changes**: `tests/phase1.rs`, `tests/phase0.rs`.
- **Exit**: criteria 11 and 12 pass; green.

## Criterion → test map

All goldens are runnable native binaries (`tests/phase0.rs`) or REPL sessions
(`tests/phase1.rs`), **never** IL-string assertions. House assertion-strength rules, carried
from Slice 1 and binding here:

- every **negative** golden asserts the diagnostic substring **and** the backticked type
  name, not merely that compilation failed;
- every **allocation-observing** golden asserts **exact** alloc and free counts, since
  balance alone cannot distinguish 1-alloc/1-free from 2/2;
- criterion 8's ordering is pinned by **interleaved observable output**, never inferred.

| # | Criterion | Test (phase) |
|---|---|---|
| 1 | Allocate then explicit `drop`: runs, alloc == free == exactly 1 | `owned_alloc_and_drop_balances_exactly_once` (P3) |
| 2 | Forgetting to dispose an `Owned` is a compile error | `unconsumed_owned_is_error` (P5) |
| 3 | `dup` of an `Owned` is a compile error naming the type | `dup_of_owned_is_error` (P2) |
| 4 | Use-after-move of an `Owned` local is a compile error (double-free unreachable) | `use_after_move_of_owned_is_error` (P5) |
| 5 | Unwrap hands the payload back and frees exactly once | `owned_unwrap_returns_payload_and_frees_once` (P4) |
| 6 | Peek twice then dispose: cell stays live, exactly one free | `peek_owned_copy_payload_keeps_cell_live` (P4) |
| 7 | Peek of a **linear** payload is a compile error | `peek_owned_linear_payload_is_error` (P4) |
| 8 | `Owned[__spy]` disposal drops the payload **then** frees, order asserted | `owned_linear_payload_drops_before_free` (P5) |
| 9 | An `Owned` in a struct makes the struct linear; dropping it frees the cell | `struct_containing_owned_is_linear_and_frees` (P2 classification, P5 free) |
| 10 | `Owned[[u8 N]]` fixed-capacity heap buffer: allocate, write, read back, free once | `owned_byte_buffer_roundtrips_and_frees_once` (P4) |
| 11 | REPL `:quit` frees a residual `Owned` (counts balance at session end) | `repl_quit_frees_residual_owned` (P6) |
| 12 | No regression: 14 examples byte-identical, 588 existing tests pass | existing goldens + example sweep (P6) |

## Phases JSON

```json
{"phases":[
  {"phase":1,"focus":"Prerequisite refactor with zero behaviour change: fold the separate structs/enums/arrays registry parameters into a single Registry borrow threaded through the checker and lowerer. is_copy is already a 4-parameter signature across ~15 call sites and adding an Owned registry would make it five; consolidating once must leave the signature smaller than it is today.","changes":["src/check.rs","src/ir.rs","src/repl.rs","src/ast.rs"],"tests":["src/check.rs","src/ir.rs"],"exit":"zero behaviour diff; all 588 existing tests pass unchanged; fmt/clippy clean"},
  {"phase":2,"focus":"The Owned[T] type as a compiler-known type constructor (NOT generics, no type variables): a Type::Owned variant plus an owned-cell registry with dedup-by-shape interning mirroring ArrayDecl/intern_array_type; the Owned[T] spelling in type position through parser/ast, which needs NO lexer change (verified: '[' is already a hard delimiter so Owned[i64] lexes as Word/LBracket/Word/RBracket, and Slice 1's peek glue already makes Owned|> scan as one word); and is_copy returning false for Owned unconditionally so linearity propagates transitively through structs and enums via Slice 1's existing rules.","changes":["src/parser.rs","src/ast.rs","src/check.rs","tests/phase0.rs"],"tests":["tests/phase0.rs"],"exit":"criteria 3 and 9 (classification half) pass; green; no regression"},
  {"phase":3,"focus":"Allocation machinery: emit the allocate/free shim wrapping malloc/free as a compiler-emitted helper following the drop-spy printf precedent (no user-facing FFI), the test-only alloc/free counter that makes disposal golden-observable without changing stdout for existing goldens, and the trap-and-abort path when malloc returns NULL. Map the cell's IrType to pointer width while keeping Ptr[T] opaque and never assumed to be u64.","difficulty":"hard","changes":["src/backend/qbe.rs","src/ir.rs","tests/phase0.rs"],"tests":["tests/phase0.rs"],"exit":"criterion 1 passes asserting exact counts; the OOM trap path exists and is exercised as far as is non-flaky; green; no regression"},
  {"phase":4,"focus":"The three access words mirroring the struct-word family: Owned as ( T -- Owned[T] ), Owned> as ( Owned[T] -- T ) which frees the cell, and Owned|> as a non-consuming ( Owned[T] -- Owned[T] T ) peek restricted to Copy payloads, with a compile error naming the type when the payload is linear. Effects are computed ad hoc at the call site from the concrete payload type.","changes":["src/check.rs","src/ir.rs","tests/phase0.rs"],"tests":["tests/phase0.rs"],"exit":"criteria 5, 6, 7, 10 pass; green; no regression"},
  {"phase":5,"focus":"Drop glue: an emit_drop arm for the cell plus a synthesized per-type destructor that drops a linear payload FIRST and then frees the cell, an ordering that is observable and therefore contractual. drop stays a Call to a per-type symbol with no new Instr or Terminator variant.","changes":["src/ir.rs","src/backend/qbe.rs","tests/phase0.rs"],"tests":["tests/phase0.rs"],"exit":"criteria 2, 4, 8 pass with criterion 8's ordering pinned by interleaved observable output; green; no regression"},
  {"phase":6,"focus":"REPL residual disposal and the regression sweep: confirm an Owned left on the residual REPL stack is freed at :quit through the existing dispose_residual path, expected to need no production change, then verify all 14 examples are byte-identical and the full pre-existing suite passes.","changes":["tests/phase1.rs","tests/phase0.rs"],"tests":["tests/phase1.rs","tests/phase0.rs"],"exit":"criteria 11 and 12 pass; green"}
]}
```
