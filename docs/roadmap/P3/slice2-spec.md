# Phase 3 Slice 2 — Heap + owning pointer + allocator (as built)

From [`phase3-slice2-brief.md`](./phase3-slice2-brief.md); D1-D10 locked except D9's counter, which R10 replaces. Base: `main` at Slice 1. This slice changed the compiler and gave the linear spine its first real resource: a heap cell whose destructor frees memory. Delivered across 5 phases (`426ea1a4` … `0cab03fb`).

## Requirements

- **R1** — One heap cell holding one value of a concrete payload type. No runtime-sized allocation; a fixed-capacity buffer is `^[u8 N]`, size in the type.
- **R2** — Compiler-known type constructor, **not generics**: `Type::OwnedCell(OwnedCellId, name)` + an interned `Vec<OwnedCellDecl>` deduped by shape, mirroring `ArrayDecl`/`intern_array_type`. Rendered name carried inline so diagnostics print `^i64` with no lookup. Builtins checked ad hoc at the call site. No type variables (Phase 4 owns those).
- **R3 (tripwire)** — Second ad-hoc constructor after arrays. A third is the signal to switch to Phase 4 generics instead.
- **R4** — `^T` is always linear; `is_copy` returns `false` unconditionally, signature unchanged. Linearity propagates transitively via Slice 1's rules.
- **R5** — A cell may hold a linear payload (`^__spy`). Disposal frees the cell first, then drops the payload (reversed in Phase 3 Slice 3, R8; the ordering choice is uniformity across every cell, not a constraint of this slice).
- **R6** — Single global allocator interface conceptually in `core`: `allocate(n) -> ptr`, `free(ptr, n)`. Not per-value.
- **R7** — One implementation: a compiler-emitted shim wrapping `malloc`/`free`, following `emit_spy_drop`. No user-facing FFI. Expected to become bound foreign words once Phase 6 lands FFI-to-libc.
- **R8** — Destructor is compiler-known. `drop` stays a `Call` to a per-type symbol (Slice 1 R16): an `emit_drop` arm plus a `synthesize_aggregate_destructors` case. No new `Instr`/`Terminator` variant.
- **R9** — `malloc` returning NULL traps and calls `exit(1)`, not `abort`, matching `emit_oob_trap`, so a test observes `Some(1)`. No NULL reaches a dereference. Revisited in Slice 3 with optional/non-null pointers.

### R10 — the allocation trace (supersedes D9)

D9's silent counter was unimplementable (nothing can read it, no mutable-global precedent, one per REPL `.so`, cannot pin ordering). Replaced by an env-gated trace:

- **Gate**: `SOOTH_TRACE_ALLOC` (`ir::TRACE_ALLOC_ENV`). Unset or empty, the shim emits nothing. `getenv` per event, not cached: caching needs the mutable-global data-symbol path that has no precedent in the emitter, and this is test-only telemetry.
- **Stream**: stdout via `printf`. Load-bearing: `printf` is stdio-buffered while the emitter's stderr path is unbuffered `dprintf(2, …)`, and the harness captures the streams separately, so a cross-stream assertion has no order to observe. One stream means program order equals transcript order.
- **Format**: one line per event, `alloc <size>` / `free <size>`, size post-R15 adjustment. No addresses, so transcripts stay exactly assertable.
- **Default silence**: `^` is user surface (unlike `__spy`), so real programs print nothing. Gate-off is itself tested (criterion 18).
- **Harness**: new trace-setting helpers beside `run_and_capture_stdout` (tests/phase0.rs) and `run_session` (tests/phase1.rs). Being on stdout, the trace is shared across REPL `.so`s with no interposition problem.

### Remaining requirements

- **R11** — Access words mirror Slice 1's struct family: constructor, consuming unwrap, non-consuming **Copy-only** peek. Peeking a linear payload is a compile error. No references; contents are reached by stack-threading, which is why heap precedes refs (Slice 4).
- **R12** — `^` sigil in type and term position:

  | operation | spelling | effect |
  |---|---|---|
  | type | `^T` | `^i64`, `^Point`, `^[u8 1024]`, `^^i64` |
  | construct | `^` | `( T -- ^T )` |
  | unwrap | `^>` | `( ^T -- T )`, frees the cell |
  | peek | `^\|>` | `( ^T -- ^T T )`, Copy payload only |
  | dispose | `drop` | frees the cell, then drops a linear payload (Phase 3 Slice 3, R8) |

  `^` was unused in `.sth` source and is not a delimiter, so `^i64` scans as one word, `^[u8 1024]` splits at the bracket, `^|>` survives the peek-glue rule, and `^>` cannot enter the conversion-word path (which needs `>` first). **No lexer change required.**
