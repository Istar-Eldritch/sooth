# Phase 4 Slice 2 — REPL monomorphization (brief)

Slice 1 landed type/row/length variables and native monomorphization, deliberately
native only. This slice makes the REPL see polymorphic words. ROADMAP.md frames it as
one problem, retention: `Session` keeps signatures but discards word *bodies* once a
line compiles to a `.so`, and a polymorphic word has no concrete instantiation to
compile at its defining line. That framing is right but incomplete. Three things the
roadmap does not mention fall out of tracing actual sessions, and one of them is a
silent wrong-code hazard.

## Recon: what already exists (measured, not assumed)

**1. The REPL does not reject polymorphic words. It silently mis-checks them.**
`word.effect` is deliberately left empty for a polymorphic word (the signature lives
entirely in `word.poly`, `src/ast.rs:367-381`), and the REPL's entry point
(`check_def` -> `check_word`, `src/check.rs:1775`/`:2505`) never looks at `word.poly`
at all. Only the native `check(module)` (`src/check.rs:1015-1070`) builds a `poly_env`
and dispatches to `check_poly_body`. So the REPL checks a polymorphic body against a
zero-arity `Sig` derived from the empty effect:

```
> : twice ( 'T -- 'T 'T ) dup ;
error: stack effect mismatch in `twice` (line 1)
  `dup` needs 1 values, but the stack holds 0
  note: declared ( -- )
```

The declared effect is reported as `( -- )`, which is not what the user wrote. A body
that happens to touch nothing is worse: `: id ( 'T -- 'T ) ;` reports `defined id`,
enters `env` with an empty signature, and `5 id .` prints `5` purely because the
compiled body is a no-op and the checker never modelled the call at all. This is not a
regression (Slice 1 scoped itself native-only on purpose), but the starting state is
"silently wrong", not "cleanly unsupported", which the spec should say out loud.

**2. Span-keyed instantiation tables cannot cross REPL lines.** The native table is
`HashMap<Span, CallInst>` (`src/ast.rs:46`, filled at `src/check.rs:3313`), and `Span`
is `{ line, col }` (`src/ast.rs:5-8`) with no session-line origin. REPL line numbering
restarts every line: two definitions typed on successive lines both report `(line 1)`.
So the table must be built fresh per line and lowered into that line's `.so`. That is
consistent with how the REPL already rebuilds registries per line, but it means the
session, not the table, has to own any cross-line memory of what was instantiated.

**3. The instantiation symbol carries no generation, and that breaks under
redefinition.** `instantiation_symbol` (`src/ast.rs:483`) is a pure function of word
name and substitution, deliberately, so the checker's table and the lowered `IrFunc`
name are minted from one source of truth and can never disagree. Natively that is a
virtue and it dedups correctly: `: main ( -- ) 5 id . 7 id . ;` emits exactly one
`sooth_mono_id__t0_i64`. At the REPL it is a hazard, because each line is its own
module and its own `.so`, loaded `RTLD_GLOBAL`, where the first-loaded definition of a
symbol wins (`src/repl.rs:495`). Ordinary words dodge this with `mangled_symbol`'s
`__gen{N}` (`src/repl.rs:114`); 8b's destructors dodge it with an epoch suffix. A
polymorphic instantiation has neither.

**4. A word body binds its callees' generations at its own defining line, forever.**
Verified:

```
> : f ( -- i64 ) 1 ;
> : g ( -- i64 ) f ;
> : f ( -- i64 ) 99 ;
> g .
1
```

`g` keeps calling the `f` it was defined against. This is the semantics a polymorphic
word's instantiation has to match, and it is the hard part: a polymorphic body is
lowered *later*, at each instantiating line, by which time the session's resolver has
moved on.

## Three traced sessions

**A. Define once, instantiate at two types.**

```
> : id ( 'T -- 'T ) ;
> 5 id .
> "hi" id .
```

Line 2 must find `id` is polymorphic (it is not in the concrete `env`), unify `'T` with
the carried stack, retrieve the retained body, lower `sooth_mono_id__t0_i64` into line
2's `.so`, and bind the call site to it. Line 3 repeats at `str`. Requires: retained
bodies, a session-level `poly_env`, a per-line instantiation table, and lowering of
instantiations into the calling line's module.

