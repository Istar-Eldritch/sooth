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
passed, 0 failed** (~40s). Full log: `/tmp/p7bs9-probes/p6-full.log`.

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

### 5. Check order

Not a *textual*/declaration-order effect — the nondeterminism is a `HashMap`
iteration-order effect (Rust's default `RandomState` reseeds per process),
which is exactly why the same binary flips between `1 1` and `2 2` across
repeated invocations with no source change. `pb2` (only one caller of `sized`
in the whole program) has no such race and is fully deterministic; the `mk`
variant (two callers grounding the same shared `sized` specialization) does.
