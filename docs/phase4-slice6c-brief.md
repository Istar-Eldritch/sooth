# Phase 4 Slice 6c — quotation-taking words at the REPL (brief)

ROADMAP.md frames this slice as "the problem is retention, the same shape slice 2 solved
for polymorphic words and 8b for drop overrides," and that "the frozen-binding question…
should follow rather than reopen" slice 2's answer (the defining line's resolver snapshot).
Both of the located rejections already in the tree agree with that framing in their own
wording: `src/repl.rs:2066` (R23, defining a quotation-taking word) says "the session does
not retain [the body] past its own definition," and `src/repl.rs:250` (R24, importing a
closure exporting one) says the same. Everything below was run or read against the built
compiler, not inferred from that framing.

**The framing is half right and half misleading, in a way that changes the shape of the
work.** Retention is real, but it is not "one more `PolyWordEntry`-shaped store." A
combinator mints no `IrFunc` and no symbol (R20) — it is inlined by *term-splice*, fresh,
at every call site, forever, whether that call site is a sibling line in the same native
module or a different REPL line entirely. Slice 2's precedent is about something
structurally different: a polymorphic word's body is checked **once**, at its own defining
line, and its frozen resolver is read later only at **lowering**, once per instantiation.
Recon 5–7 below is the load-bearing finding: because a combinator has no compile event of
its own to freeze against, the correct generalization is *not* "capture a resolver at the
combinator's defining line," it is "let every splice site's own env govern, exactly as
native already does, uniformly, with no new resolver concept at all." That is simpler than
the roadmap's framing, not harder — the actual work is plumbing (four call sites hardcode an
empty combinators map today) plus one real cross-cutting decision (recon 8: three now-
mutually-exclusive name shapes need symmetric eviction on redefinition, where today there
are two).

## Recon: measured against the built compiler, not read off ROADMAP

Probes live in `/tmp/slice6c-probes/`; the binary is `target/debug/sooth`.

**1. R23 fires exactly as documented, for a monomorphic combinator defined directly at a
session line:**

```
: myfilter ( ['T: Copy 'N] [ 'T -- bool ] -- ['T 'N] usize ) … ;
→ error: `myfilter` (line 1, col 62) declares a quotation parameter, which is not yet
  supported at the REPL
  the inliner needs the callee's body, which a session line does not retain past its own
  definition (quotation-taking words at the REPL are slice 6c)
```

**2. R24 fires exactly as documented, for importing a closure that exports one**, and
crucially, **an unexported combinator used only internally to an imported closure already
works today, no compiler change needed:**

```
\ lib_combo2.sth: `myfilter` defined but not exported, `total` (exported) uses it internally
import: lc "lib_combo2.sth" ;
lc::total .
→ imported lc
   0
```

versus exporting `myfilter` itself:

```
import: lc "lib_combo3.sth" ;   \ exports myfilter
→ error: cannot import …: it exports `myfilter` …, which takes a quotation parameter …
  (the inliner needs its body, which the session does not retain -- slice 6c)
```

This narrows the import half precisely: `eval_import` already runs `check::check` over the
whole merged closure before anything is exported (one shared env, exactly like native), so
every combinator-to-combinator call *inside* a closure is already resolved and spliced by
the time the closure compiles. The gap is only "the session retains no raw terms for a name
it lets a *later* session line call" — for an exported combinator, not an internal one.

**3. A combinator's body can call an arbitrary ordinary user word, and it works today on
native with no special casing** — confirming the hazard in recon 5-7 is real, not
hypothetical:

```
: bump ( i64 -- i64 ) 1 + ;
: apply_then_bump ( i64 [ i64 -- i64 ] -- i64 ) | q | q call bump ;
: main ( -- ) 5 [ 10 * ] apply_then_bump . ;
→ 51
```

**4. The REPL's existing frozen-binding rule, for an ordinary (non-combinator, non-spliced)
call, is exactly DESIGN.md's stated invariant** ("Every REPL word today is frozen at
whichever generation of its callees existed when it was compiled"):

```
: helper ( i64 -- i64 ) 1 + ;
: caller ( i64 -- i64 ) helper ;
5 caller .              → 6
: helper ( i64 -- i64 ) 100 + ;
5 caller .               → 6   (unchanged: caller was compiled once, against the old helper)
```

`caller` mints a real symbol at its own defining line and calls `helper`'s symbol as it
existed then — this is `resolver_with_override`, baked once, at `caller`'s own compile
event. **A combinator has no such compile event of its own** (R20), which is exactly why
this precedent does not transfer by direct analogy — there is nothing at the combinator's
own defining line to bake anything into.

