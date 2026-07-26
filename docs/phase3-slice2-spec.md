# Phase 3 Slice 2 — Heap + owning pointer + allocator (spec)

Derived from [`phase3-slice2-brief.md`](./phase3-slice2-brief.md); its decisions D1-D10 are
locked. Base: `main` at Slice 1 merged. This slice **changes the compiler** (`src/**`) and
gives the linear spine its first real resource: a heap cell whose destructor frees memory.

Revised after spec review round 1, which found the original delivery order impossible, the
observability mechanism unimplementable, and an unspecified use-after-free in unwrap.

## Requirements (traceable)

- **R1 (D1)** — The heap primitive is a single heap cell holding one value of a concrete
  payload type. No sized/growable buffer, no runtime-sized allocation. A fixed-capacity heap
  buffer is expressed as `^[u8 N]`, with the size in the type.
- **R2 (D2)** — The cell is a **compiler-known type constructor, not generics**: one interned
  registry entry per concrete payload type, deduplicated by shape, mirroring
  `ArrayDecl`/`intern_array_type` (@ast.rs:171). Builtin words are type-checked ad hoc at the
  call site from the concrete payload, mirroring `check_array_word` (@check.rs:2039). **No
  type variables are introduced**; Phase 4 still owns those.
- **R3 (D2 tripwire)** — The cell is the second ad-hoc type constructor after arrays. This
  spec adds no third and builds no general mechanism. If Slice 3 needs one, that is the signal
  to reconsider in favour of Phase 4's generics.
- **R4 (D3)** — `^T` is **always linear**, regardless of whether `T` is `Copy`. `is_copy`
  returns `false` for it **unconditionally**, with no payload lookup, so `is_copy`'s signature
  is unchanged by this slice. Linearity propagates transitively via Slice 1's existing rules,
  so a struct or enum containing a cell is linear with synthesized drop glue.
- **R5 (D4)** — A cell **may hold a linear payload** (`^__spy` is legal). Disposal drops the
  payload **first**, then frees the cell. This ordering is observable (R10) and is a contract.
- **R6 (D5)** — Allocation goes through a **single global allocator** behind an interface
  conceptually in `core`: `allocate(n) -> ptr` and `free(ptr, n)`. Not parameterized per
  value; no value carries an allocator.
- **R7 (D6)** — The one implementation is a **compiler-emitted shim** wrapping `malloc`/`free`,
  following the drop-spy's emitted `printf` helper (`emit_spy_drop` @qbe.rs:543). **No
  user-facing FFI is added.** Expected to be re-expressed as ordinary bound foreign words once
  Phase 6 lands FFI-to-libc.
- **R8 (D7)** — The destructor is **compiler-known**: `drop` on a cell frees it. User-definable
  destructor bodies are Slice 6. `drop` continues to lower to a `Call` to a per-type symbol
  (Slice 1 R16), so `emit_drop` (@ir.rs:1890) gains an arm and
  `synthesize_aggregate_destructors` (@ir.rs:793) gains a case. **No new `Instr`/`Terminator`
  variant.**
- **R9 (D8)** — A failed allocation (`malloc` returning NULL) **traps and aborts**, reusing the
  bounds-check trap pattern (`emit_oob_trap` @qbe.rs:527). No NULL may reach a dereference.
  Revisited in Slice 3, which introduces optional/non-null pointers.
- **R10 (D9, revised)** — Observability is an **env-gated allocation trace**, not a silent
  counter. The emitted shim checks one environment variable once and, when set, writes one
  event line per `allocate` and per `free` **to stderr**. Rationale, and why the alternatives
  were rejected:
  - A silent counter is unreadable: a golden observes only stdout, stderr and exit code
    (`run_and_capture_stdout` @tests/phase0.rs:9-21).
  - Unconditional printing is wrong because `^` is **user surface**, unlike the test-only
    `__spy`; real programs must not emit on every free.
  - Mutable global state has no precedent in the emitter (every `data` it writes is
    read-only, qbe.rs:30-49) and would need a new symbol-operand path in `width`/`load_op`.
  - A per-module counter would exist once per REPL `.so` (each line is its own object,
    repl.rs), making the REPL criterion vacuous. Every `.so` shares one stderr, so a trace
    does not have this problem.
  The trace gives **exact counts and ordering**, so it subsumes what the counter was for:
  distinguishing a leak from a double-free, and pinning R5's payload-before-free ordering.
  It is test-only telemetry, gated off by default, and rides the R7 rework at Phase 6.
