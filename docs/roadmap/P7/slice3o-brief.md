# P7.S3o brief — A bound on a poly combinator's own type variable has no dispatch mechanism

## Problem, confirmed live against current `main` (`1bf977c`)

`reject_user_bound_on_combinator` (`src/check/poly.rs:5919`) rejects a `'T: TraitName` bound on
a poly combinator's own type variable before its body is ever checked
(`src/check.rs:867`, inside the `is_combinator` arm of the second pre-pass loop). The comment
at the call site (`src/check.rs:865-867`) already states the reason precisely: "the scratch
records below are exactly why a user trait bound cannot ride a combinator's own type
variable — nothing here survives to carry a resolved obligation." This brief's job was to
confirm that diagnosis empirically, and to determine what the real mechanism would need to be
— not yet recon'd anywhere in the roadmap (`docs/roadmap/P7-language-prereqs.md:610-613`).

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

## Two independent gaps, not one — both confirmed by isolation

**Gap 1 — the standalone `i64`-stand-in check fails on its own, independent of any call
site.** Removing the *only* call site to `shows` from `main` entirely (so the combinator is
declared but never spliced anywhere) reproduces the identical error at the identical line.
This proves the failure originates inside `check_poly_combinator_standalone`
(`src/check.rs:891`, `src/check/poly.rs:365`) by itself: it substitutes `i64` for `'T` and
routes the body through the ordinary concrete `check_word` → `check_terms_relaxed`, which has
zero trait-bound awareness. The reason is structural, not "`i64` doesn't implement `Show`" —
`impl:` mints a trait member only under its mangled symbol (`member;Trait;module;Type`,
`src/parser.rs:433`), never under the bare member name, so a bare `show` call fails a plain
`env.get` lookup regardless of what `i64` implements. Standalone checking, as it exists today,
structurally cannot validate a bound member call at all — not "the wrong instantiation
was chosen," but "the check performed has no concept of a bound member call."

**Gap 2 — even with gap 1 bypassed, the splice site independently falls through to the same
plain lookup.** With `check_poly_combinator_standalone` also bypassed (isolating the splice
path alone), the same `main` calling `shows` still fails with `unknown word 'show' in
'main'`. `check_poly_combinator_args` (`src/check/combinators.rs:571`), called from
`inline_combinator` (`src/check/combinators.rs:347`, at line ~372) right before the callee
body is spliced in, computes a genuinely concrete `Subst` unifying the combinator's own type
variables against the caller's live stack — confirmed live via a debug print at exactly that
point:

```
PROBE inline_combinator name=shows__m0 bounds=[(0, User(TraitId(2)))] subst.ty=[(0, Struct(StructId(0), "Point"))]
```

