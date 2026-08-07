---

# Phase 4 Slice 6c: quotation-taking words at the REPL

**Status: spec.** Base `main` @ `7032670`. Depends on 6a (the inliner, `is_combinator`/
`collect_combinators`/`inline_combinator`) and 6b (`filter`/`while`, the self-tail splice).
Decisions D1–D5 locked (from `docs/phase4-slice6c-brief.md`). This slice lifts the two
REPL rejections that stand today: R23 (defining a quotation-taking word at a session line,
`src/repl.rs:~2072`) and R24 (importing a closure that exports one,
`check_no_exported_quotation_word_in_closure`, `src/repl.rs:239`).

The brief's load-bearing finding (recon 3–7): a combinator has **no compile event of its
own to freeze against**. It mints no `IrFunc` and no symbol (R20); it is inlined by
term-splice, fresh, at every call site. Slice 2's precedent (a polymorphic word's frozen
defining-line resolver, read once per instantiation at *lowering*) does not transfer,
because a poly body is checked once and never re-checked, whereas a combinator body is
re-checked and re-lowered at every splice site against **that site's own live env**
(`inline_combinator`, `src/check.rs:5136`; the IR splice arm, `src/ir.rs:2978`). So the
work is not "invent a freezing mechanism," it is **plumbing**: a session-level store the
checker's and lowerer's inline paths read, threaded into the call sites that hardcode an
empty combinators map today, plus one cross-cutting decision (three now-mutually-exclusive
name-shape stores need symmetric eviction).

## Locked decisions (from the brief)

- **D1.** No frozen resolver / env / generation / symbol for combinator retention. The
  store is a plain `HashMap<String, WordDef>`, replaced wholesale on redefinition. Every
  splice at every later call site uses that site's own live env/resolver, as native does.
- **D2.** One store, shared by monomorphic and polymorphic combinators
  (`collect_combinators`/`inline_combinator` already treat both uniformly).
- **D3.** Defining a combinator skips lowering entirely: check, then store, no `.so`, no
  symbol, no `dlopen`.
- **D4.** The three name-shape stores (`self.env`, `self.poly_words`, the new combinators
  store) are made mutually exclusive on redefinition, generalizing R8.
- **D5.** Import reuses the same store, populated from the closure's module-0 exports, not
  a separate mechanism.

## Mechanism

Reuse, not invention. The checker already routes a call to `poly.combinators.get(name)`
**before** the poly and ordinary env lookups (`src/check.rs:6147`) and inlines it
(`inline_combinator`, including the 6b self-tail branch). Lowering already splices a
combinator body in `lower_call`'s `_ =>` arm (`src/ir.rs:2978`), self-tail lowered to a
back-edge (`lower_self_tail_combinator`). Native derives the maps these paths read once,
from `module.words`: the checker's `collect_combinators` (`src/check.rs:5018`, returning
`HashMap<String, Combinator>`) and the lowerer's `combinator_bodies`
(`HashMap<String, Vec<Term>>`, built in `ir::lower`, `src/ir.rs:~1101`). Both are handed
empty at every REPL entry point today.

6c gives the REPL a session-level store and threads a **view** of it into those entry
points, so a REPL line's inline paths see exactly what a native module's do, per splice
occurrence. The session accumulates the store across lines and imports; native rebuilds it
per module. Nothing about the inline mechanism itself changes.

## Requirements by stage

### The session store (`src/repl.rs`)

- **R1.** `Session` gains `combinators: HashMap<String, WordDef>` (initialized empty in
  `Session::new`). It owns the retained combinator definitions, mono and poly, in one store
  (D2). The key is the name the checker dispatches on: the plain word name for a
  session-defined combinator; the import-internal spelling `{q}::{raw}__import{epoch}` for
  an imported one (R13). It carries no generation, epoch, or symbol (D1); a redefinition
  replaces the entry wholesale.
