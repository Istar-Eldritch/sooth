# Phase 4 Slice 6e: `if` in a polymorphic body (brief)

Today a polymorphic body that branches is rejected outright: "`if` in the polymorphic body
of `{word}` is not yet supported". This is not hypothetical either. Confirmed by compiling
against the built compiler:

```
: mymax ( 'T: Copy Ord 'T -- 'T ) over over > if drop else swap drop end ;
=> error: `if` in the polymorphic body of `mymax` (line 2) is not yet supported

: mymax ( i64 i64 -- i64 ) over over > if drop else swap drop end ;   \ monomorphic twin
=> builds, runs, prints 7 for `3 7 mymax .`
```

Two real consumers make this worth a slice rather than a someday: `max` cannot move from
`BUILTIN_WORDS` to the library until this lifts (its `Ord`-bounded body is the `mymax`
above), and slice 7 (closures) needs it because a closure-taking word worth writing branches
on something. Both were confirmed by running, not asserted.

The rejection has stood since slice 1, which deferred it deliberately rather than half-build
it: `docs/phase4-slice1-spec.md:80` records that the half-built arm it replaced both
spuriously rejected a valid program (`choose`, below) and panicked the compiler on others (a
`^i64` allocated on one arm reaching `ir.rs`'s `drop: non-empty stack`). That same doc names
the fix precisely: **pop the condition off the `PolyType` stack, run a per-arm
unconsumed-linear check, and join the two arms' move-state** — mirroring the monomorphic
`if` arm (`check_term`'s `TermKind::If` case, `src/check.rs:6137`), none of which is lifted
to `PolyType` yet.

## Recon: measured against the built compiler and read against the current checker

**1. The rejection is a single, unconditional early-out.** `poly_term`'s `TermKind::If { .. }`
arm (`src/check.rs:3666`) returns the error at `:3674` before touching the stack, `scope`, or
recursing into either branch. There is no partial machinery left over from the pre-slice-1
half-build to clean up; this is a clean insertion point, not a patch to something broken.

**2. The sibling rejection one arm down is explicitly out of scope.** `TermKind::Quotation`
in the same match (`src/check.rs:3692`) rejects a quotation literal in a polymorphic body for
an unrelated reason (R5p: `poly_term`'s stack is `Vec<PolyType>`, which has nowhere to carry
the `quot` marker, and `PolyType` gains no variant for it, D1). That is slice 7's wall, not
this one's: a quotation acquires a runtime representation there, which is what a polymorphic
body would need to carry one through. **Consequence for this slice: a `PolyType` value on the
poly stack can never be a quotation**, because the literal that would produce one is already
rejected eagerly, upstream of any `if`. The monomorphic `if` arm's quotation-condition guard
(`reject_quotation_operand(ctx, span, "if")`, reached before the `Bool` check) and its
quotation-merge-at-join complexity (`different_quotations_at_join_error`,
`quotation_versus_value_at_join_error`, `src/check.rs:6814`/`:6825`) exist only because a
monomorphic `Slot` *can* carry a quotation marker. **The poly-`if` join is strictly simpler
than its monomorphic sibling on this one axis**: no quotation condition check, no
quotation-identity-at-join case, because the input that would trigger either is already a
dead branch by construction.

**3. The condition-pop and stack-shape join are close to a direct port.** The monomorphic arm
(`src/check.rs:6137`-`~6230`): pop the condition, check it's `Bool`, clone `scope` into
`then_scope`/`else_scope`, walk each branch over a cloned stack, `leave_block` each arm
(checks locals bound *inside* the arm are fully consumed, then truncates scope back to the
pre-`if` depth), compare the two residual stacks' lengths and per-slot types
(`branch_mismatch_error`/`branch_type_mismatch_error`, `:5610`/`:5622`), then join
`scope.moves` via `Moves::join` (`:655`). `PolyType` already derives `PartialEq, Eq`
(`src/ast.rs:511`), so the per-slot type comparison is a direct `==`, no new comparison logic
needed.

**4. The move-state join is the one piece that does not port as-is, because `PolyScope` has
no three-state move representation.** The monomorphic `Moves` (`src/check.rs:597`-`670`)
tracks each linear local as `Live` / `Moved(Span)` / `MaybeMoved(Span)`; `join` (`:655`)
reconciles two arms into `MaybeMoved` on disagreement, and `unconsumed()` (`:634`) counts
`MaybeMoved` as still-leaked (correct: one path did leak it) while also making a further
`take()` on it an error (correct: not safely usable past the join either). `PolyScope.moves`
(`src/check.rs:3458`) is flatter: `HashMap<String, Option<Span>>`, `None` = live, `Some(span)`
= moved, with no encoding for "moved on exactly one arm". Verified concretely against the
`choose` example from slice 1's own spec:

```
: choose ( 'T 'T bool -- 'T ) | a b flag | flag if a b drop else b a drop end ;
```

`a` and `b` are each consumed exactly once, but at a *different point* per arm (then-arm:
`a` pushed and kept, `b` pushed-then-dropped; else-arm: the reverse). This is exactly the
program slice 1 says the old half-built arm spuriously rejected. A join that cannot
distinguish "consumed on both arms, though at different sites" (must end up `Moved`, not
leaked) from "consumed on only one arm" (must end up flagged, not silently `Live`) will
either re-introduce that false rejection or silently accept a real leak. **Locked as D2
below: lift `MoveState`'s three states to `PolyScope`, not a boolean.**

**5. `PolyScope.locals` is a flat `HashMap`, not the ordered, truncatable structure
`Scope.bound: Vec<Binding>` is.** The monomorphic `leave_block` relies on `Scope.bound` being
an append-ordered `Vec` so it can truncate back to the pre-block `depth` (`Scope::depth`,
`:702`), which is how a local bound *inside* one arm (and not the other) goes out of scope at
that arm's end and cannot leak into the join or be read past it. `PolyScope.locals`
(`src/check.rs:3458`) has no depth concept at all today, because nothing before this slice
ever left a block. **This needs the same before/after-snapshot or ordered structure that
`Scope` already has**, so an arm-local binding is (a) checked for consumption before the arm
ends and (b) removed before the join, mirroring `leave_block` exactly. This is the one place
the port is genuinely new plumbing rather than a straight lift, and it should be built to
look structurally identical to `Scope`/`leave_block`, not a parallel invention.

**6. Recursion gives nested `if` for free, the same way 6d's single-site loop fix gave every
call site the constant-stack fix for free.** `poly_term`'s `TermKind::If` arm, once it
recurses into `poly_walk(then_branch, ...)`/`poly_walk(else_branch, ...)` the way the
monomorphic arm recurses into `check_terms`, handles `if` nested inside `if` with no special
case: the inner `if` is just another term the recursive walk dispatches through the same
`poly_term` match. No test is needed to prove this beyond one dogfood example that happens to
nest, but it is worth exercising once so the claim is checked, not assumed.

**7. The existing rejection test is the one that flips meaning entirely.**
`check_poly_body_with_if_is_rejected` (`src/check.rs:8469`) asserts `choose` is rejected with
this exact message. Once the feature lands this test's *subject* becomes the load-bearing
positive case: `choose` must now compile, run, and print the larger of its two inputs at (at
least) two concrete instantiations, with the linear join actually reconciling `a`/`b`'s
per-arm consumption correctly rather than leaking or false-rejecting. The rename should make
clear it moved from a rejection witness to an acceptance witness.

**8. `mymax`'s remaining path to the library is now purely this slice.** The `Ord` bound
`mymax` needs already exists and its non-`if` operations (`over`, `over`, `>`) already
type-check on a `'T: Copy Ord` — the current rejection fires exactly at the `if`, nowhere
earlier. Once this slice lands, `mymax`'s only remaining blocker to sitting in
`lib/`-equivalent Sooth source rather than `BUILTIN_WORDS` is a decision this slice does not
need to make: whether to actually move it. That move is out of scope here (see below); this
slice only removes the compiler-side wall.

Probes live in `/tmp/mymax.sth`, `/tmp/mymax_mono.sth`, `/tmp/choose.sth` (all disposable,
rebuilt from the snippets above; the mono twin was run, not just built, printing `7`).

## Locked decisions

- **D1.** No new `Instr`/`Terminator`, no new lowering path. `poly_term`'s job is
  acceptance/rejection only; `check_poly_body`/`poly_walk` never lower anything (lowering is
  monomorphized separately, at the concrete instantiation, by the existing `check_word`
  path). This slice touches `src/check.rs` only, plus the one test file, `ROADMAP.md`, and
  dogfood examples.
- **D2.** `PolyScope`'s move tracking gains the same three states the monomorphic `MoveState`
  has (recon 4): live, moved-on-both-arms, moved-on-exactly-one-arm. Whether this is done by
  literally reusing `MoveState`/a `PolyMoves` newtype around it, or a fresh enum with the same
  three cases, is left to the spec; the *shape* is locked, a boolean is not sufficient.
- **D3.** The condition-pop path never needs a quotation guard and the join never needs a
  quotation-identity case (recon 2): a `PolyType` value is provably never a quotation, because
  the literal that would produce one is already rejected eagerly, upstream. Do not port
  `reject_quotation_operand`/`different_quotations_at_join_error`/
  `quotation_versus_value_at_join_error` to the poly side; they would be dead code by
  construction.
- **D4.** Nested `if` in a polymorphic body is accepted as a consequence of recursion (recon
  6), not built as a separate feature. One dogfood example should nest, to check the claim
  rather than assume it, but no dedicated "nested poly-if" mechanism is needed.
- **D5.** `check_poly_body_with_if_is_rejected` (`src/check.rs:8469`) is rewritten, not left
  alongside a new acceptance test: its subject (`choose`) is slice 1's own designated proof
  that a correct join doesn't false-reject, so it should become the primary positive
  regression test, renamed to say so.

## Open questions (for the spec)

- **Q1.** Concretely, how does `PolyScope` represent "a local bound inside this arm, not
  visible before it" for the `leave_block`-equivalent per-arm check (recon 5)? A snapshot of
  `locals.keys()` before the arm and a diff after is the minimal change; an ordered `Vec`
  matching `Scope.bound` exactly is more symmetric with the monomorphic side but a larger
  diff. The spec should pick one and say why, not leave it implicit.
- **Q2.** Do the new poly-side error messages get their own functions
  (`poly_branch_mismatch_error`, `poly_branch_type_mismatch_error`,
  `poly_local_unconsumed_error`-reused-or-not for an arm-local leak) or do the existing
  monomorphic ones get generalized to take a `PolyType`/`Type` via a shared trait or enum? The
  existing `poly_local_unconsumed_error` (`:4204`) and `poly_output_mismatch_error` (`:4302`)
  are already poly-specific and word-naming; the spec should decide whether the new ones
  follow that naming family or the monomorphic `branch_*_error` family's signatures
  (`ctx`/`span`-based, not `word`/`sig`-based) — these two families take different argument
  shapes today and an if-arm error could plausibly want either.
- **Q3.** What is the minimum instantiation-matrix test for the "two instantiations" exit
  criterion? `mymax`/`choose` called at an `i64` site and at a second `Copy + Ord` site (e.g.
  `f64`, or a user struct if one already satisfies both bounds) is the obvious pick; the spec
  should pin the exact second type so the test is deterministic rather than "any second
  type".
- **Q4.** Does `check_poly_combinator_standalone` (`src/check.rs:3493`, the `i64`-stand-in
  path used to check a polymorphic combinator's body once, standalone) need any change, or
  does it fall out unchanged because it delegates to the *monomorphic* `check_word` on a
  concretized copy rather than `poly_walk`? Recon did not exercise this path against an
  `if`-bearing combinator; the spec should confirm one way or the other rather than assume.

## Out of scope

- **Moving `max`/`mymax` from `BUILTIN_WORDS` to the library.** This slice removes the
  compiler-side wall; whether/when to actually relocate `max` is a separate decision for
  whenever core-library work starts (recorded in project memory as a deferred item), not a
  requirement of this slice's exit criteria.
- **The quotation-in-a-polymorphic-body rejection** (`src/check.rs:3692`). Sibling wall,
  belongs to slice 7 (recon 2).
- **Any change to `PolyType`, `PolySig`, `apply_subst`, or the monomorphization pipeline.**
  This is a checker-acceptance-only slice (D1); the concrete instantiation path is already
  correct once the poly-walk stops rejecting the body outright.
- **The `extern:`-vs-builtin intrinsic/library split itself** (which words move where, which
  stay compiler intrinsics). This slice is the gate that makes the split *possible* for
  `Ord`-bounded comparisons; it does not perform the split.
- **`while`/`times` inside a polymorphic body's `if` arms**, or any other loop-construct
  interaction. Nothing in this slice's recon found such an interaction blocked or
  special-cased; if the spec's own recon finds one, it should be called out explicitly rather
  than silently handled.

## Citations (verified against current `main`)

`poly_term`'s `TermKind::If` rejection: `src/check.rs:3666` (arm), `:3674` (message).
Sibling quotation rejection: `:3692`. Monomorphic `if` arm: `check_term`'s `TermKind::If`,
`src/check.rs:6137`. `Moves`/`MoveState`: `:597`, `:607`, `join` at `:655`, `unconsumed` at
`:634`. `Scope`/`Binding`/`depth`: `:680`, `:689`, `:702`. `leave_block`: `:5566`. `BlockEnd`: `:886`.
`branch_mismatch_error`/`branch_type_mismatch_error`: `:5610`/`:5622`.
`different_quotations_at_join_error`/`quotation_versus_value_at_join_error`:
`:6814`/`:6825`. `reject_quotation_operand`: `:6774`. `PolyScope`: `:3458`.
`check_poly_body`: `:3565`. `poly_walk`: `:3603`. `poly_term`: `:3621`.
`check_poly_combinator_standalone`: `:3493`. `poly_local_unconsumed_error`: `:4204`.
`poly_output_mismatch_error`: `:4302`. `PolyType` derive: `src/ast.rs:511`.
Existing rejection test: `src/check.rs:8469`. Only existing plan reference:
`docs/phase4-slice1-spec.md:80`. `mymax`/`choose` probes run against the built compiler on
`main` at `aa27529` (post-6d-merge): both poly bodies rejected with the exact "not yet
supported" message; the `i64` monomorphic `mymax` twin builds and prints `7`.