**B. The same instantiation on two lines.**

```
> : id ( 'T -- 'T ) ;
> 5 id .
> 7 id .
```

Line 3 needs `id@i64` again. Either re-lower it into line 3's `.so`, minting a second
global `sooth_mono_id__t0_i64` (harmless only because the body is identical, and it
grows a `.so` per repeat for the life of the session), or record instantiations in the
session and resolve line 3's call to the symbol line 2 already exported, lowering
nothing. The second is the same shape as the native dedup and is the recommendation.

**C. Redefinition, which is where it actually breaks.**

```
> : id ( 'T -- 'T ) ;
> 5 id .
> : id ( 'T -- 'T ) dup drop ;
> 7 id .
```

Line 4 instantiates the *new* `id` at `i64`. Because the symbol is a pure function of
(name, subst), it mints `sooth_mono_id__t0_i64`, the symbol line 2 already exported
from the *old* body. Under `RTLD_GLOBAL` the first-loaded wins, so line 4 silently runs
the old body. Silent wrong code, not a crash, which is the worst class. Note the
interaction with trace B: any cross-line instantiation cache must be keyed by
(name, generation, subst), not (name, subst), or the cache reintroduces the same bug by
a different route.

## Decisions the spec has to make

1. **Symbol identity.** The instantiation symbol needs a generation component at the
   REPL without losing the single-source-of-truth property that keeps the checker's
   table and the lowered name in agreement. Recommendation: thread the defining word's
   generation into `instantiation_symbol` as an explicit parameter (`None` natively),
   so both sides still mint it from one function.

2. **Instantiation retention and dedup.** Session-level map keyed by
   (word name, generation, substitution) to an already-exported symbol, checked before
   lowering. Answers trace B and bounds `.so` growth.

3. **Which env an instantiation binds.** To preserve recon 4's semantics, an
   instantiation lowered at line N must bind the callee generations current at the
   word's *defining* line, so the session must retain the resolver snapshot per
   polymorphic word, not just the body AST. The alternative (bind at the instantiating
   line) is less code and observably wrong: a body would change meaning when an
   unrelated later line redefines a callee. This is the sharper form of 8b's stale-env
   hazard: 8b could cache a check result to avoid re-checking an old body, but a
   polymorphic body *must* be re-lowered per instantiation, so the question cannot be
   cached away. This is a scoped instance of a larger, deliberately deferred question
   (whether the REPL should ever be late-bound on redefinition at all, see DESIGN.md's
   Open / deferred: REPL late binding): binding at the defining line keeps polymorphic
   words consistent with how every ordinary REPL word already behaves, which is the
   reason to pick it here, not merely that it is less code.

4. **Redefinition invalidation scope.** 8b restamps every linear type on any override
   change, because destructors are woven pervasively. Polymorphic instantiations are
   not, so the narrower ordinary-word rule (bump the generation, leave old symbols
   resident and resolvable, bind new calls to the new generation) is the right
   precedent. The spec should say which it picks and why, rather than inheriting 8b's
   heavier rule by reflex.

5. **Failure mode if the slice has to split.** Given recon 1, the fallback is a clean
   located rejection of polymorphic definitions at the REPL, mirroring the poly-`if`
   rejection Slice 1 shipped. Worth specifying as its own criterion so the tree is
   never left in the current silently-wrong state, whatever else lands.

## Scope

In: polymorphic word definitions at the REPL, instantiation at call sites on later
lines, redefinition, cross-line dedup, and the diagnostics for all of it. Out:
quotations and combinators (slices 4 and 5), generic `type:` declarations (slice 3),
`if` in a polymorphic body (still rejected, Slice 1's deferral, unchanged here).

## Exit

A REPL session defines `: id ( 'T -- 'T ) ;`, instantiates it at two different types on
later lines, instantiates it twice at the same type without recompiling it, redefines
it, and sees the new body take effect on the next call while an earlier line's call
keeps the old one. Plus a golden session in `tests/phase1.rs` covering that sequence,
and the current empty-signature miscompile (recon 1) gone.