- **R2.** The store is projected on demand into the two shapes the inline paths already
  speak, inventing **no new type** (open-question answer): a `HashMap<String, Combinator>`
  for the checker (each value borrowing a stored `WordDef`, matching `collect_combinators`'s
  return shape) and a `HashMap<String, Vec<Term>>` for lowering (matching `ir::lower`'s
  `combinator_bodies`). This requires `check::Combinator` and its construction (or a
  `pub(crate)` helper returning one for a `&WordDef`) to be visible to `repl.rs`.

### Threading, checker side (`src/check.rs`, `src/repl.rs`)

- **R3.** `check_def`, `check_def_collecting_drop_sites`, and `infer_line` grow a
  `combinators: &HashMap<String, Combinator>` parameter, threaded into their `PolyCtx`,
  replacing the locally-built `no_combinators` empty maps (recon 8, `src/check.rs:~2400`,
  `~2466`). Non-REPL / build-path callers and unit tests pass an empty map, keeping the
  concrete path byte-identical.
- **R4.** Every REPL checker call site passes the session combinators view (R2): the
  ordinary-word define (`eval_def` mono path → `check_def`), the drop-overload collector
  (`compile_drop_overload` → `check_def_collecting_drop_sites`), and every bare line
  (`run_terms` and `check_type_line` → `infer_line`). An ordinary word body or a bare line
  may then call a retained combinator and have it inlined, exactly as native inlines one
  drawn from `module.words`.

### Threading, IR side (`src/ir.rs`, `src/repl.rs`)

- **R5.** `ir::lower_word`, `ir::lower_instantiation`, and `ir::lower_line` grow a
  `combinators: &HashMap<String, Vec<Term>>` parameter, set onto the constructed
  `FuncBuilder`. Recon 8 names four hardcoded sites; **`ir::lower_line` (`src/ir.rs:1736`)
  is a fifth**, currently defaulting to `empty_combinators()` via `FuncBuilder::new`
  (`src/ir.rs:2320`). A bare REPL line that calls a combinator lowers through `lower_line`;
  without threading, `lower_call`'s combinator dispatch misses and falls through to an
  `Instr::Call` to a symbol that was never minted (a link failure). All REPL lowering
  entry points pass the session combinator-bodies view: `eval_def` → `lower_word`,
  `emit_instantiations` → `lower_instantiation`, `run_terms` → `lower_line`. Native
  `ir::lower` is untouched (it keeps building `combinator_bodies` from `module.words`).

### Defining a combinator at the REPL (D3)

- **R6.** Replace R23. In `eval_def`, after `audit_word_quotation_positions` (so a
  quotation type in a non-input position is still R7a-rejected), a word for which
  `word_declares_quotation_parameter` holds routes to a new `eval_combinator_def` instead
  of returning the "not yet supported at the REPL" error.
- **R7.** `eval_combinator_def` builds the checker combinators view **including the definee
  itself** (its new `WordDef`, with any prior same-name entry replaced), mirroring native's
  `collect_combinators(words)`, which contains the word being checked. A self-reference or
  self-tail call in the body then dispatches through the same inline path (6b R4/R6), not an
  unknown word.
- **R8.** Before storing, `eval_combinator_def` runs `check_combinator_cycles` over that
  view, so a cycle formed **across lines** (define `a`; define `b` calling `a`; redefine
  `a` calling `b`) is the same located `combinator_cycle_error`, while a self-*tail* edge
  stays permitted (6b D5). Requires `check_combinator_cycles` `pub(crate)`.
