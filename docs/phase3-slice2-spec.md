# Phase 3 Slice 2 — Heap + owning pointer + allocator (spec)

Derived from [`phase3-slice2-brief.md`](./phase3-slice2-brief.md); its decisions D1-D10 are
locked, except D9's counter, which review proved unimplementable and which R10 supersedes.
Base: `main` at Slice 1 merged. This slice **changes the compiler** (`src/**`) and gives the
linear spine its first real resource: a heap cell whose destructor frees memory.

Revised after spec review rounds 1 and 2.

## Requirements (traceable)

- **R1 (D1)** — The heap primitive is a single heap cell holding one value of a concrete
  payload type. No sized/growable buffer, no runtime-sized allocation. A fixed-capacity heap
  buffer is `^[u8 N]`, with the size in the type.
- **R2 (D2)** — The cell is a **compiler-known type constructor, not generics**: one interned
  registry entry per concrete payload type, deduplicated by shape, mirroring
  `ArrayDecl`/`intern_array_type` (@ast.rs:171). Builtin words are type-checked ad hoc at the
  call site, mirroring `check_array_word` (@check.rs:2039). **No type variables**; Phase 4
  owns those. The registry carries the rendered name inline, as `Type::Array` does
  (@ast.rs:171-186), so diagnostics can print `^i64` with no registry lookup.
- **R3 (D2 tripwire)** — The cell is the second ad-hoc type constructor after arrays. No third
  is added and no general mechanism is built. If Slice 3 needs one, that is the signal to
  reconsider in favour of Phase 4's generics.
- **R4 (D3)** — `^T` is **always linear**, whatever `T` is. `is_copy` returns `false`
  **unconditionally**, with no payload lookup, so `is_copy`'s signature is unchanged.
  Linearity propagates transitively via Slice 1's rules, so a struct or enum containing a cell
  is linear with synthesized drop glue.
- **R5 (D4)** — A cell **may hold a linear payload** (`^__spy` is legal). Disposal drops the
  payload **first**, then frees the cell. Observable via R10, therefore contractual.
- **R6 (D5)** — A **single global allocator** behind an interface conceptually in `core`:
  `allocate(n) -> ptr`, `free(ptr, n)`. Not parameterized per value.
- **R7 (D6)** — The one implementation is a **compiler-emitted shim** wrapping `malloc`/`free`,
  following `emit_spy_drop` (@qbe.rs:543). **No user-facing FFI.** Expected to be re-expressed
  as bound foreign words once Phase 6 lands FFI-to-libc.
- **R8 (D7)** — The destructor is **compiler-known**: `drop` on a cell frees it. User-definable
  destructor bodies are Slice 6. `drop` stays a `Call` to a per-type symbol (Slice 1 R16), so
  `emit_drop` (@ir.rs:1890) gains an arm and `synthesize_aggregate_destructors` (@ir.rs:793)
  gains a case. **No new `Instr`/`Terminator` variant.**
- **R9 (D8)** — A failed allocation (`malloc` returning NULL) **traps and exits non-zero**,
  reusing the bounds-check trap pattern. It calls **`exit(1)`, not `abort`**, matching
  `emit_oob_trap` (@qbe.rs:535), so a test observes `Some(1)` rather than death by signal.
  No NULL may reach a dereference. Revisited in Slice 3, which introduces optional/non-null
  pointers and gives a failed allocation somewhere to go.

### R10 (supersedes D9) — the allocation trace

D9's silent counter is unimplementable: nothing can read it, mutable globals have no precedent
in the emitter, it would exist once per REPL `.so`, and it cannot pin ordering. It is replaced
by an **env-gated allocation trace**, fully specified here because round 2 found the earlier
prose was a rationale rather than a specification.

- **Gate**: the environment variable **`SOOTH_TRACE_ALLOC`**. When unset or empty, the shim
  emits nothing. Checked with `getenv` **per event**, not cached, deliberately: caching would
  need the mutable-global data-symbol path that has no precedent in the emitter, and this is
  test-only telemetry where the cost is irrelevant.
