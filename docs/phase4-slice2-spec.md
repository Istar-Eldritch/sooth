# Phase 4 Slice 2: REPL monomorphization

Base: `main` @ `d645162`. Design input: [the brief](./phase4-slice2-brief.md), whose recon
and five decisions are grounded live against the REPL binary and are adopted here unless a
concrete problem is flagged (one is, see the trace-C note in "Success criteria"). Slice 1
landed native type/row/length variables and monomorphization, deliberately native-only
(`docs/phase4-slice1-spec.md`, D2). This slice makes the REPL see polymorphic words: define
one at the REPL, instantiate it at concrete types on later lines, redefine it, and dedup
repeated same-type instantiations, with the diagnostics for all of it.

The larger question this slice brushes against, whether the REPL should ever be late-bound on
redefinition, is deferred (`DESIGN.md`, Open / deferred: **REPL late binding for
redefinition**). This slice keeps the existing frozen-binding rule every ordinary REPL word
already follows, for consistency, and does not decide the bigger question.

## The starting state is silently wrong, not cleanly unsupported

Native `check(module)` (`src/check.rs:966`) builds a `poly_env` from every `word.poly`
(`:1020`), unifies each polymorphic call site against the concrete stack (`check_poly_call`,
`:3258`), records a `CallInst` keyed by the call-site `Span` (`:3312`), and native `lower`
(`src/ir.rs:1014`) emits one monomorphized `IrFunc` per distinct instantiation (`:1160`). The
REPL shares none of this. `eval_def` (`src/repl.rs:682`) routes every definition through
`check_def` -> `check_word` (the concrete path, `src/check.rs:1775`/`:2505`), which never reads
`word.poly`; and `run_terms` (`src/repl.rs:814`) checks each expression line through
`infer_line` (`src/check.rs:1852`), which builds an **empty** `poly_env` and discards the
never-filled instantiation table (`:1871`).

A polymorphic word leaves `word.effect` empty (the signature lives in `word.poly`,
`src/ast.rs`), so the REPL checks its body against a zero-arity `Sig` derived from that empty
effect. The observable result is not a clean "unsupported" error but a **silent miscompile**:

```
> : twice ( 'T -- 'T 'T ) dup ;
error: stack effect mismatch in `twice` (line 1)
  `dup` needs 1 values, but the stack holds 0
  note: declared ( -- )          # not what the user wrote

> : id ( 'T -- 'T ) ;
defined id                        # enters env with an empty signature
> 5 id .
5                                 # "works" only because the body is a no-op the checker never modelled
```

Removing this silent state is the floor the slice ships first and keeps whatever else slips
(R1, criterion F).

## Locked decisions

The brief's five decisions, as they bind this spec (**D1-D5**), each with the requirement that
carries it:

- **D1. Symbol identity carries a generation.** `instantiation_symbol` gains an explicit
  `generation: Option<u64>` parameter (`None` natively, `Some(g)` at the REPL), so the
  checker's `CallInst.symbol` and the lowered `IrFunc.name` are still minted from one pure
  function and cannot disagree. See R2.
- **D2. Instantiations are retained and deduped at the session, keyed by
  (name, generation, substitution).** The instantiation symbol already encodes all three, so
  the dedup key is the symbol string. Answers trace B and bounds `.so` growth. See R7.
- **D3. An instantiation binds its callees against the resolver snapshot retained from the
  word's *defining* line, not the instantiating line's.** This preserves the frozen-binding
  semantics every ordinary REPL word already has, and is why the session retains the resolver
  snapshot per polymorphic word, not just the body AST. See R4, R7.
- **D4. Redefinition follows the ordinary-word generation rule** (bump the generation, leave
  old symbols resident and resolvable, bind new calls to the new generation), not 8b's blanket
  `override_epoch` restamp: polymorphic instantiations are not woven pervasively the way
  destructors are, so the narrower rule is right. See R8.
- **D5. The fallback if the feature splits or slips is a clean located rejection of
  polymorphic REPL definitions,** mirroring Slice 1's poly-`if` rejection, shipped as its own
  criterion so the tree is never left in the silently-wrong state above whatever else lands.
  See R1, criterion F.