- **R9.** Body check, branching on shape but storing into the one store (D2):
  - a **monomorphic** combinator is checked by `check_def` (recon 9: `check_word` already
    handles a combinator identically to any word);
  - a **polymorphic** combinator is checked by `check_poly_combinator_standalone` (recon 9,
    mirroring native's `is_combinator` branch in `check`), **not** by `eval_poly_def`.
  The poly path must **bypass `eval_poly_def`'s `>= 2`-outputs deferral**: `filter`'s
  `( ['T: Copy 'N] [ 'T -- bool ] -- ['T 'N] usize )` resolves to two outputs, but a
  combinator is spliced inline and never lowered to a bundle-returning `IrFunc`, so the
  return-bundle limitation that gate guards cannot arise. Requires
  `check_poly_combinator_standalone` `pub(crate)`.
- **R10.** On a successful check, `eval_combinator_def` inserts the `WordDef` into
  `self.combinators` and performs **no lowering** (`ir::lower_word`), no
  `backend::qbe::emit`, no `driver::compile_so`, no `dlopen`, and mints no symbol or
  generation (D3). It prints the same `defined {name}` line an ordinary def prints. A
  rejected combinator def leaves the session untouched: `eval_line`'s existing
  snapshot-and-truncate of `arrays`/`owned_cells`/`refs` around `eval_expr_or_def_line`
  (`src/repl.rs:~1123`) already covers the interned-registry rollback (open-question 1),
  and no other session state is mutated before the fallible check completes.

### Mutual exclusion on redefinition (D4)

- **R11.** Generalize R8 to three stores. Combinator dispatch runs first
  (`poly.combinators.get(name)` precedes `poly.env`/`env`, `src/check.rs:6147`), so a stale
  entry in the wrong store would silently win. Redefining `name`:
  - `eval_combinator_def` evicts `self.env` and `self.poly_words`;
  - `eval_def`'s mono commit adds `self.combinators.remove(&name)` beside its existing
    `self.poly_words.remove(&name)`;
  - `eval_poly_def`'s commit adds `self.combinators.remove(&name)` beside its existing
    `self.env.remove(&name)`.
  No `arrays`/`owned_cells`/`refs` rows are purged on a combinator redefinition
  (open-question 2): those rows are positionally stable and never revisited (R9 elsewhere),
  so a stale row is inert, exactly as for an ordinary redefinition. State this in the
  eviction comment so a reviewer need not wonder.

### Import (D5)

- **R12.** Delete `check_no_exported_quotation_word_in_closure` and its call in
  `eval_import` (`src/repl.rs:1433`). An imported closure exporting a quotation-taking word
  is no longer rejected; `splice_import` retains it instead. Recon 2 already narrows the
  gap: a combinator used only *internally* to an imported closure inlines and compiles with
  the closure and needs nothing here; only a **module-0 exported** combinator, callable from
  a *later* session line, needs retention.
- **R13.** In `splice_import` (the infallible commit phase), for each module-0 export `W`
  with `word_declares_quotation_parameter(W)`: insert a remapped, name-rewritten copy of
  `W`'s `WordDef` into `self.combinators` under the internal spelling
  `{q}::{raw}__import{epoch}`, and add `import_aliases[{q}::{raw}] = internal` (plus the
  bare alias for a selectively-imported name). This is symmetric to the exported-ordinary-
  word loop, which today filters on `w.poly.is_none()` (`src/repl.rs:~1633`) and so never
  sees a poly combinator like `filter`/`while`; without the new alias, `rewrite_line_imports`
  would leave a session call to `{q}::filter` untranslated and it would fall to unknown-word.
  Recon 2/5/6: the closure is already internally self-consistent (checked under one shared
  env, `check::check` does not mutate bodies), so **no re-check** of the imported combinator
  is performed.
- **R14.** The stored copy is remapped exactly like every other imported declaration (R9's
  positional-id shift): type ids in its signature and body are shifted by the same
  `struct_base`/`array_base`/… `splice_import` already computes (reuse its `remap`). Its
  `.name` is set to the internal spelling, and its body's calls to any module-0 combinator
  (**itself included**) or exported ordinary word are rewritten to their internal
  spellings, so combinator dispatch and env lookup resolve at the session splice site. This
  body call-name rewrite is the load-bearing part (open-question answer: storing the raw
  post-check `WordDef` is **not** sufficient): `while`'s self-call `while` must become
  `{q}::while__import{epoch}`, or the self-tail recognizer (`body_tail_calls_self`, comparing
  against `comb.word.name`) misses and the splice recurses forever. For `filter`/`while` the
  type-id remap is a no-op (their bodies name only builtins), so the realistic import case
  exercises the call-name rewrite, not the id shift.
- **R15.** Two import sub-questions resolved by confirmation, not new machinery:
  - An exported combinator whose **signature** names a closure-private type is already
    rejected at the closure's own `check` (5a's export rule: a private type reachable
    through an exported signature). Confirm with a golden; add no REPL-side guard.
  - An exported combinator whose **body** calls a closure-*private* word is out of scope
    this slice. The library combinators (`filter`/`while`) call only builtins, their
    quotation parameter, and (for `while`) themselves; this shape is not manufactured here
    and needs no new machinery. Do not add a stopgap guard for it (phase-scope discipline).