- **R11 (D10)** — Access words mirror Slice 1's struct-word family: a constructor, a consuming
  unwrap, and a non-consuming **Copy-only** peek. Peeking a linear payload is a compile error,
  consistent with `S|>fi`. No reference machinery; contents are reached by stack-threading,
  which is why heap precedes refs (Slice 4).
- **R12 (brief surface-syntax section, confirmed 2026-07-26)** — The cell is spelled with a
  `^` sigil in both type and term position:

  | operation | spelling | effect |
  |---|---|---|
  | type | `^T` | `^i64`, `^Point`, `^[u8 1024]`, `^^i64` |
  | construct | `^` | `( T -- ^T )` |
  | unwrap | `^>` | `( ^T -- T )`, frees the cell |
  | peek | `^\|>` | `( ^T -- ^T T )`, Copy payload only |
  | dispose | `drop` | frees, dropping a linear payload first |

  Verified against the tree: `^` is unused in `.sth` source and is not an operator (bitwise
  ops are `and`/`or`/`xor`/`shl`/`shr`); `^` is not a delimiter so `^i64` scans as one word
  while `^[u8 1024]` splits at the bracket into `^` plus a type expression; `^|>` survives
  Slice 1's peek-glue rule as a single token. **No lexer change is required.** A bare `^` in
  type position with no payload is a located error naming the required form (`^T`).
  Because the sigil is not an identifier, the "user declares a type with this name" collision
  that a word-shaped spelling would have had cannot arise.
- **R13 (new, replaces the original registry requirement)** — **Unwrap materialises the
  payload before releasing the cell.** For a scalar payload a `Load` precedes the `free`
  call; for an aggregate payload (struct, enum, array) unwrap allocates a fresh frame slot
  and `Blit`s `size` bytes out of the cell, and only then frees. **The freed pointer is never
  handed to the stack.** This is load-bearing: at runtime an aggregate value *is* a pointer to
  its storage (ir.rs:84-88), so the naive lowering pushes the cell pointer and then frees it,
  producing a use-after-free that would pass a golden on glibc most of the time.
- **R14 (new)** — Construction is the mirror: a scalar payload is `Store`d into the allocated
  pointer; an aggregate payload is `Blit`ted from its frame slot into the cell. Peek of an
  aggregate payload must `Alloc` a fresh frame slot and `Blit` out, never alias the cell, or a
  later `drop` leaves a dangling stack slot.
- **R15 (brief risk 4, resolved)** — A zero-sized payload **is** constructible today: a
  zero-field struct is legal and lays out as size 0, align 1
  (`struct_layout_zero_field_is_size_0_align_1` @ir.rs:3086). `allocate` therefore requests
  `max(size, 1)` and `free` passes the same adjusted size, so every cell has a distinct
  address and free-once remains meaningful. Without this, `^Unit` reaches `malloc(0)`, which
  is permitted to return NULL and would fire R9's trap on a correct program.
- **R16 (new, documented consequence)** — Because `^T` is linear (R4), Slice 1's module-wide
  linear-array-element sweep (`check_no_linear_array_elements` @check.rs:371) now rejects **an
  array of cells** (`[^i64 4]`), just as it already rejects `^[__spy 2]` via the same sweep.
  This falls out of D3 plus Slice 1 rather than being new design, but it is user-visible and
  surprising next to R1's endorsement of the inverse nesting, so it is stated and tested.
- **R17 (new)** — The cell needs its own `IrType` variant so drop dispatch is not keyed off a
  bare pointer, mirroring why Slice 1 needed `IrType::Spy`. `width`/`load_op`/`store_op`
  (@qbe.rs:252-275) each need an arm or the build fails on non-exhaustive matches. The spec
  **defers cell size to `Ptr`'s existing convention** (currently a hardcoded 8 at ir.rs:325,
  already recorded to retrofit to the word-width parameter alongside `Ptr`); the cell must not
  introduce a second, independent width assumption.
- **R18 (deferred, explicitly not this slice)** — Consolidating the `structs`/`enums`/`arrays`
  registries into one borrow is **not** done here. The original justification was wrong:
  because R4 makes `is_copy` return `false` for a cell with no payload lookup, `is_copy`'s
  arity is unchanged and there is no forcing function. The real cost this slice adds is one
  more `&mut Vec<..>` interning registry threaded alongside `arrays` (~12 checker signatures
  plus the parser), which is materially cheaper than a 50-60-site consolidation. Consolidation
  also cannot be a rename: `Ctx` holds immutable borrows while interning needs a mutable one
  (the borrow split at check.rs:326-338 exists for exactly this reason), so it requires
  stripping the registries out of `Ctx`. **Trigger for revisiting**: a third interning
  registry, or any change that needs `Ctx` surgery anyway.