So the ingredients for resolution — `sig.bounds` and a concrete `Subst` binding the bounded
variable to `Point` — exist right there, at the right point, before the splice. Nothing reads
them: `grep -c '\.bounds' src/check/combinators.rs` → `0`. `poly_subst` is threaded only into
`back_edge_declared_shape` for the self-tail loop case (`combinators.rs:~490`), never into
anything bound-resolution-shaped. Once the body is spliced in and checked via
`check_terms_relaxed`, tracing `TermKind::Call`'s dispatch (`src/check/terms.rs:182` onward)
shows it checks locals, then `poly.combinators.get(name)`, then `poly.env.contains_key(name)`
(→ `check_poly_call`, reachable only for a directly-named poly *word*, not a spliced-in term),
then falls to a bare `env.get(name)`. There is no branch anywhere in that dispatch that
consults `poly.trait_resolve` or anything resembling `poly_trait_member_call`
(`src/check/poly.rs:904`, the existing abstract bound-dispatch machinery a non-combinator
poly body's walk already uses). `check_terms_relaxed` has no way to know a spliced-in term
came from a bound combinator body — it is the same concrete term-checker a fully monomorphic
call site uses, and treats the spliced-in `show` term identically to a caller-written typo.

**A third, prior gap makes both of the above moot until it is fixed: nothing ever records an
obligation for a combinator's body in the first place.** `TraitResolveCtx::obligations_of`
(`src/check/poly.rs:107`) reads `self.recorded: &[WordObligations]`, populated only by the
non-combinator pre-pass loop (`src/check.rs:793-838`), which explicitly skips every
combinator (`src/check.rs:794-796`: `if is_combinator(word) { continue; }`). So even granting
gap 1 and gap 2 a fix, `obligations_of("shows", sig)` returns `&[]` for any combinator today —
indistinguishable from "this combinator's body calls no trait member," per that function's
own documented contract (`poly.rs:103-106`). There is nothing recorded to resolve yet.

## Existing precedent for the non-combinator case (what a combinator's fix must mirror, not reinvent)

The mechanism for an ordinary (non-combinator) poly word already exists end-to-end and is the
right template, confirmed by reading it directly rather than re-deriving it:

- **Abstract recording, at declaration-check time.** `poly_trait_member_call`
  (`src/check/poly.rs:904-1038`) is the bound-directed dispatch branch `check_poly_body`'s walk
  consults ahead of ordinary dispatch: given a bare member name, it searches the word's own
  `sig.bounds` for a `Bound::User` declaring that member (with the P7.S3p disambiguation rules
  for a shared member name across two bounds), type-checks the member call *abstractly* against
  the trait member's declared signature (never against a concrete stand-in type), and pushes a
  `TraitObligation { span, var, trait_id, member }` — no symbol, since `'T` is still abstract
  here (`poly.rs:1034-1039`). This is exactly the abstract check a combinator's raw body would
  need too, and it does *not* require instantiating the combinator's `'T` at any concrete
  stand-in (`i64` or otherwise) to run — it checks the member call against the trait's own
  declared abstract signature, sidestepping gap 1 entirely for the spans it covers.
- **Concrete resolution, at each call site.** `check_poly_call` (`src/check/poly.rs:4542`,
  around `:2180-2183`) reads the recorded obligations for the callee it just resolved a
  concrete `θ` for, and calls `resolve_user_bound` (`src/check/poly.rs:5137-5196`) once per
  bound variable: an `impl:` registry lookup (linear scan, `tr.impls.iter().find(...)`) keyed
  on `(trait_id, concrete_ty)`, then, for every obligation on that variable, a lookup of the
  implementing word's *lowering symbol* (`tr.word_symbols`), inserted into a
  `trait_calls: HashMap<Span, String>` keyed by the *body's own* call-site span
  (`poly.rs:5192`). This is a synchronous, immediate resolution once a concrete type is known
  — there is no deferred "resolve later" step; `resolve_user_bound` is called exactly once per
  bound variable, right when the concrete type becomes available.
- **Delivery to lowering without touching the shared term-checker.** The `trait_calls` map
  produced above is exactly analogous to `CallInst`'s other resolved-symbol tables
  (`resolved_fields`, `builtin_overloads`) that `check_poly_call` already produces for a
  non-combinator poly word and that survive into `module.instantiations` for lowering to read.
  A combinator mints no `IrFunc` and is spliced by *term substitution* before lowering ever
  sees it (`src/check.rs:868-877`'s own comment: "It mints no `IrFunc` (R20): a call to it is
  inlined by term-splice at its concrete call sites"), so there is no `module.instantiations`
  entry for a combinator to attach a `trait_calls` map to at all — the analogous table has
  nowhere to live downstream of lowering the way it does for an ordinary poly word.

## What the fix needs, given the above — and what is still an open design question

Three things, in order, mirroring the existing mechanism as closely as the combinator's
different shape (spliced by term substitution, never lowered as its own `IrFunc`) allows:

1. **Record an obligation list for each combinator's raw, unsubstituted body**, the way the
   pre-pass loop already does for every non-combinator poly word (`src/check.rs:793-838`), so
   `trait_obligations` (`src/check.rs:786`) carries an entry for a combinator too and
   `obligations_of` stops returning `&[]` for one. **Open design question, not resolved here:**
   the non-combinator pre-pass gets this almost for free because `check_poly_body` already
   walks the whole body doing full abstract stack-effect checking, of which
   `poly_trait_member_call` is one branch. `check_poly_combinator_standalone` deliberately
   avoids that whole apparatus — the `i64`-stand-in trick exists specifically so a combinator's
   body (which may contain `call`/`times` on an *abstract* declared quotation parameter, R8/R9)
   does not need the full poly-body abstract-effect machinery duplicated for combinators. Two
   candidate shapes for the missing walk, not chosen between here:
   - **(a)** Give combinators the full `check_poly_body`-style abstract walk too, replacing
     `check_poly_combinator_standalone`'s `i64`-stand-in check outright rather than running
     alongside it. This is the most uniform option but is exactly the substantial rework
     `check_poly_combinator_standalone`'s own doc comment (`poly.rs:352-358`) explains the
     `i64` stand-in was chosen to avoid, and risks reopening whatever originally motivated
     that avoidance (not re-investigated here — out of this brief's probe budget).
   - **(b)** A narrower walk that does *only* trait-obligation recording — reusing
     `poly_trait_member_call`'s bound-search and obligation-push logic, but skipping (or
     stubbing) the operand-shape/underflow checks that logic also performs, since those still
     need a real abstract stack the narrow walk would have to build just for this purpose.
     Smaller in scope than (a) but a new, bespoke walk rather than a reuse of an existing one
     — the risk is drift between this walk's notion of "which calls are trait-member calls"
     and `poly_trait_member_call`'s own, if the two are not kept in lockstep.
   Spec must choose between these (or a third option) with a concrete recon pass over
   `check_poly_combinator_standalone`'s existing responsibilities — this brief only establishes
   that *something* must fill this gap, not which shape it takes.
2. **Resolve at the splice site.** In `inline_combinator` (`src/check/combinators.rs:347`),
   immediately after `check_poly_combinator_args` returns `poly_subst` (`combinators.rs:~372`),
   for each `(v, Bound::User(tid))` in `comb.word.poly.as_ref().unwrap().bounds`, call
   `resolve_user_bound` (`poly.rs:5137`) exactly as `check_poly_call` already does — passing the
   `Subst`'s binding for `v` as the concrete type and `poly.trait_resolve` as the registry —
   producing a `trait_calls: HashMap<Span, String>` keyed by the combinator body's own
   (pre-rename) spans. This half is comparatively mechanical: `resolve_user_bound` already
   exists, is generic over its caller, and needs no changes.
3. **Deliver the resolution into the spliced term stream without widening the shared
   term-checker.** Before `alpha_rename_locals(comb.terms, uid)` (`combinators.rs`, just after
   the splice's `poly_subst` computation), rewrite any `TermKind::Call(member)` term whose span
   is a key in the `trait_calls` map from step 2 to `TermKind::Call(resolved_symbol)` — i.e.
   pre-substitute the term to the already-mangled concrete word name (e.g.
   `show;Show;0;Point`) before the body is spliced into the caller. That mangled name is a
   real word already present in `env` (the impl's synthesized member, minted by
   `src/parser.rs:433`), so `check_term`'s existing plain `env.get` dispatch
   (`src/check/terms.rs:182`) then succeeds with **zero changes to `terms.rs` or `PolyCtx`**.
   An alternative considered and not preferred: a new `PolyCtx`-carried side table consulted
   inside `check_term`'s `Call` arm — functionally equivalent, but it touches the shared
   term-checker every other call site also runs through, where the term-rewrite approach stays
   entirely localized to the combinator splice path.

## What this does *not* touch

- `poly_trait_member_call` itself, `resolve_user_bound` itself, and the whole non-combinator
  `CallInst`/`trait_calls` mechanism (`P7.S3e`) are unmodified — this slice reuses them, adds
  no new field to `CallInst`, and mints no new representation for a resolved trait dispatch.
- Ambiguity/disambiguation rules for two bounds sharing a member name (`P7.S3p`'s rulings)
  are inherited as-is via whichever walk step 1 lands on; nothing here revisits them.
- `reject_user_bound_on_combinator`'s current located-rejection *message* is correct today and
  is exactly the diagnostic this slice removes the need for — it is not "wrong," it is the
  honest statement of the gap this brief closes.
- No new trait-declaration or `impl:` syntax; the trait/impl side of S3e is untouched.

## Exit criteria

- A poly combinator's own type variable may carry a `'T: TraitName` bound
  (`reject_user_bound_on_combinator`'s call site is removed or narrowed to cases genuinely
  still unsupported, whichever the spec's chosen mechanism leaves outstanding).
- A call to a bounded trait member inside such a combinator's body, spliced at a concrete call
  site, resolves to the correct implementing word for that call site's concrete type — proven
  by a test with **two different concrete instantiations of the same combinator call site**
  (mirroring `a_bounded_call_inside_a_combinator_body_resolves`, `poly.rs:7385`, which already
  does this for the non-combinator case) each dispatching to a distinct implementing word.
- An unsatisfied bound at a splice site (no matching `impl:` for the concrete type reached) is
  a located rejection naming the missing word and the trait, mirroring
  `unsatisfied_user_bound_error`'s existing wording for the non-combinator case — not a silent
  miss, not the misleading `unknown word` message this brief's probe observed today.
- `check_poly_combinator_standalone`'s own check no longer fails on a bound member call
  regardless of whether the combinator is ever called from anywhere (today's gap 1) — a
  declared-but-uncalled bound combinator must not error at all, since there is no concrete type
  yet to check the bound against.
- The probe program in this brief (`shows`/`Show`/`Point`) builds and runs, printing whatever
  its `show` impl prints, as a golden.

## Sizing

Not phased in this brief — the split (if any) depends entirely on which shape spec chooses for
item 1's obligation-recording walk, which is the one piece of real design work here. If spec
picks option (b) (the narrow walk), this is plausibly a single phase: the narrow walk, the
splice-site resolution call, and the term-rewrite delivery are each individually small and
mechanical once (b)'s shape is fixed. If spec picks option (a) (folding combinators into the
full `check_poly_body` abstract-walk machinery), that is likely to be phase-1-sized on its own,
with the splice-site resolution and term-rewrite as phase 2 — but that split is spec's call to
make once it has actually recon'd what replacing `check_poly_combinator_standalone`'s
`i64`-stand-in trick with a full abstract walk costs elsewhere (e.g. whether the `call`/`times`
abstract-quotation-parameter checks that trick was built to dodge would need re-deriving for
combinators specifically).

## Ready to spec: yes, with one instruction for spec-writer

Recon item 1's two candidate shapes — (a) full abstract walk vs. (b) narrow obligation-only
walk — before locking the plan; this brief deliberately leaves that choice open rather than
guessing. Everything else (splice-site resolution via `resolve_user_bound`, term-rewrite
delivery before `alpha_rename_locals`) is concrete enough to spec directly. Verify all line
citations above against live `main` first — several other P7 slices are landing in parallel
and line numbers in `check.rs`/`poly.rs`/`combinators.rs` will have drifted.
