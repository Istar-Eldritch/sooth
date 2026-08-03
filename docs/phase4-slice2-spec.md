# Phase 4 Slice 2: REPL monomorphization

Base: `main` @ `d645162`. Slice 1 landed native type/row/length variables and monomorphization, native-only. This slice makes the REPL see polymorphic words: define one at the REPL, instantiate it at concrete types on later lines, redefine it, and dedup repeated same-type instantiations, with the diagnostics for all of it. REPL late binding on redefinition in general stays deferred (`DESIGN.md`); this slice keeps the existing frozen-binding rule every ordinary REPL word already follows.

## The problem removed

Native `check`/`lower` build a `poly_env` from `word.poly`, unify each call site (`check_poly_call`), and emit one monomorphized `IrFunc` per instantiation. The REPL shared none of this: `eval_def` routed poly defs through the concrete `check_def`/`check_word` path (never reads `word.poly`), and `infer_line` built an **empty** `poly_env` and discarded the instantiation table. A poly word leaves `word.effect` empty, so its body was checked against a zero-arity `Sig`, producing a **silent miscompile** (`note: declared ( -- )` mismatch, or a silent `defined` that entered `env` with an empty signature). Removing this silent state is the floor the slice ships first.

## Locked decisions

- **D1. Symbol identity carries a generation.** `instantiation_symbol` gains `generation: Option<u64>` (`None` natively, `Some(g)` at the REPL), so `CallInst.symbol` and `IrFunc.name` are minted from one pure function. (R2)
- **D2. Instantiations retained and deduped at the session, keyed by (name, generation, substitution)** — the symbol string encodes all three, so the symbol *is* the dedup key. (R7)
- **D3. An instantiation binds its callees against the resolver snapshot from the word's *defining* line,** not the instantiating line's — preserving frozen-binding. (R4, R7)
- **D4. Redefinition follows the ordinary-word generation rule** (bump generation, leave old symbols resident/resolvable, bind new calls to the new generation), not 8b's blanket `override_epoch`. (R8)
- **D5. Fallback if the feature slips: a clean located rejection** of polymorphic REPL definitions, shipped as its own criterion (F) so the tree is never silently wrong. (R1)

Out of scope (from the brief): quotations/combinators (Slices 4-5); generic `type:` (Slice 3); `if` in a poly body (Slice 1 deferral); nested poly calls (Slice 1 R14); REPL late binding on redefinition (deferred separately).

## Requirements by stage

Diagnostics `Xn` are behavioural negative tests asserting the message *and* named identifiers. "Golden" = a runnable REPL session in `tests/phase1.rs`.

**The never-silent floor** — **R1:** `eval_def` detects `word.poly.is_some()` before the concrete path can mis-check it. Phase 1 ships a located rejection naming the word; Phase 3 upgrades to real acceptance (R3). Invariant: a poly REPL def is either cleanly rejected or correctly supported, never silently miscompiled (criterion F).

**Symbol identity** —
- **R2:** `instantiation_symbol` gains `generation: Option<u64>`; `None` reproduces today's symbol byte-for-byte, `Some(g)` appends `__gen{g}` (same device as `mangled_symbol`). `CallInst` gains `generation`. Checker mint (`check.rs`) and lowering re-mint (`ir.rs`) both call the three-arg form.
- **R2b:** `PolyCtx.env` changes type from `&HashMap<String, PolySig>` to `&HashMap<String, (PolySig, Option<u64>)>` so the generation reaches `check_poly_call`'s mint with no second channel. All three native construction sites move together (native `check`'s `poly_env`, `check_def_collecting_drop_sites`'s empty env, `infer_line`'s empty env), each passing `None`.
- **Rnat:** Addition-only. Native `check`/`lower` thread `None` everywhere; `tests/phase4_generics.rs` *and* the drop-overload goldens (`tests/phase3_resources.rs`, since R2b touches `check_word` and the drop-reachability path) stay byte-identical. Any diff to a pre-slice `.expected` is a regression.

**Retention and the definition line** —
- **R3:** A poly REPL def is checked by `check_poly_body` (native poly body-checker), deriving no concrete `Sig`. `check_poly_body` runs **first, always**: so `: twice ( 'T -- 'T 'T ) dup ;` fails the unbounded `dup` and is X1, never reaching the multi-output gate; `: pair ( 'T: Copy -- 'T 'T ) dup ;` passes the body check and reaches the gate → X3. Once the body checks clean, a def resolving to `>= 2` outputs (`sig.outputs.len() >= 2 || sig.row_out.is_some()`; a length variable never changes output count) stays a clean located deferral (X3). The poly word does **not** enter concrete `self.env`.
- **R4:** New session store `poly_words: HashMap<String, PolyWordEntry { generation: u64, word: WordDef, resolver: HashMap<String, String> }>`, alongside `drop_overloads`. `resolver` is the **frozen** callee→symbol map captured from `self.env` at the defining line (D3). Nothing is compiled at the defining line.

