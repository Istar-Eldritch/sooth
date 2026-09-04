# P7b.S9 probes — verbatim log and verdict (recon round)

Probe round for [slice9-brief](./slice9-brief.md), run against the clean tree
(worktree `p7b-s9`, HEAD `600bc1b`, `git status --porcelain` empty apart from
the brief itself, verified after every instrumentation spike). Fixtures and
raw captures live under `/tmp/p7bs9-probes/` (`log.md`, `VERDICT.md`,
`p3-pb2.err`, `p3-mkvar.err` — 501/503 filtered trace lines — `p6-full.log`,
and fixture dirs `pb2/`, `mkvar/`, `third/`, `third2/`, `paper-fixtures/`).
This doc preserves the log and verdict verbatim; the raw `/tmp` captures are
ephemeral.

## Baseline (P6)

`cargo test --no-fail-fast` at HEAD `600bc1b`: **82 test binaries, 3150 tests
passed, 0 failed** (~40s). Full log: `/tmp/p7bs9-probes/p6-full.log`. **Round-1
review correction:** this run passed by luck — `tests/phase7b_slice4.rs:427-490`
is flaky (G2's same-shape twin — different trait and text — hard-pinned to the
pre-fix coin-flip) and reds
~3/8 on rerun; see the errata below and `slice9-spec.md`'s R-NFR5.

## Verbatim log

```text
=== HEAD ===
600bc1b139c4eb4a868d10f007deb3ea865f20c0
=== git status (must be empty) ===
?? docs/roadmap/P7b/slice9-brief.md
(empty above means clean)
=== cargo build ===
   Compiling sooth v0.0.0 (/root/code/ordfruma/sooth-worktrees/p7b-s9)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 5.57s
=== P6: baseline cargo test --no-fail-fast ===
=== P1: pb2 reproduction ===
--- run 1 ---
2
2
exit=0
--- run 2 ---
2
2
exit=0
--- run 3 ---
2
2
exit=0
=== P2: mk variant reproduction ===
--- run 1 ---
1
1
exit=0
--- run 2 ---
1
1
exit=0
--- run 3 ---
1
1
exit=0
=== P3: instrumented trace, pb2 ===
=== P3: instrumented trace, mk variant ===
=== P4: clean binary, 10x mkvar ===
2 2
2 2
1 1
1 1
1 1
1 1
2 2
1 1
2 2
1 1
=== P4: clean binary, 10x pb2 (control) ===
2 2
2 2
2 2
2 2
2 2
2 2
2 2
2 2
2 2
2 2
=== nm symbol table, mkvar binary ===
0000000000001e70 T show.3b.Show.3b.6.3b.isize__m6
0000000000001e50 T show.3b.Show.3b.6.3b.usize__m6
0000000000002240 T sooth_mono_render__m6__t0_isize
0000000000002290 T sooth_mono_render__m6__t0_usize
00000000000022e0 T sooth_mono_sized__m2__t0_Widget_i64_
00000000000022a0 T sooth_mono_size_Sized_2_Widget__T0___m3__t0_i64
00000000000022c0 T sooth_mono_size_Sized_2_Widget__T0___m4__t0_i64
=== P5a: third-module mono caller, bare Widget/size ===
2
exit=0
=== P5a-ii: third-module mono caller, explicit Widget[i64] annotation ===
error: unknown type `Widget` at line 5, col 13
exit=0
=== f.sth after double-match edit ===
import: intrinsics * ;
trait: Sized['S] : size ( 'S -- i64 ) ; ;
: sized['S: Sized] ( 'S -- i64 ) size ;
export: Sized sized ;
import: self::a | Widget | ;
impl: Sized for Widget
  : size drop 99 ;
;
=== P5b: double-match, re-run pb2-shape main against modified f ===
error: import cycle: `/tmp/p7bs9-probes/pb2/a.sth` imports `/tmp/p7bs9-probes/pb2/f.sth`, which (directly or transitively) imports `/tmp/p7bs9-probes/pb2/a.sth`
exit=0
=== P5b: double-match qualified target a::Widget, g imports a+f ===
error: `Widget` is not exported from module `a` at line 4, col 17
exit=0
=== P5b retry: a exports Widget, g imports a::Widget qualified, double impl ===
error: `impl: Sized for a::Widget` at line 4, col 1 must live in the module declaring `Sized` or the module declaring `a::Widget`
exit=0
=== re-confirm original pb2 (a.sth now also exports Widget, no g/main_g involved) unaffected ===
2
2
=== P5b (take 3): impl in trait's OWN module f2b targeting a2::Widget, dispatch from a third module c2 ===
error: `impl: Sized for a2::Widget` at line 4, col 1 must live in the module declaring `Sized` or the module declaring `a2::Widget`
exit=0
=== P5c: third module wildcard-imports both a::Widget and b::Widget, bare use ===
error: wildcard import of `run` (line 4, col 1) collides with the wildcard import of `run`
exit=0
=== P5c retry: selective-import both a::Widget and b::Widget (same bare name) ===
error: `Widget` is not exported from module `b` at line 4, col 19
exit=0
=== P5c retry 2: both export Widget, selective double-import same bare name ===
error: selective import of `Widget` from module `b` (line 4, col 19) collides with the selective import of `Widget` from module `a`
exit=0
=== sanity: original pb2/main.sth unaffected by b.sth's added export ===
2
2
=== final clean check ===
?? docs/roadmap/P7b/slice9-brief.md
(only the pre-existing untracked brief should show)
```