### Out of scope and invariants preserved

- **R16.** No frozen resolver, frozen env, generation, epoch, or symbol for a combinator
  (D1). Every splice uses the call site's own live env/resolver. A falsifiable pin is
  required (criterion 4): the test that would fail if a future change silently added a
  frozen-resolver capture.
- **R17.** Backend stays QBE; `Ptr[T]` opaque; no LLVM, native backend, JIT, or comptime.
  `IrType` gains no quotation variant. A combinator keeps its compile-time-only nature: no
  runtime quotation value, no calling convention, no runtime identity (slice 7 untouched).
  `core` stays `no_std`. A program using no REPL combinator lowers byte-for-byte as today
  (the empty-map default preserves every non-REPL path).
- **R18.** Drive-by (D2): fix the stale doc comment on `collect_combinators`
  (`src/check.rs:5011`) claiming a polymorphic combinator "is excluded here" — `is_combinator`
  does not exclude poly combinators (`inline_combinator` branches on `word.poly` internally).
  Comment only, no behavior change.
- **R19.** The three 6b REPL rejection goldens
  (`repl_quotation_taking_definition_is_rejected`,
  `repl_poly_quotation_taking_definition_is_rejected`,
  `repl_self_tail_combinator_definition_is_rejected`, in `tests/phase4_combinators.rs`)
  assert exactly the rejection this slice removes. 6c converts them to acceptance / behavior
  pins (define + call), rather than deleting them, so the guarded behavior flips rather than
  vanishes.
- **R20.** ROADMAP 6c marked implemented; DESIGN.md's REPL section records that a combinator
  is retained as raw terms (no symbol) and re-spliced per session call site under that
  site's own live env (D1, and why the slice-2 frozen-resolver precedent does not apply).

## Exit criteria (goldens in `tests/phase4_combinators.rs` unless noted)

| # | criterion | test | kind | phase |
|---|-----------|------|------|-------|
| 1 | define a monomorphic combinator at a session line, call it from a *later bare line*; it inlines and runs | `repl_mono_combinator_define_and_call` | golden | 1 |
| 2 | define `while` at a session line; `0 [ dup 5 < if 1 + true else false end ] while .` prints `5`, lowering to a loop back-edge (constant stack), not an infinite splice | `repl_while_define_runs_to_fixpoint` | golden | 1 |
| 3 | define a two-output poly combinator (`filter` shape) and call it; both outputs land on the residual stack (the `>= 2`-outputs gate is bypassed for a combinator, R9) | `repl_two_output_combinator_define_and_call` | golden | 1 |
| 4 | **D1 falsifiable:** define a combinator whose body calls an ordinary helper; call it; redefine the helper; call the combinator from a *new* line; the new line's splice sees the *new* helper | `repl_combinator_splice_sees_current_helper` | golden | 1 |
| 5 | an ordinary word compiled with a combinator spliced into it keeps its baked result across a later redefinition of that combinator (R20 frozen-`.so`) | `repl_ordinary_caller_frozen_across_combinator_redefinition` | golden | 1 |
| 6 | redefining `foo` from combinator to ordinary word (and the reverse) rebinds dispatch to the new shape — the other store is evicted (D4) | `repl_redefining_combinator_shape_evicts_other_stores` | golden | 1 |
| 7 | a combinator cycle formed across session lines is the located `combinator_cycle_error` (R8) | `repl_cross_line_combinator_cycle_is_error` | golden | 1 |
| 8 | import `lib/combinators.sth`, run `while` at a session line to a fixpoint — the self-call rewrite (R14) holds and it runs in constant stack | `repl_imported_while_runs_to_fixpoint` | golden | 2 |
| 9 | import `lib/combinators.sth`, run `filter` over an array at a session line; it compacts and leaves both outputs | `repl_imported_filter_runs` | golden | 2 |
| 10 | import a closure exporting a combinator whose signature names a private type is rejected at the closure's own check (R15 confirm) | `repl_import_combinator_with_private_type_in_signature_is_rejected` | golden | 2 |
| 11 | R19: the three former-rejection goldens now assert define-and-call acceptance | `repl_quotation_taking_definition_is_rejected` (renamed/rewritten) et al. | golden | 1 |
| 12 | dogfood: a session transcript importing `lib/combinators.sth` and using `filter`/`while` matches the native example's output | `repl_combinators_dogfood_matches_native` | golden | 3 |

