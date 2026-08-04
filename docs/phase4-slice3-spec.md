# Phase 4 Slice 3: loop-carried aggregate aliasing and the back-edge copy

**Status: implemented** (base `main` @ `728a335`). A lowering-shape fix in `src/ir.rs` to a live silent-miscompile bug. No new syntax, IR instruction, backend surface, or checker change.

## The bug

An aggregate carried across a self-tail-call back-edge aliases storage the next iteration overwrites. The value flowing through a loop phi for an aggregate slot is a **pointer into aggregate storage**, not bytes; when that storage is reused across iterations, the carried value silently becomes the new contents, with no diagnostic. The trigger is an ordering one: the old value must be read *after* the new one is produced.

The cause is **storage reuse across iterations**, not the aggregate-return ABI specifically (ROADMAP's original framing). Two independent sources trigger it:
- **By-value aggregate return** (common instance, why urgent): QBE materialises a call's aggregate result into one stack slot per call site, never versioned; a back-edge arg built from it is an interior pointer into that slot. Slice 1 made this ubiquitous.
- **Inline-constructed aggregate, no call**: the constructor's storage is entry-hoisted by `push_alloc` while looping (one slot reused every iteration), reproducing the identical miscompile call-free.

Verified repros against `728a335` (got → want): struct `0 2 1 1`→`0 3 2 1`; array `0 2 1`→`0 3 2`; enum `0 2 1 1`→`0 3 2 1`; destructor `1000 1002 1001 1001`→`1000 1003 1002 1001` (double-close + leak for a real `Res`); nested projection `99 0 2 1`→`99 0 3 2`; inline constructor `0 2 1 1`→`0 3 2 1`. The two-aggregate swap (`b a`) is already correct (`1 2 1 2 2`) and is the regression guard a plausible fix breaks.

## Mechanism (confirmed against source)

`lower_word_parts` (`src/ir.rs:1978`) calls `begin_loop` (`:2259`) when `has_self_tail_call` (`src/check.rs:2130`) holds. `begin_loop` seals entry with a `Jmp` to a header and emits one `Instr::Phi` per carried slot, seeded `(entry, param)`. A tail self-call (`:2588`) records `(pred, args)` in `back_edges` (`:2114`); `finalize_loop` (`:2280`) back-patches each header phi one arm per back-edge. `field_value` (`:3214`) hands back an interior pointer (`Instr::PtrOffset` via `field_aggregate_value`, `:3193`), never a copy.

The existing loop-body alloc hoisting is the lever: `push_alloc` (`:2221`) routes any `Alloc` emitted while looping into the entry block (`alloc_struct`/`alloc_enum`/`alloc_array`, `:2768`/`:2781`/`:2794` all go through it), so a stable per-slot copy allocated once at `begin_loop` lands there.

**Why the obvious fix is dead:** copying the call result into a fresh slot at the call site fails both placements. Entry-hoisted it is a single slot again (bug returns); left in the body it is a per-iteration `alloc` (QBE emits inline, no hoisting) that breaks the constant-stack guarantee `each`/`fold` exist to demonstrate. The copy must land **on the back-edge**, into a slot allocated once in the entry block.

## Decisions (D1–D6)

- **D1 — broad rule:** every aggregate-typed carried slot gets a stable slot (the set the loop already carries, `stack[base..]`, recon 5). The narrow "only those that can alias a loop-produced call result" rule needs a nonexistent aliasing analysis. Cost: at most two blits per carried aggregate per iteration; a blit is runtime, never stack (entry-hoisted). Checkable by inspection.
- **D2 — cycle breaking via unconditional read-before-write staging.** The back-edge is a parallel assignment `stable[i] <- arg[i]`. An `arg[i]` may *be* another stable slot (swap) or be an **interior pointer into** one (`field_value` projection, a distinct `Value`), so a pointer-identity hazard test is wrong (misses criterion 7). Partition each back-edge's aggregate slots:
  - **forwarded-in-place** (`arg[i]` **is** exactly `stable[i]`): emit nothing.
  - **staged** (else): read phase `temp[i] <- arg[i]`; write phase `stable[i] <- temp[i]`.
  All reads precede all writes, so every slot an arg reads is copied out before any store. No provenance/aliasing analysis (the accurate alternative needs a transitive `PtrOffset`-chain walk over a def map `FuncBuilder` lacks, `:2082`). Over-stages the genuinely-fresh arg by one blit; immunity by inspection is worth it. Elision is by exact `Value` identity only.
- **D3 — no aggregate header phi.** An aggregate slot's phi would be degenerate (same stable-slot pointer from every pred). `begin_loop` emits no `Instr::Phi` for it; the body reads the stable-slot pointer directly (entry block dominates header/body). Scalar phis unchanged.
- **D4 — entry-arm init blit.** `begin_loop` blits the incoming param into the stable slot once in the entry block, so iteration 1 reads an initialised value. Own requirement (R3) because every repro survives iteration 1.
- **D5 — the back-edge blit is a move.** Source is dead after the edge, guaranteed by `check_linear_across_back_edge` (`src/check.rs:4062`). No second live copy; the exactly-once checker is untouched. Changes *which bytes* a later disposal sees (criterion 4).
- **D6 — scalars/references unchanged.** Scalars keep their phi. A frame-local reference across the edge is already a hard error (`check_reference_across_back_edge`, `src/check.rs:4041`); a reference into an ancestor frame is legal and unaffected.

## Requirements (all in lowering, `src/ir.rs`; no new diagnostics)

- **R1** — stable frame slot per carried aggregate via the matching `alloc_*` (entry-hoisted through `push_alloc`); entry_value is the stable-slot pointer. Set = loop's own carried slots; per-slot `value_type` classification only.
- **R1a** — **gate the transform to the user self-tail-call loop** via an explicit `begin_loop` param (e.g. `stage_aggregates: bool`). `begin_loop` has three call sites: `lower_word_parts` (on); `synthesize_struct_destructor` (`:1519`/`:1521`) and `synthesize_enum_destructor` (`:1595`/`:1597`) both off, keeping their fused-loop lowering byte-for-byte. Their cursor is always aggregate, so ungated R1–R4 would fire redundantly (the destructor loop is correct today by its own read-then-overwrite ordering). Do **not** key the gate on the incidental empty `cur_word_name`. Pinned structurally by R10.
- **R2** — no `Instr::Phi` for an aggregate carried slot; per-slot metadata records scalar (phi `Value`) vs aggregate (stable-slot `Value`, size/align, temp `Value`), indexed by **full** carried-slot position (`back_edges` split_off `:2587`, `vals[slot]` `:2293` — don't compact to scalar-only). Scoped to the **loop header only**: ordinary join phis over aggregates (`lower_if` merge `:3597`–`3610`, `lower_clauses` join `:3740`–`3743`) are unchanged (criterion 11).
- **R3** — entry-arm init blit `Blit(param, stable, size)` into the entry block via `push_alloc`. Zero-size aggregate: no blit (`size > 0` guard).
- **R4** — `finalize_loop` back-edge move blits per aggregate slot: forwarded-in-place → nothing; staged → read-phase `Blit(args[i], temp[i])` then write-phase `Blit(temp[i], stable[i])`, all reads before all writes. Appended to the **predecessor block's** instrs (before its stored `Jmp` terminator). Temps entry-hoisted, one per slot, reused. Needs a **collect-then-mutate two-pass shape** (header borrow `:2284` vs predecessor appends can't be held under one borrow). Scalar phi back-patch unchanged.
- **R4a** — correct the `entry_block` doc comment (`:2116`, the inline-constructor hazard it named as "future" already exists; after R4 hoisting no longer depends on body read order) and `push_alloc`'s doc comment (`:2221`, now routes a `Blit` as well as an `Alloc`). Verified by diff review.
- **R5** — the blit is a move (consequence of R15's crossing-set guarantee); no drop glue added/removed. Behavioural witness: criterion 4.
- **R6** — scalars/references untouched. **Two sanctioned unit-test edits** (their `Flag` tag-only enum slot loses its header phi): `clause_tails_share_one_header` (`phis.len()` 2→1; arm-count `all()` still 3, no edit) and `mixed_clause_header_and_join_predecessors_stay_disjoint` (`hphis.len()` 2→1; arm-count `all()` still 2, no edit).
- **R7** — uniform over `Struct`/`Enum`/`Array` (criteria 1–3).
- **R8** — only `Alloc`/`Blit` (extant); `qbe.rs` and `check.rs` untouched.
- **R9** — every introduced `Alloc` entry-hoisted; frame grows a fixed number of slots (criterion 6).
- **R10** — unit tests beside the loop tests (`lower_src`/`header_phis`/`loop_header`, asserting on IR structure not IL text): no aggregate header phi but a scalar one; stable `Alloc` + temp in entry block; init `Blit` in entry block; read-phase blits before write-phase (a blit is write-phase when its source is an earlier blit's destination in the same pred block) and zero blits for forwarded-in-place; and the **R1a structural test** — a recursive type's synthesized destructor (`sooth_enum_drop_0` in `lower_src` output, precedent `:6332`/`:6363`) keeps its one header phi and gains no entry-block `Blit`, red when ungated (criterion 10 is not).

**Load-bearing assumption:** a base case returning the carried aggregate returns a pointer into this frame's stable slot; safe only because aggregate return lowers to `ret %ptr` under `:S`/`:E`/`:A` (`qbe.rs:1111`, `qbe_abi_ty :264`), copied out by value at the boundary. Already relied on today.

## Success criteria (goldens in `tests/phase4_generics.rs`; constant-stack uses a file-local `ulimit -s`-bounded signal-aware runner mirroring `run_stack_bounded_golden`, `tests/phase0.rs:2713`)

| # | criterion | golden | phase |
|---|---|---|---|
| 1 | struct prints `0 3 2 1` | `struct_carried_across_back_edge_is_not_aliased` | 2 |
| 2 | array `[i64 4]` prints `0 3 2` | `array_carried_across_back_edge_is_not_aliased` | 2 |
| 3 | enum prints `0 3 2 1` | `enum_carried_across_back_edge_is_not_aliased` | 2 |
| 4 | destructor prints `1000 1003 1002 1001` (right contents, resource-safety bar) | `destructor_carried_across_back_edge_disposes_right_contents` | 2 |
| 5 | swap prints `1 2 1 2 2` (D2 regression guard) | `two_aggregates_swapped_across_back_edge_stay_correct` | 1 |
| 6 | re-producing loop, 1M iters under 1 MB stack, exits 0 | `aggregate_carried_loop_runs_in_constant_stack` | 1 |
| 7 | nested projection prints `99 0 3 2` (interior-pointer staging) | `nested_projection_carried_across_back_edge_is_not_aliased` | 2 |
| 8 | inline constructor prints `0 3 2 1` (storage-reuse witness) | `inline_constructed_aggregate_carried_across_back_edge_is_not_aliased` | 2 |
| 9 | non-zero-seeded forwarded aggregate prints `42` (catches skipped R3) | `forwarded_aggregate_reads_its_seeded_value` | 1 |
| 10 | `List` of `Res` drops in order, prints `5001 5002 5003` (disposal regression guard; R1a witness is R10's structural test) | `recursive_type_destructor_disposes_right_contents` | 1 |
| 11 | if/else re-produce-vs-forward join prints `3` (join phi survives) | `join_phi_over_carried_aggregate_survives` | 1 |

Enum golden (3) kept despite recon-2 by-construction claim: converts it to a checked fact ("uniform over the three or wrong for two"). Interior-pointer hazard's three surface forms (getter `:3374`, destructure `:3410`, enum-payload `:3698`) all funnel through one `field_value`→`PtrOffset` producer, so the getter (criterion 7) is the witness by test and the others are covered by construction. Confirmed non-producers: read-through-reference `@` copies via `alloc_aggregate` + `Blit` (`:2717`); array access is reference-mediated (`ElemAddr :2840`).

## Out of scope

Quotations/combinators (slices 4–5); non-tail and mutual recursion (real frames); references across the back-edge (already rejected); the fused destructor loops (`emit_path_steps` back-edge, gated off by R1a, byte-for-byte); call ABI; any aliasing/provenance may-alias analysis; SCC cycle detection; runtime blit-count optimisation (slices 4–5); any new diagnostic.

## Delivery (as implemented)

- **Phase 1** — regression guards green on `728a335` (criteria 5, 6, 9, 10, 11) + the bounded runner. `0cf60d3` (`tests/phase4_generics.rs`).
- **Phase 2** — the lowering fix (R1–R10) in `begin_loop`/`finalize_loop` + `FuncBuilder` carried-slot metadata; sanctioned unit-test updates; fix-witness goldens (criteria 1–4, 7, 8). The three aggregate-carrying constant-stack goldens in `tests/phase0.rs` (Parity/Step enums, `[Op 2]`+`Op`) lose their header phi and stay green. `eeb4367` (`src/ir.rs`, `tests/phase4_generics.rs`), review-fix `316578f` (`src/check.rs`).
- **Phase 3** — docs: removed the known-issue text from ROADMAP.md's slice 3 entry (marked fixed, corrected causal framing to storage-reuse), recorded D1–D6 in DESIGN.md. `5541319` (`ROADMAP.md`), `ebbd770` (`DESIGN.md`).

## Non-functional

Green (`cargo fmt --check && cargo clippy -- -D warnings && cargo test`) unchanged. No new `Instr`/`Terminator`; no backend or checker change; backend stays QBE with `Ptr` opaque; `core` stays `no_std`; constant stack preserved (every introduced `Alloc` entry-hoisted).