## Verdict (preserved from `/tmp/p7bs9-probes/VERDICT.md`)

Worktree `p7b-s9`, HEAD `600bc1b`. Repo clean throughout (`git status
--porcelain` shows only the pre-existing untracked `slice9-brief.md`; no
`src/`/`tests/` diff at any point, verified after every instrumentation spike).

### 1. P1/P2 actual outputs

- **P1 (`pb2`, verbatim S5 fixture):** `2\n2`, exit 0, **deterministic** (10/10
  runs, both instrumented and clean binaries).
- **P2 (`mk` variant, both modules' `run` calls `mk sized`):** **NOT**
  deterministic. Clean binary, 10 runs: 6× `1\n1`, 4× `2\n2`. This contradicts
  the roadmap/brief's assumed "silent `1 1`" — the actual behavior is a coin
  flip per process invocation.

### 2. Hypothesis adjudication

- **H1 (span-keyed dispatch memo) — LIVE**, and is the mechanism for the `mk`
  variant's nondeterminism. `nm` on the built `mk`-variant binary shows **one**
  `sized` specialization (`sooth_mono_sized__m2__t0_Widget_i64_`) but **two**
  `size` specializations (`..._m3...`, `..._m4...`). `sized`'s own
  monomorphization key is the grounded type's *rendered name* ("Widget_i64_"),
  which is identical for a's and b's distinct `StructId`s — so both callers'
  calls to `sized` share **one** compiled body. That body's single internal
  `size` call is wired via `trait_calls: HashMap<Span, String>` keyed on the
  `size` call's span *inside `sized`'s own source* (one fixed span,
  `poly.rs`'s `resolve_user_bound`, `trait_calls.insert(ob.span, symbol)`),
  invoked once per caller grounding — second write overwrites the first, and
  Rust's per-process randomized `HashMap` seed makes *which* caller's grounding
  gets walked/inserted last vary run to run. Decisive log lines: `p3-mkvar.err`
  shows two *correct*, distinct `resolve_user_bound` writes (`imp.module=3`/
  `_m3_` and `imp.module=4`/`_m4_`) for the same `ob.span`, yet only one `size`
  variant is actually reachable from the shared `sized` body at runtime, split
  6/4 across 10 runs.
- **H2 (operand provenance wrong) — LIVE**, and is the mechanism for `pb2`'s
  deterministic `2 2`. Decisive log line: `find_bound_impl entry ...
  ty=Struct(StructId(2), "Widget[i64]")` for **a::run's own dispatch** (span
  `module: 3`), followed by `operand provenance: ... gi=1 module=4` — a's own
  bare, unannotated `Widget` ctor call minted using **b's** generic header
  (`gi=1, module=4`), not a's own (`gi=0, module=3`). Root cause is upstream of
  any trait-impl matching: only ONE `Widget[i64]` `StructId` exists in the
  whole build (confirmed: b's and a's dispatch both reference the identical
  `StructId(2)`), because only b's `usesize` spells `Widget[i64]` explicitly
  (forcing an eager mint); a's bare, inferred `Widget` construction has no
  separate instantiation to mint against and silently reuses whichever one
  already exists in `env["Widget"]`. The `terms.rs` `select_overload` trace
  (tag `ctor-select`) never fired for `pb2` at all — meaning `select_overload`'s
  tier policy (S5's fix) never even runs here, because there is no 2-candidate
  collision to disambiguate; there is only ever one candidate. This is a
  *different*, earlier-stage bug than S5's own ctor-tier scope, confirmed by the
  `mk`-variant control: when **both** sides spell `Widget[i64]` explicitly (both
  `mk`s), `ctor-select` fires for both, and correctly picks tier-1 own-module in
  both cases (`caller_module=3 → pick module=3`, `caller_module=4 → pick
  module=4`) — so the ctor tier policy itself works once two candidates exist.
- **H3 (pattern resolution wrong) — RULED OUT.** Trace shows candidate patterns
  correctly split: `impl_idx=0` → `pattern_id=Some((false, 0, 3))` (a's own
  header), `impl_idx=1` → `pattern_id=Some((false, 1, 4))` (b's own header), in
  every run. `match_impl_target`'s `Generic` arm identity comparison is exactly
  as sound as the brief's static read (F1/F3) predicted.
- **H5 (member-word identity collapse downstream of correct dispatch) — LIVE,
  and is the *same underlying mechanism as H1*, not a distinct one.** The `nm`
  evidence (two `size` specializations exist, `sized` does not) shows the
  collapse happens at `sized`'s own monomorphization identity — a
  structural/rendered-name key, not a `(StructId, module)`-aware one — which is
  exactly H1's premise from the other direction. Treat H1/H5 as one finding in
  the spec, not two.

### Decisive trace excerpts (from `p3-pb2.err` / `p3-mkvar.err`)

pb2 — both callers, same `StructId`, b's provenance, only b's pattern matches:

```text
S9PROBE find_bound_impl entry trait=Sized span=Span { line: 7, col: 29, module: 3 } ty=Struct(StructId(2), "Widget[i64]")
S9PROBE   operand provenance: Struct(StructId(2),Widget[i64]) gi=1 module=4 args=[Int(IntType { bits: 64, signed: true })]
S9PROBE   candidate impl_idx=0 impl.module=3 pattern_id=Some((false, 0, 3)) matched=false
S9PROBE   candidate impl_idx=1 impl.module=4 pattern_id=Some((false, 1, 4)) matched=true
--
S9PROBE find_bound_impl entry trait=Sized span=Span { line: 7, col: 34, module: 4 } ty=Struct(StructId(2), "Widget[i64]")
S9PROBE   operand provenance: Struct(StructId(2),Widget[i64]) gi=1 module=4 args=[Int(IntType { bits: 64, signed: true })]
S9PROBE   candidate impl_idx=0 impl.module=3 pattern_id=Some((false, 0, 3)) matched=false
S9PROBE   candidate impl_idx=1 impl.module=4 pattern_id=Some((false, 1, 4)) matched=true
```

mk variant — dispatch correct per-operand (winners 0 and 1); the collapse is
downstream (`ctor-select` fires with both candidates and picks tier-1
correctly):

```text
S9PROBE ctor-select name=Widget span=Span { line: 7, col: 29, module: 3 } caller_module=3 candidates=[(3, "Widget[i64]__m3"), (4, "Widget[i64]__m4")] pick="module=3 symbol=Widget[i64]__m3"
S9PROBE find_bound_impl entry trait=Sized span=Span { line: 8, col: 25, module: 3 } ty=Struct(StructId(2), "Widget[i64]")
S9PROBE   operand provenance: Struct(StructId(2),Widget[i64]) gi=0 module=3 args=[Int(IntType { bits: 64, signed: true })]
S9PROBE   candidate impl_idx=0 impl.module=3 pattern_id=Some((false, 0, 3)) matched=true
S9PROBE   candidate impl_idx=1 impl.module=4 pattern_id=Some((false, 1, 4)) matched=false
S9PROBE   winner impl_idx=0 winner.module=3
--
S9PROBE ctor-select name=Widget span=Span { line: 7, col: 29, module: 4 } caller_module=4 candidates=[(3, "Widget[i64]__m3"), (4, "Widget[i64]__m4")] pick="module=4 symbol=Widget[i64]__m4"
S9PROBE find_bound_impl entry trait=Sized span=Span { line: 8, col: 25, module: 4 } ty=Struct(StructId(3), "Widget[i64]")
S9PROBE   operand provenance: Struct(StructId(3),Widget[i64]) gi=1 module=4 args=[Int(IntType { bits: 64, signed: true })]
S9PROBE   candidate impl_idx=0 impl.module=3 pattern_id=Some((false, 0, 3)) matched=false
S9PROBE   candidate impl_idx=1 impl.module=4 pattern_id=Some((false, 1, 4)) matched=true
S9PROBE   winner impl_idx=1 winner.module=4
```

### 3. P5 outcomes

- **P5a (third-module mono caller, no bound):** a bare `Widget size` call in a
  module importing both a and b compiles and dispatches silently to `2` (b's
  impl) — no diagnostic. Same root cause as H2 (single collapsed `StructId`).
  Explicitly naming `Widget[i64]` in a third module that never declared its own
  `type: Widget` header is a hard **`error: unknown type \`Widget\``** (a bare
  generic name is only visible via a declaring or importing module).
- **P5b (double-match / "real ambiguity" shape) — appears STRUCTURALLY
  UNCONSTRUCTIBLE under current rules**, not merely untested. Every attempt hit
  a guard before reaching dispatch:
  - `impl: Sized for a::Widget` written in a third module: rejected —
    `` error: `impl: Sized for a::Widget` ... must live in the module declaring
    `Sized` or the module declaring `a::Widget` `` (the existing placement rule,
    `declarations.rs:588`, works as documented).
  - Moving that second impl into the trait's *own* declaring module requires
    that module to `import: self::a` (to name `a::Widget`) — but `a` must
    already `import: self::f` (the trait's module) to declare its own
    `impl: Sized for Widget` in the first place, so `f` importing `a` is a
    **guaranteed import cycle** whenever both sides have their own impl.
    Verified directly: `error: import cycle: ... f.sth imports a.sth, which ...
    imports f.sth`. A cycle-free construction (trait module doesn't need the
    impl to reference the trait) still failed the same placement-rule check
    once the impl's target was qualified as `a2::Widget` from a module that
    isn't `a2`'s or the trait's own.
  - A third module wildcard- or selectively-importing **both** `a::Widget` and
    `b::Widget` (same bare name) is rejected outright by the **import system**,
    before any type-checking: `` error: selective import of `Widget` from
    module `b` ... collides with the selective import of `Widget` from module
    `a` ``.
  - **Implication for the spec (Q2/exit-criterion-3):** once
    `match_impl_target`'s identity comparison is fed a *correct* operand
    provenance (fixing H1/H2/H5), two distinct modules' same-shaped generic
    headers can never both match one concrete operand — their `(idx, module)`
    identities differ, and the import system prevents a caller from ever
    holding an operand whose header identity is ambiguous to it. The roadmap's
    exit criterion #3 ("a real ambiguity error, not a silent pick") may
    describe a scenario that cannot arise post-fix; the spec should either find
    a genuinely constructible ambiguity shape (candidate: a `for 'T` catch-all
    vs. a same-pattern duplicate — untested, likely a declaration-time
    duplicate-impl error rather than a dispatch-time one) or explicitly rule
    that no new ambiguity diagnostic is needed and drop/reword that exit
    criterion.
- **P5c:** not separately needed — P5b's last sub-attempt already answers "can
  a third module even construct/hold both types' values" (no: the import
  collision fires first).

### 4. Baseline

`cargo test --no-fail-fast` at HEAD `600bc1b`: **82 test binaries, 3150 tests
passed, 0 failed**, ~40s wall time. Full log: `/tmp/p7bs9-probes/p6-full.log`.
See the errata below: this run passed by luck, one test is flaky.

### 5. Check order

Not a *textual*/declaration-order effect — the nondeterminism is a `HashMap`
iteration-order effect (Rust's default `RandomState` reseeds per process),
which is exactly why the same binary flips between `1 1` and `2 2` across
repeated invocations with no source change. `pb2` (only one caller of `sized`
in the whole program) has no such race and is fully deterministic; the `mk`
variant (two callers grounding the same shared `sized` specialization) does.

## Errata (round-1 review)

Three corrections to the verbatim log/verdict above, left untouched as the
record of what the probe round observed and concluded at the time:

1. **The "P4: clean binary, 10x mkvar" block and §5's conclusion ("the same
   binary flips between `1 1` and `2 2` across repeated invocations with no
   source change") are mislabelled.** The series was rebuild+run cycles, not
   same-binary reruns — round-1 review built the mk-variant fixture directly
   and measured: one built binary is **stable** across 5 repeat runs (checked
   on two separately-built binaries, one landing `1\n1`, one landing `2\n2`,
   each stable on its own 5 reruns); only *rebuilding* flips the outcome
   (6/8 `1\n1`, 2/8 `2\n2` across 8 rebuilds). The nondeterminism is a
   build-time effect, confirmed identically by the paper round's "rebuild+run
   cycles" framing.
2. **Every exit code in this log for an error case reads `exit=0`; this is a
   harness artifact, not the compiler's behavior.** Round-1 review built the
   G3 duplicate-impl fixture directly: `sooth build main2.sth; echo $?` →
   `exit=1`. Treat every `exit=0` above an `error:` line in this log as `1`.
3. **The H1 mechanism interpretation (§2: `trait_calls.insert(ob.span,
   symbol)` as "last-writer-wins" across two groundings sharing one span) is
   superseded.** `trait_calls` is a `HashMap<Span, String>` created fresh per
   instantiation (`src/check/poly.rs:7235`) and moved onto that
   instantiation's own `CallInst` (`:7384`; field `CallInst.trait_calls`,
   `src/ast.rs:2808`) — the two decisive trace lines quoted above ("two
   *correct*, distinct `resolve_user_bound` writes—for the same `ob.span`")
   are exactly this: two separate writes into two separate maps, not one
   overwrite. The actual collapse is one stage later, in lowering's
   instantiation dedup (`src/ir/driver.rs:350-373`): a `HashSet<String>` keyed
   on `instantiation_symbol(&inst.callee, &inst.subst)`, iterating a
   randomized `HashMap`; `instantiation_symbol`'s `Type::Struct`/`Type::Enum`
   fall-through arm (`src/ast.rs:2886`) renders only the type's name, so both
   groundings mint the identical symbol and `HashSet::insert` keeps only the
   first `CallInst` reached — discarding the other grounding's `CallInst`,
   `trait_calls` map included, whole. See the adjudicated mechanism in
   [slice9-spec](./slice9-spec.md) and [slice9-brief](./slice9-brief.md)
   (V3/F5/F5b).

The nondeterminism story is now identical across every S9 doc: a build-time
effect, per-compilation flip, one binary stable on repeat runs.

## Phase-1 verdict (R1.0)

Phase-1 diagnosis gate for [slice9-spec](./slice9-spec.md) REQ-1/R1.0: pin why
a's own `Widget[i64]` ctor is absent from the candidate set at a's bare
`Widget` call in `pb2` — **absent-mint (VERDICT A)** vs
**present-but-filtered (VERDICT B)**. Method: print-only instrumentation
spike (`S9P1` `eprintln!` traces at `struct_generated_sigs`
(`src/check/declarations.rs:1824`), check.rs's env build and word loop, the
single-candidate arm and tier path (`src/check/terms.rs:932`/`:968-991`),
`mint_fallback_candidates`, `GenericTypes::instantiate_struct`
(`src/ast.rs:1310`), `resolve_user_bound`/`resolve_mono_member_call`
(around, never inside, the untouched matcher `match_impl_target`/
`find_bound_impl`), the `poly.insts.insert` site (`src/check/poly.rs:7374`),
the `trait_calls` write (`:8663`), and lowering's dedup + dispatch
(`src/ir/driver.rs:350-373`, `src/ir/func_builder/calls.rs:440`). Every edit
reverted before this section was written (see Commands).

**VERDICT: A — absent-mint.** a's own `Widget[i64]` grounding is **never
minted anywhere in the build**. Exactly one `Widget[i64]` instantiation exists
program-wide — b's, minted at *parse* time (the only `mint-struct` trace line,
emitted before check's env build; b's `usesize` spells `Widget[i64]`) — and
**both** bare ctor calls (a's and b's) select that single candidate through
the **single-candidate arm** (`terms.rs:932`). Nothing filters a's grounding:
the tier path (`select_overload`, `terms.rs:968-991`) never fires (0
`tier-select` lines) and `mint_fallback_candidates` never returns a Widget
candidate (0 non-empty `mint-fallback` lines). There is no a-provenance entry
to filter — not in the struct registry, not in `env["Widget"]`, not in
`module.instantiations`. This selects **R1.1a** (registration/application:
the bare-ctor path must mint/select the instantiation keyed to the caller's
own resolved header); R1.1b (operand normalization at the `find_bound_impl`
feed, `src/check/poly.rs:8235`) is moot — there is no a-side operand to
normalize: the operand reaching dispatch is b's mint, faithfully carrying
b's provenance `(gi=1, module=4)`, and the matcher dispatches it correctly
(only b's pattern matches).

### Discriminating evidence

Fixture `/tmp/p7bs9-phase1/pb2` (verbatim S5 pb2: f/a/b/main, unmodified;
`./main` → `2\n2`, exit 0). pb2 measured deterministic: 16/16 clean rebuilds
(`2\n2`) and 3/3 instrumented rebuilds — consistent with the spec's mechanism
section (the per-rebuild flip belongs to the `mk` variant / V3, not pb2).

1. **Mint list: exactly one `Widget[i64]` mint, b's header, parse-time.** The
   only `mint-struct` line in the whole build (one per
   `GenericTypes::instantiate_struct`):

   ```text
   S9P1 mint-struct gi=1 m=4 id=2 name=Widget[i64] args=[Int(IntType { bits: 64, signed: true })]
   ```

   Registry corroboration at lowering — a's header is `gi=0`/module 3, b's is
   `gi=1`/module 4, and the whole-program struct registry holds only b's mint
   (`Widget[i64]__m3` exists nowhere):

   ```text
   S9P1 lower-generic-headers gi=0 name=Widget m3
   S9P1 lower-generic-headers gi=1 name=Widget m4
   S9P1 lower-structs[2] name=Widget[i64]__m4 m4
   ```

   a's own impl member word exists as a word (`S9P1 word-check w20
   name=size;Sized;2;Widget['T0]__m3 m3 poly=true`) but is never
   monomorphized and never dispatched to — no `sooth_mono_size_..._m3__t0_i64`
   is minted in this build.

2. **env at check start: one candidate, b's.**

   ```text
   S9P1 env-build Widget: (sym=Widget[i64]__m4 m4)
   ```

3. **Each bare ctor call sees that single candidate and takes it in the
   single-candidate arm; the tier path never runs.** a::run is checked as w21
   (`Widget` at line 7, col 22, module 3); b::run as w24 (line 8, col 22,
   module 4):

   ```text
   S9P1 single-cand name=Widget span=(7,22,m3) fallback=false chosen=(sym=Widget[i64]__m4 m4)
   S9P1 single-cand name=Widget span=(8,22,m4) fallback=false chosen=(sym=Widget[i64]__m4 m4)
   ```

   Note even b's own bare ctor goes through the same env-single-candidate
   borrow — the eager spelling decides whose mint both callers get (the
   recon's mk-variant trace above shows the complementary case: with both
   sides spelled, two mints exist and `ctor-select` picks tier-1 correctly per
   caller — selection is fine once two candidates exist; a's candidate is
   simply never minted).

4. **Both dispatches therefore carry b's provenance and pick b's impl.** a's
   `sized` grounding (poly call at span (7,29,m3)) and b's mono member call
   (`size` inside `usesize`, span (7,34,m4)) both dispatch on the identical
   operand `Struct(StructId(2), "Widget[i64]")` — b's mint — and both select
   impl_idx=1 = b's impl:

   ```text
   S9P1 dispatch name=sized__m2 span=(7,29,m3) ty=Struct(StructId(2), "Widget[i64]")
   S9P1 dispatch winner impl_idx=1 impl_m=4 subst_ty=[(0, Int(IntType { bits: 64, signed: true }))]
   S9P1 mono-member-dispatch name=size span=(7,34,m4) operand=Struct(StructId(2), "Widget[i64]") impl_idx=1 impl_m=4
   ```

5. **The instantiation table has no a-provenance entry.**
   `module.instantiations` holds exactly one Widget-related record — a's
   `sized` CallInst, whose θ grounds `'S` to *b's* mint — plus b's mono
   member-call record; both key on `StructId(2)`:

   ```text
   S9P1 inst-insert span=(7,29,m3) callee=sized__m2 symbol=sooth_mono_sized__m2__t0_Widget_i64_ subst_ty=[(0, Struct(StructId(2), "Widget[i64]"))]
   S9P1 lower-insts span=(7,29,m3) callee=sized__m2 sym=sooth_mono_sized__m2__t0_Widget_i64_ trait_calls=[(3,34,m2)->sooth_mono_size_Sized_2_Widget__T0___m4__t0_i64]
   S9P1 inst-insert span=(7,34,m4) callee=size;Sized;2;Widget['T0]__m4 symbol=sooth_mono_size_Sized_2_Widget__T0___m4__t0_i64 subst_ty=[(0, Int(IntType { bits: 64, signed: true }))]
   ```

   Both callers lowered against b's size body: the `size` call inside f's
   `sized` dispatched via `S9P1 lower-dispatch span=(3,34,m2) ->
   sooth_mono_size_Sized_2_Widget__T0___m4__t0_i64`, and b's `usesize`
   recorded `S9P1 mono-member-record span=(7,34,m4) ->
   size;Sized;2;Widget['T0]__m4` (the `drop 2` impl). Hence `2\n2`.

6. **No dedup collision in pb2** (V3 is unreachable here): lowering's dedup
   kept `sized`'s single grounding (`S9P1 dedup callee=sized__m2 symbol=
   sooth_mono_sized__m2__t0_Widget_i64_ KEPT`; the only other dup-discarded
   lines are the unrelated `flush` monomorph). pb2's `2\n2` is the pure V2
   story.

### Fix site (selected)

**R1.1a** (registration/application): make the bare-ctor candidate
registration/application path ground at the caller's own resolved header —
mint (or select) the instantiation keyed to the caller's own header when none
exists for it — so a's call sees a's own `Widget[i64]` candidate. The
decisive selection point measured here is the **single-candidate arm**
(`src/check/terms.rs:932`, fed by the env built from `struct_generated_sigs`,
`src/check/declarations.rs:1824`), not the tier path
(`src/check/terms.rs:968-991`), which never runs in this shape.

### Commands run

```sh
cd /root/code/ordfruma/sooth-worktrees/p7b-s9
git status --porcelain        # clean pre-spike
cargo build                   # exit 0 (with S9P1 spike in src/)
cd /tmp/p7bs9-phase1/pb2
/root/code/ordfruma/sooth-worktrees/p7b-s9/target/debug/sooth build main.sth \
  >run.out 2>trace.err        # exit 0; ./main → 2\n2, exit 0
grep -c 'tier-select' trace.err    # 0
grep -c 'mint-fallback' trace.err  # 0  (mint_fallback_candidates never
                                   #  returned a Widget candidate)
grep 'mint-struct' trace.err       # the single line quoted in (1)
grep 'single-cand' trace.err       # the two lines quoted in (3)
cd /root/code/ordfruma/sooth-worktrees/p7b-s9
git checkout -- src/          # spike reverted
git status --porcelain        # empty (then only this file, post-append)
git diff --stat               # only docs/roadmap/P7b/slice9-probes.md
cargo build                   # exit 0 on the clean tree
```

Raw captures (ephemeral): `/tmp/p7bs9-phase1/pb2-trace.err` (234 lines),
`pb2-run.out`.