- **R12a** — `^`, `^>`, `^|>`, and any name with a leading `^`, are reserved at all three declaration sites (`:` word, `type:` name, local binding), with a located error naming the spelling. Required, not belt-and-braces: Sooth has no identifier validation (`expect_word_any()`), so `type: ^ x i64 ;` type-checked clean and generated words colliding exactly with the cell spellings, which the builtin arms would silently shadow. Out of scope: the general pre-existing bug that any non-identifier name crashes the backend.
- **R12b (amended in P5)** — The three words match by **exact name only**, so `^>x` and `^|>x` give the ordinary unknown-word error. Exact matching carries this alone; the original arm-ordering clause is **withdrawn as unobservable**: `check_struct_peek_word` returns `None` on a registry miss and R12a makes `^` undeclarable, so `"^|>".split_once("|>") == ("^", "")` can never hit. Verified by swapping the arms (suite stays green). Cell arm kept first as cheap defence, untested by design.
- **R13** — **Unwrap materialises the payload before releasing the cell**; the freed pointer never reaches the stack. Scalar (incl. `i64`, `__spy`, nested cell): `Load` before `free`. Aggregate: `Alloc` a fresh frame slot, `Blit` `size` bytes out, then free. Zero-sized: neither. Load-bearing because an aggregate value *is* a pointer to its storage, so the naive lowering is a use-after-free that passes on glibc most of the time.
- **R14** — Construction mirrors it: `Store` a scalar, `Blit` an aggregate from its frame slot, write nothing for zero-sized. **Peek of an aggregate `Alloc`s a fresh slot and `Blit`s out, never aliases the cell**, or a later `drop` leaves the peeked value dangling.
- **R15** — Zero-sized payloads are constructible (a zero-field struct is size 0, align 1), so `allocate` requests `max(size, 1)` and `free` passes the same adjusted size: distinct addresses, free-once stays meaningful, and `^Unit` never hits `malloc(0)` and fires R9 on a correct program.
- **R16 (consequence)** — Since `^T` is linear, Slice 1's module-wide `check_no_linear_array_elements` sweep rejects `[^i64 4]`, and nesting does not launder it (`^[^i64 4]` also rejected). So **there is no collection of resources in this slice**; first real pressure is Slice 6.
- **R17** — The cell gets its own `IrType` variant so drop dispatch is not keyed off a bare pointer, mirroring `IrType::Spy`, with a cell counterpart for **every** `IrType::Spy` match arm (~fourteen). Two beyond compilation: `field_is_linear` and `layout_field_is_linear` must return **true** (else criteria 9/9b/10 silently pass while doing nothing), and `scalar_size_align_ww` governs a cell-typed struct field. Print path is `unreachable!`. Cell size defers to `Ptr`'s existing convention; no second width assumption. **Exception (P3):** `norm_scalar` deliberately has no cell arm, so the constructor types the allocator `Call` destination as the cell directly and never relabels a `Ptr` with a `Conv`, which would hit `emit_conv`'s numeric-endpoints `unreachable!`. The REPL's `format_stack` `cell += 1` is not a width site either (`carried_slot_bytes` is 8 for every scalar).
- **R18 (deferred)** — Registry consolidation not done. The forcing function was imaginary (R4 keeps `is_copy`'s arity), and one more `&mut Vec<..>` threaded alongside `arrays` (13 checker signatures plus the parser's `TypeCtx`) is materially cheaper than a 50-60-site change. Consolidation cannot be a rename: `Ctx` holds immutable borrows while interning needs a mutable one. **Trigger**: a third interning registry, or any change needing `Ctx` surgery anyway.
- **R19** — `^`-led type-expression production: strip the leading `^`-run; if the remainder is empty, expect a following type expression (recursing into the array-bracket case or a further `^`-run), otherwise resolve the remainder as a type name. Applies in every type position, including `parse_field_type_expr` (the struct-field position) and the REPL typedef line — without the field-position case, `type: Buf b ^[u8 4] ;` fails to parse.

## Load-bearing invariants

- Backend stays QBE; IR `Ptr[T]` stays opaque; `core` stays `no_std` (the language sees an allocator interface, libc appears only in the emitted shim).
- Drop dispatch must not key off a bare pointer type (R17). No new `Instr`/`Terminator` (R8). No user-facing FFI (R7).
- Shim, trap and trace are emitted unconditionally (the `emit_spy_drop` precedent); duplicate definitions across REPL `.so`s are benign since they wrap libc and hold no state, the trace's state living in stdout.
- Phase 3 must not `drop` a cell: `emit_drop` still ended in `_ => {}` until P4, so a `drop` would have compiled to a silent leak.
- R6 and R7 are negative, structural constraints, enforced by review, not by a golden.