- **Stream**: **stdout, via `printf`**, the same stdio stream the drop-spy and `.` already use.
  This is load-bearing, not incidental. The trace cannot go to stderr: `printf` is
  stdio-buffered while the emitter's stderr path is unbuffered `dprintf(2, …)` (@qbe.rs:534),
  and the harness captures the two streams into separate buffers, so a cross-stream assertion
  has no order to observe at all. One stdio stream means **program order equals transcript
  order**, which is the entire point.
- **Format**: exactly one line per event, `alloc <size>` and `free <size>`, where `<size>` is
  the byte count actually requested (post-R15 adjustment). **No addresses**, because an
  address would make an exact transcript unassertable.
- **Default silence**: because `^` is *user surface*, unlike the test-only `__spy`, real
  programs must print nothing. Unconditional print-on-free is therefore rejected even though
  it is simpler. Gate-off behaviour is itself tested (criterion 18).
- **Harness**: `run_and_capture_stdout` (@tests/phase0.rs:9-21) neither sets environment
  variables nor is needed to; a golden that wants the trace uses a new helper that sets
  `SOOTH_TRACE_ALLOC` and returns stdout. The REPL's `run_session` (@tests/phase1.rs:9-32)
  needs the same treatment for criterion 15.
- Being on stdout, the trace is shared across REPL `.so`s with no interposition problem.

### Remaining requirements

- **R11 (D10)** — Access words mirror Slice 1's struct-word family: a constructor, a consuming
  unwrap, and a non-consuming **Copy-only** peek. Peeking a linear payload is a compile error.
  No reference machinery; contents are reached by stack-threading, which is why heap precedes
  refs (Slice 4).
- **R12 (brief surface-syntax section, confirmed 2026-07-26)** — The cell is spelled with a
  `^` sigil in both type and term position:

  | operation | spelling | effect |
  |---|---|---|
  | type | `^T` | `^i64`, `^Point`, `^[u8 1024]`, `^^i64` |
  | construct | `^` | `( T -- ^T )` |
  | unwrap | `^>` | `( ^T -- T )`, frees the cell |
  | peek | `^\|>` | `( ^T -- ^T T )`, Copy payload only |
  | dispose | `drop` | frees, dropping a linear payload first |

  Verified against the tree: `^` is unused in `.sth` source and is not an operator; `^` is not
  a delimiter so `^i64` scans as one word while `^[u8 1024]` splits at the bracket; `^|>`
  survives Slice 1's peek-glue rule as one token; `^>` cannot enter the conversion-word path,
  which requires `>` as the *first* character (@check.rs:1893). **No lexer change is required.**
- **R12a (new; round 2 A1)** — `^`, `^>` and `^|>` are **reserved**. A `:` word definition, a
  `type:` name, or a local binding whose name is one of them, **or begins with `^`**, is a
  located error naming the spelling. This is required, not belt-and-braces: Sooth has no notion
  of an identifier (a `type:` name is `expect_word_any()`, @parser.rs:570), so `type: ^ x i64 ;`
  type-checks clean today and generates `^` and `^>` words that collide *exactly* with the cell
  spellings, which the builtin arms would then silently shadow. A local named `^` shadows the
  constructor too, since locals resolve first (@check.rs:1621). The earlier claim that a
  collision "cannot arise" was false and is withdrawn.
  Out of scope: the general pre-existing bug that *any* non-identifier type or word name crashes
  the backend rather than erroring. Only the `^` slice of it is fixed here.
- **R12b (new; round 2 A3)** — The three cell words match by **exact name only**. `^>x` and
  `^|>x` (which a user will write by analogy with `Point>x`) each lex as one word and must
  produce the ordinary unknown-word error, not be reinterpreted. **Exact-name matching is what
  carries this**; the ordering half of the original claim was withdrawn in Phase 5. The worry
  was that `"^|>".split_once("|>")` yields `("^", "")` and would be probed as struct `^` with an
  empty field, but `check_struct_peek_word` returns `None` on a registry miss and R12a makes `^`
  undeclarable, so the probe can never hit. Verified by swapping the two arms: the whole suite
  stays green. Keep the cell arm first as cheap defence, but no test guards the order, because
  no behaviour depends on it.