**The instantiation line** —
- **R5:** The session builds one `HashMap<String, (PolySig, Option<u64>)>` from `poly_words` and threads it into **both** check paths: `infer_line` (expression lines, relays the filled table) and `check_def`/`check_def_collecting_drop_sites`/`check_word` (defined bodies, so a defined word can call a retained poly word). The drop-overload site collector (`src/repl.rs:612`) passes the **empty** map so drop-reachability stays byte-identical. A call records a `CallInst` with `generation = Some(g)`, keyed by the compile-unit-local `Span`.
- **R6:** The native bound check (`is_copy`/`is_ord`) fires at the REPL call site for free (`Ctx::Line` on a line, `Ctx::Word` in a def, X2). No REPL-only diagnostic path.
- **R7:** The instantiation table + poly-arity map thread into **both** `lower_line` and `ir::lower_word` (which today forwards `empty_instantiations()`/`empty_poly_arities()`) so calls resolve via `lower_poly_call`. One shared session emit step (from **both** `run_terms` and `eval_def`) decides emit-or-skip against `exported_insts: HashSet<String>`: not-yet-exported → lower one monomorphized `IrFunc` via `lower_word_parts(&symbol, &concrete_effect(...), &word.body, ...)` using the **retained snapshot** resolver (not `resolver_for(&self.env)`), external linkage, insert into `exported_insts`; already-exported → emit nothing, call binds to `CallInst.symbol` resolved under `RTLD_GLOBAL` first-loaded-wins. **Multi-output carve-out:** REPL lowering interns no return bundle, so R3 rejects `>= 2`-output defs up front (X3); every instantiation reaching R7 is single-output.

**Redefinition** — **R8:** Redefining a poly word bumps its generation to one past whichever of `self.env` or `self.poly_words` holds the name (shared per-name counter, so mono↔poly cannot collide symbols), retains new word + fresh snapshot, leaves old symbols resident/resolvable. New calls mint `__gen{N}` symbols distinct from the old. No other word's symbol restamped (contrast 8b's `override_epoch`). Closes the trace-C hazard: without the generation, re-instantiating a redefined word at an already-instantiated type would mint the old body's symbol and silently run it. A name is in exactly one of `self.env`/`self.poly_words` at a time (defining as poly evicts the ordinary entry, and vice versa).

## Success criteria

| # | criterion | kind | maps |
|---|---|---|---|
| F | poly REPL def never silently miscompiled: clean located rejection (Phase 1 floor), then correct support once Phase 3 lands | golden→golden | R1, R3 |
| 1 | **trace A**: `: id ( 'T -- 'T ) ;`, then `5 id .` / `"hi" id .` print `5` then `hi` | golden, run | R3-R7 |
| 2 | **trace B**: `id` twice at same type prints `5`/`7`; second same-type instantiation recompiles nothing | golden + unit (`exported_insts` holds one symbol) | R7 |
| 3 | **trace C** (frozen-vs-reject, single-output): `Spy` linear; `: id ( 'T -- 'T ) ;` gen0 unbounded; `: g ( -- ) 7 Spy id drop ;` binds `id`@`Spy` gen0; `g` prints `drop 7`; `: id ( 'T: Copy -- 'T ) ;` gen1; `g` again still `drop 7` (frozen); a new `7 Spy id drop` fails the `Copy` bound (X2) | golden, run | R8, R4, R5, R7 |
| 4 | consolidated exit session as one golden | golden, run | R3-R8 |
| X1 | ill-typed poly body via `check_poly_body`: `: bad ( 'T -- 'T ) dup ;` names `'T` + missing `Copy` (Slice 1 X7 wording); underflow located, not `( -- )` mismatch | negative | R3 |
| X2 | instantiating a `'T: Copy` word at a linear type is the native call-site error naming variable/type/linear reason (Slice 1 X5) | negative | R5, R6 |
| X3 | `: pair ( 'T: Copy -- 'T 'T ) dup ;` is a clean located deferral naming the word + multi-output reason, never `defined pair` | negative | R3, R7 |

**Trace-C deviation (flagged, D5).** The brief's literal `dup drop` redefinition body does not type-check under `check_poly_body` (unbounded `dup` is Slice 1 X7), and any same-arity `'T -- 'T` body is provably the identity, so old-vs-new cannot be value-witnessed at a single output. The brief's 2-output workaround is unbuildable this slice (no REPL return-bundle interning). Criterion 3 therefore witnesses the frozen-generation property through an **accept/reject contrast at a single output**; the symbol-collision property R2 guards (`instantiation_symbol(_, subst, Some(0)) != Some(1)`) is unit-pinned in Phase 2.

## Invariants

