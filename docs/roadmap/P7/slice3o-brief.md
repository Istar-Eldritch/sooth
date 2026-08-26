# P7.S3o brief — A bound on a poly combinator's own type variable has no dispatch mechanism

**Status: recon complete, ready for third spec attempt.** `reject_user_bound_on_combinator`
(`src/check/poly.rs:6129`) is a clean, correctly-worded diagnostic, not a bug — this is an
unimplemented feature, not a broken one. Two spec-review rounds found the resolution-key design
unsound; a third recon round (three parallel probes, August 2026) corrected the brief's central
latency claim, found a more fundamental blocker the prior rounds missed, and settled one open
item. The design question is now sharper and the path forward is clear. No spec exists yet.

## Purpose

This slice is a **hot-path optimization**. A combinator (an `inline` word) is spliced at its
call sites — no call frame, no indirection. When such a combinator's body calls a **bare trait
member** (`cmp` directly, not the exported `gt` wrapper), the bound on its own type variable
cannot dispatch today: `reject_user_bound_on_combinator` rejects it at the gate. The fallback
is to ship the combinator non-inline, paying a real call frame per instantiation — the shape
S3s chose for `mymax`/`mymax3` precisely to avoid this slice.

Bare trait member calls in combinators will be in the hot path of many programs: comparison
fused into a loop body, hashing into a fold, formatting into a scan. Each one forced non-inline
is a call frame the language's own design (the linear spine, the splice-time loop) exists to
eliminate. This slice closes that gap so the optimization is available, not just the fallback.

## Problem, confirmed live against current `main`

`reject_user_bound_on_combinator` rejects a `'T: TraitName` bound on a poly combinator's own
type variable before its body is ever checked (`src/check.rs:895`, inside the `is_combinator`
arm of the second pre-pass loop). The comment at the call site already states the reason
precisely: "the scratch records below are exactly why a user trait bound cannot ride a
combinator's own type variable — nothing here survives to carry a resolved obligation."

**Live repro, with the rejection bypassed** (probed in throwaway worktrees, not shipped):

```
import: intrinsics * ;
type: Point x i64 y i64 ;
trait: Show 'T show ( &'T -- ) ;
impl: Show for Point
  : show | p | p drop ;
;
: shows inline ( &'T: Show -- ) show ;
: main ( -- ) 1 2 Point |p| &p shows p drop ;
```

`cargo run -- build probe.sth` (with `reject_user_bound_on_combinator` bypassed) gives a clean,
located diagnostic — not an ICE, not a miscompile:

```
error: error: unknown word `show` in `shows` (line 7)
```

No panic, no wrong symbol linked. It fails legibly, but for the wrong reason: the message
reads like a typo, not "bound dispatch on a combinator is unsupported." `reject_user_bound_
on_combinator`'s own message is the honest one; this is what happens if that gate is removed
without also fixing the underlying gap.

## Three independent gaps behind the rejection

**Gap 1 — the standalone `i64`-stand-in check fails on its own, independent of any call
site.** `check_poly_combinator_standalone` (`src/check/poly.rs:361`) substitutes `i64` for `'T`
and routes the body through the ordinary concrete checker, which has zero trait-bound
awareness. `impl:` mints a trait member only under its mangled symbol
(`member;Trait;module;Type`), never the bare member name, so a bare `show` (or `cmp`) call
fails a plain `env.get` lookup regardless of what `i64` implements. Standalone checking, as it
exists today, structurally cannot validate a bound member call at all.

