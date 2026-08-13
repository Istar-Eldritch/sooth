# Phase 4 Slice 6g spec: combinator splices learn 6f's granting rule

Derives from [`docs/phase4-slice6g-brief.md`](./phase4-slice6g-brief.md). The brief's recon
was built and run — not read — against current `main` (`6b7094f`), after 10a merged
(`e87bcae`), 6h merged (`ab14a9f`), and `src/check.rs` was split into `src/check/*.rs`
submodules (`6b7094f`). This spec takes the brief as ground truth. Every `file:line` anchor
below is the brief's own, re-verified against the split tree; the anchors this spec adds that
the brief does not give (`lib/arrays.sth`'s stale paragraph, `tests/phase4_combinators.rs`'s
import helper, and the `filter_while` corpus row) are cited where they appear.

**Checker-acceptance-plus-one-diagnostic.** Edits are confined to the liveness/granting layer
now spread across `src/check/engine.rs` (`releasable_into`), `src/check/terms.rs` (the three
existing grant sites, the splice call site, the mono `Bind` arm, and one visibility bump),
`src/check/combinators.rs` (`inline_combinator`, `check_poly_combinator_args`),
`src/check/poly.rs` (the poly `Bind` arm) and `src/check.rs`
(`check_literal_against_declared_effect`, one added `use`), plus tests, one stale library
comment, and `ROADMAP.md`. The only new *behaviour* outside liveness is D5's bind-collision
diagnostic. No `Instr`/`Terminator`, no `Type`/`IrType`, no lowering, no `qbe.rs`. A program
that compiles today and still compiles after this slice lowers byte-for-byte, and this spec
keeps that claim true by declining to edit any corpus-pinned example (see Q-corpus below).

## One plumbing consequence of the split, not just relocation

`check_terms_relaxed` (`src/check/terms.rs:50`) is currently module-private (a bare `fn`, no
`pub(super)`), unlike `check_terms` (`src/check/terms.rs:11`, already `pub(super)` and
re-exported via `check.rs`'s `use self::terms::check_terms;` at `src/check.rs:58`). D1
(`inline_combinator` in `combinators.rs`) and R2 (`check_literal_against_declared_effect` in
`check.rs`) both need to call `check_terms_relaxed` from outside `terms.rs`. **Fix: bump
`check_terms_relaxed` to `pub(super)` and add a sibling `use self::terms::check_terms_relaxed;`
next to the existing `check_terms` import in `check.rs`**, mirroring the exact pattern
`check_terms` already uses. **The import must land in the same change as R2's call site, not
earlier: until `check.rs` actually calls the function the import is dead and `cargo clippy -- -D
warnings` fails it as `unused import`.** The `pub(super)` bump is independent and can land alone.
No other visibility change is needed:
`live`/`at`/`base_depth`/`outer_releasable`/`siblings` are already in scope at all three
`releasable_into` call sites and at the `inline_combinator` call site in `terms.rs` (same
`check_term` function), so R1's added `live`/`at` parameters and D1's `granted`-computation both
drop into an already-live scope with no further threading.

## The bug in three programs

Recon 4's three programs are the whole slice. Every combinator body is spliced at its call site
by `inline_combinator` (`src/check/combinators.rs:227`), whose body-check
(`src/check/combinators.rs:374`) calls the plain `check_terms` (`src/check/terms.rs:11`) — the
root entry point 6f's own doc comment reserves for "a word body, a REPL line, a `case` clause:
nothing is ancestor to those" — instead of `check_terms_relaxed` (`src/check/terms.rs:50`) with
a `releasable_into`-computed grant. `call` (`src/check/terms.rs:288`), `times`
(`src/check/terms.rs:376`) and an `if` arm (`src/check/terms.rs:818`) all do the relaxed thing;
the splice is the one nested-invocation shape on the wrong side of the fork. Every array is
`Copy`, so naming one never enters move-tracking (D2), and `Liveness::dead`
(`src/check/engine.rs:777`) is the only guard left — a guard the splice never grants into. The
body-splice runs unconditionally for both mono and poly combinators (`comb.word.poly.is_some()`
branches only the *argument* check, `check_poly_combinator_args` `src/check/combinators.rs:414`
vs the mono loop inside `inline_combinator`), and `while` is itself polymorphic, so one fix site
covers both.

The programs (verbatim from the brief; the `import:` line for `c::` names is harness, the
program body is unaltered):

```sooth
\ P-times-accept: the times doorway grants `a` into the loop body, so the aliased
\ mutable borrow of `arr` is allowed. Accepts today, must keep accepting.
: main ( -- )
  0 4 fill | a |
  a | arr |
  4 [ | i | &!arr i >usize &!> i ! ] times
  &arr 2 >usize &> @ .
  arr drop ;

\ P-times-reject: one later use of `a` withholds the same grant. Rejects today
\ (`aliased by a`). Proves the grant above is load-bearing, not incidental.
: main ( -- )
  0 4 fill | a |
  a | arr |
  4 [ | i | &!arr i >usize &!> i ! ] times
  arr drop
  &a 0 >usize &> @ .
  a drop ;

\ P-splice: the identical shape routed through a combinator splice, which
\ carries no grant. Rejects today (recon 1's repro); D1 makes it compile.
: main ( -- )
  0 4 fill | a |
  a [ 4 > ] c::filter drop drop ;
```

Recon 1's rejection, verbatim, run on the built compiler:

```text
0 4 fill | a |
a [ 4 > ] c::filter drop drop
→ error: cannot borrow `arr__inl0` mutably in `main`: it is aliased by `a`
```

The doorways grant, the splice does not. D1 fixes the splice. But D1 alone is not shippable, for
two reasons the brief measured and this spec locks below.

## Why D1 alone is not shippable

**The granting rule is already wrong at a loop back-edge, with no 6g change (recon 9).** Run on
the built compiler, this compiles and prints `0` then `9` — a mutation through `arr` silently
visible through `a` on the next iteration, no combinator involved:

```sooth
\ P-wrap (verbatim): accepts today, prints 0 then 9. Must reject after R1.
: main ( -- )
  0 4 fill | a |
  2 [ | i | a | arr | &a 0 >usize &> @ . true if &!arr 0 >usize &!> 9 ! else end arr drop ] times ;
```

`releasable_into` (`src/check/engine.rs:811`) grants an ancestor name on
`!references(rest, name)`, where `rest` is only the *remaining* sibling terms. Inside a
`back_edge = true` body execution wraps around to the body's first term, so a name used *earlier*
in the body is still live where the grant is handed down. 6g multiplies this hole's blast radius:
every combinator call sited inside a loop (legal since 6d) becomes a fourth doorway, and 10b/10c
turn `times`/`if` themselves into splices. Shipping D1 while leaving this open trades a false
rejection for a silent wrong value. R1 closes it. **This is the sequencing constraint: R1 lands
and is validated before the relaxation can mask it.**

**Placebo warning, measured.** Appending `a drop` after P-wrap's loop flips it to a *rejection on
today's compiler* — for an unrelated reason (`references(rest, "a")` at
`src/check/engine.rs:790` sees the trailing use and withholds the grant). A reject golden written
that way passes green with and without the fix and pins nothing. **The reject golden's program
must have no use of `a` after the loop.**