## Load-bearing invariants (must not break)

- Backend stays **QBE**; no LLVM.
- IR `Ptr[T]` stays **opaque**: never assumed to be a `u64`.
- Drop dispatch must **not** key off a bare pointer type (R17).
- `core` stays `no_std`. The language sees an allocator *interface*; libc appears only in the
  emitted shim.
- No new `Instr`/`Terminator` variant (R8). No user-facing FFI (R7).
- `emit_drop`'s existing array arm is `unreachable!` for a linear element; R16 keeps that guard
  valid, and the implementation must confirm it rather than leave a live `unreachable!`
  adjacent to a new linear type.
- R6 and R7 are structural, negative constraints, enforced by review rather than by a golden.

## Delivery phases

**Sequencing rule** (the round-1 failure): *a golden that observes an allocation or a free
cannot land before the drop glue exists.* Only compile-error goldens and unwrap-only goldens
can land earlier.

### Phase 1 — The `^T` type: interning, classification, surface

- `Type` variant + owned-cell registry with dedup-by-shape interning, threaded like `arrays`
  including session persistence in the REPL (R2). `^T` in type position through parser/ast,
  with the bare-`^` diagnostic (R12). `is_copy` returns `false` unconditionally (R4).
- **Exit is unit-level** (no `^` value can exist yet, since the constructor lands in Phase 3):
  `is_copy` of a cell is `false`; two mentions of `^i64` intern to one entry; `^i64`,
  `^[u8 4]`, `^^i64` parse in type position; a bare `^` errors; `[^i64 4]` is rejected by the
  existing linear-array sweep (R16). Green; no regression.
- **Changes**: `src/ast.rs`, `src/parser.rs`, `src/check.rs`, `src/ir.rs`, `src/repl.rs`.

### Phase 2 — Allocation: shim, env-gated trace, OOM trap  *(hard)*

- Emit the `allocate`/`free` shim wrapping `malloc`/`free` (R6, R7), the env-gated stderr
  trace (R10), the NULL-return trap (R9), and the `max(size, 1)` adjustment (R15). Add the
  `IrType` variant and its `width`/`load_op`/`store_op` arms (R17).
- The shim and trap follow `emit_spy_drop`/`emit_oob_trap` closely; **the trace is the new
  part** (an env check plus a per-event write), so budget the difficulty there.
- **Exit is unit-level**: the emitted IL contains the shim, the trap and the gated trace, in
  the style of the existing emitter assertions. Nothing calls the shim yet. Green; no
  regression.
- **Changes**: `src/backend/qbe.rs`, `src/ir.rs`.

### Phase 3 — The three access words

- Constructor, consuming unwrap, Copy-only peek (R11, R12), with the copy-in / copy-out
  ordering rules (R13, R14) and the compile error for peeking a linear payload.
- **Exit**: criteria 2, 3, 3b, 4, 5, 5b, 7, 9, 16 pass. These are the compile-error goldens
  plus unwrap, which frees directly without needing drop glue. Green; no regression.
- **Changes**: `src/check.rs`, `src/ir.rs`, `tests/phase0.rs`.

### Phase 4 — Drop glue, ordering, and every allocation-observing golden

- `emit_drop` arm plus a synthesized per-type destructor that drops a linear payload **first**
  and then frees (R5, R8).
- **Exit**: criteria 1, 1b, 6, 8, 9b, 10, 11, 12, 13, 14 pass. Green; no regression.
- **Changes**: `src/ir.rs`, `src/backend/qbe.rs`, `tests/phase0.rs`.

### Phase 5 — REPL residual disposal and regression sweep

- Confirm a cell on the residual REPL stack is freed at `:quit` through the existing
  `dispose_residual` path (@repl.rs:477). Expected to need no production change beyond
  Phase 1's session-persistent registry.
- **Exit**: criteria 15 and 17 pass. Green.
- **Changes**: `tests/phase1.rs`, `tests/phase0.rs`.

## Criterion → test map

All goldens are runnable native binaries (`tests/phase0.rs`) or REPL sessions
(`tests/phase1.rs`), never IL-string assertions. Inherited house rules, binding here:

- every **negative** golden asserts the diagnostic substring **and** the backticked type name;
- criterion 4 additionally asserts the **move site**, per Slice 1's rule;
- every **allocation-observing** golden asserts the **exact ordered trace transcript**, since
  a count alone cannot distinguish a leak from a double-free, and an unordered check cannot
  pin R5.

| # | Criterion | Test (phase) |
|---|---|---|
| 1 | Construct then explicit `drop`: trace shows exactly one alloc and one free | `owned_alloc_and_drop_traces_one_pair` (P4) |
| 1b | Frees are real, not counted: ~100k construct-and-dispose iterations of a large payload complete without exhausting memory | `owned_alloc_dispose_loop_does_not_exhaust_memory` (P4) |
| 2 | Forgetting to dispose a cell is a compile error | `unconsumed_owned_is_error` (P3) |
| 3 | `dup` of `^i64` (a **Copy** payload, proving the cell is linear regardless) is a compile error naming `^i64` | `dup_of_owned_is_error` (P3) |
| 3b | `over` of `^i64` is a compile error | `over_of_owned_is_error` (P3) |
| 4 | Use-after-move of a cell local errors, **naming the move site** | `use_after_move_of_owned_is_error` (P3) |
| 5 | Unwrap returns the payload value and frees exactly once | `owned_unwrap_returns_payload_and_frees_once` (P3) |
| 5b | Unwrap of an **aggregate** payload: a field read after the free yields the correct value (proves R13's copy-out-before-free) | `owned_unwrap_aggregate_copies_out_before_free` (P3) |
| 6 | Peek twice then dispose: both peeked values correct and equal, exactly one free | `peek_owned_copy_payload_keeps_cell_live` (P4) |
| 7 | Peek of a **linear** payload is a compile error | `peek_owned_linear_payload_is_error` (P3) |
| 8 | `^__spy` disposal: the spy's tag prints **before** the free event in the trace (R5 ordering) | `owned_linear_payload_drops_before_free` (P4) |
| 9 | A struct containing a cell is linear: `dup` on it errors naming the struct | `struct_containing_owned_is_linear` (P3) |
| 9b | Dropping that struct frees the cell | `dropping_struct_with_owned_frees_cell` (P4) |
| 10 | An **enum** variant carrying a cell, built behind an `if` so the tag is a runtime value, dropped: exactly one free for the active variant | `enum_variant_with_owned_frees_on_drop` (P4) |
| 11 | Nested `^^i64`: two allocs, two frees, **inner freed before outer** | `nested_owned_frees_inner_before_outer` (P4) |
| 12 | Zero-sized payload (`^Unit` over a zero-field struct) constructs and disposes: one alloc, one free (R15's `max(size,1)`) | `owned_zero_sized_payload_allocs_and_frees_once` (P4) |
| 13 | `^[u8 N]` fixed-capacity buffer: construct from a filled array, peek, `get` a byte, `drop`; exactly one alloc and one free | `owned_byte_buffer_peek_get_and_free_once` (P4) |
| 14 | OOM traps: under an address-space limit, a single construct aborts; a sentinel after it does **not** print; exit code non-zero | `owned_alloc_failure_traps_and_aborts_native` (P4) |
| 15 | REPL `:quit` frees a residual cell | `repl_quit_frees_residual_owned` (P5) |
| 16 | Arrays of linear elements stay rejected in both directions: `^[__spy 2]` and `[^i64 4]` are compile errors (R16) | `owned_of_linear_array_is_error` + `array_of_owned_is_error` (P1 unit, P3 golden) |
| 17 | No regression: all 14 examples byte-identical; no pre-existing test regresses | existing goldens + example sweep (P5) |

Notes on two criteria whose technique matters:

- **Criterion 13** is deliberately *not* "write, read back". With three words and no references
  there is no way to mutate a payload in place: peek yields a copy, and `set` on an array is
  functional and consuming, so a write-back round trip would be `^> … set … ^`, which is a
  second alloc/free pair. In-place heap mutation waits for Slice 4's refs.
- **Criterion 14** uses an address-space limit on the child process (`ulimit -v`, or
  `setrlimit` via `pre_exec`), not host memory exhaustion. The brief's "one huge allocation"
  probe is impossible: the constructor is `( T -- ^T )`, so the payload must already exist on
  the data stack, and you can never request more than you can already hold. The assertion
  shape follows the existing bounds-trap golden: sentinel before, absent sentinel after,
  non-zero exit.

## Honest cost note

`^[u8 1024]` is a fixed-capacity heap buffer, but the payload transits the data stack on the
way in and on the way out, so construction copies the buffer once onto the stack and once into
the cell. It is correct, not free. A genuinely cheap large buffer wants runtime-sized
allocation, which is Phase 6's `alloc` layer.

## Phases JSON

```json
{"phases":[
  {"phase":1,"focus":"The ^T owning-cell type as a compiler-known type constructor (NOT generics, no type variables): a Type variant plus an owned-cell registry with dedup-by-shape interning mirroring ArrayDecl/intern_array_type, threaded like arrays including session persistence in the REPL; ^T in type position through parser/ast with a located error for a bare ^ with no payload; and is_copy returning false unconditionally for a cell (no payload lookup, so is_copy's arity is unchanged). No lexer change is needed: ^ is not a delimiter so ^i64 is one word, ^[u8 4] splits at the bracket, and ^|> survives the existing peek glue. Exit is unit-level because no ^ value can exist until the constructor lands in phase 3.","changes":["src/ast.rs","src/parser.rs","src/check.rs","src/ir.rs","src/repl.rs"],"tests":["src/ast.rs","src/check.rs","src/ir.rs"],"exit":"unit tests: is_copy of a cell is false; two mentions of ^i64 intern to one entry; ^i64/^[u8 4]/^^i64 parse in type position; a bare ^ errors; [^i64 4] is rejected by the existing linear-array sweep; green; no regression"},
  {"phase":2,"focus":"Allocation machinery with nothing calling it yet: emit the allocate/free shim wrapping malloc/free as a compiler-emitted helper following the drop-spy printf precedent (no user-facing FFI), the ENV-GATED allocation trace writing one event line per alloc and per free to stderr when the variable is set (chosen over a silent counter, which no golden can read; over unconditional printing, which is wrong because ^ is user surface unlike the test-only __spy; over a mutable global, which has no precedent in the emitter; and over a per-module counter, which would exist once per REPL .so), the trap-and-abort path when malloc returns NULL, and the max(size,1) adjustment so a zero-sized payload never reaches malloc(0). Add the IrType variant plus its width/load_op/store_op arms, deferring cell size to Ptr's existing convention rather than introducing a second width assumption.","difficulty":"hard","changes":["src/backend/qbe.rs","src/ir.rs"],"tests":["src/backend/qbe.rs"],"exit":"unit-level: the emitted IL contains the shim, the NULL trap and the gated trace; green; no regression"},
  {"phase":3,"focus":"The three access words: ^ as ( T -- ^T ), ^> as ( ^T -- T ) which frees the cell, and ^|> as a non-consuming ( ^T -- ^T T ) peek restricted to Copy payloads with a compile error naming the type on a linear payload. Construction stores a scalar payload into the allocated pointer and blits an aggregate payload from its frame slot; unwrap MATERIALISES the payload before releasing the cell (a Load for a scalar, a fresh frame Alloc plus a Blit for an aggregate) so the freed pointer is never handed to the stack; peek of an aggregate allocates a fresh frame slot and blits out rather than aliasing the cell.","changes":["src/check.rs","src/ir.rs","tests/phase0.rs"],"tests":["tests/phase0.rs","src/check.rs"],"exit":"criteria 2, 3, 3b, 4, 5, 5b, 7, 9, 16 pass; green; no regression"},
  {"phase":4,"focus":"Drop glue and every allocation-observing golden: an emit_drop arm for the cell plus a synthesized per-type destructor that drops a linear payload FIRST and then frees, an ordering that the trace makes observable and therefore contractual. drop stays a Call to a per-type symbol with no new Instr or Terminator variant. Confirm the existing unreachable! guard on emit_drop's linear-array arm remains valid given the cell is a new linear type.","changes":["src/ir.rs","src/backend/qbe.rs","tests/phase0.rs"],"tests":["tests/phase0.rs","src/ir.rs"],"exit":"criteria 1, 1b, 6, 8, 9b, 10, 11, 12, 13, 14 pass, each asserting the exact ordered trace transcript; green; no regression"},
  {"phase":5,"focus":"REPL residual disposal and the regression sweep: confirm a cell left on the residual REPL stack is freed at :quit through the existing dispose_residual path, expected to need no production change beyond phase 1's session-persistent registry, then verify all 14 examples are byte-identical and no pre-existing test regresses.","changes":["tests/phase1.rs","tests/phase0.rs"],"tests":["tests/phase1.rs","tests/phase0.rs"],"exit":"criteria 15 and 17 pass; green"}
]}
```
