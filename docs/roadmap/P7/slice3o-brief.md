# P7.S3o brief — A bound on a poly combinator's own type variable has no dispatch mechanism

**Status: parked.** `reject_user_bound_on_combinator` (`src/check/poly.rs:5919`) is a clean,
correctly-worded diagnostic, not a bug — this is an unimplemented feature, not a broken one.
Two spec-review rounds (below) found the real mechanism needs a source-derived resolution key
threaded through both the checker and lowering, a transitive "skip the stand-in check" rule,
and a fix for a materialized-quotation key collision — core inlining-machinery surgery for a
narrow feature already scope-cut once before (S3e R9/R17). Not worth forcing green right now;
this brief stands as the recon record if it's ever picked back up. No spec exists for this
slice.

## Problem, confirmed live against current `main`

`reject_user_bound_on_combinator` rejects a `'T: TraitName` bound on a poly combinator's own
type variable before its body is ever checked (`src/check.rs:867`, inside the `is_combinator`
arm of the second pre-pass loop). The comment at the call site (`src/check.rs:865-867`) already
states the reason precisely: "the scratch records below are exactly why a user trait bound
cannot ride a combinator's own type variable — nothing here survives to carry a resolved
obligation."

**Live repro, with the rejection bypassed** (probed in a throwaway scratch worktree, not
shipped anywhere):

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

`cargo run -- build probe.sth` gives a clean, located diagnostic — not an ICE, not a
miscompile:

```
error: error: unknown word `show` in `shows` (line 7)
```

No panic, no wrong symbol linked. It fails legibly, but for the wrong reason: the message
reads like a typo, not "bound dispatch on a combinator is unsupported." `reject_user_bound_
on_combinator`'s own message is the honest one; this is what happens if that gate is removed
without also fixing the underlying gap.

## Three independent gaps behind the rejection

**Gap 1 — the standalone `i64`-stand-in check fails on its own, independent of any call
site.** `check_poly_combinator_standalone` (`src/check/poly.rs:365`) substitutes `i64` for `'T`
and routes the body through the ordinary concrete checker, which has zero trait-bound
awareness. `impl:` mints a trait member only under its mangled symbol
(`member;Trait;module;Type`), never the bare member name, so a bare `show` call fails a plain
`env.get` lookup regardless of what `i64` implements. Standalone checking, as it exists today,
structurally cannot validate a bound member call at all.

**Gap 2 — the splice site independently falls through to the same plain lookup.**
`check_poly_combinator_args` (`src/check/combinators.rs:571`), called from `inline_combinator`
(`src/check/combinators.rs:347`), computes a genuinely concrete `Subst` binding the combinator's
own type variables to the caller's type — confirmed live via a debug print at exactly that
point. Nothing reads it: `grep -c '\.bounds' src/check/combinators.rs` → `0`. Once the body is
spliced and checked via `check_terms_relaxed`, there is no branch anywhere in `TermKind::Call`'s
dispatch that consults `poly.trait_resolve` or anything resembling `poly_trait_member_call`
(`src/check/poly.rs:904`, the existing abstract bound-dispatch machinery a non-combinator poly
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
(`src/check/poly.rs:904-1038`, pushes a `TraitObligation { span, var, trait_id, member }` with
no symbol, since `'T` is still abstract); concrete resolution per call site via
`resolve_user_bound` (`src/check/poly.rs:5137-5196`, an `impl:` lookup plus a lowering-symbol
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
its **own** `IrFunc` (`lower_materialized`, `src/ir/func_builder/mod.rs:925`, with its own
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
materialized literal in the *caller's* body.

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
This is currently *latent*: reaching it needs a quotation inside a combinator body to capture a
`'T` local, which fails today for an unrelated, pre-existing reason (`unknown word x__inl0`
across the materialization boundary) — so the hole is real but accidentally gated, not yet
constructible.

Smaller findings from the same round, not independently blocking but indicative of how much is
still unsettled: the REPL rejection (mirroring the non-combinator case's REPL carve-out) was
placed at a site with the wrong caller for the scenario it's meant to cover; a resolution
channel's payload type was never pinned down across the two phases that would produce and
consume it; a generic body calling an *unbounded* combinator that itself splices a *bounded*
one isn't covered by the rejection meant to close that path. Two reviewers, working
independently, converged on the same root methodological gap: every design probe exercised
exactly one splice of the enclosing combinator, and the remaining hole in both the uid key and
the SplicePath key only appears at two.

## What this does *not* touch, if picked back up

- `poly_trait_member_call`, `resolve_user_bound`, `CallInst`, and the whole non-combinator
  `trait_calls` mechanism (`P7.S3e`) would be reused, not modified.
- Ambiguity/disambiguation rules for two bounds sharing a member name (`P7.S3p`'s rulings)
  would be inherited as-is.
- `reject_user_bound_on_combinator`'s current message is correct and stays exactly as it is
  unless and until this is unblocked.

## If this is ever picked back up

The open design work is not "choose an obligation-recording walk shape" (that part — a narrow
scan mirroring `poly_trait_member_call`, skipping the stand-in check only for a
member-dispatching body — is settled and was not where either review round found a hole). It
is: (1) make the stand-in-check skip transitive over the splice tree, not keyed on a single
combinator's own scan result; (2) either prefix a materialized quotation's resolution key with
the splice path in force at its interning site on both the checker and lowering side, or give it
an explicit located rejection with a test, rather than leaving the collision latent behind an
unrelated bug; (3) re-probe every design shape with **two** splices of the enclosing combinator,
not one — that discipline is what round 2 found round 1 missing, and there is no reason to
believe two is the last depth that matters until it's actually checked.

## Ready to spec: no, park it

Recommend leaving `reject_user_bound_on_combinator` as-is and moving on to other P7 backlog
items. Revisit only if a concrete program actually needs bound dispatch on a combinator's own
type variable — at which point start from "If this is ever picked back up" above rather than
from a spec draft, since the last two drafts were both found unsound in review.