## Delivery phases (as built)

Sequencing rule: a golden observing an allocation or free cannot land before the machinery producing the observation. Compile-error goldens, and unwrap (which calls the free shim directly), land earlier.

1. **The `^T` type** (`426ea1a4`, `9a6c32a8`): `Type` variant + interned registry threaded like `arrays` incl. REPL session persistence; the `^` type-position parse rule (strip the leading `^`-run; empty remainder expects a following type expression, otherwise resolve the remainder as a type name) in every type position including `parse_field_type_expr` and the REPL typedef line, without which `type: Buf b ^[u8 4] ;` fails to parse; `is_copy` false; R12a reserved names (extended in the follow-up to enum variants and slots). Unit-level exit, no `^` value exists yet. *ast.rs, parser.rs, check.rs, ir.rs, repl.rs, lexer.rs (tests).*
2. **Allocation machinery, nothing calling it** (`3fcb37ec`, `f4985f89`) *(hard)*: shim, gated stdout trace, NULL trap, `max(size,1)`, `IrType` variant and all Spy-parallel arms, trace-reading test helpers. The trace was the genuinely new part. *backend/qbe.rs, ir.rs, tests/phase0.rs, tests/phase1.rs.*
3. **The three access words** (`bcb75ddd`, `6fbddc12`): constructor, unwrap, Copy-only peek with the copy-in/copy-out rules for all four payload shapes. *check.rs, ir.rs, backend/qbe.rs, repl.rs, tests/phase0.rs.*
4. **Drop glue and the trace goldens** (`af95650e`, `3a50070a`, `8f9fee25`): `emit_drop` arm plus synthesized per-type destructor (originally dropping a linear payload before freeing; reversed in Phase 3 Slice 3, R8); `emit_drop`'s linear-array `unreachable!` guard reconfirmed. *ir.rs, check.rs, tests/phase0.rs.*
5. **REPL residual disposal + regression sweep** (`c705dc8a`, `2cf3759f`, `0cab03fb`): `dispose_residual` needed no change beyond P1's registry, but a **carve-out production change** was required: `format_stack` had no `OwnedCell` arm, so a residual cell printed a raw heap address; it now prints a `<^i64>` placeholder like other non-printable aggregates, and criterion 15 asserts the placeholder. Strictly a Phase 3 obligation, landed here (commit `6fbddc1`'s message claims the arm but its diff never touches `src/repl.rs`; do not audit by commit message). *repl.rs, tests/phase0.rs, tests/phase1.rs.*

## Criterion → test map

Goldens are runnable native binaries (`tests/phase0.rs`) or REPL sessions (`tests/phase1.rs`), except criteria 14 and 20, deliberately emitter- and lexer-level, and criteria 16 and 19, which are unit-level in `src/check.rs` and `src/parser.rs` respectively (parse/check errors with no runtime to observe). House rules: every negative golden asserts the diagnostic substring **and** the backticked type name; criterion 4 also asserts the move site; every trace-observing golden asserts the **exact ordered transcript** (a count cannot distinguish a leak from a double-free), criterion 1b excepted.