**The splice's `back_edge` is observable today, through a name-hygiene defect (recon 10).**
`alpha_rename_locals` (`src/ast.rs:1208`) renames the callee's *locals*; `rename_call`
(`src/ast.rs:1229`) deliberately leaves a call to a word or builtin untouched (its final
`name.to_string()`). A caller local sharing a builtin's name is read *in place of that builtin*
inside the spliced body, silently. On the built compiler:

```text
1 >usize | len |  9 4 fill [ 4 > ] c::filter . drop  len drop   → prints 1
                  9 4 fill [ 4 > ] c::filter . drop             → prints 4
```

No diagnostic either way. This is the one route by which a granted caller name can appear as a
`Call` inside a spliced body, so D4's "the splice's flag is unobservable" argument depends on
closing it. D5 closes it at the root. **D5 lands before or with the relaxation.**

**The argument path has its own wrong-side `check_terms` (recon 8).**
`check_literal_against_declared_effect` (`src/check.rs:1318`, the `check_terms` at
`src/check.rs:1348`) runs the caller's quotation *literal* against the declared parameter effect,
in the caller's own scope, through the plain root entry point. It is reached from both of
`inline_combinator`'s argument paths (the mono loop at `src/check/combinators.rs:268`,
`check_poly_combinator_args` at `src/check/combinators.rs:480`) and rejects *before* the body
splice, so D1 alone cannot discharge the `while`-nested-in-a-combinator shape. R2 grants it the
same set.

## Locked decisions

- **D1 (from the brief).** `inline_combinator`'s body-check (`src/check/combinators.rs:374`)
  becomes `check_terms_relaxed` with a `releasable_into`-computed `outer_releasable` set. This is
  the one call site on the wrong side of 6f's contract; the three correct sites are the pattern.

- **D2 (from the brief).** No change to `Moves`, `aliasing_origin` (`src/check/engine.rs:1067`),
  or Copy-array move-blindness. Every array is `Copy` because `check_no_linear_array_elements`
  (`src/check/declarations.rs:587`, invoked at `src/check/declarations.rs:495`) rejects, at
  declaration/registry time, any array *type* whose element is non-`Copy` — independent of which
  constructor built the value (post-6h there are two: `fill` and the raw `[ Type ; Count ]`
  constructor). So a `Copy` local never enters the move map, `moved_site` is `None` forever, and
  `aliasing_origin`'s `moved_site(&b.name).is_none()` filter can never exclude an array. This is a
  permanent property of the language, not a gap.