**5. Poly-word instantiation's frozen resolver (`PolyWordEntry.resolver`, read in
`emit_instantiations`, `src/repl.rs:1041`) is read at *lowering* time, once per distinct
instantiation, and is *never* consulted by the checker.** A poly body is type-checked
exactly once, at its own defining line, over an abstract `PolyType` stack
(`check_poly_body`), and confirmed directly: a poly body cannot even call an arbitrary
concrete word (`bump`'s `i64` parameter can't unify with an abstract `'T`, tested and
confirmed — `error: 'helper' … expects i64, but the type variable 'T is not a concrete
type`). So slice 2 never had to answer "which env does the checker use for a *re-checked*
body," because a poly body is never re-checked. A combinator's body *is* re-checked (and
re-lowered) at every splice site (`inline_combinator`, `src/check.rs:5158`+, calls
`check_terms` with the **caller's own live `env`**; the IR-side splice,
`FuncBuilder::lower_call`'s combinator arm, `src/ir.rs:2978`, calls `self.lower_terms` on
**the caller's own `FuncBuilder`**). Neither side threads or reserves a place for a separate
frozen environment — because, on native, there has never been a distinction to thread.

**6. Given 5, native's inliner is already built as "whichever context is doing the
splicing governs" — with no resolver capture anywhere for a combinator.** This is the
default behavior already shipped and load-bearing (three review rounds' worth, per 6a/6b's
merge history); 6c does not need to invent a freezing mechanism, it needs to let the
REPL's per-line env do exactly what native's single module-wide env already does, per
splice occurrence.

**7. This resolution is also the one consistent with DESIGN.md's own stated invariant, read
literally.** "Calls are direct and baked at the calling line's compile time" — for a spliced
combinator, the inner call is baked into machine code exactly once, at the moment some
*caller's* line compiles (splicing is a fresh compile event every time, at the call site);
there is no earlier compile event to have baked it at instead. Treating the combinator's own
`while ( … ) … ;` line as if it minted something to freeze against would be inventing a
compile event the architecture doesn't have, not preserving an existing invariant.

**8. What the checker/lowering side needs is exactly four now-hardcoded empty maps, each
with its own comment already naming this slice:**

```
src/check.rs:2400  check_def_collecting_drop_sites: "R23: a quotation-taking word cannot be
                    defined at the REPL, so the session has no combinators to inline; the
                    map is empty here."
src/check.rs:2466  infer_line: "R23: no session-defined combinators to inline (see
                    infer_line's twin)."
src/ir.rs:1994      lower_word: empty_combinators()
src/ir.rs:2027      lower_instantiation: empty_combinators()
```

Each needs a real `&HashMap<String, Combinator>` (check side) / `&HashMap<String, Vec<Term>>`
(IR side) threaded from a new session-level store, mirroring exactly what native derives
once from `module.words` via `collect_combinators` (`src/check.rs:4990-5001`) and the equivalent
gather in `ir::lower_word`'s outer caller (`src/ir.rs:1074-1086`) — except the REPL's version
accumulates across lines and imports instead of being built fresh from one module.

**9. A monomorphic combinator's own definition needs no new standalone-check machinery —
`check_word` already handles it identically to any ordinary word.** Native's `check()`
(`src/check.rs:1283`+) routes a mono word to `check_word` regardless of `is_combinator`; only
the *poly* case branches to `check_poly_combinator_standalone` (stand-in types) instead of
`check_poly_body`. So `eval_def`'s existing call to `check_def` already validates a mono
combinator's own body correctly, once R23 stops short-circuiting before it — the missing
step is not "check it specially," it's "don't lower/emit/dlopen it, and don't reject it."

**10. A name collision the current two-store model (`self.env` / `self.poly_words`)
doesn't have to worry about, but a three-store model will: `poly.combinators.get(name)` is
consulted *before* the ordinary env lookup** (`src/check.rs:6149`, with the comment "R5/R14:
a call to a polymorphic word is intercepted before [this]" immediately after — combinator
dispatch is checked first of all three). Redefining a name from "combinator" to "ordinary
word" (or the reverse) must therefore purge the *other* store(s) or a stale combinator
entry would keep winning dispatch over a fresh ordinary definition of the same name — R8
already does exactly this today for the env/poly_words pair (`src/repl.rs`, "the definee's
own name is removed so that redefining a name from poly to ordinary binds…"); it needs a
third leg.

## Decided (locked, one at a time)

**D1. No frozen resolver, no frozen env, and no generation/symbol machinery for combinator
retention.** Locked by recon 3–7: a combinator mints no `IrFunc` and no symbol (R20), so
there is nothing for `RTLD_GLOBAL` to collide on and no compile event of its own to freeze
against. The retention store is a plain `HashMap<String, WordDef>` (or equivalently `word` +
derived `terms`), replaced wholesale on redefinition — no epoch, no shared generation
counter, no `next_shared_generation` call. Every splice, at every later call site, uses that
call site's own live env/resolver, exactly as native's inliner already does uniformly. This
is not a weaker guarantee than slice 2's: DESIGN.md's frozen-at-compile-time invariant is
about not perturbing an *already-compiled* caller (recon 4, still true here — a caller who
already spliced a combinator's terms keeps that compiled result forever, R20 alone
guarantees this, since nothing at runtime ever calls "the combinator" by symbol). It is
silent on what a combinator's *own* inner calls bind to, because on native that question
never arises, and recon 7 argues the splice-site-governs reading is the one that doesn't
invent a compile event the architecture doesn't have.

**D2. Retention is one store, shared by mono and poly combinators.** `collect_combinators`
(`src/check.rs:4990`) and `inline_combinator` (`src/check.rs:5150`) already treat both
uniformly (the "poly combinators are excluded" line in `collect_combinators`'s own doc
comment is stale/wrong — `is_combinator` does not exclude them, and `inline_combinator`
explicitly branches on `comb.word.poly.as_ref()` internally). The REPL store follows the
same shape: no separate mono/poly combinator tables.

**D3. Defining a combinator skips lowering entirely — check, then store, no `.so`.** A
combinator's own defining line does today's `check_def`-shaped validation (recon 9: no new
standalone-check path needed for the mono case; the poly case routes to
`check_poly_combinator_standalone` instead of `check_poly_body`, mirroring native's own
branch) and, on success, inserts into the combinators store. It never reaches
`ir::lower_word`, `backend::qbe::emit`, or `dlopen` — there is no symbol to mint.

**D4. The three name-shape stores (`self.env`, `self.poly_words`, and the new combinators
store) are made mutually exclusive on redefinition, generalizing R8.** Locked by recon 10:
combinator dispatch runs before the ordinary lookup, so a stale entry left in the wrong
store after a name moves between shapes would silently win. Redefining `name` as any one
shape must evict it from the other two.

**D5. Import (R24) reuses the same store, populated from the closure's exports, not a
separate mechanism.** Locked by recon 2: an imported closure is already internally
self-consistent (one shared env across the whole closure, like native), so nothing about
*checking* changes for the import case. `eval_import` gains one more step, symmetric to how
it already splices ordinary exported words/types into the session: for each module-0 export
that `word_declares_quotation_parameter`, insert its `WordDef` into the combinators store
(alpha-renamed/id-remapped exactly like every other imported declaration, R9's positional-id
shift) instead of rejecting it via R24.

## Open questions the spec must answer

- **Where the combinators store actually lives and how it interacts with `self.arrays`/
  `self.owned_cells`/`self.refs` interning.** A combinator's own signature or body can name
  an array/cell/ref type exactly like any word's can; `eval_line`'s existing
  snapshot-and-truncate-on-error pattern for those registries must cover a rejected
  combinator definition the same way it covers a rejected ordinary one.
- **The exact eviction rule for D4.** Symmetric three-way removal is the obvious answer, but
  the spec should state it as precisely as R8 states the two-way case today, including
  whether redefining a *combinator* under the same name needs to purge any per-line
  `arrays`/`cells`/`refs` growth from the *previous* definition (probably not — those rows
  are positionally stable and never revisited, per R9 elsewhere — but say so explicitly
  rather than let a reviewer wonder).
- **What `check_def`'s and `infer_line`'s public signatures should look like post-change.**
  Both currently hardcode `no_combinators`/`empty_combinators()` locally (recon 8); the spec
  should decide whether they grow a new parameter (mirroring `poly_env`'s existing threading)
  or whether the REPL builds the `Combinator` wrapper values itself and passes a
  `&HashMap<String, Combinator>` down, matching `collect_combinators`'s return shape exactly
  so no new type is invented.
- **Redefinition test shape for the exit criterion.** "Defining a quotation-taking word,
  calling it, and redefining it, with the frozen-binding rule holding across the
  redefinition" is satisfied trivially by D1+D3 (a caller compiled before the redefinition
  keeps its own baked `.so` forever, R20-guaranteed) — but the spec should still pin it as a
  golden, and additionally pin the D1 decision itself: define a combinator whose body calls
  an ordinary helper word, call it once, redefine the helper, call the combinator again from
  a *new* line, and assert the new line's splice sees the *new* helper (this is the
  falsifiable version of D1 — the test that would fail if a future change silently added a
  frozen-resolver capture after all).
- **Whether the import case (D5) needs its own alpha-renaming/id-remapping pass distinct
  from `splice_import`'s existing one**, or whether storing the closure's already-remapped
  `WordDef`s (post `check::check(&mut module)`, which does not mutate bodies — recon 5/6 —
  so the closure's own terms are exactly what a native `collect_combinators` would have
  gathered) is sufficient with no extra work.
- **Whether an exported combinator's own signature can name a type private to the closure**,
  the same question 5a's export rule already answers for ordinary exported words (a private
  type reachable through an exported signature is rejected at the closure's own `check`, so
  this should already be covered — confirm rather than assume).

## Out of scope

Any frozen-resolver or frozen-env mechanism for a combinator's inner calls (D1) — the
roadmap's framing suggested generalizing slice 2's precedent; recon 3–7 falsifies that this
is needed at all. A dedicated "standalone check" path for monomorphic combinators — recon 9:
`check_word` already handles them. Nested constant-stack loops (slice 6d, independent).
Runtime quotation values / closures / an `IrType` quotation variant (slice 7) — nothing here
gives a combinator a runtime identity; it is exactly as compile-time-only as it is on native
today. Fixing the stale "poly combinators are excluded" doc comment on `collect_combinators`
(D2) belongs in the implementation diff as a drive-by, not a separate slice, but should not
be forgotten. The 6a bind-then-pass alias limitation and the fixed-array-codegen timeout
(both pre-existing, noted in 6b's brief) — unrelated to this slice.