- **R13 (round 1)** — **Unwrap materialises the payload before releasing the cell.** The freed
  pointer is never handed to the stack. By payload shape:
  - **scalar** (including `i64`, `__spy`, and a **nested cell**, which is pointer-width): a
    `Load` precedes the `free` call;
  - **aggregate** (struct, enum, array): `Alloc` a fresh frame slot, `Blit` `size` bytes out of
    the cell, then free;
  - **zero-sized**: neither load nor blit; a zero-field struct never emits a blit (guarded in
    the frontend, @qbe.rs:826-828), so the cell is simply freed.
  This is load-bearing: at runtime an aggregate value *is* a pointer to its storage
  (@ir.rs:84-88), so the naive lowering pushes the cell pointer and then frees it, a
  use-after-free that would pass a golden on glibc most of the time.
- **R14 (round 1)** — Construction is the mirror: a scalar payload is `Store`d into the
  allocated pointer; an aggregate payload is `Blit`ted from its frame slot; a zero-sized payload
  writes nothing. **Peek of an aggregate must `Alloc` a fresh frame slot and `Blit` out, never
  alias the cell**, or a later `drop` leaves the peeked value dangling.
- **R15 (brief risk 4, resolved)** — A zero-sized payload **is** constructible: a zero-field
  struct is legal and lays out size 0, align 1 (@ir.rs:3086). `allocate` therefore requests
  `max(size, 1)` and `free` passes the same adjusted size, so every cell has a distinct address
  and free-once stays meaningful. Without it, `^Unit` reaches `malloc(0)`, which may return NULL
  and fire R9's trap on a correct program.
- **R16 (documented consequence)** — Because `^T` is linear (R4), Slice 1's module-wide sweep
  (`check_no_linear_array_elements` @check.rs:371, which iterates the whole array registry) now
  rejects **an array of cells** (`[^i64 4]`), just as it already rejects `^[__spy 2]`. The
  restriction attaches to the array type itself, not to its position, so nesting does not
  launder it: `^[^i64 4]` is rejected too. Allowing it would need an element-wise drop loop in
  the synthesized destructor, which is exactly what Slice 1 deferred. Consequence: **there is no
  collection of resources in this slice**; the first real pressure for one is Slice 6.
- **R17 (revised; round 2 B1)** — The cell needs its own `IrType` variant so drop dispatch is
  not keyed off a bare pointer, mirroring `IrType::Spy`. **Every existing `IrType::Spy` match
  arm needs a cell counterpart** (roughly fourteen exhaustive matches), not the three the
  earlier draft named. Two are load-bearing beyond compilation:
  - `field_is_linear` (@ir.rs:170) and `layout_field_is_linear` (@ir.rs:587) must return
    **true** for a cell, or a struct or enum containing one is never marked linear and
    criteria 9, 9b and 10 silently pass while doing nothing;
  - `scalar_size_align_ww` (@ir.rs:327) governs a cell-typed struct field, which criterion 9b
    depends on.
  The print path (@qbe.rs:798) must be `unreachable!`, since the checker rejects printing a
  cell. Cell size **defers to `Ptr`'s existing convention** (currently hardcoded 8 at
  @ir.rs:325, already recorded to retrofit to the word-width parameter alongside `Ptr`); the
  cell must not introduce a second, independent width assumption. The REPL residual's cell
  arm is **not** such a site: `format_stack` counts 8-byte carried cells (`carried_slot_bytes`
  = 8 for every scalar), so its `cell += 1` holds whatever width `Ptr` retrofits to.