**Load-bearing guards (mutation-test each — the test must fail when the guarded behavior is
deleted, per the project's coverage convention):**

- **Criterion 2** fails if R5's `lower_line` combinator threading is reverted (a bare/`while`
  line link-fails or splices forever).
- **Criterion 3** fails if R9 routes a poly combinator through `eval_poly_def` (the two-output
  `filter` is wrongly deferred as "resolves to 2 outputs").
- **Criterion 4** fails if any frozen-resolver/env capture is added for a combinator (D1) — the
  new line would see the *old* helper.
- **Criterion 6** fails if R11's `self.combinators.remove` is dropped from a redefinition path
  (the stale combinator entry keeps winning dispatch).
- **Criterion 7** fails if `check_combinator_cycles` is not run at the REPL (R8) — the cycle
  type-checks then splices forever.
- **Criterion 8** fails if R14's body self-call rewrite is deleted — the imported `while`'s
  self-call misses the self-tail recognizer and the splice recurses forever.

## Sanctioned edits

`src/repl.rs`: new `Session.combinators` field, `eval_combinator_def`, the R6 route in
`eval_def`, R11 eviction in all three define paths, R12 deletion + R13/R14 retention in
`eval_import`/`splice_import`, and the new checker/IR view builders. `src/check.rs`:
`pub(crate)` on `Combinator`, `check_combinator_cycles`, `check_poly_combinator_standalone`;
the R3 parameter on `check_def`/`check_def_collecting_drop_sites`/`infer_line`; the R18
doc-comment fix. `src/ir.rs`: the R5 parameter on `lower_word`/`lower_instantiation`/
`lower_line`. `tests/phase4_combinators.rs` (goldens + the R19 conversions), any import
fixtures, an `examples/`/session dogfood. `ROADMAP.md` 6c marked implemented; DESIGN.md REPL
section (R20). No change to the inline mechanism itself (`inline_combinator`, the splice arm,
the self-tail back-edge); no `Instr`/`Terminator`/`qbe.rs` change; no behavior a non-REPL
program relies on.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Session combinator store; thread a combinators view into check_def/check_def_collecting_drop_sites/infer_line and lower_word/lower_instantiation/lower_line (the fifth, under-counted site); define-at-REPL for mono and poly combinators (eval_combinator_def, bypassing the >=2-output gate); D4 three-way eviction; and the define/call/redefine/cycle/D1-falsifiable goldens including the R19 rejection-to-acceptance conversions",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "Import retention (D5): drop the R24 rejection, store id-remapped and body-call-name-rewritten module-0 exported combinators in the same store with their aliases, and the import goldens including the imported-while self-call-rewrite witness and the private-type-in-signature confirmation",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "Out-of-scope guards, the stale collect_combinators doc-comment drive-by, the import/session dogfood parity golden, and ROADMAP/DESIGN documentation",
      "difficulty": "standard"
    }
  ]
}
```
