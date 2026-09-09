# P7.S8 follow-up: splice-origin span in unsatisfied-bound reporting (brief)

Linear: [SOO-7](https://linear.app/ordfruma/issue/SOO-7/p7-follow-up-s8-unsatisfied-ord-attribution-splice-origin-span-in).

## Trigger

P7.S8 (`docs/roadmap/P7-language-prereqs.md`) made `lib/core/cmp.sth`'s six surface
comparisons (`eq`/`lt`/`gt`/`lte`/`gte`/`ne`) and `Ord`'s `cmp` member `inline`, so a
user's `lt`/`gt`/... call gets `cmp`'s body spliced into it at check time with no call
frame. Its own follow-up list named a diagnostic-attribution gap it deliberately left
unfixed, "a diagnostics feature with its own design surface, not a uid fix." This brief
is that follow-up.

## The gap (measured)

```sooth
import: intrinsics * ;
import: core::prelude * ;

type: Blip n i64 ;
: main ( -- )
  1 Blip |a| 2 Blip |b|
  &a &b lt drop
  a drop b drop ;
```

`Blip` has no `impl: Ord`. Building this reports:

```text
error: cannot instantiate `'T` of `cmp` with `&Blip` in `main` (line 146, col 3)
  `&Blip` does not satisfy `Ord`: no `( &Blip &Blip -- Ordering )` found
```

Line 146, col 3 is `lib/core/cmp.sth`'s own `cmp` token — the line inside `lt`'s body
(`: lt inline ['T: Ord] ( 'T 'T -- Bool ) cmp ...`), not line 7 of `main.sth`, where the
user actually wrote `lt`. The callee named is `cmp`, the spliced trait member, not `lt`,
the word the user called. The second line (what `impl: Ord` would have to provide) is
unaffected and correct.

## Root cause

Check-time inlining, not lowering. `inline_combinator` (`src/check/combinators.rs:313`)
splices a callee's body terms (`comb.terms`) into the caller's walk, alpha-renamed for
locals but with **spans left untouched** (`rename_terms` copies `term.span` verbatim). So
once `lt`'s body is spliced into `main`, every term in the walk carries `lt`'s own spans —
`lib/core/cmp.sth`'s spans — including the `cmp` call inside it. `cmp` is itself a bound
trait member, so checking that inner call recurses through `resolve_splice_member_call`
(`src/check/poly.rs:1607`), and its bound-satisfaction failure raises
`unsatisfied_user_bound_error` (`src/check/poly.rs:9786`) with the `span` and the callee
`name` (`"cmp"`) live at that point in the walk — both already inside `lib/core/cmp.sth`,
because the splice replaced the user's call site before the checker ever saw it.

`ctx.rendered_word()` (the enclosing word name in the error, `main`) is unaffected — the
`Ctx` doesn't change across a splice. Only the *callee name* and *span* inside the message
are wrong.

There is currently **no mechanism at all** for recovering the original call site once a
splice starts: `Provenance` (`src/check/engine.rs:130`) tracks `member_splice_stack:
Vec<usize>` (word indices, for member-splice-cycle detection) and `splice_uid: Option<u32>`
/ `poly.combinator_name` (the *innermost* active splice's own uid/name, saved and restored
per nesting level), but nothing records the outermost splice's own call-site span or
surface name. That is a gap, not a bug in an existing mechanism — matching the Linear
issue's framing of this as a new design surface rather than an extension of the P7.S8 uid
rule (`MEMBER_SPLICE_SUFFIX`/`splice_uid_stack`, which is a *lowering*-time, not
check-time, concern and is unrelated to this gap).

## What this is not

- **Not the P7.S10 recursion-budget mechanism.** That guard is at lowering
  (`lower_resolved_word_call`, `src/ir/func_builder/calls.rs`) and bounds an unbounded
  *lowering-time* recursion. This gap is a *check-time* span/name attribution problem on an
  otherwise-terminating, correctly-rejected program — nothing here loops or overflows.
- **Not a `member_splice_stack` extension.** That stack exists to detect a member-splice
  cycle (a bound dispatch reaching back to a member already being spliced); it holds word
  indices, not spans, and re-purposing it conflates two different questions (is this a
  cycle vs. where did this call originate).

## Scope (settled — 2026-09-10 interview)

A splice-origin span and callee name threaded through the check-time splice walk, read by
the affected error builders in place of the innermost spliced-body span/name when a splice
is active.

- A new `Provenance` field — a single `splice_origin: Option<(Span, String)>`, not a stack.
  Set once when the *first* (outermost) `inline_combinator` splice opens (`prov.splice_uid`
  was `None` beforehand) and left untouched by any splice nested inside it — the outermost
  frame is always the term the user actually wrote in their own body before any splicing
  started, whatever library combinators are nested underneath it, so no intermediate frame
  needs preserving.
- Both `unsatisfied_user_bound_error` (`src/check/poly.rs:9786`, call sites `:1963`/`:9129`)
  and `unresolved_trait_obligation_error` (`src/check/poly.rs:9834`, call site `:1982` — the
  R17 backstop one function over, reached under identical splice conditions) read
  `prov.splice_origin` when `Some`, in place of the live `span`/`name`, when the failing
  bound is discovered inside a splice. A further scan of any other error site reachable from
  inside `inline_combinator`'s body walk while `prov.splice_uid.is_some()` is implementer
  due-diligence, not expected to surface more genuine cases (these two share the one
  trait-member-call-checking function that runs under a splice; nothing else raises off a
  bound-satisfaction failure there).
- **Message shape: bare rename, no dispatch-chain annotation.** The fixed message names
  only the outermost call (`lt`) at its real call site — no `` `lt` -> `cmp` (member of
  trait `Ord` for ...) `` chain in the `combinator_cycle_error`/P7.S10 house style. The
  second line (`` `Blip` does not satisfy `Ord`: no `( &Blip &Blip -- Ordering )` found ``)
  already names `Ord` and the missing signature, so a chain would be redundant here; that
  style earns its keep in `combinator_cycle_error` because a *cycle* is inherently about the
  path, which a single bound-satisfaction failure is not.

## Out of scope

- Any change to lowering, `MEMBER_SPLICE_SUFFIX`, `member_splice_depth`, or the P7.S10
  splice budget.
- The REPL trait/impl accumulation follow-up (moot: the REPL is gone, P7.S9).
- Multi-file span rendering (the message shows no file for the wrong-attributed span
  either; out of scope unless the fix's own testing needs it to disambiguate).

## Exit (draft, for spec-writer to firm up)

The repro above reports `lt`'s call site in `main.sth` (its real line/col) and names `lt`
as the callee, not `cmp` at `lib/core/cmp.sth`'s line. The second line
(`` `Blip` does not satisfy `Ord`: no `( &Blip &Blip -- Ordering )` found ``) is unchanged.
A golden test pins the exact new message text. `cargo fmt --check && cargo clippy -- -D
warnings && cargo test` is green.