- **D3 (from the brief).** `lib/arrays.sth`'s header paragraph blaming aliasing for the
  inline-everything/no-`while` shape (`lib/arrays.sth:18`–`28`, the block "`sort`'s merge logic
  is inlined … Inlining and dropping `while` are the only shapes found that dodge both.") is
  deleted, its rationale retested and found false. **`sort`'s code is not restructured**, and
  **`sort`'s own per-word doc comment justifying a fixed-bound `times` over stopping early on the
  `u32` length bound is unrelated to aliasing and must stay.**

  **Blocker, measured: `lib/arrays.sth` does not exist.** Not tracked on any branch
  (`git log --all -- lib/arrays.sth` is empty), not untracked in the worktree; `lib/` holds only
  `combinators.sth`. So D3's paragraph deletion and T-sort are both unbuildable as written, and
  the line/paragraph anchors above are unverifiable. Phase 4 must first establish whether the file
  is arriving: if it is not in the tree when Phase 4 starts, **drop D3 and T-sort from the slice**
  and record that the dogfood is unmeasured, rather than authoring an `arrays.sth` (out of scope,
  and the deliverable is the deletion of a claim, not new library code). Nothing else in the slice
  depends on either.

- **D4 (from the brief).** Both new relaxed calls pass `back_edge = true`.
  - At `check_literal_against_declared_effect` this is **required for soundness**: the terms
    scanned are the caller's own literal, which references caller locals directly, so a granted
    name provably appears in that scan; with `false` its last use would be treated as final even
    though the callee re-executes the literal per iteration.
  - At the body splice, `true` matches `call`/`times` and is the conservative value (a granted
    name used inside is pinned live for the whole body). Recon 10's hygiene defect is what would
    otherwise make the splice's flag observable; **D5 closes it, making the splice's value a
    uniformity choice rather than a soundness one.** Do **not** write a test claiming to pin the
    splice's flag, and do **not** treat a green suite as evidence it is right.

- **D5 (from the brief).** Reject binding a local whose name collides with a callable name. Sites:
  the mono `TermKind::Bind` arm (`src/check/terms.rs:141`) and the poly one
  (`src/check/poly.rs:333`, inside `poly_term` `src/check/poly.rs:316`), alongside the existing
  `reject_variant_local` (`src/check.rs:886`) and `reject_duplicate_local` (`src/check.rs:906`).
  The predicate `is_builtin_word_name` (`src/check/declarations.rs:101`) already exists;
  `extern_redeclaration_error` (`src/check/declarations.rs:107`) is the precedent for rejecting a
  declaration that reuses a builtin/word name, and `reject_variant_local` is the precedent for
  rejecting a *local* that collides with another namespace (enum variants). This closes recon 10
  at the root rather than patching splice hygiene, and it turns "a granted caller name can never
  appear as a `Call` in a spliced body" from false-by-counterexample into true-by-construction.

  **Coverage differs by site, deliberately.** The mono arm checks builtins, `env`, `poly.env` and
  `poly.combinators`. The poly arm checks **builtins and `env` only**, because `poly_term`
  (`src/check/poly.rs:316`) has no `PolyCtx` parameter at all: reaching `poly.env`/`poly.combinators`
  there means changing the signature of `pub fn check_poly_body` and editing `src/repl.rs`, which
  is out of this slice's sanctioned files. **Recorded gap:** after 6g, a polymorphic word may still
  bind a local named after a combinator or poly word — names the poly arm does **not** check,
  since combinators/poly words are only in `poly.combinators`/`poly.env`, unreachable from
  `poly_term`. A builtin- or `env`-word-named bind (e.g. `| len |`, `len` being a builtin) *is*
  caught by the poly arm; the gap is specifically a combinator/poly-word name, so this compiles:

  ```sooth
  : pick ( 'T 'T -- 'T ) | filter | | other | other drop filter ;
  ```

  Nobody has been able to construct a wrong-value witness through the poly arm — the hygiene
  defect needs an `alpha_rename_locals` splice, and no shape routes a poly-bound shadowing local
  into one — so this is scoped as **uniformity, not soundness**. If someone does build that witness,
  closing it is a separate slice that owns the `check_poly_body` signature change.

  *Measured (brief), with the compiler rather than a text scan:* a build enforcing D5 at both bind
  sites compiles every file in `examples/` and `lib/` unchanged and passes the entire suite (all
  20 test binaries, 1513 tests, zero failures), and rejects recon 10's shadowing program; blast
  radius is zero. A regex over `|...|` is **not** a valid way to re-check this: `|` is overloaded
  between bind groups and clause dispatch, and a naive scan reports false clashes on clause bodies
  like `examples/shapes.sth:9`'s `| Circle dup * 3.14159 *`.

- **R1 (new, forced by recon 9).** `releasable_into` (`src/check/engine.rs:811`) gains
  `live: &Liveness, at: usize`, and its filter splits — a name bound in the current invocation
  (`idx >= base_depth`) keeps today's rule verbatim; an ancestor name is granted only if the
  caller's own liveness says it is dead there:

  ```rust
  .filter(|(idx, b)| {
      if *idx >= base_depth {
          !references(rest, &b.name)
      } else {
          outer_releasable.contains(&b.name) && live.dead(&b.name, at + 1)
      }
  })
  ```

  **The index is `at + 1`, not `at`, and this is the whole correctness of the rule.**
  `nested_uses` (`src/check/engine.rs:723`) attributes a use found inside `terms[at]` to `at`
  itself, and `dead` (`src/check/engine.rs:777`) is `last < at`, so `live.dead(name, at)` is
  always false whenever the granted-into term is itself the user of the name — the entire reason
  one grants. Asking at `at + 1` reproduces exactly what `!references(rest, name)` meant ("no
  residual use after this term") while still catching recon 9's wrap-around, because a use
  anywhere in a `back_edge = true` body is recorded as `IMMORTAL_IN_BODY`
  (`src/check/engine.rs:637`, via `record_granted_use` `src/check/engine.rs:712`), which is not
  `<` any index.

  **R1 is provably sound relative to today's compiler, not merely measured green.**
  `Liveness::scan` (`src/check/engine.rs:640`) records a `Call` use at its index and `nested_uses`
  (`src/check/engine.rs:723`) records uses inside `If`/`Quotation` at the containing index;
  `references` (`src/check/engine.rs:790`) recurses over exactly the same three `TermKind`
  variants, and `TermKind` has no other nesting variant. So if `references(rest, name)` is true,
  some use sits at index `>= at + 1` and `dead(name, at + 1)` is false: **`dead(name, at + 1)`
  implies `!references(rest, name)`, so R1 grants a strict subset of what HEAD grants.**
  Over-permissiveness relative to HEAD is impossible by construction. The `idx >= base_depth`
  branch is unchanged verbatim and carries no new risk.

  **What R1 actually does, stated plainly, because it is broader than "a wrap-around fix".**
  Inside a `back_edge = true` body, `record_granted_use` (`src/check/engine.rs:712`) writes
  `IMMORTAL_IN_BODY` for *any* mention of a granted ancestor name, including one `nested_uses`
  finds inside `terms[at]` itself, and `usize::MAX < at + 1` is never true. Therefore: **an
  ancestor name mentioned anywhere inside a back-edge body is never re-granted into a block nested
  in that body; `releasable_into`'s ancestor branch is inert inside every loop and every `call`ed
  quotation.** Do not sell R1 as narrow. In particular a combinator call over a *named* array
  inside a `times` body compiles under D1+R2 and is **rejected** under D1+R2+R1 (measured). That
  rejection is correct by the language's own rule — `filter` mutates in place, so the next
  iteration re-reads a mutated `a` through a second name — so it must not be advertised as a win
  this slice delivers.

  **Accepted cost: R1 also rejects sound programs that compile today.** The class needs the
  ancestor name to appear *by name* inside a back-edge body; a combinator splice never does (the
  array arrives on the stack and callee locals are alpha-renamed), which is why the splice,
  `while` and `sort` shapes all survive. Two witnesses, both measured accepting on HEAD and
  rejecting under R1:

  ```sooth
  \ b-call-nested-only: one invocation, no wrap-around anywhere, `a` mentioned
  \ nowhere else. HEAD accepts and prints 9; R1 rejects. Pin this as a reject
  \ golden so the behaviour change is recorded rather than discovered later.
  : main ( -- )
    0 4 fill | a |
    [ true if a | arr | &!arr 0 >usize &!> 9 ! &arr 0 >usize &> @ . arr drop else end ] call ;

  \ d2-times-mutate-only: write-only across the back edge, nothing reads the
  \ stale value. HEAD accepts; R1 rejects.
  : main ( -- )
    0 4 fill | a |
    2 [ | i | true if a | arr | &!arr 0 >usize &!> 9 ! arr drop else end ] times ;
  ```

  **A refinement that would recover both was built and is unsound — do not re-propose it.** The
  candidate: for a back-edge body, grant iff the name is unmentioned in the siblings *before* and
  *after* `at` (excluding `terms[at]`'s own subtree). It restores both witnesses above and still
  rejects recon 9, but it accepts this, which prints `0` then `9`:

  ```sooth
  \ danger: read and mutation both inside the granted-into term, in a loop.
  \ Iteration 2's read through `a` sees iteration 1's write through `arr`.
  \ HEAD accepts it too (a second pre-existing silent wrong value beyond recon 9);
  \ R1 as specified rejects it; the refinement does not.
  : main ( -- )
    0 4 fill | a |
    2 [ | i | true if a | arr | &a 0 >usize &> @ . &!arr 0 >usize &!> 9 ! arr drop else end ] times ;
  ```

  So `danger` is a second justification for R1 and a second behaviour change to document, and the
  over-strictness above is the honest price of closing it: the checker cannot distinguish
  "mentioned inside the granted-into term but never read across the edge" from "read across the
  edge" without machinery beyond this slice.

  *Measured on a build with R1 applied to the three existing call sites alone (no D1, no R2):*
  recon 9's program rejects; `danger` rejects; the two-level execute-once grant chain below still
  compiles; a three-level version of it also still compiles; a
  wrap-around-through-an-earlier-sibling-literal shape rejects; the full suite stays green (1513
  tests).

  ```sooth
  \ P-nest2 (verbatim): accepts today (prints 9), must keep accepting. Two levels of
  \ execute-once nesting, `a` used only inside the innermost one. R1 written with `at`
  \ instead of `at + 1` rejects this.
  : main ( -- )
    0 4 fill | a |
    true if
      true if
        a | arr | &!arr 0 >usize &!> 9 ! &arr 0 >usize &> @ . arr drop
      else end
    else end ;
  ```

- **R2 (new, forced by recon 8).** `check_literal_against_declared_effect` takes the same grant
  and its `check_terms` (`src/check.rs:1348`) becomes `check_terms_relaxed(..., granted, true)`.
  Its three non-combinator callers — `materialize_quotation_at_boundary`
  (`src/check/captures.rs:320`) and the two `if`-arm quotation-merge sites
  (`src/check/terms.rs:954`, `src/check/terms.rs:970`) — pass an empty set, preserving their
  behaviour exactly (with an empty grant, `back_edge` only feeds `record_granted_use`, which fires
  only for names in `outer_releasable`). Whether those three shapes deserve a grant is 7b's and
  10c's question, not this slice's.

- **Q2 (was open, now decided).** The parameter is `granted: &HashSet<String>`, computed by the
  caller. The set must reach `check_literal_against_declared_effect`, which sits two frames below
  `inline_combinator` and has neither a sibling list nor an index of its own. One `HashSet`
  threaded down beats re-deriving position in a function that has no position.

## Open questions the spec must answer

**Q-corpus — does any shipped `.sth` need editing to positively pin the fix, and what does it
cost?** `examples/filter_while.sth` passes `scores` straight from its producer word into `filter`,
never bound to a local, and its header comment says so to dodge this bug. Binding first would
convert it into a positive pin. **Decision: leave the example untouched.** It is a row of `CORPUS`
in `tests/qbe_baseline.rs` (`tests/qbe_baseline.rs:38`), whose golden asserts the emitted `.ssa`
is byte-identical to `tests/qbe_baseline/filter_while.ssa`. A `scores` bind introduces a slot and
changes the lowering, so editing the source forces a sanctioned baseline regeneration and a review
of a generated `.ssa` diff, and it falsifies this spec's "everything lowers byte-for-byte" claim.
**What the decision costs:** `filter_while.sth` stays a documents-a-limitation example rather than
becoming a positive pin, so the pin is carried instead by the new accept goldens and the `sort`
dogfood (T-splice, T-while, T-sort), which are the shape a user actually hits. **What it buys:**
the byte-for-byte invariant stays literally true, and no corpus baseline is regenerated or
reviewed. (6h regenerated every baseline in that corpus, so "avoid a baseline regeneration" is no
longer the deciding cost; the decision now rests on keeping the byte-for-byte claim literally true
and not touching a corpus-pinned source.) The example's comment is left as-is: its code genuinely
still binds-through-producer, so the comment still accurately describes the code.

**Q-witness — which accept goldens witness D1 versus R2?** Decided by recon 8, and empirically
confirmed by hand-implementing R1+D1+R2 and building every combination (see the verification note
below): D1 is the body splice, R2 is the argument-path literal check, and for any combinator over
an **aliased local** both passes see the same name, so **T-while needs both, and no clean
single-decision witness for R2 exists.**

- **T-splice** (P-splice: `filter` over a bound array) needs only the body grant. Reverting D1
  turns it red; reverting R2 **leaves it green** (verified: `filter`'s own predicate literal
  `[ 4 > ]` never mentions the array, so `check_literal_against_declared_effect`'s pass has
  nothing to do with it). It is the clean D1 witness.
- **T-while** (`while` over an array bound to a local, then re-bound to a second local the loop
  actually borrows) rejects at **both** passes without the corresponding fix: the literal-check
  pass (`check_literal_against_declared_effect`, R2) sees the rebind directly in the literal it
  type-checks; the body-splice pass (`while`'s own prewritten `p call`, spliced by
  `inline_combinator`, D1) re-checks the same literal against the real runtime slots once it
  actually executes. Reverting **either** reds it.

R2 is not redundant with D1 despite this: reverting R2 alone (D1 present) still turns T-while red
while **T-splice stays green** — R2's own pass is the one that actively rejects T-while when
unrelaxed, it is simply never the *only* pass in play for this shape, since `while` is itself a
combinator and its own body-splice (D1) touches the same literal a second time. No shape is known
to exist where R2 alone (D1 absent) accepts something D1 alone (R2 absent) rejects, or vice versa,
for this bug class — the two passes are structurally coupled for any combinator literal. **T-while
pins D1+R2 jointly; its non-redundancy argument rests on the half that does discriminate against
T-splice, not on a clean separation from D1.**

**Verification note.** R1+D1+R2 were hand-implemented and built in a scratch tree (not merely read)
specifically to find this witness, after an earlier draft's constructed T-while program turned out
to already compile clean on HEAD (not a witness of anything). The program below was confirmed by
actually building all four configurations — true HEAD, full R1+D1+R2, R2 reverted alone, D1 reverted
alone — and matches every claim above exactly; see the mutation criteria below for the two
confirming variants (M1: mentioning the alias anywhere in the literal reds it, per R1; M2: using the
original name again after the loop reds it, per the ordinary `references(rest, name)` rule).

The T-while program:

```sooth
\ T-while: `input` returns a fresh array via a producer word (so the harness's
\ absolute-path import works the same way T-splice's does); `a` binds it, then
\ `arr` re-binds it (arrays are Copy, so this duplicates the handle, not the
\ storage -- `a` and `arr` now alias one region). `a` is never read again. The
\ loop writes `arr[i]=9` for i in 0..4 via `c::while`, then prints `arr[0]`.
\ Accepts only under D1+R2; reverting EITHER reds it with the exact same
\ `aliased by a` error (verified) -- pins D1+R2 JOINTLY, not R2 alone (Q-witness).
\ Builds, prints 9.
import: c "<lib>/combinators.sth" ;   \ absolute path supplied by the harness helper
: input ( -- [i64 4] ) 0 4 fill | s | s ;
: main ( -- )
  input | a |
  a | arr |
  0 [ | i | &!arr i >usize &!> 9 ! i 1 + dup 4 < ] c::while drop
  &arr 0 >usize &> @ . ;
```

**Q-sort-array — which of the two arrays `sort` returns holds the sorted result?** The dogfood
golden must say. `sort` returns `ra rs` (`lib/arrays.sth:124`): sorted first, leftover scratch on
top, and it needs a caller-supplied scratch of the same length. A golden that reads the
top-of-stack array after `a::sort` reads the scratch and prints zeros, which looks like a broken
fix and invites a golden written to expect zeros. Measured: fed by producer words, `sort` prints
`1 2 3 4`; the same call with both arrays bound to locals fails today with `cannot borrow cs__inl0
mutably: it is aliased by s0`, which is the bug this slice fixes.

The T-sort program, constructed from `sort`'s signature and `examples/array_totals_hand.sth`'s
array-write idiom (the Phase-4 implementer must confirm it prints `1 2 3 4`; the harness supplies the
absolute import path via the lib-path helper of [Sanctioned files](#sanctioned-files)):

```sooth
\ T-sort: data and same-length scratch arrays BOUND TO LOCALS before the call —
\ that binding is exactly what fails today (`aliased by s0`) and what 6g fixes.
\ `sort` returns `ra rs` (sorted deeper, scratch on top): drop the scratch `rs`,
\ read the sorted `ra`. Expects 1 2 3 4.
import: a "<lib>/arrays.sth" ;   \ absolute path supplied by the harness helper
: main ( -- )
  0 4 fill | d |
  &!d 0 >usize &!> 4 !
  &!d 1 >usize &!> 2 !
  &!d 2 >usize &!> 1 !
  &!d 3 >usize &!> 3 !
  0 4 fill | s |
  d s [ | x y | x y - ] a::sort
  | ra rs | rs drop
  &ra 0 >usize &> @ .
  &ra 1 >usize &> @ .
  &ra 2 >usize &> @ .
  &ra 3 >usize &> @ .
  ra drop ;
```

**Q-order — ordering of D5 against D1/R2.** D5 is what makes D4's splice-side argument hold, so it
lands **before** the relaxation (Phase 2, ahead of Phase 3's D1/R2). R1 lands **first** (Phase 1),
alone, so the tightening is validated green before any relaxation can mask a mistake in it. See
[Phased delivery](#phased-delivery).

## Invariant recorded (was true, nothing else records it)

Grants are handed out **capture-blind**: `releasable_into` (`src/check/engine.rs:811`) never
consults captures. Capture-awareness lives at the *use* site — `live_derivs`
(`src/check/engine.rs:953`) and `aliasing_origin` (`src/check/engine.rs:1067`) each compute
`capture_alive_names` (`src/check/engine.rs:849`) and check `!live.dead(name, at) ||
captured.contains(name)`. `capture_alive_names` is a fixpoint over stack slots and bound locals
covering both `Known` markers and 7b's erased-closure `surviving` sets. `live.dead` has exactly
four call sites: those two, plus two internal to the capture machinery
(`src/check/engine.rs:859` area, `past_last_use_capture` `src/check/engine.rs:923`). A future
slice adding a consumer of `live.dead` on an aliasing path without the capture disjunct would break
this silently. R1 adds a `live.dead(&b.name, at + 1)` call inside `releasable_into`, which is a
*grant* site, not an aliasing-use site, so it does not need the disjunct — but it is now a fifth
`live.dead` caller, and the count above is why that is safe.

Second half of the same invariant: **`dead`'s `None` arm is fail-open.** It returns
`outer_releasable.contains(name)`, so a name `Liveness::scan` fails to record is granted, not
withheld. Today that is safe because `scan` (`src/check/engine.rs:640`) and `references`
(`src/check/engine.rs:790`) traverse the identical three nesting variants, which is exactly what
makes R1 a strict subset of HEAD's grants. **A future `TermKind` with nested terms added to
`references` but not to `scan` would silently open a hole.** (Checked against 6h: its
`TermKind::ArrayCtor(Type)` carries a type, not terms, so it is not such a variant, and it also
does not touch the R1 soundness argument.)

## Mechanism

1. `check_terms_relaxed` (`src/check/terms.rs:50`) is bumped from bare `fn` to `pub(super)`. No
   signature change to `check_terms`, `check_terms_relaxed`, or `Liveness`. The matching
   `use self::terms::check_terms_relaxed;` in `check.rs`, beside its existing
   `use self::terms::check_terms;` (`src/check.rs:58`), belongs to **step 4** — it is an unused
   import until R2's call site exists, and unused imports are a `clippy -- -D warnings` error.
2. `releasable_into` (`src/check/engine.rs:811`) gains `live: &Liveness, at: usize` and R1's split
   filter. Its three existing call sites (`call` `src/check/terms.rs:288`, `times`
   `src/check/terms.rs:376`, `if` `src/check/terms.rs:818`) and the new fourth already have both in
   scope, in `check_term`'s own parameters (`src/check/terms.rs:98`).
3. `inline_combinator` (`src/check/combinators.rs:227`) gains `granted: &HashSet<String>`; its
   body-check (`src/check/combinators.rs:374`) becomes `check_terms_relaxed(..., granted, true)`.
   Its sole call site (`src/check/terms.rs:661`, inside `check_term`'s `TermKind::Call` dispatch)
   computes `releasable_into(scope, base_depth, outer_releasable, &siblings[at + 1..], live, at)`,
   exactly as its three neighbours do, and passes the result as `granted`.
4. `check_poly_combinator_args` (`src/check/combinators.rs:414`) and
   `check_literal_against_declared_effect` (`src/check.rs:1318`) gain the same `granted` parameter;
   `check.rs` gains `use self::terms::check_terms_relaxed;` (step 1) and the latter's `check_terms`
   (`src/check.rs:1348`) becomes `check_terms_relaxed(..., granted, true)`. Its three non-combinator callers (`src/check/captures.rs:320`, `src/check/terms.rs:954`,
   `src/check/terms.rs:970`) pass `&HashSet::new()`.
5. The mono `TermKind::Bind` arm (`src/check/terms.rs:141`) rejects a local name that is a builtin
   (`is_builtin_word_name`, `src/check/declarations.rs:101`), a word in `env`, a poly word
   (`poly.env`), or a combinator (`poly.combinators`); the poly `TermKind::Bind` arm
   (`src/check/poly.rs:333`) rejects a name that is a builtin or a word in `env` only (D5's
   coverage-by-site split). Both are modelled on `extern_redeclaration_error`
   (`src/check/declarations.rs:107`) / `reject_variant_local` (`src/check.rs:886`).

One new diagnostic (D5). No lowering, IR, or `Type` change.

## Sanctioned files

- `src/check/engine.rs` — `releasable_into`'s new `live`/`at` params and R1's split filter, plus
  the R1 unit test in this file's `#[cfg(test)] mod tests` (`releasable_into` lives here, and per
  the split each cluster module holds its own tests).
- `src/check/terms.rs` — bump `check_terms_relaxed` to `pub(super)`; update the three existing
  `releasable_into` call sites; compute and pass `granted` at the `inline_combinator` call site
  (`:661`); D5 at the mono `Bind` arm (`:141`).
- `src/check/combinators.rs` — `inline_combinator` and `check_poly_combinator_args` gain `granted`;
  the body-splice `check_terms` (`:374`) becomes `check_terms_relaxed(..., granted, true)`.
- `src/check/poly.rs` — D5 at the poly `Bind` arm (`:333`), builtins/`env` only.
- `src/check.rs` — `check_literal_against_declared_effect` gains `granted` and its `check_terms`
  (`:1348`) becomes `check_terms_relaxed(..., granted, true)`, in the same change as the
  `use self::terms::check_terms_relaxed;` import (unused earlier, so clippy-fatal earlier).
- `tests/phase4_slice6g.rs` (new) — the goldens below. `combinators_import`
  (`tests/phase4_combinators.rs:69`) builds an absolute-path `import:` line because `run_src` writes
  the source under `temp_dir()` so a relative `import:` does not resolve — but it is **hardcoded** to
  `"{}/lib/combinators.sth"` and cannot be repointed at `lib/arrays.sth` as-is. Phase 3/4 add a
  lib-path-parameterized version (e.g. `fn lib_import(qualifier, lib_file)` generalizing the existing
  one, or a second `arrays_import` mirroring its shape) so T-sort can import `lib/arrays.sth` while
  T-splice/T-while import `lib/combinators.sth`.
- `lib/arrays.sth` — delete only the aliasing-workaround paragraph (`:18`–`28`, D3). No code change;
  `sort`'s fixed-bound-`times` rationale stays. (**Does not exist in this tree; see D3.**)
- `ROADMAP.md` — mark 6g implemented; correct the stale texts named in the brief: the "Next
  action" pointer (`ROADMAP.md:608`, still reads "Phase 4 Slice 10a"); the 6g entry's
  `self_tail`-conditioned `back_edge` that D4 rejects (`ROADMAP.md:1719`, the 6g entry starts
  `ROADMAP.md:1691`; constant `true` is correct); and the 6g entry's stale "no other array
  constructor exists" claim (`ROADMAP.md:1699`), superseded post-6h by D2's reworded reasoning
  (Copy-ness comes from `check_no_linear_array_elements`, not from `fill` being the sole
  constructor).

## Exit criteria (goldens in `tests/phase4_slice6g.rs`)

| ID | Test | Kind | Phase | Source in → expected out |
| --- | --- | --- | --- | --- |
| U1 | `releasable_into_withholds_a_name_used_in_a_back_edge_body` | unit | 1 | over a synthetic `Scope`+`Liveness` built by `scan`: an ancestor name with `IMMORTAL_IN_BODY` is **absent** from the grant; an ancestor name in `outer_releasable` that the body never mentions is **present** (`None`-arm half, shows the tightening does not over-tighten) |
| T-wrap | `if_inside_a_loop_reading_an_alias_is_an_error` (P-wrap) | reject | 1 | `cannot borrow … mutably` + `aliased by`. **Behaviour change**: accepted today, runs and prints `0` then `9` |
| T-bcall | `single_call_body_naming_the_alias_is_an_error` (b-call-nested-only) | reject | 1 | `aliased by`. **Behaviour change**: accepted today, prints `9`. No use of the name after the invocation |
| T-d2 | `write_only_across_a_back_edge_is_an_error` (d2-times-mutate-only) | reject | 1 | `aliased by`. **Behaviour change**: accepted today |
| T-danger | `read_and_mutate_inside_a_looped_grant_is_an_error` (danger) | reject | 1 | `aliased by`. **Behaviour change**: accepted today, runs and prints `0` then `9` |
| T-nest2 | `two_level_execute_once_grant_still_accepted` (P-nest2) | accept | 1 | builds, prints `9`. Discriminates `at + 1` from `at` (the `at` form rejects it) |
| T-doorway-ok | `times_doorway_grants_the_bound_alias` (P-times-accept) | accept | 1 | builds, prints `2` |
| T-doorway-no | `later_use_withholds_the_times_grant` (P-times-reject) | reject | 1 | `aliased by a`. **Pre-existing-behaviour regression guard, not a 6g pin**: the `references(rest, "a")` rule that withholds the times-doorway grant on a later use is unchanged by R1, so no mutation in [Mutation-required criteria](#mutation-required-criteria) reds it. It guards the doorway boundary against future drift; it does not pin anything 6g introduces. |
| T-shadow | `binding_a_local_named_after_a_builtin_is_rejected` (`len`) | reject | 2 | D5 diagnostic naming the collided builtin. **Behaviour change**: accepted today, prints `1` silently |
| T-splice | `bound_array_passed_to_filter_is_accepted` (P-splice) | accept | 3 | builds. **D1 witness** |
| T-while | `while_over_an_aliased_array_local_is_accepted` | accept | 3 | builds, prints `9`. **Joint D1+R2 witness (reverting either reds it with the same `aliased by` error; see Q-witness)** |
| T-sort | `sort_called_with_bound_array_locals_runs` | accept | 4 | `lib/arrays.sth`'s shipped `sort` over a bound array + comparator; reads the sorted array (`ra`, not the scratch `rs`) and prints `1 2 3 4` |
| T-green | whole suite green | regression | 1–4 | `cargo fmt --check && cargo clippy -- -D warnings && cargo test` |
| T-roadmap | ROADMAP 6g implemented; both stale texts corrected | doc | 4 | prose |

T-sort is the dogfood and the honest measure of the bug's cost: today you cannot call the library's
own `sort` on arrays you have named. It is also **blocked**: `lib/arrays.sth` is absent from the
tree (see D3).

## Mutation-required criteria

Run each in a **throwaway copy of the worktree**, never the shared one (a concurrent reviewer has
previously mistaken an in-place mutation for a real bug). Keep `target/` copies **off** `/tmp` (a
32G tmpfs shared across sessions; an unrelated session has been ENOSPC'd by orphaned scratch dirs).
There is no tooling for this; copy the tree elsewhere and build there.

Each mutation names the phase it is runnable in and the exact test it turns red, **and the
direction of the mutation matches the kind of golden named** (the dual trap: a full R1 revert is a
*loosening* and can only turn a *reject* golden red; the `at + 1`→`at` change is a *tightening* and
can only turn an *accept* golden red). **No witness listed here needs a later phase's code to
compile** — the Phase 1 witnesses use no combinator, so they compile in an R1-only tree.

- **M-R1-full (Phase 1, the loosening).** Revert `releasable_into`'s filter to the pre-R1
  `(idx >= base_depth || outer_releasable.contains(name)) && !references(rest, name)` → the
  **reject** goldens **T-wrap**, **T-bcall**, **T-d2** and **T-danger** go green-again-red (they
  compile once more). T-nest2 must **not** be listed here: a loosening cannot turn an accept golden
  red. Under the reverted build T-wrap and T-danger must not merely compile: each must **run and
  print `0` then `9`**. A reject test that only checks "fails" cannot distinguish this from a type
  error, and the wrong value is the point.
- **M-R1-index (Phase 1, the tightening).** Change `at + 1` to `at` alone → **T-nest2** (accept)
  goes red, proving the index is load-bearing. This mutation belongs *only* to the `at + 1`→`at`
  change, never to the full revert.
- **M-U1 (Phase 1).** Delete the `live.dead(…)` conjunct → **U1** red — but only its **first**
  half: the `None`-arm half passes trivially through `dead`'s fail-open arm as long as the
  `outer_releasable.contains` conjunct survives, so deleting the `live.dead` conjunct turns only the
  `IMMORTAL_IN_BODY` half red. Stated as a two-sided pin it would read stronger than it is.
  Additionally: delete the `idx >= base_depth` fast path (route every name through the ancestor
  branch) → a Phase-1 accept golden that binds-and-uses within one invocation (T-doorway-ok) red,
  proving the two branches are not interchangeable.
- **M-D5 (Phase 2).** Remove the D5 rejection at both bind sites → **T-shadow** red: the program
  compiles and prints `1` again (the shadowing silent wrong value). Assert the value `1`, not just
  "compiles".
- **M-D1 (Phase 3).** Revert `inline_combinator`'s body-check to plain `check_terms` → **T-splice**
  red (the recon-1 rejection is raised inside the spliced body, at `filter`'s own `&!arr`) **and**
  **T-while** red (verified: `while`'s own prewritten body re-checks the caller's literal against
  the real runtime slots via its internal `p call`, gated by this same body-splice grant). Do not
  claim T-while stays green here — it does not, and an earlier draft's claim that it did was wrong.
- **M-R2 (Phase 3).** Revert `check_literal_against_declared_effect` to plain `check_terms` →
  **T-while** red and **T-splice green** (verified: `filter`'s own predicate literal never mentions
  the array, so this pass has nothing to reject there). Both halves matter: without the second, R2
  looks redundant with D1; without the first, M-R2 could pass by accident. T-splice staying green is
  the half of Q-witness that *does* discriminate R2 from D1 — T-while going red under *both*
  M-D1 and M-R2 is expected and correct, not a sign either mutation is redundant.
- **M-T-while-bounds (Phase 3, T-while's own soundness boundary, not a revert).** Two variants of
  T-while's own source, built and confirmed against the full R1+D1+R2 tree, must both still
  **reject**, proving the accept golden is not accidentally over-permissive: (1) mention the
  original name (`a`) anywhere inside the while-literal (e.g. read it once, mid-loop) — R1's
  `IMMORTAL_IN_BODY` marking correctly withholds the grant, since a name mentioned anywhere in a
  back-edge body is never dead there; (2) use `a` again in `main` after the loop — the ordinary
  `references(rest, name)` rule correctly excludes it from `releasable_into`'s output. Both keep
  rejecting with the same `aliased by` error under the full fix; if either newly accepts, the grant
  computation has become unsound, not merely stricter.
- **M-compose (Phase 3, the phasing trap).** R1 lands in Phase 1 and D1 in Phase 3, so a
  composition failure would not surface for two phases. Build R1+D1+R2 together and assert: the
  splice shape (T-splice) accepts, the `sort` dogfood (T-sort) accepts and sorts, recon 9 (T-wrap)
  rejects, the in-loop combinator-over-named-array shape rejects, suite green. No earlier revision
  checked this composition.
- **Not pinnable, stated so it is not faked (D4).** Flipping the *body splice's* `back_edge` from
  `true` to `false` changes no test and no probe once D5 has closed recon 10's hygiene defect. Do
  not write a test that claims to pin it; do not treat a green suite as evidence the value is right.
  **The literal check's `back_edge = true` is not pinned by M-R2 or by anything else in this
  slice's corpus either** (measured: flipping it alone, flipping the splice's flag alone, and
  flipping both together each leave the full suite green, including a constructed
  read-through-`a`-then-write-through-`arr` witness inside a `c::while` literal that re-executes
  per iteration). It is still the correct value under D4's soundness argument above — the terms
  scanned are the caller's own literal, re-executed per iteration, so treating a granted name's
  last use as final would be wrong — but that argument is unobserved by any test today, not
  pinned by one. Do not write a test that claims to pin this flag either.

The R1 unit test (U1) is warranted directly because the end-to-end goldens reach `releasable_into`
only through several nested invocations. Precedent: 6f's R6 walk-stop test, which became a unit
test for the same reason. It constructs a `Scope` plus a `Liveness` built via `scan` over synthetic
terms and covers both halves (the `IMMORTAL_IN_BODY` withhold and the `None`-arm still-granted case)
so the tightening is shown not to over-tighten.

## Out of scope

Restructuring `lib/arrays.sth`'s `sort` (D3 requires only the stale rationale to go). Any change to
`Moves`/move-tracking for `Copy` types (D2). Whether the three non-combinator
`check_literal_against_declared_effect` callers deserve a grant (R2). The `PolyType::Ref` gap.
Closing D5's poly-arm coverage gap (the `check_poly_body` signature change is a separate slice). Any
lowering, IR, or diagnostic-text change beyond D5's new diagnostic. Editing any corpus-pinned
example (Q-corpus).

**Sequencing: after 6h, before 10b/10c.** 10a has merged (`e87bcae`) so the old blocker is gone.
The 10b/10c relation is a prerequisite, not a preference (recon 5): both convert a doorway that
grants into a splice that does not (10b for every `times`, 10c for every `if`), so P-times-accept
regresses at 10b unless this slice has landed. The 6h relation is the opposite — pure mechanical
courtesy, no dependency either way — but 6h has merged (`ab14a9f`) and its anchors are already
folded in above.

## Phased delivery

**Phase 1 (hard) — R1, the tightening, alone.** Bump `check_terms_relaxed` to `pub(super)` — the
`check.rs` import waits for Phase 3, which is the first phase with a call site there;
`releasable_into` gains `live`/`at` and the `at + 1` split filter; the three existing call sites are
updated; U1 unit test; T-wrap / T-bcall / T-d2 / T-danger / T-doorway-no reject goldens; T-nest2 /
T-doorway-ok accept goldens; mutations M-R1-full, M-R1-index, M-U1. Lands first and independently:
measured green on the existing suite with no relaxation present, so if anything in it is wrong the
failure is visible before the relaxation can mask it. No combinator splice is touched here, so every
Phase-1 golden compiles in this tree.

**Phase 2 (standard) — D5, the bind-collision reject.** The two `Bind` arms reject a colliding local
name (mono: builtin/word/poly/combinator; poly: builtin/word only); new diagnostic; T-shadow reject
golden; mutation M-D5. Lands before the relaxation so D4's splice-side argument holds by
construction. Measured blast radius (brief) is zero, so no existing golden changes.

**Phase 3 (standard) — D1 + R2, the relaxation.** `granted` threaded through `inline_combinator`,
`check_poly_combinator_args`, `check_literal_against_declared_effect`; both `check_terms` calls
become `check_terms_relaxed(..., granted, true)`; `check.rs` gains
`use self::terms::check_terms_relaxed;` here, with its first call site; T-splice (clean D1 witness) and T-while (joint
D1+R2 witness, reds under either revert) accept goldens; mutations M-D1, M-R2,
M-T-while-bounds, M-compose. T-wrap / T-bcall / T-d2 / T-danger / T-doorway-no / T-shadow must
stay red across this phase.

**Phase 4 (standard) — dogfood, D3, docs.** **Start by checking whether `lib/arrays.sth` exists; it
does not as of Phase 1, and D3 says to drop T-sort and the paragraph deletion if it is still absent.**
If present: T-sort over its shipped `sort` with bound
array locals (absolute-path import helper), reading the sorted array `ra` not the scratch `rs`;
delete the stale aliasing-workaround paragraph in `lib/arrays.sth` (`:18`–`28`); correct ROADMAP's
"Next action" pointer (`ROADMAP.md:608`), the 6g entry's stale `self_tail`-conditioned `back_edge`
text (`ROADMAP.md:1719`), and the 6g entry's stale "no other array constructor exists" claim
(`ROADMAP.md:1699`) when that entry is rewritten; mark 6g implemented (T-roadmap).

```json
{
  "phases": [
    { "phase": 1, "focus": "releasable_into loop-aware grant with at+1 index; check_terms_relaxed to pub(super) only, no check.rs import (unused there until phase 3, clippy-fatal); unit test; four reject and two accept goldens (no combinator, R1-only tree); mutations M-R1-full/M-R1-index/M-U1", "difficulty": "hard" },
    { "phase": 2, "focus": "D5 reject a local name colliding with a callable at both Bind arms (mono builtin/word/poly/combinator, poly builtin/word only); T-shadow reject golden; mutation M-D5", "difficulty": "standard" },
    { "phase": 3, "focus": "D1+R2 grant threaded into inline_combinator and the literal check, adding the check.rs check_terms_relaxed import alongside its first call site; T-splice (clean D1 witness) and T-while (joint D1+R2 witness) accept goldens; mutations M-D1/M-R2/M-T-while-bounds/M-compose", "difficulty": "standard" },
    { "phase": 4, "focus": "if lib/arrays.sth exists (absent as of phase 1 - if still absent, drop T-sort and D3 per D3's blocker note): sort dogfood golden reading ra via absolute-path import, delete stale arrays.sth workaround paragraph; correct ROADMAP next-action and self_tail back_edge text", "difficulty": "standard" }
  ]
}
```
