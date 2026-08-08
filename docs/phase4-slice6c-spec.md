# Phase 4 Slice 6c: quotation-taking words at the REPL

**Status: implemented** (phases 1–3: `bff48cd`, `14f0645`, `be61e72`). Base `main` @ `7032670`.
Depends on 6a (inliner: `is_combinator`/`collect_combinators`/`inline_combinator`) and 6b
(`filter`/`while`, self-tail splice). Decisions D1–D5 locked (`docs/phase4-slice6c-brief.md`).
Lifted the two REPL rejections that stood: R23 (defining a quotation-taking word at a session
line) and R24 (importing a closure that exports one).

**Load-bearing finding:** a combinator has no compile event of its own to freeze against. It
mints no `IrFunc` and no symbol; it is inlined by term-splice, fresh, at every call site. The
slice-2 frozen-resolver precedent does not transfer (a poly body is checked once, a combinator
body is re-checked/re-lowered at every splice site against that site's live env). So the work is
**plumbing**: a session-level store the checker's and lowerer's inline paths read, threaded into
call sites that hardcoded an empty combinators map, plus symmetric eviction across three
name-shape stores.

## Locked decisions

- **D1.** No frozen resolver/env/generation/symbol. Store is `HashMap<String, WordDef>`,
  replaced wholesale on redefinition; every later splice uses that site's own live env, as native.
- **D2.** One store, shared by mono and poly combinators.
- **D3.** Defining a combinator skips lowering entirely: check, store, no `.so`, no symbol, no `dlopen`.
- **D4.** The three name-shape stores (`self.env`, `self.poly_words`, new combinators store) are
  mutually exclusive on redefinition (generalizing R8).
- **D5.** Import reuses the same store, populated from the closure's module-0 exports.

## Mechanism

Reuse, not invention. The checker routes a call to `poly.combinators.get(name)` before poly/env
lookups (`src/check.rs:6147`) and inlines it (`inline_combinator`, with the 6b self-tail branch).
Lowering splices the body in `lower_call`'s `_ =>` arm (`src/ir.rs:2978`), self-tail as a
back-edge. Native derives the maps these paths read from `module.words`
(`collect_combinators` → `HashMap<String, Combinator>`; `combinator_bodies` →
`HashMap<String, Vec<Term>>`). 6c gives the REPL a session-level store and threads a **view** into
those entry points; the session accumulates across lines/imports, native rebuilds per module. The
inline mechanism itself is unchanged.

## Requirements (all implemented)

### Session store (`src/repl.rs`)
- **R1.** `Session.combinators: HashMap<String, WordDef>` (empty in `new`), mono+poly in one
  store. Key = the name the checker dispatches on: plain word name for session-defined;
  `{q}::{raw}__import{epoch}` for imported (R13). No generation/epoch/symbol; redefinition
  replaces wholesale.
- **R2.** Projected on demand into the two shapes the inline paths speak (no new type): a
  `HashMap<String, Combinator>` for the checker, a `HashMap<String, Vec<Term>>` for lowering.
  Required `check::Combinator` / its construction visible to `repl.rs`.

### Threading, checker side (`src/check.rs`, `src/repl.rs`)
- **R3.** `check_def`, `check_def_collecting_drop_sites`, `infer_line` grow a
  `combinators: &HashMap<String, Combinator>` param, threaded into `PolyCtx`, replacing local
  `no_combinators`. Non-REPL/build callers and unit tests pass an empty map (concrete path
  byte-identical).
- **R4.** Every REPL checker call site passes the session view: mono define
  (`eval_def`→`check_def`), drop-overload collector
  (`compile_drop_overload`→`check_def_collecting_drop_sites`), bare lines
  (`run_terms`/`check_type_line`→`infer_line`).

### Threading, IR side (`src/ir.rs`, `src/repl.rs`)
- **R5.** `lower_word`, `lower_instantiation`, `lower_line` grow a
  `combinators: &HashMap<String, Vec<Term>>` param set onto `FuncBuilder`. **`lower_line`
  (`src/ir.rs:1736`) was the under-counted fifth site**; without it a bare line calling a
  combinator misses dispatch and link-fails on an unminted symbol. REPL entry points pass the
  view: `eval_def`→`lower_word`, `emit_instantiations`→`lower_instantiation`,
  `run_terms`→`lower_line`. Native `ir::lower` untouched.

### Defining a combinator at the REPL (D3)
- **R6.** Replaced R23. In `eval_def`, after `audit_word_quotation_positions`, a word for which
  `word_declares_quotation_parameter` holds routes to `eval_combinator_def`.
- **R7.** `eval_combinator_def` builds the checker view **including the definee itself** (prior
  same-name entry replaced), mirroring native's `collect_combinators`; a self/self-tail call
  dispatches through the inline path.
- **R8.** Before storing, runs `check_combinator_cycles` over that view, so a cross-line cycle is
  the located `combinator_cycle_error` while a self-*tail* edge stays permitted. `pub(crate)`.
- **R9.** Body check branches on shape, stores into the one store: mono via `check_def`; poly via
  `check_poly_combinator_standalone` (**not** `eval_poly_def`), **bypassing the `>= 2`-outputs
  deferral** (a combinator is spliced inline, never lowered to a bundle-returning `IrFunc`, so
  the return-bundle limit cannot arise). `pub(crate)`.
- **R10.** On success, insert `WordDef` into `self.combinators`; **no** lowering/emit/`compile_so`/
  `dlopen`/symbol/generation. Prints the same `defined {name}`. A rejected def leaves the session
  untouched via `eval_line`'s existing snapshot-and-truncate rollback.

### Mutual exclusion on redefinition (D4)
- **R11.** Combinator dispatch runs first, so a stale wrong-store entry would silently win.
  Redefining `name`: `eval_combinator_def` evicts `self.env` and `self.poly_words`; `eval_def`
  mono commit adds `self.combinators.remove(&name)`; `eval_poly_def` commit adds
  `self.combinators.remove(&name)`. No `arrays`/`owned_cells`/`refs` purge (rows positionally
  stable, inert); eviction comment states this.

### Import (D5)
- **R12.** Deleted `check_no_exported_quotation_word_in_closure` and its call in `eval_import`.
  Only a **module-0 exported** combinator needs retention (internal-only ones inline with the
  closure).
- **R13.** In `splice_import` (infallible commit), for each module-0 export `W` with
  `word_declares_quotation_parameter(W)`: insert a remapped, name-rewritten copy into
  `self.combinators` under `{q}::{raw}__import{epoch}`, add `import_aliases[{q}::{raw}] = internal`
  (plus bare alias for selective import). Symmetric to the exported-ordinary-word loop (which
  filters `w.poly.is_none()` and so never sees `filter`/`while`). No re-check.
- **R14.** Stored copy remapped like every imported declaration (positional-id shift via reused
  `remap`); `.name` set to internal spelling; body calls to any module-0 combinator (**itself
  included**) or exported ordinary word rewritten to internal spellings. The **body self-call
  rewrite is load-bearing**: `while`'s self-call must become `{q}::while__import{epoch}` or
  `body_tail_calls_self` misses and the splice recurses forever. For `filter`/`while` the id
  remap is a no-op, so the realistic case exercises the call-name rewrite.
- **R15.** Two sub-questions confirmed, no new machinery: an exported combinator whose signature
  names a private type is already rejected at the closure's own `check` (golden, no REPL guard);
  an exported combinator whose body calls a private word is out of scope (no stopgap guard).

### Invariants preserved
- **R16.** No frozen resolver/env/generation/epoch/symbol (D1). Falsifiable pin required
  (criterion 4).
- **R17.** QBE only; `Ptr[T]` opaque; no LLVM/native/JIT/comptime; `IrType` gains no quotation
  variant; no runtime quotation value/convention/identity; `core` stays `no_std`. A program using
  no REPL combinator lowers byte-for-byte as today (empty-map default).
- **R18.** Drive-by: fixed the stale `collect_combinators` doc comment (`src/check.rs:5011`)
  claiming poly combinators are "excluded here". Comment only.
- **R19.** The three 6b rejection goldens converted to acceptance/behavior pins (define + call),
  not deleted, so the guarded behavior flips rather than vanishes.
- **R20.** ROADMAP 6c marked implemented; DESIGN.md REPL section records that a combinator is
  retained as raw terms (no symbol) and re-spliced per session call site under that site's own
  live env, and why the slice-2 frozen-resolver precedent does not apply.

## Exit criteria (goldens in `tests/phase4_combinators.rs`)

| # | criterion | test | phase |
|---|-----------|------|-------|
| 1 | mono combinator defined, called from a later bare line; inlines and runs | `repl_mono_combinator_define_and_call` | 1 |
| 2 | define `while`; `0 [ dup 5 < if 1 + true else false end ] while .` prints `5` via loop back-edge (constant stack) | `repl_while_define_runs_to_fixpoint` | 1 |
| 3 | two-output poly combinator (`filter` shape) defined and called; both outputs land (>=2-output gate bypassed) | `repl_two_output_combinator_define_and_call` | 1 |
| 4 | **D1 falsifiable:** body calls a helper; redefine helper; new-line splice sees the *new* helper | `repl_combinator_splice_sees_current_helper` | 1 |
| 5 | ordinary word with a spliced combinator keeps its baked result across a later combinator redefinition | `repl_ordinary_caller_frozen_across_combinator_redefinition` | 1 |
| 6 | redefining `foo` combinator↔ordinary rebinds dispatch; other store evicted (D4) | `repl_redefining_combinator_shape_evicts_other_stores` | 1 |
| 7 | cross-line combinator cycle is the located `combinator_cycle_error` | `repl_cross_line_combinator_cycle_is_error` | 1 |
| 8 | import `lib/combinators.sth`, run `while` to fixpoint; self-call rewrite holds, constant stack | `repl_imported_while_runs_to_fixpoint` | 2 |
| 9 | import `lib/combinators.sth`, run `filter` over an array; compacts, leaves both outputs | `repl_imported_filter_runs` | 2 |
| 10 | import a combinator whose signature names a private type is rejected at the closure's check | `repl_import_combinator_with_private_type_in_signature_is_rejected` | 2 |
| 11 | R19: the three former-rejection goldens now assert define-and-call acceptance | `repl_quotation_taking_definition_is_rejected` et al. | 1 |
| 12 | dogfood: import + `filter`/`while` session transcript matches native example output | `repl_combinators_dogfood_matches_native` | 3 |

**Load-bearing guards (mutation-tested):**
- Crit 2 fails if R5's `lower_line` threading is reverted (link-fail or infinite splice).
- Crit 3 fails if R9 routes a poly combinator through `eval_poly_def`.
- Crit 4 fails if any frozen-resolver/env capture is added (D1).
- Crit 6 fails if R11's `self.combinators.remove` is dropped from a redefinition path.
- Crit 7 fails if `check_combinator_cycles` is not run at the REPL.
- Crit 8 fails if R14's body self-call rewrite is deleted.

## Sanctioned edits (as implemented)

`src/repl.rs`: `Session.combinators`, `eval_combinator_def`, R6 route, R11 three-path eviction,
R12 deletion + R13/R14 retention, checker/IR view builders. `src/check.rs`: `pub(crate)` on
`Combinator`/`check_combinator_cycles`/`check_poly_combinator_standalone`; R3 params; R18
doc-comment fix. `src/ir.rs`: R5 params on `lower_word`/`lower_instantiation`/`lower_line`.
`tests/phase4_combinators.rs`: goldens + R19 conversions. `ROADMAP.md`/`DESIGN.md` (R20). No
change to the inline mechanism, `Instr`/`Terminator`/`qbe.rs`, or any non-REPL behavior.

## Implementation notes

- **Phase 1** (`bff48cd`): session store + view threading through check and IR (incl. the fifth
  `lower_line` site), `eval_combinator_def` for mono and poly (>=2-output gate bypassed), D4
  three-way eviction, and the define/call/redefine/cycle/D1 goldens incl. R19 conversions.
  Touched `src/backend/qbe.rs`, `src/check.rs`, `src/ir.rs`, `src/repl.rs`,
  `tests/phase4_combinators.rs`.
- **Phase 2** (`14f0645`): import retention (D5) — dropped R24, stored id-remapped and
  body-call-name-rewritten module-0 exported combinators with aliases; import goldens incl. the
  imported-`while` self-call-rewrite witness and the private-type-in-signature confirmation.
- **Phase 3** (`be61e72`): out-of-scope guards, the `collect_combinators` doc-comment drive-by,
  the import/session dogfood parity golden, and ROADMAP/DESIGN docs.