- **Green** unchanged: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.
- **No in-process JIT / comptime interpreter**: the REPL still compiles each line to a `.so` and `dlopen`s it; an instantiation is one more `IrFunc` resolved cross-line under `RTLD_GLOBAL`, exactly as per-line destructor glue.
- **No new `Instr`/`Terminator`, no new lowering mechanism**: every lowered instantiation is single-output, reusing `lower_word_parts`' scalar-return path; interns no REPL-side return bundle (deferred).
- **`Type` gains no variant**; `Slot` stack stays concrete.
- Backend stays **QBE**; `Ptr` opaque; `core` stays `no_std`.
- **Frozen binding preserved** (D3); general late-binding deferred to its own track.

## Key risks

- **Symbol collision under `RTLD_GLOBAL`** — enforced by the generation component (R2) + (name, generation, subst) dedup (R7); symbol-level unit pin + criterion 3 end-to-end.
- **Stale callee binding at instantiation** — *both* halves of the callee binding are frozen at the defining line and stored per poly word (R4/D3): the callee→symbol `resolver` *and* the callee arity map (`ir_lower_env`). Lowering an instantiation uses both, never the instantiating line's live `self.env`, so a callee redefined at a different symbol *or* a different arity/return type in between cannot change the poly word's meaning (symbol swap) or its ABI (arity mismatch reading an uninitialized slot). Pinned by `poly_instantiation_freezes_callee_arity_across_a_differing_redefinition` (differing arity → correct frozen output, not garbage) and its same-arity value-witnessed control `poly_instantiation_freezes_callee_value_across_a_same_arity_redefinition` in `tests/phase1.rs`.
- **`.so` growth per repeat** — `exported_insts` skips already-exported symbols. Pinned by criterion 2's unit.
- **Native regression from R2/R2b** — every native site passes `None`; guarded by `tests/phase4_generics.rs` *and* the drop-overload goldens.

## Delivery (as implemented)

Each phase leaves the tree coherent and green; Phase 1 alone satisfies the never-silent floor.

- **Phase 1 — Never-silent floor (D5, recon 1).** `eval_def` rejects a `word.poly.is_some()` def with a located error naming the word, before the concrete path mis-checks the empty effect. No retention/symbol/lowering change. Exit: criterion F rejection golden.
  - `4a9a09ea` reject polymorphic word definitions; `8a95ee16` include location; `ea21097b` omit spurious location for empty-bodied case — `src/check.rs`, `src/repl.rs`, `tests/phase1.rs`
- **Phase 2 — Generation-parameterized symbol infra (D1, R2/R2b).** `instantiation_symbol` gains `generation`; `CallInst` gains `generation`; `PolyCtx.env` retyped to carry the generation, all three native sites → `(sig, None)`; native mint + lowering re-mint use the three-arg form. Exit: Rnat + `None`/`Some(g)` and `Some(0) != Some(1)` unit tests.
  - `bdf1a126` track instantiation generations — `src/ast.rs`, `src/check.rs`, `src/ir.rs`; `4730af45` use tracked generation in lowering re-mint — `src/ir.rs`
- **Phase 3 — The feature (D2, D3, core).** Accept a poly def via `check_poly_body`; retain body + frozen resolver + generation in `poly_words`; keep it out of `self.env`. Thread one session `poly_env` into `infer_line` *and* the word-def check path (drop collector keeps empty map). Thread the instantiation table + poly-arity map into `lower_line` *and* `ir::lower_word`; shared emit step lowers per not-yet-exported instantiation against the snapshot resolver, `exported_insts` dedup. Reject `>= 2`-output defs (X3). `twice`'s rejection changes from the blanket wording to the real X1. Exit: criteria 1, 2, X1, X2, X3, F upgraded; drop-overload goldens green.
  - `a4a279d0` propagate instantiation call sites through check and lower — `src/backend/qbe.rs`, `src/check.rs`, `src/ir.rs`, `src/repl.rs`; `c248a817` Phase 3 support + dedup tests — `src/check.rs`, `src/repl.rs`, `tests/phase1.rs`
- **Phase 4 — Redefinition (D4, R8).** Shared per-name generation bump across `self.env`/`poly_words`; fresh snapshot; old symbols resident; new calls bind the new generation; no blanket restamp. Exit: criterion 3 single-output frozen-vs-reject golden.
  - `68aa0683` implement phase changes — `src/repl.rs`, `tests/phase1.rs`
- **Phase 5 — Consolidated exit golden + docs.** Full ROADMAP exit session as one `tests/phase1.rs` golden; note decisions in `ROADMAP.md`, confirm `DESIGN.md`'s late-binding deferral cross-references this slice. Exit: criterion 4.
  - `07b0d060` phase 4 slice 2 exit golden — `ROADMAP.md`, `tests/phase1.rs`
