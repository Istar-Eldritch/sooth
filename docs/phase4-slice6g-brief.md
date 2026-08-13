# Phase 4 Slice 6g — combinator splices bypass 6f's granting rule (brief)

6f built a mechanism (`Liveness`'s `outer_releasable`/`back_edge`, `releasable_into`) that
lets a nested invocation — an `if` arm, a `call`'d quotation body, a `times` body — learn
that the caller has already proven an ancestor-bound name has no residual use past this
point, so that name can die inside the nested body instead of defaulting to "never dead
here". `inline_combinator` (`src/check/combinators.rs:227`), which splices a combinator's
body (`filter`/`map`/`fold`/`while`/any user quotation-taking word) at its call site, is a
fourth nested-invocation shape and was never wired into it: it calls the plain
`check_terms` (`src/check/terms.rs:11`) — the entry point 6f's own doc comment reserves for
"a word body, a REPL line, a `case` clause: nothing is ancestor to those" — instead of
`check_terms_relaxed` (`src/check/terms.rs:50`) with a computed `outer_releasable` set.

**Anchors are against `e87bcae` (post-10a).** 10a has merged; every anchor below was
re-verified against that commit, and every behavioural claim was built and run, not read.
A previous revision of this brief and its spec were anchored to `491cf54` and are
superseded: 10a moved every function this slice touches by 100+ lines and gave
`check_literal_against_declared_effect` a new `row: &[Type]` parameter. The *shape* of the
mechanism survived 10a unchanged; only positions and that one signature moved.

**Both blockers this note warned about have since landed, and a third, bigger one arrived
with them: re-anchored against current `main` (`6b7094f`).** Slice 6h merged
(`ab14a9f`), and separately `src/check.rs` was split into `src/check/*.rs` submodules
(`6b7094f`) — every function this slice touches moved to a different *file*, not just a
different line, so every anchor below is a fresh lookup, not an offset. Re-verified directly
against the current tree rather than assumed from either merge's branch snapshot:

- `fill` remains in `BUILTIN_WORDS` (now `src/check/declarations.rs:63`), so D5's predicate
  is unaffected by 6h.
- 6h's raw `[ Type ; Count ]` constructor still rejects linear elements, so recon 2 and D2
  hold, and its `TermKind::ArrayCtor(Type)` carries a type, not nested terms, so it is not a
  nesting variant and does not touch the R1 soundness argument below.
- `Liveness`/`scan`/`dead`/`references`/`nested_uses`/`record_granted_use`/
  `IMMORTAL_IN_BODY`/`releasable_into`/`capture_alive_names`/`live_derivs`/`aliasing_origin`
  all moved together into **`src/check/engine.rs`** (`struct Liveness` `:629`,
  `IMMORTAL_IN_BODY` `:637`, `Liveness::scan` `:640`, `record_granted_use` `:712`,
  `nested_uses` `:723`, `dead` `:777`, `references` `:790`, `releasable_into` `:811`,
  `capture_alive_names` `:849`, `live_derivs` `:953`, `aliasing_origin` `:1067`).
- `check_terms`/`check_terms_relaxed`/the mono `TermKind::Bind` arm/the three existing
  `releasable_into` call sites (`if`/`call`/`times`)/the `inline_combinator` call site all
  moved into **`src/check/terms.rs`** (`check_terms` `:11`, `check_terms_relaxed` `:50`, mono
  `Bind` arm `:141`, `check_term` `:98`, the `call`-site `releasable_into` call `:288`, the
  `times`-site call `:376`, the `if`-site call `:818`, the `inline_combinator` call site
  `:661`).
- `inline_combinator` and `check_poly_combinator_args` moved into
  **`src/check/combinators.rs`** (`inline_combinator` `:227`, its body-splice `check_terms`
  call `:374`, `check_poly_combinator_args` `:414`).
- The poly `TermKind::Bind` arm (inside `poly_term`) moved into **`src/check/poly.rs`**
  (`poly_term` `:316`, poly `Bind` arm `:333`).
- `is_builtin_word_name`/`BUILTIN_WORDS`/`extern_redeclaration_error` moved into
  **`src/check/declarations.rs`** (`BUILTIN_WORDS` `:63`, `is_builtin_word_name` `:101`,
  `extern_redeclaration_error` `:107`).
- `check_literal_against_declared_effect`, its `check_terms` call, `reject_variant_local`,
  `reject_duplicate_local` and `find_zero_unsafe_element` (6h's D3) **stayed in the
  top-level `src/check.rs`** (now 2530 lines): `find_zero_unsafe_element` `:320`,
  `reject_variant_local` `:886`, `reject_duplicate_local` `:906`,
  `check_literal_against_declared_effect` `:1318`, its `check_terms` call `:1348`.

**One real plumbing consequence, not just relocation: `check_terms_relaxed` is currently
module-private (a bare `fn`, no `pub(super)`) in `terms.rs`, unlike `check_terms` (already
`pub(super)`, re-exported via `check.rs`'s `use self::terms::check_terms;`).** D1
(`inline_combinator` in `combinators.rs`) and R2 (`check_literal_against_declared_effect` in
`check.rs`) both need to call it from outside `terms.rs`. Fix: bump `check_terms_relaxed` to
`pub(super)` and add a sibling `use self::terms::check_terms_relaxed;` next to the existing
`check_terms` import in `check.rs` — mirroring the exact pattern `check_terms` already uses.
No other visibility change is needed: `live`/`at`/`base_depth`/`outer_releasable`/`siblings`
are already in scope at all three `releasable_into` call sites and at the `inline_combinator`
call site in `terms.rs` (same `check_term` function), so R1's added `live`/`at` parameters and
D1's `granted`-computation both drop into an already-live scope with no further threading.

## Recon: measured against the built compiler

**1. The bug 6b's recon 10 deferred has a single confirmed root cause, and it still
reproduces.** Minimal repro, run on `e87bcae`:

```text
0 4 fill | a |
a [ 4 > ] c::filter drop drop
→ error: cannot borrow `arr__inl0` mutably in `main`: it is aliased by `a`
```

**2. Every array in Sooth is `Copy`, so naming one never enters move-tracking.** `fill` is
the only array constructor and rejects a linear element outright. A `Copy` local is, by
`Moves`' own doc comment, absent from the map, so `moved_site` is `None` forever and
`aliasing_origin`'s (`src/check/engine.rs:1067`) `moved_site(&b.name).is_none()` filter can
never exclude an array. `Liveness::dead` (`src/check/engine.rs:777`) is the only remaining
guard. This is a correct, permanent
property of the language, not a gap (D2).

**3. `check_terms` vs `check_terms_relaxed` is the fork, and `inline_combinator` is on the
wrong side.** `call` (`src/check/terms.rs:288`), `times` (`src/check/terms.rs:376`) and an
`if` arm (`src/check/terms.rs:818`) all compute
`releasable_into(scope, base_depth, outer_releasable, &siblings[at + 1..])` and pass the
result into `check_terms_relaxed`. `inline_combinator`'s body splice (`src/check/combinators.rs:374`, the tail of
the function, shared by every combinator regardless of `comb.word.poly`) calls plain
`check_terms`, which hardcodes an empty `outer_releasable` and `back_edge = false`
(`src/check/terms.rs:11` area, `check_terms`'s own call sites).

**4. The doorway grant is load-bearing, and the splice path lacks it.** Three programs, all
run on `e87bcae`:

```sooth
\ ACCEPTS: the times doorway grants `a` into the loop body, so the aliased
\ mutable borrow of `arr` is allowed
: main ( -- )
  0 4 fill | a |
  a | arr |
  4 [ | i | &!arr i >usize &!> i ! ] times
  &arr 2 >usize &> @ .
  arr drop ;

\ REJECTS (`aliased by a`): one later use of `a` withholds the same grant.
\ This is what proves the grant above is load-bearing, not incidental.
: main ( -- )
  0 4 fill | a |
  a | arr |
  4 [ | i | &!arr i >usize &!> i ! ] times
  arr drop
  &a 0 >usize &> @ .
  a drop ;

\ REJECTS: the identical shape routed through a combinator splice, which
\ carries no grant at all (recon 1's repro).
: main ( -- )
  0 4 fill | a |
  a [ 4 > ] c::filter drop drop ;
```

This is the whole slice in three programs: the doorways grant, the splice does not.

**5. It is a prerequisite of 10b/10c, not merely sequenced before them.** 10b deletes the
`times` intrinsic and moves `times` into `lib/combinators.sth`; 10c makes `if`/`cond`
ordinary words. Both convert a doorway that grants into a splice that does not, so recon
4's first program regresses at 10b unless this slice has landed. 10b would otherwise have
to implement D1 itself as unplanned scope.

**6. `inline_combinator`'s `check_terms` is one shared exit point for mono and poly.**
`comb.word.poly.is_some()` branches only the *argument* check
(`check_poly_combinator_args` `src/check/combinators.rs:414` vs the mono loop inside
`inline_combinator`); the body splice at `src/check/combinators.rs:374` runs unconditionally
for both. `while` is itself polymorphic, so one
fix site covers both.

**7. `inline_combinator` lacks `siblings`/`at`/`base_depth`/`outer_releasable`, and its one
call site has all four.** It is called from exactly one place (`src/check/terms.rs:661`, inside
`check_term`'s `TermKind::Call` dispatch), and `check_term` takes `live`, `at`, `terms`,
`base_depth` and `outer_releasable` as its own parameters, the same values `call`/`times`/
`if` use a few hundred lines away in the same function.

**8. There is a second wrong-side `check_terms`, on the argument path.**
`check_literal_against_declared_effect` (`src/check.rs:1318`) runs the caller's quotation *literal*
against the declared parameter effect, in the caller's own scope, through the plain root
entry point (`src/check.rs:1348`). It is reached from both of `inline_combinator`'s argument
paths (the mono loop at `src/check/combinators.rs:268`, `check_poly_combinator_args` at
`src/check/combinators.rs:480`) and rejects *before* the
body splice does, so D1 alone cannot discharge the `while`-nested-in-a-combinator shape.
Its three non-combinator callers are `materialize_quotation_at_boundary`
(`src/check/captures.rs:320`) and the two `if`-arm quotation-merge sites
(`src/check/terms.rs:954`, `src/check/terms.rs:970`); whether those deserve a grant is
7b's and 10c's question, not this slice's.

**9. The same hole is already open today, with no 6g change, and it is a silent wrong
value.** Run on `e87bcae`:

```sooth
\ ACCEPTS today, prints `0` then `9`: a mutation through `arr` is silently
\ visible through `a` on the next iteration. No combinator involved.
: main ( -- )
  0 4 fill | a |
  2 [ | i | a | arr | &a 0 >usize &> @ . true if &!arr 0 >usize &!> 9 ! else end arr drop ] times ;
```

So the granting rule is already wrong at the `if`-inside-`times` doorway, independent of
this slice. What 6g does is multiply its blast radius: every combinator call sited inside a
loop (legal since 6d) becomes a fourth doorway, and 10b/10c turn `times` and `if`
themselves into splices. Shipping D1 while leaving this open would trade a false rejection
for a silent wrong answer, which is not a trade this slice may make.

**Placebo warning, measured.** Appending `a drop` after that loop flips the program to a
*rejection on today's compiler* — for an unrelated reason (`references(rest, "a")` at
`references` (`src/check/engine.rs:790`) sees the trailing use and withholds the grant). A
reject golden written that way
passes green with and without the fix and pins nothing. The golden's program must have no
use of `a` after the loop.

**10. `back_edge` is *not* unobservable at a splice, and the reason is a name-hygiene
defect.** `alpha_rename_locals` (`src/ast.rs:1208`) renames the callee's *locals*;
`rename_call` (`src/ast.rs:1229`) deliberately leaves a call to a word or builtin
untouched (its final `name.to_string()`). A caller local sharing a name with a builtin the
spliced body calls internally is therefore read *in place of that builtin*, silently. Run
on `e87bcae`:

```text
1 >usize | len |  9 4 fill [ 4 > ] c::filter . drop  len drop   → prints 1
                  9 4 fill [ 4 > ] c::filter . drop             → prints 4
```

No diagnostic either way. This is a silent wrong value, not a loud failure, and it is the
one route by which a granted caller name can appear as a `Call` inside a spliced body.
Any argument that a splice's `back_edge` is unobservable depends on closing it (D5).

## Decided (locked)

**D1. `inline_combinator`'s body check (`src/check/combinators.rs:374`) becomes `check_terms_relaxed` with a
`releasable_into`-computed grant.** Forced by recon 3/6/7: this is the one call site on the
wrong side of 6f's own contract, and the three correct sites are the pattern to copy.

**D2. No change to `Moves`, `aliasing_origin`, or Copy-array move-blindness.** Recon 2 is a
property of the language, not a bug.

**D3. `lib/arrays.sth`'s `sort` header comment documenting the inline-everything/no-`while`
workaround is deleted, its rationale having been retested and found false.** `sort`'s code
is *not* restructured. **`sort`'s own doc comment separately justifies a fixed-bound
`times` over stopping early on the `u32` length bound — that rationale is unrelated to
aliasing and must stay.** Note `lib/arrays.sth` is currently untracked; if it has not
landed when 6g starts, the `sort` dogfood golden and this edit move to whichever commit
brings it in. Nothing in the suite builds that file today, so the dogfood also needs an
import helper of the kind `tests/phase4_combinators.rs` already has for
`lib/combinators.sth` (`run_src` writes to `temp_dir()`, so relative imports do not
resolve).

**D4. Both new relaxed calls pass `back_edge = true`, and at the literal check this is
required for soundness rather than chosen by convention.** At
`check_literal_against_declared_effect` the terms scanned are the caller's own literal,
which references caller locals directly, so a granted name provably does appear in that
scan; with `false` its last use would be treated as final even though the callee
re-executes the literal per iteration. At the body splice, `true` matches `call`/`times`
and is the conservative value (a granted name used inside is pinned live for the whole
body). Recon 10's hygiene defect is what would otherwise make the splice's flag
observable; D5 closes it, making the splice's value a uniformity choice rather than a
soundness one. Do not write a test claiming to pin the splice's flag, and do not treat a
green suite as evidence it is right.

**D5 (new). Reject binding a local whose name collides with a callable name.** Sites: the
mono `TermKind::Bind` arm (`src/check/terms.rs:141`) and the poly one
(`src/check/poly.rs:333`, inside `poly_term` `:316`), alongside the existing
`reject_variant_local` (`src/check.rs:886`) and `reject_duplicate_local` (`src/check.rs:906`)
checks. `is_builtin_word_name` (`src/check/declarations.rs:101`) already exists as the
predicate, and `extern_redeclaration_error` (`src/check/declarations.rs:107`) is the existing
precedent for rejecting a declaration
that reuses a builtin or word name; `reject_variant_local` is the precedent for rejecting a
*local* that collides with another namespace (enum variants). This closes recon 10 at the
root rather than patching splice hygiene, and it turns "a granted caller name can never
appear as a `Call` in a spliced body" from false-by-counterexample into true-by-construction.

**Coverage differs by site, deliberately.** The mono arm checks builtins, `env`,
`poly.env` and `poly.combinators`. The poly arm checks **builtins and `env` only**, because
`poly_term` has no `PolyCtx` parameter at all: reaching `poly.env`/`poly.combinators` there
means changing the signature of `pub fn check_poly_body` and editing `src/repl.rs`, which is
out of this slice's sanctioned files. **Recorded gap:** after 6g, a polymorphic word may
still bind a local named after a combinator or poly word
(`: pick ( 'T 'T -- 'T ) | len | | other | other drop len ;` compiles). Nobody has been able
to construct a wrong-value witness through the poly arm — the hygiene defect needs an
`alpha_rename_locals` splice, and no shape routes a poly-bound shadowing local into one — so
this is scoped as uniformity, not soundness. If someone does build that witness, closing it
is a separate slice that owns the `check_poly_body` signature change.

*Measured, with the compiler rather than a text scan:* a build enforcing D5 at both bind
sites compiles every file in `examples/` and `lib/` unchanged and passes the entire suite,
all 20 test binaries, 1513 tests, zero failures. It also rejects recon 10's shadowing
program. Blast radius is zero. (A regex over `|...|` is *not* a valid way to check this: `|`
is overloaded between bind groups and clause dispatch, and a naive scan reports false
clashes on clause bodies like `examples/shapes.sth:9`'s `| Circle dup * 3.14159 *`.)

**R1 (new, forced by recon 9). `releasable_into` (`src/check/engine.rs:811`) gains `live: &Liveness, at:
usize`, and its filter splits: a name bound in the current invocation
(`idx >= base_depth`) keeps today's `!references(rest, name)` rule verbatim; an ancestor
name is granted only if the caller's own liveness says it is dead there.**

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
`nested_uses` (`src/check/engine.rs:723`) attributes a use found *inside* `terms[at]` to
index `at` itself, and `dead` (`src/check/engine.rs:777`) is `last < at`, so
`live.dead(name, at)` is always false whenever the
term being granted into is itself the thing that uses the name — which is the entire reason
one grants. Asking at `at + 1` reproduces exactly what `!references(rest, name)` already
meant ("no residual use *after* this term") while still catching recon 9's wrap-around,
because a use anywhere in a `back_edge = true` body is recorded as `IMMORTAL_IN_BODY`
(`src/check/engine.rs:637`, via `record_granted_use` `src/check/engine.rs:712`), which is not
`<` any index.

**R1 is provably sound relative to today's compiler, not merely measured green.**
`Liveness::scan` (`src/check/engine.rs:640`) records a `Call` use at its index and
`nested_uses` (`src/check/engine.rs:723`) records uses inside `If`/`Quotation` at the
containing index; `references` (`src/check/engine.rs:790`)
recurses over exactly the same three `TermKind` variants, and `TermKind` has no other
nesting variant. So if `references(rest, name)` is true, some use sits at index `>= at + 1`
and `dead(name, at + 1)` is false: **`dead(name, at + 1)` implies `!references(rest, name)`,
so R1 grants a strict subset of what HEAD grants.** Over-permissiveness relative to HEAD is
impossible by construction. The `idx >= base_depth` branch is unchanged verbatim and carries
no new risk.

**What R1 actually does, stated plainly, because it is broader than "a wrap-around fix".**
Inside a `back_edge = true` body, `record_granted_use` (`src/check/engine.rs:712`) writes `IMMORTAL_IN_BODY`
for *any* mention of a granted ancestor name, including one `nested_uses` finds inside
`terms[at]` itself, and `usize::MAX < at + 1` is never true. Therefore: **an ancestor name
mentioned anywhere inside a back-edge body is never re-granted into a block nested in that
body; `releasable_into`'s ancestor branch is inert inside every loop and every `call`ed
quotation.** Do not sell R1 as narrow. In particular a combinator call over a *named* array
inside a `times` body compiles under D1+R2 and is **rejected** under D1+R2+R1 (measured).
That rejection is correct by the language's own rule — `filter` mutates in place, so the next
iteration re-reads a mutated `a` through a second name — so it must not be advertised as a
win this slice delivers.

**Accepted cost: R1 also rejects sound programs that compile today.** The class needs the
ancestor name to appear *by name* inside a back-edge body; a combinator splice never does
(the array arrives on the stack and callee locals are alpha-renamed), which is why the
splice, `while` and `sort` shapes all survive. Two witnesses, both measured accepting on
HEAD and rejecting under R1:

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

**A refinement that would recover both was built and is unsound — do not re-propose it.**
The candidate: for a back-edge body, grant iff the name is unmentioned in the siblings
*before* and *after* `at` (excluding `terms[at]`'s own subtree). It restores both witnesses
above and still rejects recon 9, but it accepts this, which prints `0` then `9`:

```sooth
\ danger: read and mutation both inside the granted-into term, in a loop.
\ Iteration 2's read through `a` sees iteration 1's write through `arr`.
\ HEAD accepts it too (a second pre-existing silent wrong value beyond recon 9);
\ R1 as specified rejects it; the refinement does not.
: main ( -- )
  0 4 fill | a |
  2 [ | i | true if a | arr | &a 0 >usize &> @ . &!arr 0 >usize &!> 9 ! arr drop else end ] times ;
```

So `danger` is a second justification for R1 and a second behaviour change to document, and
the over-strictness above is the honest price of closing it: the checker cannot distinguish
"mentioned inside the granted-into term but never read across the edge" from "read across the
edge" without machinery beyond this slice.

*Measured on a build with R1 applied to the three existing call sites alone (no D1, no
R2):* recon 9's program rejects; `danger` rejects; a two-level non-back-edge grant chain that
compiles today still compiles (below); a three-level version of it also still compiles; a
wrap-around-through-an-earlier-sibling-literal shape rejects; the full suite stays green
(1513 tests).

*Measured on a build with all three together (R1+D1+R2), which no earlier revision checked
and which the phasing hides until Phase 3:* the splice shape accepts, the `sort` dogfood
accepts and sorts, recon 9 rejects, the in-loop combinator call rejects, suite green. **Put
this composition check in the spec: R1 lands in Phase 1 and D1 in Phase 3, so a composition
failure would not surface for two phases.**

```sooth
\ ACCEPTS today (prints 9) and must keep accepting: two levels of
\ execute-once nesting, `a` used only inside the innermost one.
\ R1 written with `at` instead of `at + 1` rejects this.
: main ( -- )
  0 4 fill | a |
  true if
    true if
      a | arr | &!arr 0 >usize &!> 9 ! &arr 0 >usize &> @ . arr drop
    else end
  else end ;
```

**R2 (new, forced by recon 8). `check_literal_against_declared_effect` takes the same grant
and its `check_terms` (`src/check.rs:1348`) becomes `check_terms_relaxed`.** Its three non-combinator
callers pass an empty set, preserving their behaviour exactly: with an empty grant,
`back_edge` only feeds `record_granted_use`, which fires only for names in
`outer_releasable`.

**Q2 (was open). The parameter is `granted: &HashSet<String>`, computed by the caller.**
Recon 8 decides it: the set must reach `check_literal_against_declared_effect`, which sits
two frames below `inline_combinator` and has neither a sibling list nor an index of its own.
One `HashSet` threaded down beats re-deriving position in a function that has no position.

## Invariant worth recording while it is still true

Grants are handed out **capture-blind** — `releasable_into` never consults captures — and
capture-awareness lives at the *use* site: `live_derivs` (`src/check/engine.rs:953`) and
`aliasing_origin` (`src/check/engine.rs:1067`) each compute `capture_alive_names`
(`src/check/engine.rs:849`) and check
`!live.dead(name, at) || captured.contains(name)`. `capture_alive_names` is a fixpoint over
stack slots and bound locals covering both `Known` markers and 7b's erased-closure
`surviving` sets. `live.dead` has exactly four call sites: those two, plus two internal to
the capture machinery (`src/check/engine.rs:859` area, `past_last_use_capture`
`src/check/engine.rs:923`). R1 adds a fifth, inside
`releasable_into` — but that is a *grant* site, not an aliasing-use site, so it needs no
disjunct. A future slice adding a consumer of `live.dead` on an aliasing path without the
capture disjunct would break this silently, and nothing currently records it.

Second half of the same invariant: **`dead`'s `None` arm is fail-open.** It returns
`outer_releasable.contains(name)`, so a name `Liveness::scan` fails to record is granted,
not withheld. Today that is safe because `scan` (`src/check/engine.rs:640`) and `references`
(`src/check/engine.rs:790`)
traverse the identical three nesting variants, which is exactly what makes R1 a strict
subset of HEAD's grants. **A future `TermKind` with nested terms added to `references` but
not to `scan` would silently open a hole.** (Checked against the in-flight 6h: its
`TermKind::ArrayCtor(Type)` carries a type, not terms, so it is not such a variant.)

## Open questions the spec must answer

- **Does any shipped `.sth` need editing to positively pin the fix, and what does that cost?**
  `examples/filter_while.sth` binds through producer words to dodge this bug, and its
  comment says so. Binding first would convert it into a positive pin. **But that example is
  in `tests/qbe_baseline.rs`'s `CORPUS`, whose golden asserts the emitted `.ssa` is
  byte-identical to `tests/qbe_baseline/filter_while.ssa`** — so editing its source requires
  sanctioning a baseline regeneration and reviewing that diff. **Re-derive this after 6h
  merges rather than inheriting the answer:** 6h regenerates every baseline in that corpus
  anyway, so "avoid a baseline regeneration" will no longer be the deciding cost, and the
  blanket "everything lowers byte-for-byte" claim has to be re-stated against the merged
  tree either way.
- **D1 versus R2 witnesses — partly answered, and one half is not available.** Reverting R2
  with D1 present turns a `while`-shaped accept golden red while the plain splice golden
  stays green, so R2 is demonstrably not redundant. The converse does **not** hold: every
  `while`-with-a-borrowing-literal shape goes red under a D1 revert *too*, because `while`
  is itself a combinator and therefore traverses the argument path and its own body splice.
  No discriminating shape is known to exist. The spec must state that the `while` golden
  pins D1+R2 jointly and rest R2's non-redundancy on the half that does discriminate,
  rather than asserting a clean separation.
- **Which of the two arrays `sort` returns holds the sorted result** — the dogfood golden
  must say. `sort` returns `ra rs` (`lib/arrays.sth:124`): sorted first, leftover scratch on
  top, and it needs a caller-supplied scratch of the same length. A golden that reads the
  top-of-stack array after `a::sort` reads the scratch and prints zeros, which looks like a
  broken fix and invites a golden written to expect zeros. Measured: fed by producer words,
  `sort` prints `1 2 3 4`; the same call with both arrays bound to locals fails today with
  `cannot borrow cs__inl0 mutably: it is aliased by s0`, which is the bug this slice fixes.

## Test and mutation requirements

- **Mutation phasing is a trap this slice has to avoid, in both directions.** R1 is a pure
  tightening and can land alone; D1/R2 are the relaxation. A reject golden for recon 9
  discriminates R1 in a relaxation-free tree, but **any golden whose program needs D1 to
  compile at all cannot go red when R1 is reverted in an R1-only tree**, so it is not an R1
  witness and must not be listed as one. The dual trap is the one an earlier revision fell
  into: **reverting R1 to the pre-R1 filter is a *loosening*, and a loosening can never turn
  an *accept* golden red.** So the accept golden that discriminates `at + 1` from `at`
  belongs only to the `at + 1` → `at` mutation, never to the full-revert mutation, whose only
  witness is the reject golden. Each mutation must name the phase it is runnable in and the
  specific test it turns red, and the direction of the mutation must match the kind of
  golden named.
- The wrong-value cases must assert the *value*, not just that the program compiles: under
  a reverted build recon 9's program must run and print `0` then `9`. A reject test that
  only checks "fails" cannot distinguish this from a type error, and the wrong value is the
  point.
- R1 additionally warrants a direct `#[cfg(test)]` unit test on `releasable_into`
  constructing a `Scope` plus a `Liveness` built via `scan` over synthetic terms, because
  the end-to-end goldens reach the rule only through several nested invocations. Precedent:
  6f's R6 walk-stop test, which became a unit test for the same reason. The unit test must
  cover both the `IMMORTAL_IN_BODY` case and the `None`-arm case (granted, never mentioned,
  still granted), so the tightening is shown not to over-tighten. **Note which half each
  mutation kills:** the `None`-arm half passes trivially through `dead`'s fail-open arm as
  long as the `outer_releasable.contains` conjunct survives, so deleting the `live.dead`
  conjunct turns only the *first* half red. Stated as a two-sided pin it reads stronger than
  it is.
- Run mutations in a **throwaway copy** of the tree, never the shared worktree: a concurrent
  reviewer has previously mistaken an in-place mutation for a real bug. There is no tooling
  for this, so the spec should say the mechanism outright, and keep `target/` copies off
  `/tmp` (a 32G tmpfs shared across sessions; an unrelated session has already been
  ENOSPC'd by orphaned scratch dirs).

## Out of scope

Restructuring `lib/arrays.sth`'s `sort` (D3 requires only the stale rationale to go). Any
change to `Moves`/move-tracking for `Copy` types (D2). Whether the three non-combinator
`check_literal_against_declared_effect` callers deserve a grant (R2). The `PolyType::Ref`
gap. Any lowering, IR, or diagnostic-text change beyond D5's new diagnostic.

**Sequencing: after 6h, before 10b/10c.** 10a has merged (`e87bcae`), so the old blocker is
gone. The 10b/10c relation is a prerequisite, not a preference (recon 5). The 6h relation is
the opposite — pure mechanical courtesy, no dependency either way — but it is worth honouring:
6h is well advanced, moves every anchor in this document, has hunks inside two functions this
slice edits, and regenerates every QBE baseline. Landing 6g first buys a conflict in exactly
those two functions plus a re-measure against a tree whose lowering changed anyway; landing it
second costs one re-anchoring pass.

Note `ROADMAP.md`'s own "Next action" pointer (line 589) still reads "Phase 4 Slice 10a" and
its 6g entry still prescribes the `self_tail`-conditioned `back_edge` that D4 rejects; both are
stale text that the ROADMAP edit should correct.