- **R18 (deferred)** — Consolidating the `structs`/`enums`/`arrays` registries into one borrow
  is **not** done here. The original justification was wrong: because R4 makes `is_copy` return
  `false` with no payload lookup, its arity is unchanged and there is no forcing function. The
  real added cost is one more `&mut Vec<..>` interning registry threaded alongside `arrays`
  (13 checker signatures plus the parser's `TypeCtx` @parser.rs:314), materially cheaper than a
  50-60-site consolidation. Consolidation also cannot be a rename: `Ctx` holds immutable borrows
  while interning needs a mutable one (the borrow split at @check.rs:326-338 exists for exactly
  this reason), so it would require stripping the registries out of `Ctx`. **Trigger**: a third
  interning registry, or any change needing `Ctx` surgery anyway.
- **R19 (new; round 2 A2)** — The `^` **type-position parse rule**, because `^T` arrives in two
  token shapes: `^i64`/`^^i64`/`^Point` are a single word with a leading `^`-run, while
  `^[u8 4]`/`^^[u8 4]` are a word of *only* `^`s followed by `LBracket`. The rule: **strip the
  leading `^`-run; if the remainder is empty, expect a following type expression; otherwise
  resolve the remainder as a type name.** Apply it in every type position, explicitly including
  `parse_field_type_expr` (@parser.rs:597-603) and the REPL's `parse_typedef_line`
  (@parser.rs:228). Without the field position, `type: Buf b ^[u8 4] ;` fails to parse, which is
  precisely the buffer case R1 advertises.

## Load-bearing invariants (must not break)

- Backend stays **QBE**; no LLVM.
- IR `Ptr[T]` stays **opaque**: never assumed to be a `u64`.
- Drop dispatch must **not** key off a bare pointer type (R17).
- `core` stays `no_std`. The language sees an allocator *interface*; libc appears only in the
  emitted shim.
- No new `Instr`/`Terminator` variant (R8). No user-facing FFI (R7).
- The shim, trap and trace are emitted **unconditionally**, matching the `emit_spy_drop`
  precedent, so a program that uses no cell simply never calls them. Duplicate shim definitions
  across REPL `.so`s are benign for the same reason the spy's are: they wrap libc and hold no
  state, and the trace's state lives in stdout, not in the module.
- **Phase 3 must not `drop` a cell.** `emit_drop` ends in `_ => {}` (@ir.rs:1904), so between
  the cell existing (P3) and its arm landing (P4) a `drop` would compile to a silent leak rather
  than a loud failure. No P3 golden may rely on it.
- R6 and R7 are structural, negative constraints, enforced by review rather than by a golden.

## Delivery phases

**Sequencing rule**: a golden that observes an allocation or a free cannot land before the
machinery that produces the observation. Compile-error goldens, and unwrap (which calls the
free shim directly rather than going through drop glue), can land earlier.

### Phase 1 — The `^T` type: interning, classification, surface, reserved names

- `Type` variant + owned-cell registry with dedup-by-shape interning, name rendered inline
  (R2), threaded like `arrays` including REPL session persistence. The R19 parse rule in every
  type position including struct fields and the REPL typedef line. `is_copy` false
  unconditionally (R4). The R12a reserved-name diagnostic at all three declaration sites.
- **Exit is unit-level** (no `^` value exists until P3): `is_copy` of a cell is false; two
  mentions of `^i64` intern to one entry; `^i64`, `^^i64`, `^[u8 4]`, `^^[u8 4]` parse in slot
  *and* struct-field position; a bare `^` with no payload errors; criteria 16 and 19 pass; the
  three lexer claims are asserted (criterion 20). Green; no regression.
- **Changes**: `src/ast.rs`, `src/parser.rs`, `src/check.rs`, `src/ir.rs`, `src/repl.rs`,
  `src/lexer.rs` (tests only).

### Phase 2 — Allocation: shim, env-gated trace, OOM trap  *(hard)*

- Emit the `allocate`/`free` shim (R6, R7), the `SOOTH_TRACE_ALLOC`-gated stdout trace (R10),
  the NULL-return `exit(1)` trap (R9), and the `max(size, 1)` adjustment (R15). Add the
  `IrType` variant and **every** `IrType::Spy`-parallel match arm (R17). Add the trace-reading
  test helpers to `tests/phase0.rs` and `tests/phase1.rs`.
- The shim and trap follow `emit_spy_drop`/`emit_oob_trap` closely; **the trace is the new
  part**, so budget difficulty there.
- **Exit is unit-level**: the emitted IL contains the shim, the NULL check plus trap call
  (criterion 14), and the gated trace. Nothing calls the shim yet. Green; no regression.
- **Changes**: `src/backend/qbe.rs`, `src/ir.rs`, `tests/phase0.rs`, `tests/phase1.rs`.

### Phase 3 — The three access words

- Constructor, consuming unwrap, Copy-only peek (R11, R12, R12b), with the copy-in / copy-out
  rules (R13, R14) for all four payload shapes.
- The constructor must type the allocator `Call`'s destination as the cell directly. It must
  **not** relabel a `Ptr` result with a `Conv` the way the spy constructor does: `norm_scalar`
  (@qbe.rs:522) is the one `IrType::Spy` arm Phase 2 deliberately left without a cell
  counterpart, so a `Conv` on a cell reaches `emit_conv`'s numeric-endpoints `unreachable!`.
- **Exit**: criteria 2, 3, 3b, 4, 5, 5b, 7, 9, 18 pass. Green; no regression.
- **Changes**: `src/check.rs`, `src/ir.rs`, `tests/phase0.rs`.

### Phase 4 — Drop glue, ordering, and the allocation-observing goldens

- `emit_drop` arm plus a synthesized per-type destructor that drops a linear payload **first**
  and then frees (R5, R8). Confirm `emit_drop`'s linear-array `unreachable!` guard stays valid
  now that a new linear type exists.
- **Exit**: criteria 1, 1b, 6, 8, 9b, 10, 11, 12, 13, 17 pass. Green; no regression.
- **Changes**: `src/ir.rs`, `src/backend/qbe.rs`, `tests/phase0.rs`.

### Phase 5 — REPL residual disposal and regression sweep

- Confirm a cell on the residual REPL stack is freed at `:quit` via `dispose_residual`
  (@repl.rs:477); the disposal path itself needs no production change beyond P1's session
  registry.
- **Production-change carve-out (Phase 3 review finding)**: `format_stack` (@repl.rs:180)
  has no `Type::OwnedCell` arm, so a live cell on the residual stack falls into the
  scalar catch-all and prints as a raw heap address (nondeterministic, so it can't be
  asserted in a golden). Every other non-printable aggregate (`Struct`/`Enum`/`Array`)
  gets a `<Name>` placeholder arm instead; `OwnedCell` already carries its rendered name
  inline, so add a matching `<^i64>`-style arm. This is a real production change beyond
  the session registry, and criterion 15's golden should assert the placeholder string,
  not a value. It is strictly a **Phase 3** obligation (the arm is needed the moment `^`
  is constructible at the REPL), landed here; commit 6fbddc1's message claims the arm but
  its diff never touches `src/repl.rs`, so do not audit this by commit message.
- Criterion 21's golden is an **exact-name regression pin, not an arm-ordering guard**, and
  nothing else guards the ordering either: see R12b, whose ordering clause is withdrawn
  here as unobservable. Do not add a test for it; there is no failure mode to pin.
- **Exit**: criteria 15 and 21 pass. Green.
- **Changes**: `src/repl.rs`, `tests/phase1.rs`, `tests/phase0.rs`.

## Criterion → test map

All goldens are runnable native binaries (`tests/phase0.rs`) or REPL sessions
(`tests/phase1.rs`), never IL-string assertions, **except** criteria 14 and 20, which are
deliberately emitter-level and lexer-level unit assertions and are marked as such.

Inherited house rules, binding here:

- every **negative** golden asserts the diagnostic substring **and** the backticked type name;
- criterion 4 additionally asserts the **move site**, per Slice 1's rule;
- every **trace-observing** golden asserts the **exact ordered stdout transcript**, since a
  count alone cannot distinguish a leak from a double-free. **Criterion 1b is the one
  exception**: it runs with the gate off and asserts a memory bound instead.

| # | Criterion | Test (phase) |
|---|---|---|
| 1 | Construct then `drop`: transcript is exactly one `alloc` and one `free` | `owned_alloc_and_drop_traces_one_pair` (P4) |
| 1b | Frees are real: ~100k construct-and-dispose iterations of `[u8 1024]` under a 64 MB `RLIMIT_AS`, gate **off**, exit 0. A fake free necessarily exceeds the limit and trips R9 | `owned_alloc_dispose_loop_stays_within_memory_bound` (P4) |
| 2 | Forgetting to dispose a cell is a compile error | `unconsumed_owned_is_error` (P3) |
| 3 | `dup` of `^i64` (a **Copy** payload, proving the cell is linear regardless) errors naming `^i64` | `dup_of_owned_is_error` (P3) |
| 3b | `over` of `^i64` is a compile error | `over_of_owned_is_error` (P3) |
| 4 | Use-after-move of a cell local errors, **naming the move site** | `use_after_move_of_owned_is_error` (P3) |
| 5 | Unwrap returns the payload value; transcript is one `alloc`, one `free` | `owned_unwrap_returns_payload_and_frees_once` (P3) |
| 5b | Unwrap of an **aggregate** payload: a field read **after** the free is correct (proves R13) | `owned_unwrap_aggregate_copies_out_before_free` (P3) |
| 6 | Peek twice then dispose: both peeked values correct and equal, exactly one `free` | `peek_owned_copy_payload_keeps_cell_live` (P4) |
| 7 | Peek of a **linear** payload is a compile error | `peek_owned_linear_payload_is_error` (P3) |
| 8 | `^__spy` disposal: transcript is `drop 7` **then** `free 8`, one stdout stream so the order is real | `owned_linear_payload_drops_before_free` (P4) |
| 9 | A struct containing a cell is linear: `dup` on it errors naming the struct | `struct_containing_owned_is_linear` (P3) |
| 9b | Dropping that struct frees the cell | `dropping_struct_with_owned_frees_cell` (P4) |
| 10 | Enum variant carrying a cell, built behind an `if`: dropping the cell-carrying variant frees exactly once; dropping the **other** variant frees **zero** times | `enum_variant_with_owned_frees_on_drop` (P4) |
| 11 | `^^[u8 24]`: sizes are deliberately distinct (inner 24, outer 8) so the transcript `alloc 24 / alloc 8 / free 24 / free 8` proves **inner freed before outer**, which equal sizes could not | `nested_owned_frees_inner_before_outer` (P4) |
| 12 | Zero-sized payload: transcript shows `alloc 1` / `free 1`, witnessing R15's `max(size,1)`. Asserting the **size** matters: glibc's `malloc(0)` returns non-NULL, so a count-only test passes even if the adjustment is deleted | `owned_zero_sized_payload_allocs_one_byte` (P4) |
| 13 | `^[u8 N]`: construct from a filled array, peek, `get` a byte, `drop`; exactly one `alloc`/`free`. **Then** peek an aggregate, dispose the cell, and read the peeked copy, proving R14's peek does not alias | `owned_byte_buffer_peek_get_and_free_once` + `peek_aggregate_does_not_alias_cell` (P4) |
| 14 | OOM trap exists: the emitted IL contains the NULL check and the `exit(1)` trap call | `emitted_alloc_shim_has_null_trap` (P2, emitter-level) |
| 15 | REPL `:quit` frees a residual cell | `repl_quit_frees_residual_owned` (P5) |
| 16 | Arrays of linear elements stay rejected in every position: `[^i64 4]`, `^[__spy 2]` and `^[^i64 4]` are compile errors (R16) | `array_of_owned_is_error` + `owned_of_linear_array_is_error` (P1) |
| 17 | No regression: all 14 examples produce **byte-identical stdout**; no pre-existing test regresses | existing goldens + example sweep (P4/P5) |
| 18 | **Gate off by default**: construct and drop with `SOOTH_TRACE_ALLOC` unset emits only the program's own output. Without this, a regression inverting the gate ships green | `alloc_trace_is_silent_when_unset` (P3) |
| 19 | `^` is reserved: `type: ^ …`, `: ^ …`, and a local named `^` are each located errors (R12a) | `reserved_caret_type_name_is_error` + `..._word_name_...` + `..._local_...` (P1) |
| 20 | Lexer claims hold: `^\|>` is one token, `^^i64` is one token, `^[u8 4]` splits at the bracket (R12) | lexer unit asserts beside the existing `\|>` glue tests (P1, lexer-level) |
| 21 | `^>x` and `^\|>x` produce the ordinary unknown-word error, not a reinterpretation (R12b) | `caret_field_suffix_is_unknown_word` (P5); covers exact-name matching only, R12b's ordering half being unobservable |

## Why criterion 14 is not a runtime golden

Round 2 showed the `ulimit -v` probe is unsound, not merely awkward: a limit low enough to make
a small `malloc` return NULL will usually make `execve`/`ld.so` fail first, so the sentinel
never prints; a limit high enough to exec cleanly leaves glibc's arena mapped so the allocation
succeeds; and the failure can arrive as a signal, which makes the harness's `.code()` unwrap
panic. The brief's own escape hatch applies ("assert the trap path exists by other means rather
than shipping a flaky golden"). Runtime OOM behaviour is revisited in Slice 3 under R9, where
optional pointers give a failed allocation somewhere to go. Note `RLIMIT_AS` remains sound for
criterion 1b, where a 64 MB limit clears process startup comfortably and only a genuine leak
crosses it.

## Honest cost note

`^[u8 1024]` is a fixed-capacity heap buffer, but the payload transits the data stack on the way
in and on the way out, so construction copies the buffer once onto the stack and once into the
cell. Correct, not free. A genuinely cheap large buffer wants runtime-sized allocation, which is
Phase 6's `alloc` layer.

## Phases JSON

```json
{"phases":[
  {"phase":1,"focus":"The ^T owning-cell type as a compiler-known type constructor (NOT generics, no type variables): a Type variant plus an owned-cell registry with dedup-by-shape interning mirroring ArrayDecl/intern_array_type, carrying the rendered name inline so diagnostics print ^i64 with no lookup, threaded like arrays including REPL session persistence. The ^ type-position parse rule (strip the leading ^-run; if the remainder is empty expect a following type expression, otherwise resolve the remainder as a type name) applied in EVERY type position including parse_field_type_expr and the REPL parse_typedef_line, without which type: Buf b ^[u8 4] fails to parse. is_copy returns false unconditionally for a cell. A located reserved-name diagnostic for ^, ^> and ^|> and any name with a leading ^ at all three declaration sites (type name, word name, local binding), which is required because Sooth has no identifier validation and type: ^ x i64 type-checks clean today, generating words that collide exactly with the cell spellings. No lexer change is needed beyond tests.","changes":["src/ast.rs","src/parser.rs","src/check.rs","src/ir.rs","src/repl.rs","src/lexer.rs"],"tests":["src/ast.rs","src/check.rs","src/ir.rs","src/lexer.rs","tests/phase0.rs"],"exit":"unit-level: is_copy of a cell is false; two mentions of ^i64 intern to one entry; ^i64, ^^i64, ^[u8 4] and ^^[u8 4] parse in slot and struct-field position; a bare ^ errors; criteria 16, 19 and 20 pass; green; no regression"},
  {"phase":2,"focus":"Allocation machinery with nothing calling it yet: the allocate/free shim wrapping malloc/free as a compiler-emitted helper following the drop-spy precedent (no user-facing FFI), emitted unconditionally; the trace gated on the SOOTH_TRACE_ALLOC environment variable, checked per event via getenv (not cached, since caching needs the mutable-global path that has no precedent in the emitter) and written to STDOUT via printf rather than stderr, because printf is stdio-buffered while the emitter's stderr path is unbuffered dprintf and the harness captures the streams separately, so only one stdio stream makes program order equal transcript order; the event format is exactly one line per event, 'alloc <size>' and 'free <size>', with no addresses so transcripts stay assertable; a NULL-return trap calling exit(1) rather than abort so a test observes Some(1); and the max(size,1) adjustment so a zero-sized payload never reaches malloc(0). Add the IrType variant and EVERY IrType::Spy-parallel match arm (about fourteen), critically field_is_linear and layout_field_is_linear returning TRUE for a cell, without which a struct or enum containing one is never marked linear and three criteria silently pass while doing nothing. Add the trace-reading test helpers.","difficulty":"hard","changes":["src/backend/qbe.rs","src/ir.rs","tests/phase0.rs","tests/phase1.rs"],"tests":["src/backend/qbe.rs","tests/phase0.rs"],"exit":"unit-level: the emitted IL contains the shim, the gated trace, and the NULL check plus exit(1) trap call (criterion 14); green; no regression"},
  {"phase":3,"focus":"The three access words: ^ as ( T -- ^T ), ^> as ( ^T -- T ) which frees the cell, and ^|> as a non-consuming ( ^T -- ^T T ) peek restricted to Copy payloads with a compile error naming the type on a linear payload. The three match by exact name only, so ^>x and ^|>x give the ordinary unknown-word error, and the cell-peek arm is exact-matched or ordered before check_struct_peek_word since '^|>'.split_once('|>') yields ('^',''). Construction stores a scalar payload, blits an aggregate payload from its frame slot, and writes nothing for a zero-sized payload; unwrap MATERIALISES the payload before releasing the cell (a Load for a scalar including a nested cell, a fresh frame Alloc plus a Blit for an aggregate, neither for a zero-sized payload) so the freed pointer is never handed to the stack; peek of an aggregate allocates a fresh frame slot and blits out rather than aliasing the cell. Phase 3 must not drop a cell: emit_drop still ends in a catch-all, so a drop here would compile to a silent leak until phase 4 adds the arm.","changes":["src/check.rs","src/ir.rs","tests/phase0.rs"],"tests":["tests/phase0.rs","src/check.rs"],"exit":"criteria 2, 3, 3b, 4, 5, 5b, 7, 9, 18 pass; green; no regression"},
  {"phase":4,"focus":"Drop glue and the allocation-observing goldens: an emit_drop arm for the cell plus a synthesized per-type destructor that drops a linear payload FIRST and then frees, an ordering the single-stream trace makes observable and therefore contractual. drop stays a Call to a per-type symbol with no new Instr or Terminator variant. Confirm emit_drop's linear-array unreachable guard remains valid now that a new linear type exists. Goldens assert exact ordered stdout transcripts, with sizes chosen to discriminate (nested cells use ^^[u8 24] so inner and outer differ, and the zero-sized case asserts alloc 1 to witness the max(size,1) adjustment that glibc would otherwise mask); criterion 1b is the sole exception, running with the gate off under a 64 MB RLIMIT_AS so a fake free necessarily trips the trap.","changes":["src/ir.rs","src/backend/qbe.rs","tests/phase0.rs"],"tests":["tests/phase0.rs","src/ir.rs"],"exit":"criteria 1, 1b, 6, 8, 9b, 10, 11, 12, 13, 17 pass; green; no regression"},
  {"phase":5,"focus":"REPL residual disposal and the regression sweep: confirm a cell left on the residual REPL stack is freed at :quit through the existing dispose_residual path, which needs no production change beyond phase 1's session-persistent registry, plus the one carve-out production change of a Type::OwnedCell placeholder arm in format_stack (without it a residual cell prints a nondeterministic raw heap address and criterion 15's golden cannot assert an exact transcript), then verify all 14 examples produce byte-identical stdout and no pre-existing test regresses.","changes":["src/repl.rs","tests/phase1.rs","tests/phase0.rs"],"tests":["src/repl.rs","tests/phase1.rs","tests/phase0.rs"],"exit":"criteria 15 and 21 pass; green"}
]}
```