| # | Criterion | Test (phase) |
|---|---|---|
| 1 | Construct then `drop`: exactly one `alloc`, one `free` | `owned_alloc_and_drop_traces_one_pair` (P4) |
| 1b | Frees are real: ~100k construct/dispose of `[u8 1024]` under 64 MB `RLIMIT_AS`, gate off, exit 0 | `owned_alloc_dispose_loop_stays_within_memory_bound` (P4) |
| 2 | Forgetting to dispose is a compile error | `unconsumed_owned_is_error` (P3) |
| 3 | `dup` of `^i64` (Copy payload, so this proves the cell itself is linear) errors naming `^i64` | `dup_of_owned_is_error` (P3) |
| 3b | `over` of `^i64` is a compile error | `over_of_owned_is_error` (P3) |
| 4 | Use-after-move of a cell local errors, naming the move site | `use_after_move_of_owned_is_error` (P3) |
| 5 | Unwrap returns the payload; one `alloc`, one `free` | `owned_unwrap_returns_payload_and_frees_once` (P3) |
| 5b | Unwrap of an aggregate: field read after the free is correct (R13) | `owned_unwrap_aggregate_copies_out_before_free` (P3) |
| 6 | Peek twice then dispose: values correct and equal, exactly one `free` | `peek_owned_copy_payload_keeps_cell_live` (P4) |
| 7 | Peek of a linear payload is a compile error | `peek_owned_linear_payload_is_error` (P3) |
| 8 | `^__spy` disposal: `free 8` then `drop 7`, one stream so the order is real (reversed in Phase 3 Slice 3, R8) | `owned_linear_payload_frees_before_dropping_payload` (P4) |
| 9 | Struct containing a cell is linear: `dup` errors naming the struct | `struct_containing_owned_is_linear` (P3) |
| 9b | Dropping that struct frees the cell | `dropping_struct_with_owned_frees_cell` (P4) |
| 10 | Enum variant carrying a cell behind an `if`: cell-carrying variant frees once, other variant zero times | `enum_variant_with_owned_frees_on_drop` (P4) |
| 11 | `^^[u8 24]`: distinct sizes (24/8) prove outer freed before inner, which equal sizes could not (reversed in Phase 3 Slice 3, R8) | `nested_owned_frees_outer_before_inner` (P4) |
| 12 | Zero-sized payload traces `alloc 1`/`free 1`, witnessing `max(size,1)` (glibc's `malloc(0)` is non-NULL, so a count-only test would pass with the adjustment deleted) | `owned_zero_sized_payload_allocs_one_byte` (P4) |
| 13 | `^[u8 N]` construct/peek/`get`/`drop`, one pair; then peek an aggregate, dispose, read the copy (R14 non-aliasing) | `owned_byte_buffer_peek_get_and_free_once` + `peek_aggregate_does_not_alias_cell` (P4) |
| 14 | Emitted IL contains the NULL check and `exit(1)` trap call | `emitted_alloc_shim_has_null_trap` (P2, emitter-level) |
| 15 | REPL `:quit` frees a residual cell (asserts the `<^T>` placeholder, not a value) | `repl_quit_frees_residual_owned` (P5) |
| 16 | `[^i64 4]`, `^[__spy 2]`, `^[^i64 4]` are compile errors (R16) | `array_of_owned_is_error` + `owned_of_linear_array_is_error` (P1) |
| 17 | No regression: all 14 examples byte-identical | existing goldens + example sweep (P4/P5) |
| 18 | Gate off by default emits only the program's own output | `alloc_trace_is_silent_when_unset` (P3) |
| 19 | `^` reserved at type name, word name, local (R12a) | `reserved_caret_type_name_is_error` + `..._word_name_...` + `..._local_...` (P1) |
| 20 | Lexer claims: `^\|>` one token, `^^i64` one token, `^[u8 4]` splits at the bracket | lexer unit asserts (P1) |
| 21 | `^>x` / `^\|>x` give the ordinary unknown-word error | `caret_field_suffix_is_unknown_word` (P5); exact-name matching only, R12b's ordering half unobservable |

## Why criterion 14 is not a runtime golden

The `ulimit -v` probe is unsound, not merely awkward: a limit low enough to make a small `malloc` return NULL usually makes `execve`/`ld.so` fail first, so the sentinel never prints; a limit high enough to exec cleanly leaves glibc's arena mapped so the allocation succeeds; and the failure can arrive as a signal, panicking the harness's `.code()` unwrap. Runtime OOM behaviour is revisited in Slice 3, where optional pointers give a failed allocation somewhere to go. `RLIMIT_AS` stays sound for criterion 1b, where 64 MB clears startup and only a genuine leak crosses it.

## Honest cost note

`^[u8 1024]` is a fixed-capacity heap buffer, but the payload transits the data stack both ways, so construction copies the buffer once onto the stack and once into the cell. Correct, not free. A genuinely cheap large buffer wants runtime-sized allocation: Phase 6's `alloc` layer.

`getenv` runs per allocation and per free (R10), not just on a test path: the gate check is on the permanent allocator path in release builds too. Accepted deliberately, since caching would need a mutable global with no precedent in the emitter; recorded here so it is not rediscovered as a surprise later.

A runtime OOM golden (criterion 14's Slice 3 revisit) needs a deterministic way to make `malloc` return NULL. `ulimit -v`/`RLIMIT_AS` fails during `ld.so` startup before `main` runs (see above), so it cannot express "fail this one small allocation". The known-good technique is interposing `malloc` via `LD_PRELOAD` to return NULL for small sizes: deterministic, and it needs no resource limits. Not implemented in this slice; recorded so the Slice 3 revisit starts from a working approach.