Explicitly out of scope, carried from the brief: quotations and combinators (Slices 4-5);
generic `type:` declarations (Slice 3); `if` in a polymorphic body (still rejected, Slice 1's
own deferral, untouched here); nested polymorphic calls, a polymorphic body calling another
polymorphic word with a variable propagated (Slice 1 R14, still out); and REPL late binding on
redefinition in general (deferred separately in `DESIGN.md`, not this slice's decision).

## Requirements by stage

Requirement IDs `Rn`; diagnostics `Xn` (each a behavioural negative test asserting the message
*and* the named identifiers). "Golden" means a runnable REPL session in `tests/phase1.rs`
(source lines in -> expected transcript out), never an IL-string assertion.

### The never-silent floor (`src/repl.rs`)

**R1: A polymorphic REPL definition is never silently miscompiled.** `eval_def`
(`src/repl.rs:682`) detects `word.poly.is_some()` before the concrete `check_def` path can
mis-check it against the empty effect. Phase 1 ships this as a **located rejection** naming the
word (replacing both the bogus `note: declared ( -- )` mismatch and the silent `defined`).
Phase 3 replaces the rejection with real acceptance (R3). The **invariant** (a poly REPL def is
either cleanly rejected or correctly supported, never silently miscompiled) is the criterion
(F): met by Phase 1's rejection, upgraded by Phase 3, and if Phase 3+ slip, the Phase 1
rejection is the shipped floor and its golden stands.

### Symbol identity (`src/ast.rs`, `src/check.rs`, `src/ir.rs`)

**R2: `instantiation_symbol` carries a generation (D1).** `instantiation_symbol`
(`src/ast.rs:483`) gains a third parameter `generation: Option<u64>`: `None` reproduces today's
symbol byte-for-byte (`sooth_mono_{word}__{parts}`); `Some(g)` appends a `__gen{g}` component
(`sooth_mono_{word}__{parts}__gen{g}`), the same `__gen{N}` device `mangled_symbol`
(`src/repl.rs:114`) already uses for ordinary REPL words. `CallInst` (`src/ast.rs:468`) gains
`generation: Option<u64>`, set by whoever records the instantiation. The single-source-of-truth
property is preserved: the checker's mint (`src/check.rs:3312`) and lowering's independent
re-mint (`src/ir.rs:1161`) both call `instantiation_symbol(callee, subst, generation)`, so the
call-site `Instr::Call` target and the emitted `IrFunc.name` remain two deterministic
computations of one function. Every existing native call site passes `None`; native output is
byte-identical (Rnat).

**R2b: the generation reaches `check_poly_call`'s mint through `PolyCtx`, one named change.**
`check_poly_call` mints the symbol (`src/check.rs:3312`) from whatever generation its context
carries, so the generation must reach it without a second lookup channel. `PolyCtx.env`
(`src/check.rs:43`) therefore changes type from `&HashMap<String, PolySig>` to
`&HashMap<String, (PolySig, Option<u64>)>`: the generation rides alongside each `PolySig`, the
mint reads `(sig, gen)` and calls `instantiation_symbol(name, &subst, *gen)` and sets
`CallInst.generation = *gen`. This is its own explicit piece of work because all three sites
that construct a `PolyCtx.env` live in `src/check.rs` and must move together: native `check()`'s
`poly_env` build (`:1020`, each value becomes `(sig, None)`), `check_def_collecting_drop_sites`'s
`empty_poly_env` (`:1811`), and `infer_line`'s `empty_poly_env` (`:1871`). Every native site
passes `None`, so `instantiation_symbol` appends nothing and native symbols, `CallInst`s, and the
`tests/phase4_generics.rs` suite stay byte-identical. The REPL supplies `Some(g)` later (R5).

**Rnat: Native regression, addition-only.** No existing native golden or unit test changes its
expected output. Native `check`/`lower` thread `None` through R2 everywhere, so every emitted
symbol, and the whole `tests/phase4_generics.rs` suite, is unchanged. The R2b `PolyCtx.env`
type change touches native `check()`'s body walk (`check_word`) and the drop-reachability path
(`check_def_collecting_drop_sites` -> `check_drop_overload_reachability`), so the native suite
*and* the `drop`-overload goldens (`tests/phase3_resources.rs`) are part of this guard, not just
`tests/phase4_generics.rs`. A diff to any pre-slice `.expected`/assertion is a regression, not an
update.

### Retention and the definition line (`src/repl.rs`, `src/check.rs`)

**R3: A polymorphic REPL definition is checked by `check_poly_body` (fixes recon 1).**
`eval_def`, for a poly word, runs `check_poly_body(word, sig, &env, structs, enums, arrays)`
(`src/check.rs:2884`, the native poly body-checker) instead of the concrete
`check_def`/`check_word` path, and derives **no** concrete `Sig` from the empty effect.
`: id ( 'T -- 'T ) ;` yields `defined id`; a valid body checks clean; an ill-typed body is a
real located error via the native pass (X1), not the recon-1 zero-arity mismatch. The poly word
does **not** enter the concrete `self.env` (mirroring how a `drop` overload is kept out of
`env`, `src/repl.rs` comment at `:270`), so `next_generation`'s `env` lookup and every concrete
call-site lookup never see it. **`check_poly_body` runs first, always**: the multi-output gate
below only ever sees a body that already type-checked. So `: twice ( 'T -- 'T 'T ) dup ;`, an
unbounded `dup`, fails `check_poly_body` itself and is X1 (naming `'T` and the missing `Copy`
bound), never reaching the gate despite its two outputs; `: pair ( 'T: Copy -- 'T 'T ) dup ;`
passes the body check (the bound is declared) and is the one that reaches the gate, X3. Gating
arity before checking the body would make `twice` X3 instead and contradict this ordering.

Once the body checks clean, a poly def whose `PolySig` resolves to `>= 2` concrete outputs
stays a **clean located rejection** (X3), deferred: REPL return-bundle interning is out of scope
this slice (R7's multi-output carve-out), mirroring Slice 1's D2 carve-out for a monomorphic
multi-output word defined at the REPL. The precise trigger is `sig.outputs.len() >= 2 ||
sig.row_out.is_some()` (`src/ast.rs`'s `PolySig`): a length variable sizes an array *within* one
output slot and never changes the output count, so it is not part of the trigger, only a
genuine second output slot or an output row variable is. `: id ( 'T -- 'T ) ;` (one output) is
accepted; `: pair ( 'T: Copy -- 'T 'T ) dup ;` (two outputs) is rejected as deferred, never
silently truncated to one. This keeps the never-silent floor (criterion F) intact for the
multi-output case the moment defs start being accepted.

**R4: Session retention of body + resolver snapshot + generation (D3).** A new session store
`poly_words: HashMap<String, PolyWordEntry>` where
`PolyWordEntry { generation: u64, word: WordDef, resolver: HashMap<String, String> }`, alongside
`drop_overloads` (`src/repl.rs:274`) and for the same reason: the body must survive its
defining line because it is lowered *later*, at each instantiating line, from an AST the session
would otherwise have thrown away. `resolver` is the **frozen** callee-name -> mangled-symbol map
captured from `self.env`'s generations at the defining line (D3); it, not the instantiating
line's `resolver_for(&self.env)` (`src/repl.rs:126`), is what an instantiation of this word
binds its callees against, so an unrelated later redefinition of a callee cannot change this
body's meaning. Nothing is compiled at the defining line: a polymorphic word has no concrete
instantiation to lower there.

### The instantiation line (`src/check.rs`, `src/repl.rs`, `src/ir.rs`)

**R5: The session poly-env is threaded into *every* REPL check path, expression *and*
definition, through one shared map.** The session builds a
`HashMap<String, (PolySig, Option<u64>)>` from `poly_words` (each entry ->
`(entry.word.poly, Some(entry.generation))`, the generation riding alongside so `check_poly_call`
mints the generation-stamped symbol, R2/R2b). That one map is threaded into both check paths, not
two parallel ones:

- **Expression line:** `infer_line` (`src/check.rs:1852`) gains the poly-env parameter and relays
  the filled instantiation table to its caller (today it builds an empty `poly_env` and drops the
  table, `:1871`). A `build`-path caller passes the empty map (Slice 1's D2 behaviour, now "empty
  poly-env" rather than "no poly-env").
- **Word-definition body:** the definition check path `check_def` -> `check_def_collecting_drop_sites`
  -> `check_word` (`src/check.rs:1775`/`:1797`/`:2505`) gains the same poly-env parameter, so a
  *defined word's own body* can call a retained polymorphic word. `eval_def` (`src/repl.rs:687`)
  passes the real session map and relays the filled instantiation table back for lowering (R7).
  The drop-overload site-collection caller (`src/repl.rs:612`, which feeds the native-shared
  `check_drop_overload_reachability`) passes the **empty** map, so that path stays byte-identical
  (a `drop` overload is never polymorphic). A consequence, not a gap: a `drop` overload's own
  body is a defined word body too, but it never sees the real map, so a `drop` overload that
  calls a retained poly word gets an ordinary `unknown word` error (the poly word is also kept
  out of concrete `self.env`, R3), not a miscompile. The never-silent floor holds; the surface a
  defined body can poly-call is narrower than "every defined word" by exactly this one carve-out.

This closes the recon hole in both directions: without the definition-path thread, *no defined
word could ever call a polymorphic word*, only bare top-level lines could, a far larger scope
hole than the slice intends (criterion 3's `g` is exactly such a defined word, and criterion 1's
traces are lines). `check_word` already takes a `&mut PolyCtx`, so this is a threading change, not
a new checker path. A call to a retained poly word on either path unifies its `PolySig` against
the carried stack via the existing `check_poly_call`, checks its bounds (R6/X2), and records a
`CallInst` (with `generation = Some(g)`) keyed by the compile-unit-local `Span` (a line's span for
a line, the body's span for a def). Spans never cross compile units (REPL line numbering restarts
per line, recon 2), which is exactly why the table is per-compile-unit and the *session* owns
cross-unit memory (R7), not the `Span`-keyed table.

**R6: Bound checking reaches the REPL unchanged (D3 boundary).** Because the REPL routes through
`check_poly_call`, the native bound check (`src/check.rs:3291`, `Copy` via `is_copy`, `Ord` via
`is_ord` at `:2826`) fires at the REPL call site with no new code, on either path (a bare line
or a defined word's body): instantiating a `'T: Copy` word at a linear concrete type is the
native located error (`Ctx::Line` phrasing on a line, `Ctx::Word` phrasing in a def, X2). No
REPL-only diagnostic path is added.

**R7: Per-compile-unit instantiation lowering into the compiling module, with cross-line dedup
(D2), for both expression lines and word definitions.** The recorded instantiation table and a
poly-arity map (`name -> input arity`, built from `poly_words`, the REPL analogue of native
`lower`'s `poly_arities` at `src/ir.rs:1045`) are threaded into **both** lowering entry points so
a call site resolves through `lower_poly_call` (`src/ir.rs:3284`, reached at `:2316`) rather than
the name-keyed `env`/`resolve`:

- `lower_line` (`src/ir.rs:1678`) for a bare expression line, via its `FuncBuilder`.
- `ir::lower_word` (`src/ir.rs:1904`) for a defined word's body, which today forwards
  `empty_instantiations()`/`empty_poly_arities()` to `lower_word_parts` (`:1920`); it gains the
  table + poly-arity map and forwards them instead, so a poly call inside a defined body
  (criterion 3's `g`) lowers to its per-site symbol. `lower_word_parts` already accepts both, so
  this is threading, not a new lowering path.

For each recorded instantiation, one shared session step (called from **both** `run_terms` and
`eval_def`, not two parallel copies) decides emit-or-skip against `exported_insts: HashSet<String>`
(the already-exported instantiation symbols; the symbol encodes name+generation+subst, so it *is*
the (name, generation, subst) key D2 asks for):

- **not yet exported:** the compiling module additionally lowers one monomorphized `IrFunc` via
  `lower_word_parts(&symbol, &concrete_effect(sig, subst, arrays), &word.body, false, env,
  RESOLVER, regs, &unit_table, &poly_arities)` (`src/ir.rs:1930`, `concrete_effect` at
  `:1862`), where `RESOLVER` is built from the retained snapshot (R4), **not**
  `resolver_for(&self.env)`. The func is emitted with external linkage so a later line resolves
  it under `RTLD_GLOBAL` (the same cross-line resolution the per-line synthesized destructors
  already rely on, `src/repl.rs:495`). The symbol is inserted into `exported_insts`.
- **already exported:** nothing is emitted; the call site still binds to `CallInst.symbol`,
  which `RTLD_GLOBAL`'s first-loaded-wins resolves to the earlier line's export. Bounds `.so`
  growth (trace B).

**Multi-output carve-out (never-silent, D2 boundary).** REPL lowering interns no return bundle
(`word_ret_ty` at `src/ir.rs:1841` falls back to first-output-only when no bundle exists; the
"REPL's registries intern no bundle at all" note lives at `src/ir.rs:208`), so a poly
instantiation resolving to `>= 2` outputs would silently drop all but the first, the exact
miscompile class this slice removes. This slice therefore never lowers such an instantiation:
R3 rejects the poly def up front (X3), so every instantiation reaching R7 is single-output and
uses `lower_word_parts`' scalar-return path wholesale, no bundle interned, no new lowering
mechanism. Interning REPL-side bundles is real new lowering work, explicitly deferred.

A polymorphic body that itself calls another polymorphic word is out of scope (Slice 1 R14), so
each compile unit's instantiations are non-nested and `concrete_effect`'s single output is a
concrete scalar before Slice 1's return ABI lowers it, unchanged.

### Redefinition (`src/repl.rs`)

**R8: Redefinition follows the ordinary-word generation rule (D4).** Redefining a poly word
bumps its generation to one past whichever of `self.env` (ordinary) or `self.poly_words` (poly)
currently holds the name (a shared per-name counter, so a mono<->poly redefinition cannot
collide symbols), retains the new `word` + a **fresh** resolver snapshot at the new generation
(R4), and leaves every old instantiation symbol resident (`.so`s are never `dlclose`d) and
resolvable. A new call site unifies against the new `PolySig` at the new generation, minting
`__gen{N}` symbols distinct from the old (R2). No other word's symbol is restamped (contrast
8b's `override_epoch`, `src/repl.rs:297`). An earlier line's already-compiled call keeps its old
symbol, matching every ordinary REPL word and `DESIGN.md`'s deferred late-binding note. This is
what closes the brief's trace-C hazard: without the generation, re-instantiating a redefined
word at a type it was already instantiated at would mint the old body's symbol and silently run
it under first-loaded-wins.

A name is in exactly one of `self.env` or `self.poly_words` at a time: defining `id` as poly
evicts a prior ordinary `WordEntry` for `id` from `self.env` (and a later ordinary redefinition
of `id` evicts it back out of `poly_words`), so the two stores stay mutually exclusive per name
and a call to `id` never has to arbitrate between a poly-env entry and a concrete `env` entry for
the same name. No criterion in this slice exercises a mono<->poly toggle on one name; this rule
is what keeps that path from being a silent precedence question if a future test does.

## Success criteria

Each maps to a runnable golden in `tests/phase1.rs`; each `Xn` to a behavioural negative test.
Unit tests sit beside their stage (`src/ast.rs`, `src/check.rs`, `src/ir.rs`, `src/repl.rs`).

| # | criterion | kind | maps |
|---|---|---|---|
| F | a polymorphic REPL definition is never silently miscompiled: it is a clean located rejection (Phase 1 floor) or, once Phase 3 lands, correctly supported (criterion 1). The recon-1 `note: declared ( -- )` / silent `defined` state is gone | golden (rejection) then golden (support) | R1, R3 |
| 1 | **trace A**: `: id ( 'T -- 'T ) ;`, then `5 id .` and `"hi" id .` on later lines, print `5` then `hi` (define once, instantiate at two types) | golden, run | R3, R4, R5, R6, R7 |
| 2 | **trace B**: `: id ( 'T -- 'T ) ;`, `5 id .`, `7 id .` print `5` then `7`; the second same-type instantiation recompiles nothing (dedup) | golden, run + unit (`exported_insts` holds one symbol after both lines) | R7 |
| 3 | **trace C** (frozen-vs-reject, single-output throughout): with `Spy` defined (the linear stand-in), `: id ( 'T -- 'T ) ;` (gen0, unbounded); `: g ( -- ) 7 Spy id drop ;` binds `id`@`Spy` at gen0 (an unbounded `'T` instantiates at a *linear* type); calling `g` prints `drop 7`; `: id ( 'T: Copy -- 'T ) ;` (gen1, adds the `Copy` bound); calling `g` again still prints `drop 7` (frozen to gen0's body); a *new* `7 Spy id drop` line now fails the `Copy` bound (X2's `Ctx::Line` message naming `'T`, `Spy`, the linear reason), because gen1 governs new instantiations while gen0's compiled call is untouched | golden, run | R8, R4, R5, R7 (R2 symbol-distinctness unit-pinned) |
| 4 | the consolidated exit session (define, instantiate at two types, instantiate twice at one type, redefine, new body takes effect while an earlier line's call keeps the old one) runs as one `tests/phase1.rs` golden | golden, run | R3-R8 |
| X1 | an ill-typed polymorphic body at the REPL is a real located error via `check_poly_body`: `: bad ( 'T -- 'T ) dup ;` names `'T` and the missing `Copy` bound (Slice 1's X7 wording), and a body that underflows its declared inputs is located, not the recon-1 `( -- )` mismatch, not a silent `defined` | negative, message + `'T` | R3 |
| X2 | instantiating a `'T: Copy` REPL word at a linear concrete type on a later line is the native call-site error naming the variable, the type, and the linear reason (Slice 1's X5, reached through the REPL poly-env) | negative, message + `'T` + type | R5, R6 |
| X3 | a polymorphic REPL definition resolving to `>= 2` outputs is a clean located deferral, not a silent single-output truncation: `: pair ( 'T: Copy -- 'T 'T ) dup ;` names the word and the deferred multi-output reason (R7 carve-out), never `defined pair` | negative, message | R3, R7 |

**Deviation from the brief, flagged (D5/trace C).** The brief's literal trace-C redefinition
body `: id ( 'T -- 'T ) dup drop ;` does **not** type-check under `check_poly_body`: `dup` of an
unbounded `'T` is Slice 1's X7 rejection (the REPL only accepted it before because recon 1 never
ran the poly checker on it). More, any same-arity `'T -- 'T` body is provably the identity at a
single input, so old-vs-new cannot be witnessed through its output *value* at all. The brief's
own workaround, a 2-output redefinition (`5` vs `5 5`), is **not buildable at the REPL this
slice**: REPL lowering interns no return bundle (R7's carve-out), so a 2-output instantiation
would silently drop its second output, exactly the miscompile this slice exists to remove.
Interning REPL-side bundles is real new lowering work, out of scope here (the "no new mechanism"
invariant), so criterion 3 does not depend on it.

Criterion 3 therefore witnesses the frozen-generation property through an **accept/reject
contrast at a single output**, no bundle involved. `id` starts unbounded (`( 'T -- 'T )`, gen0),
so it instantiates at *any* type including a linear one; the caller `g` binds `id`@`Spy` at gen0
and stays observable (`drop 7`). Redefining `id` to add a `Copy` bound (`( 'T: Copy -- 'T )`,
gen1) leaves `g`'s compiled gen0 call untouched (frozen: still `drop 7`) while a *new* `id`@`Spy`
instantiation now fails the `Copy` bound (X2). That is the same generation-freezing property the
brief's trace C wanted, single-output throughout. Verified against the native mechanics: an
unbounded `'T -- 'T` does instantiate at a linear `Spy` (`check_poly_call` enforces only the
bounds a signature declares, and gen0 declares none, `src/check.rs:3291`), and adding a bound
gates only *new* instantiations (each `CallInst` is minted per site at check time; an earlier
compiled instantiation is never retroactively re-checked). The end-to-end symbol-collision
property R2 guards, distinct symbols for the same (name, subst) across generations under
`RTLD_GLOBAL`, is pinned by Phase 2's unit assertion
(`instantiation_symbol(_, subst, Some(0)) != instantiation_symbol(_, subst, Some(1))`); it cannot
be *value*-witnessed end-to-end at a single output, because a same-type body difference is exactly
what single-output `'T -- 'T` forbids. The alternative, adopting REPL bundle interning to keep a
value contrast, was rejected as scope this slice deliberately excludes.

## Non-functional / invariants

- **Green** unchanged: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.
- **No in-process JIT, no comptime interpreter** (CLAUDE.md): the REPL still compiles each line
  to a `.so` and `dlopen`s it; a monomorphized instantiation is one more `IrFunc` in that line's
  module, resolved cross-line under `RTLD_GLOBAL`, exactly as the per-line destructor glue is.
- **No new `Instr`/`Terminator` and no new lowering mechanism**: every instantiation this slice
  lowers is single-output (R3/R7 reject a poly REPL def resolving to `>= 2` outputs, X3), so it
  reuses Slice 1's `lower_word_parts` scalar-return path wholesale; this slice adds retention,
  threading, a generation component, and the multi-output deferral, but interns no REPL-side
  return bundle (that would be new lowering work, out of scope).
- **`Type` gains no variant**; the `Slot` stack stays concrete (Slice 1's S1 invariant holds).
- Backend stays **QBE**; `Ptr` opaque; `core` stays `no_std`.
- **Frozen binding preserved**: an instantiation binds the defining line's callee generations
  (D3), consistent with every ordinary REPL word; the general late-binding question is deferred
  to its own design track (`DESIGN.md`).

## Out of scope

Quotations/`call` and combinators (Slices 4-5); generic `type:` declarations (Slice 3);
`if` in a polymorphic body (still rejected, Slice 1's deferral, unchanged); nested polymorphic
calls / a polymorphic body calling another polymorphic word (Slice 1 R14); a monomorphic
multi-output word *defined at the REPL* (Slice 1 R11's D2 carve-out, unchanged); a **polymorphic**
multi-output instantiation at the REPL, which is cleanly rejected here rather than silently
truncated (R3/R7/X3), REPL return-bundle interning being deferred; REPL late
binding on redefinition in general (`DESIGN.md`, its own track). Native `check`/`lower` behaviour
is unchanged except the mechanical `None`-threading of R2 (Rnat).

## Key risks

- **Symbol collision under `RTLD_GLOBAL` (R2, R7, R8).** Two lines instantiating the same
  redefined word at the same type must reach two distinct symbols, or first-loaded-wins silently
  runs the wrong body (trace C, the worst class: silent wrong code). *Enforced* by the
  generation component (R2) plus the (name, generation, subst) dedup key (R7): a new generation
  can never mint an old generation's symbol, and dedup within a generation is exact. Pinned at
  the symbol level by Phase 2's unit assertion that
  `instantiation_symbol(_, subst, Some(0)) != instantiation_symbol(_, subst, Some(1))` (same
  name+subst, distinct generation, distinct symbol), and end-to-end by criterion 3's
  frozen-vs-reject contrast (`g` still runs its gen0 `id`@`Spy` while a new `id`@`Spy` is rejected
  under gen1). A single-output slice cannot value-witness the collision directly (a same-type
  body difference is what `'T -- 'T` forbids), which is why the symbol-level unit is the primary
  pin here.
- **Stale resolver at instantiation (R4, R6, D3).** Lowering an instantiation at line N against
  line N's *current* generations would let an unrelated later redefinition of a callee change
  this body's meaning, observably wrong. Mitigation: the resolver snapshot is frozen at the
  defining line and stored per poly word (R4); lowering uses it, never `self.env`. Unlike 8b,
  which could *cache* a re-check result, a polymorphic body must be re-lowered per instantiation,
  so the snapshot cannot be cached away and must be retained. Pinned by criterion 3's frozen
  witness `g`.
- **`.so` growth per repeated instantiation (R7).** Without dedup, trace B mints a fresh global
  symbol per repeat for the session's life (harmless only because the body is identical).
  Mitigation: `exported_insts` skips emission for an already-exported symbol. Pinned by
  criterion 2's unit assertion.
- **Native regression from the R2/R2b signature changes (Rnat).** `instantiation_symbol`,
  `CallInst`, and `PolyCtx.env` are on the native hot path; a stray non-`None` generation would
  perturb native symbols, and the R2b `PolyCtx.env` type change touches native body-checking
  (`check_word`) and drop-reachability (`check_def_collecting_drop_sites`). Mitigation: every
  native site passes `None`; the guard is the unmodified `tests/phase4_generics.rs` suite
  *and* the `drop`-overload goldens (`tests/phase3_resources.rs`), not just the former. Phase 3,
  which threads a *real* poly-env into the word-def check path, carries its own native guard: the
  drop-overload collector (`src/repl.rs:612`) still passes the empty map, so drop-reachability is
  byte-identical.

## Current-state anchors (confirmed against `d645162`)

- `instantiation_symbol(word, subst)` (pure, `struct_drop_symbol`-shaped): `src/ast.rs:483`;
  `CallInst { callee, subst, symbol, out_arity, output_types, bundle }`: `src/ast.rs:468`;
  `Span { line, col }` (`Hash`): `src/ast.rs:5`. -> R2.
- `PolyCtx { env: &HashMap<String, PolySig>, insts }` (the type R2b changes to carry the
  generation): `src/check.rs:43`. -> R2b.
- Native `check` builds `poly_env` from `word.poly`, records instantiations, stores
  `module.instantiations`: `src/check.rs:1020`/`:1122`; `check_poly_call` mints the symbol and
  the `CallInst` (`bundle: None` at insertion): `src/check.rs:3312`; bound check `is_copy`/`is_ord`:
  `src/check.rs:3291`/`:2826`; the multi-output bundle-interning loop over `insts` runs only in
  native `check`'s post-pass (`intern_bundle_struct` for `out_arity >= 2`), which the REPL's
  `infer_line`/`eval_def` paths have no analogue of, hence R7's multi-output carve-out.
  -> R2, R5, R6, R7.
- `check_def` -> `check_def_collecting_drop_sites` -> `check_word` (concrete, never reads
  `word.poly`): `src/check.rs:1775`/`:1797`/`:2505`; `check_def_collecting_drop_sites` is called
  by both `check_def` (word-def path) and the REPL drop-overload site collector (`src/repl.rs:612`,
  which must keep the empty poly-env); `check_poly_body` (the native poly body-checker):
  `src/check.rs:2884`; `infer_line` builds an empty `poly_env` and drops the table:
  `src/check.rs:1852`/`:1871`. -> R1, R3, R5.
- REPL: `Session`: `src/repl.rs:241`; `WordEntry { sig, generation, symbol }`: `src/repl.rs:96`;
  `mangled_symbol` / `next_generation` / `resolver_for`: `src/repl.rs:114`/`:120`/`:126`;
  `drop_overloads` (out-of-`env` retention precedent) / `override_epoch` (the *rejected* blanket
  restamp): `src/repl.rs:274`/`:297`; `eval_def` (calls `check_def` then `ir::lower_word`):
  `src/repl.rs:682`; `run_terms` and its `ir::lower_line` call: `src/repl.rs:814`/`:854`.
  -> R1, R3, R4, R5, R7, R8.
- Native `lower` monomorphization loop (re-mints via `instantiation_symbol`, builds
  `poly_arities`): `src/ir.rs:1160`/`:1045`; `lower_line`: `src/ir.rs:1678`; `lower_word` (today
  forwards `empty_instantiations()`/`empty_poly_arities()` to `lower_word_parts`):
  `src/ir.rs:1904`/`:1920`; `lower_word_parts` / `concrete_effect`: `src/ir.rs:1930`/`:1862`;
  `word_ret_ty` first-output-only fallback when no bundle (the REPL carve-out): `src/ir.rs:1841`
  (and the "REPL's registries intern no bundle at all" note at `:208`); call-site table lookup and
  `lower_poly_call`: `src/ir.rs:2316`/`:3284`. -> R2, R7.

## Delivery

Each phase leaves the tree in a coherent, green state. Phase 1 alone satisfies the never-silent
floor (criterion F), so if the feature splits the tree is never left silently wrong.

- **Phase 1 - Never-silent floor (D5, recon 1).** `eval_def` rejects a `word.poly.is_some()`
  definition with a located error naming the word, before the concrete `check_def` path
  mis-checks the empty effect. Removes the silent miscompile immediately and is the guaranteed
  shippable floor. Exit: criterion F (a rejection golden asserting *both* `: id ( 'T -- 'T ) ;`
  and `: twice ( 'T -- 'T 'T ) dup ;` produce the blanket located rejection).
- **Phase 2 - Generation-parameterized symbol infra (D1, R2/R2b).** `instantiation_symbol` gains
  `generation: Option<u64>`; `CallInst` gains `generation`; **`PolyCtx.env` changes type to carry
  the generation alongside each `PolySig` (R2b), all three native construction sites updated to
  `(sig, None)`**; native mint and lowering re-mint both call the three-arg form. Pure infra,
  regression-guarded across the native suite *and* the drop-overload goldens (the R2b change
  touches the drop-reachability path). Exit: Rnat, plus unit tests for the `None`/`Some(g)`
  spellings and the `Some(0) != Some(1)` symbol-distinctness that pins R2's collision property.
  Tree still rejects poly REPL defs (Phase 1 floor intact).
- **Phase 3 - The feature (D2, D3, core).** Accept a poly REPL def: check via `check_poly_body`,
  retain body + frozen resolver snapshot + generation in `poly_words` (R4), remove Phase 1's
  rejection. Thread the one session `poly_env` into **both** `infer_line` (lines) *and* the
  word-def check path `check_def`/`check_def_collecting_drop_sites`/`check_word` (defined bodies),
  so a defined word can call a poly word (R5); the drop-overload collector keeps the empty map.
  Thread the instantiation table + poly-arity map into **both** `lower_line` *and* `ir::lower_word`
  (R7); a shared emit step (from `run_terms` and `eval_def`) lowers one monomorphized `IrFunc`
  per not-yet-exported instantiation against the snapshot resolver, into the compiling module,
  with `exported_insts` dedup (D2). Reject a poly def resolving to `>= 2` outputs as deferred
  (R3/X3), so no bundle is ever needed. Bound checking reaches the REPL for free (R6). At this
  point `twice`'s rejection changes from the Phase 1 blanket wording to the real X1 diagnostic
  (naming `'T` and the missing `Copy` bound), so that golden's expected text updates too, not
  just `id`'s. Exit: criteria 1, 2, X1, X2, X3 (and criterion F upgraded to correct support),
  with the drop-overload goldens still green (Phase 3's own native guard).
- **Phase 4 - Redefinition (D4).** Shared per-name generation bump across `self.env` and
  `poly_words`; fresh snapshot at the new generation; old symbols resident; new calls bind the
  new generation; no blanket restamp. Exit: criterion 3 (the frozen-vs-reject `Spy` golden,
  single-output).
- **Phase 5 - Consolidated exit golden + docs.** The full ROADMAP exit session as one
  `tests/phase1.rs` golden; note the slice's decisions and the retained frozen-binding choice in
  `ROADMAP.md` (Phase 4 Slice 2 entry) and `DESIGN.md` (the late-binding deferral already
  cross-references this slice). Exit: criterion 4.

## Phases JSON

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Never-silent floor (D5, recon 1): in eval_def (src/repl.rs:682), detect word.poly.is_some() and return a located error naming the word and line, before the concrete check_def -> check_word path (src/check.rs:1775/:2505) can mis-check a polymorphic body against the zero-arity Sig derived from its empty effect. This removes the recon-1 silent miscompile (the bogus `note: declared ( -- )` stack-effect mismatch, and the silent `defined id` that enters env with an empty signature) immediately, and is the guaranteed-shippable floor: if the real feature (phases 3-4) slips, this rejection is what ships, so the tree is never left silently wrong. No retention, no symbol change, no lowering. Exit: criterion F (a tests/phase1.rs golden asserting `: id ( 'T -- 'T ) ;` and `: twice ( 'T -- 'T 'T ) dup ;` both produce the located rejection, not `defined id` and not the `( -- )` mismatch).",
      "effort": "low",
      "difficulty": "easy"
    },
    {
      "phase": 2,
      "focus": "Generation-parameterized symbol infra (D1, R2/R2b): instantiation_symbol (src/ast.rs:483) gains a third parameter generation: Option<u64> -- None reproduces today's symbol byte-for-byte, Some(g) appends a __gen{g} component matching mangled_symbol's __gen{N} device (src/repl.rs:114); CallInst (src/ast.rs:468) gains generation: Option<u64>. R2b: PolyCtx.env (src/check.rs:43) changes type from &HashMap<String,PolySig> to &HashMap<String,(PolySig,Option<u64>)> so check_poly_call reads (sig, gen) and mints the generation-stamped symbol with no second channel; all THREE sites that build a PolyCtx.env move together -- native check()'s poly_env (src/check.rs:1020, values become (sig, None)), check_def_collecting_drop_sites's empty_poly_env (src/check.rs:1811), and infer_line's empty_poly_env (src/check.rs:1871). Update the native checker mint (src/check.rs:3312, setting CallInst.generation from the tuple's gen, None natively) and the native lowering re-mint (src/ir.rs:1161) to call the three-arg form, preserving the single-source-of-truth property that the call-site Instr::Call target and the emitted IrFunc.name are two deterministic computations of one function. Every native site passes None so native output is byte-identical. Checker/AST/native only; the tree still rejects poly REPL defs (phase 1 floor intact). Exit: Rnat -- the unmodified tests/phase4_generics.rs AND the drop-overload goldens (tests/phase3_resources.rs, since R2b touches check_word and the drop-reachability path) and the full native suite green, plus unit tests asserting instantiation_symbol's None and Some(g) spellings and that Some(0) != Some(1) for the same name+subst (the symbol-distinctness that pins R2's collision property).",
      "effort": "medium",
      "difficulty": "medium"
    },
    {
      "phase": 3,
      "focus": "The feature (D2, D3, core): accept a polymorphic REPL definition, both as bare-line callee AND as callee inside a defined word's body. In eval_def, for word.poly.is_some(), run check_poly_body (src/check.rs:2884) instead of the concrete check_def path (R3, fixes recon 1) and derive no concrete Sig from the empty effect; retain the word in a new session store poly_words: HashMap<String, PolyWordEntry { generation, word: WordDef, resolver: HashMap<String,String> }>, alongside drop_overloads (src/repl.rs:274), where resolver is the frozen callee-name->symbol map captured from self.env at the defining line (R4, D3). Keep the poly word out of the concrete self.env. R5 (blocker fix): build ONE session poly_env (HashMap<String,(PolySig, Option<u64>)> from poly_words) and thread it into BOTH infer_line (src/check.rs:1852, today it builds an empty poly_env and drops the table at :1871) AND the word-def check path check_def -> check_def_collecting_drop_sites -> check_word (src/check.rs:1775/:1797/:2505) via a new poly-env parameter, so a defined word (e.g. `: g ( -- ) 7 Spy id drop ;`) can call a retained poly word; the drop-overload site collector (src/repl.rs:612) keeps the EMPTY map so drop-reachability stays byte-identical. Each path relays its filled instantiation table; a call site unifies via check_poly_call and records a CallInst with generation = Some(g), the native bound check (src/check.rs:3291/:2826) reaching the REPL for free (R6). R7 (blocker fix): thread the instantiation table plus a poly-arity map (name->input arity from poly_words, REPL analogue of native lower's poly_arities at src/ir.rs:1045) into BOTH lower_line (src/ir.rs:1678) AND ir::lower_word (src/ir.rs:1904, today forwarding empty_instantiations()/empty_poly_arities() to lower_word_parts at :1920), so a poly call inside a defined body lowers to its per-site symbol via lower_poly_call (src/ir.rs:3284/:2316); a shared emit step called from BOTH run_terms and eval_def lowers, for each instantiation not already in a session HashSet exported_insts, one monomorphized IrFunc via lower_word_parts (src/ir.rs:1930) against concrete_effect(sig, subst, arrays) (src/ir.rs:1862) using the RETAINED SNAPSHOT resolver (R4/D3), not resolver_for(&self.env), emitted with external linkage into the compiling module so a later line resolves it under RTLD_GLOBAL; an already-exported symbol emits nothing (D2 dedup, bounds .so growth, trace B). check_poly_body runs first, always: only a body that already type-checked can reach the multi-output gate, so `: twice ( 'T -- 'T 'T ) dup ;` (unbounded dup) fails check_poly_body itself and is X1, never reaching the gate despite its two outputs, while `: pair ( 'T: Copy -- 'T 'T ) dup ;` passes the body check and is the one X3 actually rejects; gating arity before the body check would make twice X3 instead and must not happen. Once the body checks clean, reject a poly def whose PolySig resolves to sig.outputs.len() >= 2 || sig.row_out.is_some() as a clean located deferral (R3/X3) -- a length variable alone never changes the output count, so it is not part of the trigger -- since REPL lowering interns no return bundle (word_ret_ty first-output-only, src/ir.rs:1841) and would otherwise silently truncate; this keeps every lowered instantiation single-output, no new lowering mechanism. Remove phase 1's rejection at the definition site; twice's rejection now changes to the real X1 diagnostic (naming 'T + the missing Copy bound), so that golden's expected text updates too. Exit: criteria 1, 2, X1, X2, X3 (criterion F upgraded to correct support), with the drop-overload goldens (tests/phase3_resources.rs) still green as Phase 3's own native-regression guard.",
      "effort": "high",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "Redefinition (D4, R8): redefining a poly word bumps its generation to one past whichever of self.env (ordinary WordEntry, src/repl.rs:96) or self.poly_words currently holds the name (a shared per-name counter so a mono<->poly redefinition cannot collide symbols), retains the new word plus a fresh resolver snapshot at the new generation, and leaves every old instantiation symbol resident (.so's are never dlclose'd) and resolvable. A new call site unifies against the new PolySig at the new generation, minting __gen{N} symbols distinct from the old (R2); no other word's symbol is restamped (contrast 8b's override_epoch blanket rule, src/repl.rs:297). This closes the brief's trace-C hazard: without the generation, re-instantiating a redefined word at a type it was already instantiated at would mint the old body's symbol and silently run it under RTLD_GLOBAL first-loaded-wins. Exit: criterion 3, a single-output frozen-vs-reject golden (the brief's 2-output `5` vs `5 5` witness is unbuildable at the REPL this slice because no return bundle is interned, R7 carve-out, and a same-arity 'T -- 'T body is provably the identity so it cannot be value-witnessed): define Spy (linear); `: id ( 'T -- 'T ) ;` gen0 unbounded; `: g ( -- ) 7 Spy id drop ;` binds id@Spy at gen0 (unbounded instantiates at a linear type); calling g prints `drop 7`; `: id ( 'T: Copy -- 'T ) ;` gen1 adds the Copy bound; calling g again still prints `drop 7` (frozen to gen0); a NEW `7 Spy id drop` line fails the Copy bound (X2's Ctx::Line message naming 'T, Spy, the linear reason), witnessing that gen1 governs new instantiations while gen0's compiled call is untouched. R2's symbol-distinctness is unit-pinned in phase 2 (Some(0) != Some(1)).",
      "effort": "medium",
      "difficulty": "hard"
    },
    {
      "phase": 5,
      "focus": "Consolidated exit golden and docs: add one tests/phase1.rs golden session covering the full ROADMAP exit sequence (define `: id ( 'T -- 'T ) ;`, instantiate at two different types on later lines, instantiate twice at the same type without recompiling, redefine, and see the new body take effect on the next call while an earlier line's call keeps the old one), asserting the transcript and the recon-1 empty-signature miscompile gone; note the slice's decisions in ROADMAP.md's Phase 4 Slice 2 entry and confirm DESIGN.md's REPL-late-binding deferral already cross-references this slice. Exit: criterion 4.",
      "effort": "low",
      "difficulty": "easy"
    }
  ]
}
```