**Gap 2 — the splice site independently falls through to the same plain lookup.**
`check_poly_combinator_args` (`src/check/combinators.rs:571`), called from `inline_combinator`
(`src/check/combinators.rs:347`), computes a genuinely concrete `Subst` binding the combinator's
own type variables to the caller's type — confirmed live via a debug print at exactly that
point. Nothing reads it: `grep -c '\.bounds' src/check/combinators.rs` → `0`. Once the body is
spliced and checked via `check_terms_relaxed`, there is no branch anywhere in `TermKind::Call`'s
dispatch that consults `poly.trait_resolve` or anything resembling `poly_trait_member_call`
(`src/check/poly.rs:908`, the existing abstract bound-dispatch machinery a non-combinator poly
body's walk already uses). `check_terms_relaxed` treats the spliced-in `show` term identically
to a caller-written typo.

**Gap 3 — nothing ever records an obligation for a combinator's body in the first place.**
`TraitResolveCtx::obligations_of` (`src/check/poly.rs:107`) reads `self.recorded:
&[WordObligations]`, populated only by the non-combinator pre-pass loop
(`src/check.rs:793-838`), which explicitly skips every combinator
(`if is_combinator(word) { continue; }`) — indistinguishable, per that function's own
documented contract, from "this body calls no trait member."

## Existing precedent for the non-combinator case

The mechanism for an ordinary (non-combinator) poly word already exists end-to-end: abstract
recording during `check_poly_body`'s walk via `poly_trait_member_call`
(`src/check/poly.rs:908-1038`, pushes a `TraitObligation { span, var, trait_id, member }` with
no symbol, since `'T` is still abstract); concrete resolution per call site via
`resolve_user_bound` (`src/check/poly.rs:5313`, an `impl:` lookup plus a lowering-symbol
lookup, inserted into a `trait_calls: HashMap<Span, String>`); delivery to lowering via
`CallInst`'s other resolved-symbol tables, which survive into `module.instantiations`. A
combinator mints no `IrFunc` and is spliced by term substitution before lowering ever sees it,
so there is no `module.instantiations` entry for it to attach a `trait_calls` map to — the
analogous table has nowhere to live downstream the way it does for an ordinary poly word.

## Round 1 review: the uid-based key is unsound

A first spec draft closed gaps 1–3 with a narrow obligation-recording scan (mirroring
`poly_trait_member_call`, skipping the `i64` stand-in check only when the scan found a bound
member call) plus a resolution key of `(caller word symbol, splice uid, body span)`, on the
premise that the checker's and lowering's per-splice uid counters mint identical sequences per
word. Two reviewers independently disproved this with a live repro:

```sooth
: bump inline ( i64 -- i64 ) 1 add ;
: seed ( -- [ -- i64 ] ) [ 10 bump ] ;
: main ( -- ) 5 bump . seed call . ;
```

`bump` is spliced once directly in `main` and once inside a quotation literal that lowers as
its **own** `IrFunc` (`lower_materialized`, `src/ir/func_builder/mod.rs:1061`, with its own
`FuncBuilder` and `inline_uid: 0`). The checker walks that same body under the *enclosing
word's* `Provenance` instead — the two counters diverge by both the caller symbol
(`seed__m0` vs `seed__m0__quot0`) and the uid itself. Worse: an *annotated* quotation literal is
checked twice (once at the literal site, once at the materialization boundary), so the checker
can mint uid `0` for a splice *inside* the literal while lowering mints uid `0` for a *direct*
splice in the same caller at a different span. Make both splices the same bounded combinator at
two concrete types and the counters' lookup collision becomes a **silent dispatch to the wrong
`impl:`** — the exact soundness property the feature exists to guarantee. The miss case (falling
through to `self.env.get(name).expect("checked user word exists")`,
`src/ir/func_builder/calls.rs:728`) is the friendlier failure; it just ICEs.

The fix proposed and probed in response: replace the counter with a **`SplicePath`** — the
chain of enclosing combinator call-site `Span`s, outermost first, maintained as a push/pop stack
at the same two splice sites (`src/check/combinators.rs:505-507`,
`src/ir/func_builder/calls.rs:638-639`), with nothing counted so nothing can drift. Verified
sound against direct splices, transitive (combinator-inside-combinator) splices, and a
materialized literal in the *caller's* body — but only at one splice of the enclosing
combinator (see round 2).

## Round 2 review: the SplicePath key still collides, and R13's suppression breaks R7

Two more, independently-found holes, both reproduced live against `main`:

**The stand-in check's suppression breaks the rewrite it depends on.** The design skips
splice-site bound resolution under `check_poly_combinator_standalone`'s `i64` stand-in walk (to
avoid demanding `impl: Trait i64` for an instantiation that never happens), delivered by a
resolution channel that is `None` there. But an **unbounded** combinator whose body splices a
**bounded** one still runs through the stand-in walk (the skip is keyed on the *outer*
combinator's own — empty — obligation scan), and the `None` channel there suppresses not just
resolution but the term-rewrite that makes the bare member name resolvable at all. The result:
the inner splice's bare `show` call fails a plain `env.get` inside the *outer* combinator's
definition-time stand-in check — a hard rejection at exactly the site the design's own test
plan asserts must be clean. The skip needs to be transitive over the splice tree, not
per-combinator.

**The SplicePath key still collides — one splice-depth deeper than any probe checked.**
`lower_materialized` mints a fresh `FuncBuilder` (and the checker's materialization walk runs at
path depth 0) for *every* materialized quotation, dropping the enclosing splice-site prefix on
both sides. That's fine for exactly one splice of the enclosing combinator — every probe in the
round-1 fix only ever exercised one. Splice it twice and the collision reappears:

```sooth
import: intrinsics * ;
: bump inline ( i64 -- i64 ) 1 add ;
: mk   inline ( -- [ -- i64 ] ) [ 10 bump ] ;
: seedA ( -- [ -- i64 ] ) mk ;
: seedB ( -- [ -- i64 ] ) mk ;
: main ( -- ) seedA call . seedB call . ;
```

```
$ nm _probe2 | grep quot
00000000000011c0 T seedA__m0__quot0
0000000000001200 T seedB__m0__quot0
```

One source quotation literal (`[ 10 bump ]`, one span, one `mk`) mints two distinct `IrFunc`s
from two splices of `mk`. Make `mk` bounded and both splices would write the same collapsed
key with two different implementing symbols — round 1's failure mode, surviving the redesign.

> **Correction (round 3 recon):** the brief claimed this collision was *latent* behind an
> unrelated `x__inl0` materialization-boundary bug. That claim is **wrong**. The third recon
> round bypassed `reject_user_bound_on_combinator` and probed the bounded `mk` shape directly:
> `x__inl0` never fires. A more fundamental collision — the `i64` stand-in scratch
> instantiations clobbering real `i64` instantiations — is the actual blocker, and it is
> reachable through regular poly word calls, not just materialized quotations. See "Round 3
> recon" below.

Smaller findings from the same round, not independently blocking but indicative of how much is
still unsettled: the REPL rejection (mirroring the non-combinator case's REPL carve-out) was
placed at a site with the wrong caller for the scenario it's meant to cover; a resolution
channel's payload type was never pinned down across the two phases that would produce and
consume it; a generic body calling an *unbounded* combinator that itself splices a *bounded*
one isn't covered by the rejection meant to close that path. Two reviewers, working
independently, converged on the same root methodological gap: every design probe exercised
exactly one splice of the enclosing combinator, and the remaining hole in both the uid key and
the SplicePath key only appears at two.

## Round 3 recon — three parallel probes (August 2026)

Three probes ran in isolated worktrees, each bypassing `reject_user_bound_on_combinator`
temporarily (reverted, nothing committed). All three confirmed `reject_user_bound_on_combinator`
still fires for a bounded combinator, and that bypassing it alone does not ICE or panic — the
body fails a legible `env.get` lookup, confirming the feature is unimplemented, not broken.

### Probe 1 — the span-keyed `insts` collision (the real blocker, and pre-existing)

The brief's claim that the round-2 collision is gated behind the `x__inl0` materialization bug
is **wrong**. With the rejection bypassed, `x__inl0` never fires in any probe shape. Instead, a
more fundamental collision was found — and a follow-up probe pinned its precise root:

**`poly.insts` is keyed by `Span` (`src/check/poly.rs:4854`), but a spliced combinator body's
spans are not unique per instantiation context.** `inline_combinator` alpha-renames locals
only (`src/check/combinators.rs:504-506`) — the spliced terms keep their original body spans —
and re-walks the body with the real `PolyCtx` (`check_terms_relaxed`,
`src/check/combinators.rs:527`). Every splice of the same combinator then inserts its inner
poly calls' `CallInst`s at the *same* spans (`check_poly_call`'s `insts.insert(span, ...)`):
last write wins. Lowering reads `instantiations.get(&span)` (`src/ir/func_builder/calls.rs:344`)
and the driver monomorphizes one `IrFunc` per surviving `(callee, θ)`
(`src/ir/driver.rs:262-270`) — so with two splices at two types, all splices dispatch to one
monomorph and the other type's is never emitted.

With the rejection bypassed, inline `mymax3` works correctly for single-type usage
(i64-only: prints `9`; f64-only: prints `9`), but **miscompiles when both types are used**:
prints `5` instead of `9` for i64, and `gt__i64` vanishes from the binary — the f64 splice's
record is the last writer, so the i64 splice's `gt` call lowers to a call to `gt@f64` on i64
bits. Silent miscompile, not a legible error.

**The defect is not bound-specific and does not require the stand-in check.** An unbounded,
stock-compiler probe (`: pid ( 'T -- 'T ) |x| x ;` called from `: c inline ( 'T -- 'T ) pid ;`,
spliced at both `i64` and `f64`) reproduced it on unmodified `main`: only
`sooth_mono_pid__m0__t0_f64` was emitted, and `1 c .` printed `0`, not `1`. A poly combinator
calling a poly word at two concrete types miscompiles *today* — nothing in `lib`/`core`
reaches it (combinators call builtins, intrinsics, or quotation parameters, never plain poly
words), which is the only reason it has never bitten. The stand-in check is NOT a writer
here: the build path already isolates its `insts` into a scratch `HashMap`
(`src/check.rs:898-920`, comment at line 812: "records nothing that [survives]"), mirroring
the REPL path. The collision is purely between two real splices at two types.

This collision is reachable through **regular poly word calls** (`gt`), not just materialized
quotations — it is easier to reach than the brief suggested. It is the primary blocker for any
third spec attempt, and it is also a **live soundness hole in unmodified `main`** that warrants
a guard independent of this slice (see Open items).

### Probe 2 — option (b) is sound: reject materialized quotations

The round-2 SplicePath collision was found inside materialized quotations. Option (b) —
instead of prefixing the resolution key, **reject bound dispatch inside materialized quotations
within a bounded combinator** — is sound and does not block the motivating program.

Confirmed: `mymax`/`mymax3`'s `~[ ]` arms are spliced by `branch`/`lower_if` into basic blocks,
**never materialized** — zero `__quot` symbols in `nm` for both the current non-inline build
and the inline-flipped build. `branch`'s `~[ ]` arms are inline-only phantoms spliced via
`lower_if` (`src/ir/func_builder/calls.rs:547-554`), never materialized into separate `IrFunc`s.
Materialization is triggered only when a quotation escapes as a value (passed as an argument,
stored, returned), not by an inline combinator's `~[ ]` arm.

The case option (b) would reject — a bounded combinator whose body materializes a quotation
that dispatches a bound member — is currently unconstructible anyway (pre-existing restrictions
on quotation outputs and runtime quotation parameters gate it). When it becomes constructible,
rejecting it is correct: the materialized quotation gets its own `IrFunc` with no splice-site
prefix, so two splices of the enclosing combinator would collide on the same key. A located
error is the sound resolution, not a prefixed key.

### Probe 3 — the transitive skip and the real-word vs bare-member split

The motivating program calls `gt` (a real exported word), which **already works transitively**
with the gate bypassed — `gt` resolves via `check_poly_call`, not `env.get`. `3 7 outer .` →
`7`, exit 0. The gap only manifests for **bare trait members** (`cmp` directly): `unknown word
cmp in inner` during the inner combinator's standalone check.

`poly_trait_member_call` is called **only** in `poly_call_term` (the poly-body path); the
concrete/splice path (`check_terms_relaxed` in `terms.rs`) calls it **zero** times. Both the
standalone check and the splice site converge on `check_terms_relaxed`, which has no bound
dispatch.

The transitive skip itself is cheap — a `Provenance` flag (the threaded state that already
carries `inline_uid` and `self_tail_combinator` through both splice sites). But the real work
is **injecting bound dispatch into `check_terms_relaxed`**: threading `poly_trait_member_call`
plus the `sig`/`TraitCtx` needed to resolve the bound at the splice site. Transitivity is the
cheap part; resolvability is the hard part.

Two splices confirmed: same failure mode, no new failure at depth two for the bare-member case.
The two-splice discipline (round 2's methodological correction) is what found the `i64`
collision in probe 1 — testing with two concrete types is the shape that exposes it.

## What this does *not* touch, if picked back up

- `poly_trait_member_call`, `resolve_user_bound`, `CallInst`, and the whole non-combinator
  `trait_calls` mechanism (`P7.S3e`) would be reused, not modified.
- Ambiguity/disambiguation rules for two bounds sharing a member name (`P7.S3p`'s rulings)
  would be inherited as-is.
- `reject_user_bound_on_combinator`'s current message is correct and stays exactly as it is
  unless and until this is unblocked.

## Open design items (revised by round 3)

The open design work is not "choose an obligation-recording walk shape" (that part — a narrow
scan mirroring `poly_trait_member_call`, skipping the stand-in check only for a
member-dispatching body — is settled and was not where any review round found a hole). The
three items, as revised by the round-3 probes:

**Item 1 — fix the span-keyed `insts` collision (new, the primary blocker; reframed by
follow-up probe).** The defect is `insts: HashMap<Span, CallInst>` under splicing, not the
`i64` stand-in per se. Four fix shapes, in increasing invasiveness:

1a. **Overwrite-detector guard (safety net, should ship on `main` independent of this slice).**
In `check_poly_call`'s `insts.insert(span, ...)`, error when the insert would overwrite an
existing entry with a *different* θ (same span, different `callee`/`Subst`). Every program that
trips it miscompiles silently today, so the rejection is behavior-preserving on correct
programs; it converts the live soundness hole into a legible located diagnostic with one
golden test (the `c`/`pid` fixture). Small, local, no design surgery. Downgrade path once the
real fix lands: the guard's error site is exactly where the per-splice key arrives.

1b. **Isolate the stand-in check's `insts` — already done.** The build path already uses a
scratch `PolyCtx` with local `HashMap`s for the stand-in check (`src/check.rs:898-920`,
comment at line 812: "records nothing that [survives]"), mirroring the REPL path
(`check_poly_combinator_repl`, `src/check/poly.rs:431`). The stand-in's `insts` never touch
the production table. No work needed; the two-splice miscompile is purely between real
splices.

1c. **Per-splice instantiation records (the real fix).** Give a spliced body's inner poly
calls per-splice identity. Candidate shapes: (i) key `insts` by `(Span, SplicePath)` pushed at
the two splice sites that already exist (`src/check/combinators.rs`,
`src/ir/func_builder/calls.rs`) — this is round 2's SplicePath applied to the existing table,
and it inherits round 2's latent materialized-quotation hole *for ordinary poly calls* (option
(b) gates only bound members, not a `gt` inside a `[ ... ]` that escapes a combinator body);
(ii) mint fresh synthetic spans per splice copy — dies on round 1's check/lower
counter-divergence under materialization and two-pass literal checking; (iii) **splice log +
lowering-side instantiation**: the checker already computes θ at the splice site
(`check_poly_combinator_args` returns it) — log `(caller_span, comb_name, θ)` per splice, and
let *lowering* derive the spliced body's inner-call instantiations from θ plus the stack types
at the splice, mirroring how it already monomorphizes an ordinary poly body under a θ
(`concrete_effect` + body walk). Shape (iii) adds no new check/lower key-consistency
invariant — the splice `inline_uid` already threads both sides — and puts resolution where θ
is actually known; it is the most promising, but all three need the two-splice, two-type
oracle before any is trusted.

1d. **Materialized-quotation corner — CLOSED (probed August 2026).** The round-2 latent
hole (a plain poly call inside a quotation that escapes a combinator body into its own
`IrFunc` with fresh `FuncBuilder`/depth-0 path) is **not constructible today**. Three
independent gates block every path to it: (1) an audit gate rejects quotation types with
type variables as combinator outputs (`src/check/audits.rs:463`); (2) the `x__inl0` gate
rejects capturing a local across the materialization boundary; (3) the slice-7 gate rejects
`call` on a materialized runtime quotation (`src/check/terms.rs:1239`). Without type
variation, both splices produce the same `insts` entry (no collision). The non-capturing
fixed-type case compiles and works correctly (one monomorph, correct output). Two different
combinators at different types also work correctly (both monomorphs emitted, different spans).
The 1a overwrite-detector guard would also catch this case if any of the three gates are
lifted in the future — so the corner is gated-but-watched, not a blocker. No additional probe
is needed.

**Item 2 — reject materialized quotations (settled by probe 2).** Option (b) is sound: reject
bound dispatch inside materialized quotations from bounded combinators rather than prefixing
the resolution key. The motivating program is unblocked by design (its `~[ ]` arms are spliced,
not materialized — zero `__quot` symbols). The case the rejection targets is currently
unconstructible and would be correct to reject when constructible. This item is closed.

**Item 3 — transitive skip + dispatch injection (revised by probe 3).** The transitive skip is
cheap (a `Provenance` flag), but the real work is injecting bound dispatch into
`check_terms_relaxed`: threading `poly_trait_member_call` plus the `sig`/`TraitCtx` needed to
resolve the bound at the splice site. The motivating program (`gt` calls) already works without
this; the value is specifically for combinators calling bare trait members (`cmp`), which is the
hot-path case this slice exists to optimize. The skip must be transitive over the splice tree
(round 2's finding, confirmed by probe 3), not per-combinator.

**Methodological discipline (confirmed by all three probes).** Re-probe every design shape with
two splices of the enclosing combinator and two concrete types where one matches the `i64`
stand-in. That discipline is what found the `i64` collision (probe 1) and confirmed the
transitive failure shape is stable at depth two (probe 3). Do not assume two is the last depth
that matters until it's actually checked.

## Ready to spec: yes

P7.S3s (`Ord` as a library trait) is the concrete program this slice was parked waiting for — a
bounded `inline` comparison. S3s deliberately ships the six comparisons *non-inline* rather than
attempting this slice under its schedule pressure, which would bias the design toward a key that
works for `lt` rather than one sound in general (the shape of both prior failures). In exchange
S3s hands this slice the oracle neither review round had: a correct non-inline implementation to
differential-test against.

The oracle harness is in tree (`tests/phase7_slice3s_oracle.rs`): it builds
`examples/poly_if.sth` and diffs program output and resolved `impl:` symbols (`nm`/`objdump`)
per `mymax*` entry point. Until S3o lands there is no second variant, so it diffs the source
against itself — proving the plumbing works. S3o flips `mymax`/`mymax3` back to `inline` and the
harness gains a real second variant. The harness also notes `mymax` is never called from `main`
(only `mymax3`), so S3o needs a new fixture calling both words — the "at two splices, at three"
diff doesn't exist without it.

The entry conditions are met: the motivating program exists, the oracle harness exists, and the
round-3 recon has corrected the brief's latency claim, found the real blocker, and settled
option (b). The design question is now sharp enough to spec: solve the `i64` stand-in collision,
inject bound dispatch into the splice path, reject the materialized-quotation case, and
differential-test against the non-inline oracle at two splices and two types.
