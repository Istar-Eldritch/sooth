# P7b.S8 recon probe log — linear iterators (round P8)

- Date: 2026-09-05. HEAD: `ae6fdd7` (P7b.S6 + P7b.S10 landed; suite green in the
  p7b-s8 worktree, verified this date).
- Binary: worktree `target/debug/sooth` (dev profile, invoked directly; no cargo
  build during the round).
- Fixtures: `/tmp/p8-probes/*` (scratch only, not committed). Every invocation
  resolves imports against the shared fixture manifest
  (`tests/fixtures/sooth.pkg`) via `--manifest`.
- Method: four probe workers ran the brief's P8-1..P8-6 in parallel; each kept a
  verbatim log (P8-exhausted, P8-wall, P8-range, P8-consumers), concatenated
  verbatim below (each keeps its own header block). All commands were re-run
  once; outputs were stable. One worker (P8-wall) hit its runtime ceiling and its
  log was finalized from its session state; its citations were reconciled before
  assembly.

## Round-level finding — the briefed trait row does not parse (refutes the brief's premise)

`trait: Iterator['It: * -> *] : next ( 'It['T] -- Option['T] 'It['T] ) ;` is
rejected at **declaration time**. The member-row gate `member_shape_is_supported`
(`src/parser.rs:379`) admits a variable-headed App (`'F['U]` — S2's shipped
`Functor.map` shape) but rejects any **ctor-headed** App over the trait variable:
`Option['T]`, `Step['T 'It['T]]`, even non-nested `Step['T 'T]` (P8-exhausted b7).
The rejecting arm is the `Generic` case at `src/parser.rs:399-404`, message
`src/parser.rs:441-446`, enforced from `parse_trait_member_effect`'s post-parse
loop (`src/parser.rs:3947-3975`). The fence is therefore *not* about nesting —
it is "a ctor application may not appear in a member row". The brief's "the
compiler-side delta for `next` itself is close to zero" is **refuted**: the delta
is real, and it sits in the same function whose quotation-row arm S7 lifts
(`app_in_member_quotation_row_error`) — the 260905 sequencing note is confirmed
at the level of a shared gate function, not just a shared file.

What already grounds today (measured, per probe): variable-headed App rows
(`'It['T]`); concrete-applied types in ordinary signatures (`Option[i64]`,
`Step[i64 Count]`); two-param generic enums (`Step['T 'Rest]` declares; its ctors
work from any word whose *declared* input row mentions the header — grounding is
from the declared row, not the live stack, so `More` from `main` is "unknown
word" while `mk` from a `( i64 i64 -- Step[i64 i64] )` body works). A plain-word
count-up `next` and a plain-word List `next` both ground and run with **zero
compiler changes**; the E2E shapes the exit criteria want are only blocked by the
declaration-time gates and the impl-target fences below.

## Compiler-change ledger (union of the four logs; nothing was changed during the round)

1. **Member-row gate** (`src/parser.rs:399-404`): admit ctor-headed Apps in
   member rows. Blocks the trait declaration itself — every later probe
   compounds on this one arm.
2. **S2-6 concrete impl-target fence** (`src/parser.rs:4314-4323` →
   `src/ast.rs:2095`/`:2134-2143`, `is_concrete` `src/ast.rs:2480-2483`):
   rejects `impl: Iterator for Range[i64]` *and* `impl: Iterator for Count`
   ("the impl target `X` is concrete"). Ruling needed: lift for fully-applied
   ctor targets, or keep the fence and shrink the exit. The predicted kind error
   for non-generic targets **does not exist** — no code compares the trait's
   `var_kind` against the target.
3. **D5 borrow gate on generic member bodies** (`src/check/poly.rs:6572-6604`,
   message `:11251`): a generic-target `impl: Iterator for Range` grounds and
   registers, but its body dies at the first aggregate field read of a
   `Range['ctor0]` local. The arithmetic gap (`1 add` on `'T`) is never reached.
4. **Compound-type cross-call fence** (`src/check/poly.rs:4372-4380`): a poly
   word cannot call another poly word over compound types. The probe's
   plain-word proxy for bound dispatch was fenced by this — *member* dispatch
   through a bound (the `q map` precedent) is expected to work but is
   **unmeasurable until ledger item 1 lands**. Not recorded as a blocker;
   recorded as pending.
5. **S6 wall refined (P8-wall)**: the recorded wall is **field-shape-specific,
   not member-specific** — constructing a ctor whose field is the bare-`Generic`
   `^Self['T]` self-reference panics in `poly_bind_construction_arg`
   (`src/check/poly.rs:6117`; construction binding `:6235`) in *any* poly body
   (impl member and plain generic word alike), while plain-field ctors construct
   fine inside member bodies. Consequence: a plain-field `Range.next` does **not**
   need the wall lifted; List-reconstructing members (`map`, append) **do**.
6. **Exhausted-case semantics measured (P8-exhausted)**: with the `Option` row,
   arm parity forces the exhausted iterator to the *caller* — an explicit drop
   inside the exhausted arm is itself the parity breaker (the row demands a
   `Count` from every arm), call-site drop is enforced (forgetting it is a
   compile error), and a never-moved frame local is reclaimed implicitly. The
   `Step` shape internalizes the final drop inside `next`'s `Done` arm — one
   canonical site. Both shapes built and ran (`0 1 2`).
7. **Map semantics pinned (P8-wall P8-5)**: a bound-generic body cannot produce
   `'It['U]` and the iterator cannot be duplicated through a bound; per-impl
   List `map` is additionally wall-blocked (ledger item 5). The brief's
   "map must be per-impl member or for_each" reading is confirmed, with the
   per-impl variant blocked.
8. **P8-6 IR evidence (recorded, no verdict)**: the plain-word consuming loop
   lowers to **one frame with a back-edge**; the per-element work is a real
   indirect call; `list-next` is a real monomorphized frame, **not spliced**;
   Option dispatch after `next` inlines into the caller; a two-consumer chain is
   two dedicated frames; emitted `.ssa` byte-identical on re-emission.
9. **Fixture-surface notes**: intrinsics must be imported in CLI fixtures; ctor
   words need a prior signature mention of their header; member bodies are
   checked at impl declaration (no dispatch needed to trigger checks); struct
   accessors are fenced on poly targets while enum match-destructuring is not;
   the exhausted List-`next` Nil arm must construct the empty remainder
   (`drop None Nil` — the bare `drop None` fails arm parity; keeping the matched
   Nil hits the variant-escape rule, `src/check/poly.rs:11544`).

---

## p8-exhausted — P8-1 exhausted case + nested-App parser probes (P7b.S8)

Date: 2026-09-05
HEAD: ae6fdd7 (ae6fdd7fcf5b3025b69ceaa888d2fed5b81dc1b4, clean tree)
binary: /root/code/ordfruma/sooth-worktrees/p7b-s8/target/debug/sooth (invoked directly; no cargo build)
fixtures dir: /tmp/p8-probes (headline probes) + /tmp/p8-probes/isolate (attribution sub-probes)
manifest for every invocation: /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg

Every headline command was run once, then re-run once; all outputs were identical
(stable; no nondeterminism observed). One early harness note: a single shell line that
ran two sooth invocations back-to-back interleaved their stderr (P8-1c's error appeared
under the P8-1b marker). All probes were subsequently run as one invocation per command
and are stable. Repo untouched: read-only access + binary invocation only.

Summary verdicts

- P8-1a  refuted  — the Iterator trait does NOT declare today: `Option['T]` (a named
                    generic type applied over the trait variable) is fenced in trait
                    member signatures. `'It['T]` itself is fine.
- P8-1b  surprise — the two-param enum declaration parses; its ctors work from ordinary
                    words (b5/b6) but are "unknown word" when called from `main` / any
                    `( -- )` word (grounding rule, see P8-1b notes).
- P8-1c  refuted  — `Step['T 'It['T]]` in a trait member fails with the SAME fence as
                    P8-1a. The fence is the Generic shape (named type applied over
                    variables), not the nesting (b7). The nested-App question is never
                    reached.
- P8-1d(i)   refuted — with signature `( Count -- Option[i64] Count )`, the explicit drop
                    in the exhausted arm is itself the arm-parity breaker (d1b).
- P8-1d(ii)  split   — call-site drop is enforced (d5 error, verbatim below); the
                    inside-`next` drop is NOT enforced (d4 builds and runs; n1 shows
                    never-moved frame locals are reclaimed implicitly).
- P8-1d(iii) confirmed — both admissible shapes build and print `0\n1\n2\n`, exit 0:
                    d2 (Option shape, drop at the call site) and d3 (Step shape,
                    canonical drop inside `nexts`'s Done arm).

===========================================================================
P8-1a — does the Iterator trait itself declare today?

---

Hypothesis: `trait: Iterator['It: * -> *] : next ( 'It['T] -- Option['T] 'It['T] ) ; ;`
declares cleanly — `'It['T]` as a plain row operand is S2's shipped Functor.map shape.

Fixture: /tmp/p8-probes/p8-1a-iterator-declare.sth

```sth
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;
import: core::option * ;
trait: Functor['F: * -> *] :
  map ( 'F['T] [ 'T -- 'U ] -- 'F['U] ) ;
;
impl: Functor for Option
  : map swap ~[ ( Some ) Some> swap call Some ] ~[ ( None ) drop drop None ] Option? ;
;
trait: Iterator['It: * -> *] :
  next ( 'It['T] -- Option['T] 'It['T] ) ;
;
: main ( -- ) 3 Some [ 1 sub ] swap map . ;
```

(Functor trait/impl included only so `main` is a complete program; the new element
under test is the Iterator trait.)

Command:

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth build /tmp/p8-probes/p8-1a-iterator-declare.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (verbatim, both runs identical):

```text
error: trait `Iterator`'s member at line 12, col 3 has an unsupported signature shape (only concrete, array, and reference types over the trait's type variable -- plus a trait-var-headed application or an App-free quotation -- are supported)
exit=1
```

Verdict: REFUTED. The member `next ( 'It['T] -- Option['T] 'It['T] )` is rejected.
Isolation (fixtures in /tmp/p8-probes/isolate, same command shape):

- iso1.sth — `next ( 'It['T] -- 'It['T] )` → builds, exit 0. The trait-var-headed App
  in plain slots is fine, exactly the S2 Functor.map shape.
- iso2.sth — `next ( 'It['T] -- Option['T] )` → same unsupported-signature error.
- iso3.sth — `next ( Option['T] -- 'It['T] )` → same error. Position-independent.
So the fenced element is `Option['T]`: a NAMED generic type applied over the trait's
type variable. Fence site: `member_shape_is_supported` (src/parser.rs:379); the App arm
admits only the trait's own variable as application head (`PolyType::App { head, .. } =>
*head == 0`, src/parser.rs:387); a named-type application is a `PolyType::Generic`,
rejected by the Generic arm (src/parser.rs:400-404); enforced at the member gate
(`return Err(unsupported_trait_member_shape_error(...))`, src/parser.rs:3971); message
constructor `unsupported_trait_member_shape_error` (src/parser.rs:441-446).

Attribution refinement (b7.sth / b8.sth in isolate/):

- b7 — member `m ( 'F['T] -- Step['T 'T] )` with `type: Step['T 'Rest] | ... ;` (a
  non-nested Generic over trait variables) → SAME unsupported-signature error. So the
  fence is the Generic-over-variables shape, not anything about nesting.
- b8 — member `m ( 'F['T] -- Option[i64] )` (Generic over a fully concrete arg) →
  builds, exit 0. Concrete applications are materialized and admissible.
Consequence for the brief: the S8 `next` signature as written is not admissible without
a fence lift in `member_shape_is_supported`'s Generic arm (named-type application over
trait-variable arguments), or a different exhausted-case vehicle. Note the brief's
sequencing note names `parse_poly_app_arg` (src/parser.rs:5209) as a candidate fence
site; the observed error routes through `member_shape_is_supported` instead.

===========================================================================
P8-1b — does a two-param generic enum parse?

---

Hypothesis: `type: Step['T 'Rest] | Done | More 'T 'Rest ;` parses standalone.

Fixture: /tmp/p8-probes/p8-1b-step-two-param.sth

```sth
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;
type: Step['T 'Rest] | Done | More 'T 'Rest ;
: main ( -- ) 5 6 More drop ;
```

Command:

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth build /tmp/p8-probes/p8-1b-step-two-param.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (verbatim, both runs identical):

```text
error: unknown word `More` in `main` (line 5)
exit=1
```

Verdict: SURPRISE (split). The type DECLARATION itself parses — no diagnostic points at
the `type:` line, and the arity is not the problem (a one-param control, b2, fails the
same way; a two-param `Pair['A 'B]`, b3, ditto). What fails is calling the generic
enum's constructor from `main`.

Correction fixture b5 (/tmp/p8-probes/isolate/b5.sth) — same type, ctor called from an
ordinary word:

```sth
import: intrinsics * ;
type: Step['T 'Rest] | Done | More 'T 'Rest ;
: mk ( i64 i64 -- Step[i64 i64] ) More ;
: main ( -- ) 5 6 mk drop ;
```

→ builds, exit 0 (both runs). b6 — the nullary `Done` from a helper
(`: mk ( -- Step[i64 i64] ) Done ;`) → builds, exit 0. So: two-param generic enums
declare, and their ctors are usable from ordinary words.

Root observation (surprise finding; shapes all fixture authorship). A generic enum
constructor called in a PLAIN body grounds its type variables from the ENCLOSING
WORD'S DECLARED INPUT ROW; if that row offers no match, the dispatch reports the ctor
as an unknown word (error format at src/check.rs:1432):

- c4 — `: mk ( i64 -- Option[i64] ) Some ;` → builds (declared input row has `i64`).
- z1 — `: z ( -- ) 7 Some drop ;` → "error: unknown word `Some` in `z` (line 4)" — the
  live body stack DOES hold an `i64` (from the literal `7`), so grounding demonstrably
  does NOT use the running body stack in plain bodies.
- main is NOT special: o1 (main defined first, calls a later helper) and o2 (a helper
  defined after main uses `Some`) both build; z1 fails identically in a helper.
- Non-generic ctors need no grounding: c1 (inline `type: MaybeInt | None | Some v i64 ;`
  with `7 Some drop` in main) builds; z2 (imported `True` in main) builds.
- Inside splices (`~[ ... ]`), grounding uses the live splice row — d2's arms call
  `Some` with only an `i64` on the arm row and build (see P8-1d).
- Explicit instantiation is not available for ctors: z3 — `7 Some[i64] drop` in main →
  "error: `Some` (line 3) takes no type arguments; only a call to a polymorphic word
  may be explicitly instantiated".

===========================================================================
P8-1c — 'It['T] as a type argument INSIDE Step[...] (the nested-App probe)

---

Hypothesis: `trait: StepIt['It: * -> *] : next ( 'It['T] -- Step['T 'It['T]] ) ; ;`
parses/checks; if not, capture the error and locate the fence.

Fixture: /tmp/p8-probes/p8-1c-nested-app-trait.sth

```sth
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;
type: Step['T 'Rest] | Done | More 'T 'Rest ;
trait: StepIt['It: * -> *] :
  next ( 'It['T] -- Step['T 'It['T]] ) ;
;
: main ( -- ) 5 6 More drop ;
```

Command:

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth build /tmp/p8-probes/p8-1c-nested-app-trait.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (verbatim, both runs identical):

```text
error: trait `StepIt`'s member at line 6, col 3 has an unsupported signature shape (only concrete, array, and reference types over the trait's type variable -- plus a trait-var-headed application or an App-free quotation -- are supported)
exit=1
```

Verdict: REFUTED — and the failure is NOT a nested-App parse problem. The member's
`Step['T 'It['T]]` is a named (Generic) type applied over variables, which
`member_shape_is_supported` rejects wholesale (src/parser.rs:400-404, enforcement
src/parser.rs:3971, message src/parser.rs:441). b7 proves the nesting is irrelevant: a
NON-nested `Step['T 'T]` member fails with the identical error. The parser never gets
to ask the nested-App question. Ruling input: the `Step` exhausted-case shape is
inadmissible in a trait MEMBER signature today; it is admissible in ordinary word
signatures (d3 below uses `( Count -- Step[i64 Count] )` with concrete arguments, which
builds). A fence lift in the same Generic arm that P8-1a needs would admit both.

===========================================================================
P8-1d — Count exhausted case: where does the final drop live?

---

Type under test: `type: Count cur i64 limit i64 ;` (named-field struct, declared
inline; its non-generic ctor `Count` works in main — c1 precedent).

--- P8-1d first attempt (recorded for the record) ---
Fixture: /tmp/p8-probes/p8-1d1-drop-inside-arm.sth. My continue arm ended
`... Count Some` — but the freshly built `Count` sits ON TOP of the saved `i64`, so
`Some` consumed the Count. Output (verbatim):

```text
error: type mismatch in `nextc` (line 13)
  `Some` expected `i64`, found `Count`
  note: declared ( Count -- Option[i64] Count )
exit=1
```

This is a fixture bug, not a compiler finding (p5.sth reproduces it standalone:
`: p5 ( i64 i64 -- Option[i64] Count ) over 1 add swap Count Some ;`). Superseded by
d1b. It did yield two load-bearing mechanics facts, verified with declared-effect
micro-probes (t1/t2.sth):

- `&field @` is NON-consuming: it leaves the struct below the projected value
  (t1: `( Count -- i64 )` via `&cur @` errors "body leaves 2 values").
- Signature outputs are written bottom-to-top (t2: `( Count -- i64 Count )` errors
  "body leaves `Count` where the declaration requires `i64`" for a body leaving
  `[Count, i64]`).

--- P8-1d(i) the literal shape with a correct arm (drop inside the exhausted arm) ---
Fixture: /tmp/p8-probes/p8-1d1b-drop-inside-arm-corrected.sth

```sth
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;
import: core::option * ;

type: Count cur i64 limit i64 ;

\ P8-1d literal shape, corrected arm: the exhausted arm pushes None and
\ explicitly drops the Count; the continue arm pushes Some(cur) and an
\ advanced Count. Signature as specified in the probe task.
: nextc ( Count -- Option[i64] Count )
  &cur @ swap &limit @ rot swap lt
  ~[ &cur @ swap &limit @ swap drop | c l | c Some c 1 add l Count ]
  ~[ drop None ]
  if ;

: loopc ( Count -- )
  nextc swap
  ~[ ( Some ) Some> . loopc ]
  ~[ ( None ) drop drop ]
  Option? ;

: main ( -- ) 0 3 Count loopc ;
```

Command:

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth build /tmp/p8-probes/p8-1d1b-drop-inside-arm-corrected.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (verbatim, both runs identical):

```text
error: the quotations passed to `if` leave different stack shapes: an earlier one leaves `Option[i64] Count`, this one leaves `Option[i64]` in `nextc` (line 14)
exit=1
```

Verdict for (i): REFUTED. Under the specified signature `( Count -- Option[i64] Count )`
the explicit drop in the exhausted arm is exactly what breaks arm parity: the continue
arm returns element + remainder, the exhausted arm returns only the Option. `if`'s
contract requires both arms to share the output row `..b`
(lib/core/bool.sth, `: if inline ( ..a Bool ~[ ..a -- ..b ] ~[ ..a -- ..b ] -- ..b )`);
the join checker emits the error (src/check.rs:2983, emitted from the arm join at
src/check.rs:2651-2657). There is no arm arrangement that both drops the Count inside
the exhausted arm and keeps the signature: the asymmetry (one path preserves the
iterator, the other consumes it) is irreducible under a shared output row.

--- P8-1d(iii) working shape A: Option, drop at the call site ---
Fixture: /tmp/p8-probes/p8-1d2-option-drop-at-call-site.sth

```sth
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;
import: core::option * ;

type: Count cur i64 limit i64 ;

\ P8-1d working Option shape: BOTH arms return ( Option[i64], Count );
\ the original Count is dropped inside the continue arm after its fields
\ are copied; the exhausted arm returns the Count unconsumed. The final
\ drop therefore lives at the CALL SITE (loopc's None arm).
: nextc ( Count -- Option[i64] Count )
  &cur @ swap &limit @ rot swap lt
  ~[ &cur @ swap &limit @ swap drop | c l | c Some c 1 add l Count ]
  ~[ None swap ]
  if ;

: loopc ( Count -- )
  nextc swap
  ~[ ( Some ) Some> . loopc ]
  ~[ ( None ) drop drop ]
  Option? ;

: main ( -- ) 0 3 Count loopc ;
```

Command:

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth run /tmp/p8-probes/p8-1d2-option-drop-at-call-site.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (verbatim, both runs identical):

```text
0
1
2
exit=0
```

Verdict: CONFIRMED. Both arms return `( Option[i64], Count )` — the exhausted arm hands
the Count back unconsumed (`None swap`), the continue arm copies the fields, explicitly
drops the original, and builds the advanced Count. The specified signature is kept; the
build passes; the loop prints 0/1/2 and exits 0. The final drop lives AT THE CALL SITE
(loopc's None arm, second `drop`).

--- P8-1d(iii) working shape B: Step, canonical drop inside next ---
Fixture: /tmp/p8-probes/p8-1d3-step-drop-inside.sth

```sth
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;
import: core::option * ;

type: Step['T 'Rest] | Done | More 'T 'Rest ;
type: Count cur i64 limit i64 ;

\ P8-1d Step shape: the exhausted (Done) arm drops the Count INSIDE nexts --
\ one canonical drop site. Both arms leave exactly Step[i64 Count].
: nexts ( Count -- Step[i64 Count] )
  | c |
  &c &cur @ &c &limit @ lt
  ~[ &c &cur @ dup 1 add &c &limit @ c drop Count More ]
  ~[ c drop Done ]
  if ;

: loops ( Count -- )
  nexts
  ~[ ( More ) More> swap . loops ]
  ~[ ( Done ) drop ]
  Step? ;

: main ( -- ) 0 3 Count loops ;
```

Command:

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth run /tmp/p8-probes/p8-1d3-step-drop-inside.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (verbatim, both runs identical):

```text
0
1
2
exit=0
```

Verdict: CONFIRMED. The Step shape makes both arms leave exactly `Step[i64 Count]`
(delta 0 on both paths — `More` packs element + advanced Count, `Done` carries
nothing), so the exhausted arm CAN consume and drop the Count inside `nexts`: the
"one canonical drop site" the brief describes is realizable, with the final drop
INSIDE nexts's Done arm (`c drop Done`). The caller's Done arm drops only the Done
shell. Note this uses CONCRETE `Step[i64 Count]` in an ordinary word signature, which
is admissible today; the GENERIC trait-member version is what P8-1c fenced.

--- P8-1d(ii) teeth, inside the arm: REFUTED ---
Fixture: /tmp/p8-probes/p8-1d4-forgot-inside-drop.sth — identical to d3 except the
Done arm forgets `c drop` (arm reads `~[ Done ]`).
Command: same build command as d3 (file swapped).
Output (verbatim, both runs identical):

```text
(nothing — build succeeds)
exit=0
```

Runtime (sooth run, both runs identical): `0\n1\n2\n`, exit 0.
Baseline n1 (/tmp/p8-probes/isolate/n1.sth): `: leaky ( Count -- ) | c | ;` — a local
bound and never used at all — also builds. So the checker reclaims never-moved frame
locals at word end and is NOT path-sensitive about them: forgetting the explicit drop
inside `next`'s exhausted arm is NOT a compile error. (ii) as stated is refuted for the
inside case. The program behaves identically to d3 (the compiler back-stops the drop).

--- P8-1d(ii) teeth, at the call site: CONFIRMED ---
Fixture: /tmp/p8-probes/p8-1d5-forgot-call-site-drop.sth — identical to d2 except
loopc's None arm keeps a single `drop` (forgetting the Count's drop).
Command: same build command as d2 (file swapped).
Output (verbatim, both runs identical):

```text
error: the quotations passed to `Option?` leave different stack shapes: an earlier one leaves nothing, this one leaves `Count` in `loopc` (line 19)
exit=1
```

Verdict: CONFIRMED — this is the linear protocol's enforced teeth: a forgotten drop at
the call site is a compile error, reported as a cross-arm row mismatch naming the
leftover `Count` (same join checker, src/check.rs:2983 via the `?`-eliminator arm loop,
src/check.rs:2651-2657).

===========================================================================
P8-1 ruling inputs (what the brief asked this round to measure)

---

1. Where the final drop CAN live, per shape:
   - Option shape (`next : ( It -- Option[T] It )`): the drop CANNOT live inside
     next's exhausted arm (d1b — arm parity). It must live at the call site on every
     None path (d2) — every consuming loop pays a `drop drop` on its None arm.
   - Step shape (`next : ( It -- Step[T It] )`): the drop CAN live inside next's Done
     arm as the one canonical site (d3), and the caller's Done arm drops only the
     shell. However the compiler does not enforce the inside drop (d4/n1) — the
     enforced discipline is the row/call-site one (d5).
2. The Step shape is inadmissible in a generic trait MEMBER signature today (P8-1c,
   same fence as P8-1a's `Option['T]`): both need the same one-arm lift in
   `member_shape_is_supported` (src/parser.rs:400-404) to admit named-type
   applications over the trait's variable. Concrete-applied forms (`Step[i64 Count]`,
   `Option[i64]`) are admissible today (d3, b8).
3. Compiler changes NOT made (per task rules): the fence lift above is the only
   compiler-side delta this round's evidence points at; it is a checker/parser gate,
   not a lowering change.

Mechanics notes for future fixture authors (all verified this round):

- `if`'s FIRST quotation is the TRUE branch; `lt` is `( deeper top -- deeper<top )`
  (cmp.sth derives the six comparisons from `cmp ( 'T 'T -- Ordering )` with
  `| lhs rhs |` binding lhs to the deeper operand).
- `?`-eliminator dispatch arms must be annotated: bare `~[ ... ]` arms to `Step?`
  fail with "an arm of `Step?` requires a variant tag: annotate the quotation with the
  variant it handles, as in `~[ ( Circle ) ... ]` ... a forwarded quotation carries no
  annotation, so it cannot stand in for an arm" (observed on the first d3 draft).
- `<Ctor>` destructurers push first-declared field DEEPEST: `More>` leaves
  `[T, Rest]` with the Rest on top; `&field @` is non-consuming.
- Generic enum ctors in plain bodies ground from the enclosing word's DECLARED input
  row (P8-1b notes); inside splices they ground from the live splice row.
- Struct construction is `cur' limit' Count` (first field pushed first).

Fixture index (all under /tmp/p8-probes/ unless noted):

- p8-1a-iterator-declare.sth, p8-1b-step-two-param.sth, p8-1c-nested-app-trait.sth
- p8-1d1-drop-inside-arm.sth (superseded), p8-1d1b-drop-inside-arm-corrected.sth
- p8-1d2-option-drop-at-call-site.sth, p8-1d3-step-drop-inside.sth
- p8-1d4-forgot-inside-drop.sth, p8-1d5-forgot-call-site-drop.sth
- isolate/: iso1 iso2 iso3 b2 b3 b4 b5 b6 b7 b8 c1 c2 c3 c4 c5 c6 c7 c8 c9 m1 n1 o1 o2
  p5 p12 t1 t2 t5 z1 z2 z3 slice4-verbatim.sth

---

## P8 probe log — p8-wall (S8 construction wall + map semantics)

Date: 2026-09-05
HEAD: ae6fdd7 (worktree /root/code/ordfruma/sooth-worktrees/p7b-s8, clean, LOCKED — no repo files touched)
Binary: target/debug/sooth (prebuilt; invoked from the repo cwd; no cargo build run)
Fixtures dir: /tmp/p8-probes/ (all `p8-wall-*`; captured command outputs in /tmp/p8-probes/p8-wall-out/)
Manifest for every invocation: /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg

Reproducibility: every command below was run twice; outputs are byte-identical
except Rust's panic banner, which embeds the process/thread id
(`thread 'main' (3450322)`) — the sole nondeterminism observed. Exit codes
identical across runs. Stability capture: /tmp/p8-probes/p8-wall-out/*.1.txt vs*.2.txt.

Background: the recorded S6 wall is `monoid_for_list_append_construction_wall_is_recorded`
(tests/phase7b_slice6.rs:410); its comment (tests/phase7b_slice6.rs:396-403) attributes
the panic to the declared `^List['T]` self-reference field arriving at
`poly_bind_construction_arg` as a bare `PolyType::Generic` (comment cites the S6-era
line 6072; at this HEAD the same catch-all is src/check/poly.rs:6117, fn at 5998).

Mechanics discovered en route (they shaped every fixture):

- `swap`/`drop` are intrinsics and must be imported (`import: intrinsics * ;`);
  the cargo-test harness injects that line, a raw CLI fixture must write it.
- A generic-enum ctor word (`Some`, `Cons`, `Nil`) resolves in a module only once
  some word's signature names its header (e.g. a `showopt`/`mkopt` helper before
  `main`); an unmentioned header in `main ( -- )` gives `unknown word`. Reproduced
  under both the `--manifest` route and an ancestor-`sooth.pkg` tree
  (/tmp/p8-probes/p8-wall-pkg/), so it is not a `--manifest` artifact.
  Ship-shaped fixtures place a sig-mentioning helper before first ctor use,
  exactly like tests/phase7b_slice4.rs:505-519 and tests/phase7b_slice6.rs:100-114.

---

### P8-2a — construction of the impl's own generic struct (Box2, plain field) in a member body

Hypothesis (from the task + slice8-brief P8-2): mirroring the recorded witness with
Box2 — a trait-member body constructing the impl's own generic struct — hits the
`poly_bind_construction_arg` wall.

Fixture evolution is recorded because it carries findings: the witness shape
`Monoid['T] for Box2` with `combine ( 'T 'T -- 'T )` cannot be completed (getting an
element out of a `Box2` inside a poly member body is fenced — Interlude A), and the
task's suggested member sig `wrap ( 'T -- Box2['T] )` is itself fenced (Interlude B).
The final fixture uses a dispatchable Wrap member that constructs `Box2` from a
freshly supplied element.

Fixture (/tmp/p8-probes/p8-wall-2a-box2-member-construct.sth):

```sth
import: intrinsics * ;
type: Box2['T] v 'T ;
trait: Wrap['F: * -> *] :
  rebox ( 'F['E] 'E -- 'F['E] ) ;
;
impl: Wrap for Box2
  : rebox swap drop Box2 ;
;
: mk2i ( i64 -- Box2[i64] ) Box2 ;
: main ( -- ) 5 mk2i 7 rebox drop ;
```

Command (verbatim, run twice):

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth build /tmp/p8-probes/p8-wall-2a-box2-member-construct.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (build): (empty stderr) — exit: 0
Command: `./target/debug/sooth run /tmp/p8-probes/p8-wall-2a-box2-member-construct.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg`
Output (run): (empty stdout) — exit: 0

Verdict: **refuted (the wall does NOT fire for a plain-field struct) — the
load-bearing positive result.** A trait-member body CAN construct the impl's own
generic struct when the ctor's fields are plain `'T`/concrete fields. The poly
construction path (`poly_construct_generic`, src/check/poly.rs:6235) binds Box2's
`Var(0)` field from the member's variable via the `Var` arm of
`poly_bind_construction_arg` (src/check/poly.rs:5998) and leaves a symbolic
`Generic{Box2, [Var]}`; an earlier malformed-body variant even rendered the bound
result ("body leaves `Box2[Box2['ctor0]]`"), proving the symbolic bind. The S6 wall
is therefore NOT "any construction of the impl's own generic struct" — it is
specific to the bare-`Generic` self-reference field shape (P8-2b). This un-blocks
an S8 `Range` with plain fields (i64 cur/len): its `next` may construct the
advanced `Range` inside the member body without the wall.

#### Interlude A: struct accessors are fenced on poly targets

An earlier 2a variant (`impl: Monoid for Box2 : combine drop Box2> Box2 ;`, after a
`Box2[i64]`-mentioning helper so `Box2>` resolves) produced:

```text
error: `Box2>` is not permitted on a generic type `Box2['ctor0]` in `combine` (member of trait `Monoid` for `Box2['T0]`) (line 8)
exit: 1
```

Site: `poly_op_on_variable_error`, src/check/poly.rs:10986 (format at 11016). Contrast: enum
match-destructuring (`Cons>`/`Some>` + `List?`/`Option?`) over a poly target is
permitted and is what all shipped S6 impls do. Asymmetry worth an S8 note: inside a
member body you can take an enum impl target apart but not a struct impl target.

#### Interlude B: the task's suggested member sig is itself fenced

/tmp/p8-probes/p8-wall-tmp-2a-task-suggested-sig.sth — `trait: Wrap['F: * -> *]`
with member `wrap ( 'T -- Box2['T] ) ;`:

```text
error: trait `Wrap`'s member at line 4, col 3 has an unsupported signature shape (only concrete, array, and reference types over the trait's type variable -- plus a trait-var-headed application or an App-free quotation -- are supported)
exit: 1
```

Site: `member_shape_is_supported` (src/parser.rs:379), App arm `*head == 0`
(src/parser.rs:387); error text at src/parser.rs:443. Only the trait's own variable
may head an application in a member signature; `Box2['T]` is headed by the concrete
generic. Same fence as P8-5a.

---

### P8-2b — single non-recursive Cons construction inside an impl member body (the S8 shape)

Hypothesis: `impl: Prepend for List` whose member body performs one non-recursive
Cons construction fires the recorded wall.

Fixture (/tmp/p8-probes/p8-wall-2b-prepend-list-cons.sth):

```sth
import: intrinsics * ;
import: core::list * ;
trait: Prepend['P: * -> *] :
  prepend ( 'P['T] 'T -- 'P['T] ) ;
;
impl: Prepend for List
  : prepend swap ^ Cons ;
;
: mkempty ( -- List[i64] ) Nil ;
: main ( -- ) 3 mkempty ^ Cons 7 prepend drop ;
```

`swap ^ Cons` is the convention-correct body: `Cons 'T rest ^List['T]` consumes
element-below / tail-cell-on-top (cf. `: mklist ( i64 -- List[i64] ) Nil ^ Cons ;`,
tests/phase7b_slice6.rs:130-134; `^` is the built-in owned-cell wrap word,
src/check/word_families.rs:114). A literal `prepend Cons` body would mis-bind field
0 from the list operand and still panic at the same site.

Command (verbatim, run twice):

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth build /tmp/p8-probes/p8-wall-2b-prepend-list-cons.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (verbatim; only the pid varies between runs):

```text

thread 'main' (3450322) panicked at src/check/poly.rs:6117:18:
internal error: entered unreachable code: a generic `type:` field is never Generic { is_enum: true, idx: 0, module: 1, args: [Var(0)], len_args: [], name: "List" }
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
exit: 101
```

Verdict: **confirmed.** Same failure site as the recorded witness: the catch-all
`other => unreachable!("a generic`type:`field is never {other:?}")` at
src/check/poly.rs:6117 (the fn `poly_bind_construction_arg` starts at
src/check/poly.rs:5998). The field ptys come straight from the generic decl's
stored field types (src/check/poly.rs:6369-6382), and the `^List['T]`
self-reference field is stored as a bare `PolyType::Generic` — which no arm
(`Var`/`Concrete`/`App`/`OwnedCell`) covers. Non-recursive, single construction,
no `main` required: a main-less twin (/tmp/p8-probes/p8-wall-tmp-2b-nomain.sth)
panics identically, so the wall fires at **impl declaration** (member bodies are
checked eagerly), not at dispatch.

---

### P8-2c — enum-ctor construction (`Some`) inside a member body, same impl style

Hypothesis: in the same `Prepend` impl style, a member body constructing `Some`
works (S6 Functor-for-Option precedent), at this HEAD.

Fixture (/tmp/p8-probes/p8-wall-2c-prepend-option-some.sth):

```sth
import: intrinsics * ;
import: core::option * ;
import: hosted::show | . | ;
trait: Prepend['P: * -> *] :
  prepend ( 'P['T] 'T -- 'P['T] ) ;
;
impl: Prepend for Option
  : prepend swap drop Some ;
;
: showopt ( Option[i64] -- ) ~[ ( Some ) Some> . ] ~[ ( None ) drop ] Option? ;
: main ( -- ) 3 Some 7 prepend showopt ;
```

Command (verbatim, run twice):

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth run /tmp/p8-probes/p8-wall-2c-prepend-option-some.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (run, verbatim):

```text
7
exit: 0
```

(build: empty stderr, exit 0.)

Verdict: **confirmed.** Enum-ctor construction inside an impl member body grounds
and runs end-to-end at this HEAD (S6 precedent holds). `prepend` dispatches on
`Option[i64]`, the member body drops the old value and reconstructs `Some`, stdout
is `7`, exit 0.

---

### P8-2d — is the wall trait-member-specific? Plain generic word (no trait)

Hypothesis (task): if a plain generic word constructs Box2, the wall is specific to
poly member bodies.

Fixture (/tmp/p8-probes/p8-wall-2d-plain-generic-box2.sth):

```sth
import: intrinsics * ;
type: Box2['T] v 'T ;
: mk2['T] ( 'T -- Box2['T] ) Box2 ;
: main ( -- ) 5 mk2 drop ;
```

Command (verbatim, run twice):

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth run /tmp/p8-probes/p8-wall-2d-plain-generic-box2.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (run, verbatim): (empty stdout) — exit: 0 (build: empty stderr, exit 0)

Verdict: **confirmed (plain-field construction works outside traits too)** — but
the sharper half is 2d2 below: the plain word constructing `Cons` walls
identically, so the wall is NOT specific to trait members at all.

#### P8-2d2 — plain generic word constructing Cons

Fixture (/tmp/p8-probes/p8-wall-2d2-plain-generic-cons.sth):

```sth
import: intrinsics * ;
import: core::list * ;
: cons2['T] ( 'T List['T] -- List['T] ) ^ Cons ;
: main ( -- ) 5 Nil cons2 drop ;
```

Command (verbatim, run twice):

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth build /tmp/p8-probes/p8-wall-2d2-plain-generic-cons.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (verbatim; only the pid varies):

```text

thread 'main' (3450403) panicked at src/check/poly.rs:6117:18:
internal error: entered unreachable code: a generic `type:` field is never Generic { is_enum: true, idx: 0, module: 1, args: [Var(0)], len_args: [], name: "List" }
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
exit: 101
```

Verdict: **surprise vs the task's framing.** The wall is NOT trait-member-specific.
It fires in any *polymorphic* construction of a ctor carrying the bare-`Generic`
self-reference field — impl member body (2b) or plain generic word (2d2) alike.
What spares mono bodies (`: mklist ( i64 -- List[i64] ) Nil ^ Cons ;`, a shipped
golden at tests/phase7b_slice6.rs:130) is concreteness, not member-ness. Exact
statement of the wall at this HEAD: constructing a generic ctor whose field list
contains the self-referential `^Self['T]` field (stored as a bare
`PolyType::Generic`) panics at src/check/poly.rs:6117 whenever the construction is
checked symbolically (poly body). Plain-field ctors (Box2, Some, Nil) are
unaffected in both member and plain-word bodies.

---

### P8-5 — map semantics: can a bound-generic body produce `'It['U]`?

#### P8-5a — the brief's Iterator trait does not even declare (headline)

Hypothesis (task): declare `trait: Iterator['It: * -> *] : next ( 'It['T] --
Option['T] 'It['T] ) ;` and try a generic `imap` producing `'It['U]`.

Fixture (/tmp/p8-probes/p8-wall-5a-imap-cannot-produce.sth):

```sth
import: intrinsics * ;
import: core::list * ;
import: core::option * ;
trait: Iterator['It: * -> *] :
  next ( 'It['T] -- Option['T] 'It['T] ) ;
;
impl: Iterator for List
  : next ~[ ( Cons ) Cons> | v rest | v Some rest ^> ] ~[ ( Nil ) drop None Nil ] List? ;
;
: imap['It: Iterator 'T 'U] ( 'It['T] [ 'T -- 'U ] -- 'It['U] )
  next drop swap drop ;
;
: main ( -- ) 3 Nil ^ Cons [ 1 sub ] imap drop ;
```

Command (verbatim, run twice):

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth build /tmp/p8-probes/p8-wall-5a-imap-cannot-produce.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (verbatim):

```text
error: trait `Iterator`'s member at line 5, col 3 has an unsupported signature shape (only concrete, array, and reference types over the trait's type variable -- plus a trait-var-headed application or an App-free quotation -- are supported)
exit: 1
```

Verdict: **surprise (stronger than the brief's claim).** The brief's exact
`next ( 'It['T] -- Option['T] 'It['T] )` cannot be declared at this HEAD: the
S2-3 member-shape fence `member_shape_is_supported` (src/parser.rs:379, App arm
`*head == 0` at src/parser.rs:387, error at src/parser.rs:443) permits only
trait-var-headed applications in member signatures; `Option['T]` is headed by the
concrete generic. The brief's "the compiler-side delta for `next` itself is close
to zero" is refuted: S8 cannot write step 1 (the trait) without lifting this
fence or reshaping `next`. The fence site is exactly the class of parser fence
sites the brief's sequencing note flags for S7 coordination
(`member_shape_is_supported` lives in src/parser.rs). Note the fence rejects the
whole trait at declaration — the impl and `imap` below it were never reached.

#### P8-5b — with a fence-legal Iterator, the generic body cannot produce `'It['U]`

The fence-legal Iterator carries only trait-var-headed applications
(`next ( 'It['T] -- 'It['T] )`; a concrete impl still grounds). imap attempts the
only concrete construction available for the output (`Some`).

Fixture (/tmp/p8-probes/p8-wall-5b-imap-some-output-attempt.sth):

```sth
import: intrinsics * ;
import: core::list * ;
import: core::option * ;
trait: Iterator['It: * -> *] :
  next ( 'It['T] -- 'It['T] ) ;
;
impl: Iterator for List
  : next ~[ ( Cons ) Cons> | v rest | v drop rest ^> ] ~[ ( Nil ) drop Nil ] List? ;
;
: mkopt ( i64 -- Option[i64] ) Some ;
: imap['It: Iterator 'T 'U] ( 'It['T] [ 'T -- 'U ] -- 'It['U] )
  drop next Some ;
: main ( -- ) 3 Nil ^ Cons [ 1 sub ] imap drop ;
```

Command (verbatim, run twice):

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth build /tmp/p8-probes/p8-wall-5b-imap-some-output-attempt.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (verbatim):

```text
error: stack effect mismatch in `imap`
  body leaves `Option['It['T]]`, but the declared outputs are `'It['U]`
exit: 1
```

Verdict: **confirmed — the pin.** The bound-generic body cannot construct
`'It['U]`: the only constructible `* -> *` values are concrete-header ctors, and
the checker refuses the substitution at the output position (located error, not a
panic; it does not bind `'It := Option`). Together with 5a this pins the brief's
claim by two gates: at this HEAD a bound-generic map can neither consume
element-wise (no `Option`-returning member can be declared) nor produce
`'It['U]`. Map must be a per-impl member (List's reconstruction then hits the
P8-2b wall) or the exit must mean a consuming `for_each`.

#### P8-5c — the reconstruction attempt cannot even duplicate the abstract iterator

Fixture (/tmp/p8-probes/p8-wall-5c-imap-cons-reconstruct-attempt.sth):

```sth
import: intrinsics * ;
import: core::list * ;
trait: Iterator['It: * -> *] :
  next ( 'It['T] -- 'It['T] ) ;
;
impl: Iterator for List
  : next ~[ ( Cons ) Cons> | v rest | v drop rest ^> ] ~[ ( Nil ) drop Nil ] List? ;
;
: mklist1 ( i64 -- List[i64] ) Nil ^ Cons ;
: imap2['It: Iterator 'T 'U] ( 'It['T] [ 'T -- 'U ] -- 'It['U] )
  drop next dup ^ Cons ;
: main ( -- ) 3 Nil ^ Cons [ 1 sub ] imap2 drop ;
```

Command (verbatim, run twice):

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth build /tmp/p8-probes/p8-wall-5c-imap-cons-reconstruct-attempt.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (verbatim):

```text
error: cannot `dup` a generic type applied to a variable in `imap2` (line 11)
  `'It['T]` is conservatively linear: it may carry a linear argument at some instantiation, so it cannot be duplicated
exit: 1
```

Verdict: **confirmed (additional gate).** The Cons-reconstruction attempt cannot
even get its two operands: `dup` on an abstract `'It['T]` is fenced
(`poly_copy_generic_error`, src/check/poly.rs:10947 — conservative linearity of
generic applications). Without `dup` the body has at most one value, under
`Cons`'s two-field arity; the S8-relevant reading is that an iterator is
single-ownership through the protocol, so P8-4's consuming loops must be shaped
without duplicating the iterator.

---

### Summary for the S8 spec

1. The S6 wall is field-shape-specific, not member-specific: constructing a ctor
   with the bare-`Generic` `^Self['T]` self-reference field panics at
   src/check/poly.rs:6117 in ANY poly body (impl member P8-2b, plain generic word
   P8-2d2); plain-field ctors construct fine in member bodies (P8-2a, P8-2c, P8-2d).
   Consequence: a plain-field `Range.next` (construct the advanced Range) does NOT
   need the wall lifted; List-reconstructing members (map/append) DO.
2. The brief's `Iterator.next` signature is undeclarable at this HEAD
   (S2-3 fence, src/parser.rs:379/387/443) — an S8-blocking compiler delta the
   brief did not predict ("delta close to zero" refuted), in exactly the parser
   fence region the S7 sequencing note reserves.
3. A bound-generic map producing `'It['U]` is impossible (P8-5b), and the
   iterator cannot even be duplicated through a bound (P8-5c): the brief's
   reading ("map must be per-impl member or for_each") is confirmed, with the
   per-impl List map additionally blocked by the P8-2b wall.
4. Fixture-surface notes for whoever writes the S8 goldens: intrinsics must be
   imported in CLI fixtures; ctor words need a prior signature mention of their
   header; member bodies are checked at impl declaration (no dispatch needed to
   trigger checks); struct accessors (`X>`, `&x @`) are fenced on poly targets
   while enum match-destructuring is not.

Exploratory fixtures kept under /tmp/p8-probes/: p8-wall-tmp-2b-nomain.sth (wall
without main), p8-wall-tmp-2a-task-suggested-sig.sth (fence on the suggested sig),
p8-wall-tmp-some-min.sth / p8-wall-tmp-mkopt.sth / p8-wall-tmp-named-import.sth /
p8-wall-tmp-some-show.sth (ctor-minting quirk), p8-wall-pkg/ (ancestor-manifest
route control), p8-wall-out/ (double-run stability captures).

---

## p8-range — P7b.S8 probe round: Range impl-target shapes (P8-3)

Date: 2026-09-05
HEAD: ae6fdd7
Binary: target/debug/sooth (called directly; no cargo build; repo locked and clean)
Worktree: /root/code/ordfruma/sooth-worktrees/p7b-s8
Fixtures: /tmp/p8-probes/p8-3a/ /tmp/p8-probes/p8-3b/ /tmp/p8-probes/p8-3c/ /tmp/p8-probes/p8-3d/
Manifest for every invocation: /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
Determinism: every command below was run twice; all outputs were byte-identical across runs. No non-determinism observed. One fixture self-fix (my own, not a compiler behavior) is recorded under P8-3d.

### Round-level finding: the brief's trait row itself does not parse

The slice8 brief's core shape — `next ( 'It['T] -- Option['T] 'It['T] )` — is
rejected at TRAIT DECLARATION time, before any impl target is examined. A
constructor application (`Option['T]`, a `PolyType::Generic`) is not an
admissible trait-member shape: `member_shape_is_supported` (src/parser.rs:379)
returns `false` for `PolyType::Generic { .. }` (src/parser.rs:399-404). The gate
loop runs in `parse_trait_member_effect` (src/parser.rs:3920, loop at
src/parser.rs:3952-3974) and raises `unsupported_trait_member_shape_error`
(src/parser.rs:441). The brief's claim "the compiler-side delta for `next`
itself is close to zero" is therefore wrong for the `Option['T]` half of the
row; only the `'It['T]` App slots are the shipped S2 shape. P8-1's `Step['T
'It['T]]` alternative dies on the same gate (a ctor application too).

All P8-3 target-shape probes were therefore run twice: once with the brief's
verbatim row (captures the gate), once with a reduced row
`next ( 'It['T] -- 'It['T] )` that parses, isolating the impl-target question.

---

### P8-3a — concrete-APPLIED target `impl: Iterator for Range[i64]` (verbatim row)

#### Hypothesis

`Range[i64]` may be rejected as an impl target; the task asks whether the
rejecting site is `member_app_abstract_target_error` and whether that guard
rejects abstract heads only or any applied target.

#### Fixture (/tmp/p8-probes/p8-3a/p8-3a-concrete-applied.sth)

```sth
\ P8-3a: concrete-APPLIED impl target — `impl: Iterator for Range[i64]`.
\ The trait member `next` applies the trait header variable ('It['T]).
\ Question 1: does Range[i64] parse/ground as an impl target at all?
\ Question 2: which guard rules — S2-6's concrete fence (parser.rs:4314) or
\ ground_member_poly's non-Generic guard (ast.rs:2370)?
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;
import: core::option * ;

trait: Iterator['It: * -> *] :
  next ( 'It['T] -- Option['T] 'It['T] ) ;
;

type: Range['T] cur 'T limit 'T ;

impl: Iterator for Range[i64]
  : next
    | r |
    &r &cur @ | cur0 |
    &r &limit @ | lim |
    cur0 lim lt
    ~[ cur0 Some cur0 1 add lim Range ]
    ~[ None ]
    if ;
;

: main ( -- ) 0 3 Range | it | drop drop ;
```

#### Command

```text
./target/debug/sooth build /tmp/p8-probes/p8-3a/p8-3a-concrete-applied.sth --manifest tests/fixtures/sooth.pkg
```

#### Output (both runs identical)

```text
error: trait `Iterator`'s member at line 12, col 3 has an unsupported signature shape (only concrete, array, and reference types over the trait's type variable -- plus a trait-var-headed application or an App-free quotation -- are supported)
exit=1
```

#### Verdict

REJECTED — but upstream of the impl-target question: the round-level finding
above fires first (S2-3 shape gate, src/parser.rs:399-404 via
src/parser.rs:3952-3974). The reduced-row probe below answers the actual
target question.

---

### P8-3a-reduced — `impl: Iterator for Range[i64]` with a parsable row

#### Hypothesis

With the row reduced to `( 'It['T] -- 'It['T] )`, does `Range[i64]` parse as an
impl target, and which guard rules it?

#### Fixture (/tmp/p8-probes/p8-3a/p8-3a-reduced-nooption.sth)

```sth
\ P8-3a-reduced: same concrete-applied target question, with the trait row
\ reduced to `next ( 'It['T] -- 'It['T] )` (the Option['T] ctor application
\ is rejected by the S2-3 member-shape gate before any impl is seen). This
\ isolates whether `impl: Iterator for Range[i64]` parses/grounds as a
\ target, and what the member body does to the free member local 'T.
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;
import: core::option * ;

trait: Iterator['It: * -> *] :
  next ( 'It['T] -- 'It['T] ) ;
;

type: Range['T] cur 'T limit 'T ;

impl: Iterator for Range[i64]
  : next
    | r |
    &r &cur @ | cur0 |
    &r &limit @ | lim |
    cur0 lim lt
    ~[ cur0 1 add lim Range ]
    ~[ r ]
    if ;
;

: main ( -- ) 0 drop ;
```

#### Command

```text
./target/debug/sooth build /tmp/p8-probes/p8-3a/p8-3a-reduced-nooption.sth --manifest tests/fixtures/sooth.pkg
```

#### Output (both runs identical)

```text
error: trait member `next` of `Iterator` (line 18, col 5) applies the trait variable `'It`, but the impl target `Range[i64]` is concrete
  an application-headed member has no monomorphic representation (its applied arguments are member locals); implement the trait for a constructor target with a type variable instead
exit=1
```

#### Verdict

REJECTED — by the S2-6 CONCRETE fence, not by the abstract-target guard. The
fired message ("the impl target `Range[i64]` is concrete") is
`member_app_concrete_target_error` (src/ast.rs:2095), raised only from
`fence_member_app_against_concrete_target` (src/ast.rs:2134-2143), called only
inside `if target.is_concrete()` in `parse_impl_member_body`
(src/parser.rs:4314-4323). `ImplTarget::is_concrete` is
`matches!(pattern, PolyType::Concrete(_))` (src/ast.rs:2480-2483), so
`Range[i64]` folds to a fully CONCRETE target pattern: the target parser takes
the ctor path (src/parser.rs:4119-4148) but a ctor application with no variable
argument and no padding mints the monomorphic instantiated type (the
"concrete target ... folds to PolyType::Concrete(t)" doc, src/parser.rs:4002
and src/parser.rs:4064). Confirmed empirically: only the is_concrete branch
can produce this message.

Nuance ruling (the task's question): `member_app_abstract_target_error`
(src/ast.rs:2072, raised at src/ast.rs:2370 in `ground_member_poly`'s App arm)
rejects ANY non-Generic target on the generic path — a bare-variable abstract
target (`for 'T`) and, in principle, other non-Generic patterns. But a
fully-concrete ctor application (`Range[i64]`, `Count`) never REACHES it: it
folds to `Concrete` at the target parser and is fenced earlier by S2-6. So the
S7 brief's claim is confirmed in effect ("concrete-applied impl targets are
located errors today") but its cited site is the wrong twin: the fence that
fires is S2-6 (`member_app_concrete_target_error`), not
`member_app_abstract_target_error`.

---

### P8-3a-fence — control: App-headed member vs truly concrete target `i64`

#### Hypothesis

The same S2-6 fence should fire for a plainly concrete target, confirming the
fence is the general concrete-target rejector.

#### Fixture (/tmp/p8-probes/p8-3a/p8-3a-concrete-fence.sth)

```sth
\ P8-3a-fence: the S2-6 concrete fence, exercised directly — an App-headed
\ member (`'It['T]`) grounded against a truly CONCRETE target (`i64`).
\ Expected: member_app_concrete_target_error ("the impl target `i64` is
\ concrete"), firing at parser.rs:4323 before ground_member_type.
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;
import: core::option * ;

trait: Iterator['It: * -> *] :
  next ( 'It['T] -- 'It['T] ) ;
;

impl: Iterator for i64
  : next | x | x ;
;

: main ( -- ) 0 drop ;
```

#### Command

```text
./target/debug/sooth build /tmp/p8-probes/p8-3a/p8-3a-concrete-fence.sth --manifest tests/fixtures/sooth.pkg
```

#### Output (both runs identical)

```text
error: trait member `next` of `Iterator` (line 15, col 5) applies the trait variable `'It`, but the impl target `i64` is concrete
  an application-headed member has no monomorphic representation (its applied arguments are member locals); implement the trait for a constructor target with a type variable instead
exit=1
```

#### Verdict

CONFIRMED — identical S2-6 fence for `i64`. `Range[i64]` and `i64` are the same
case to the compiler: both fold to `Concrete` targets.

---

### P8-3b — phantom parameter `type: Range['U] cur i64 limit i64 ;`

#### Hypothesis

The brief asks "does a phantom parameter even survive header grounding?".

#### Fixture (/tmp/p8-probes/p8-3b/p8-3b-phantom.sth)

```sth
\ P8-3b: phantom-parameter target — `type: Range['U] cur i64 limit i64 ;`
\ with `impl: Iterator for Range` (bare ctor, padded to Range['ctor0]).
\ The trait row says next yields Option['ctor0], but cur is i64.
\ Question: does the type error fire, or does the impl var bind to i64
\ (the "surprise: it grounds?" outcome)?
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;
import: core::option * ;

trait: Iterator['It: * -> *] :
  next ( 'It['T] -- Option['T] 'It['T] ) ;
;

type: Range['U] cur i64 limit i64 ;

impl: Iterator for Range
  : next
    | r |
    &r &cur @ | cur0 |
    &r &limit @ | lim |
    cur0 lim lt
    ~[ cur0 Some cur0 1 add lim Range ]
    ~[ None ]
    if ;
;

: main ( -- ) 0 3 Range | it | drop drop ;
```

#### Command

```text
./target/debug/sooth build /tmp/p8-probes/p8-3b/p8-3b-phantom.sth --manifest tests/fixtures/sooth.pkg
```

#### Output (both runs identical)

```text
error: type variable `'U` at line 15, col 13 is bound by `type: Range`'s header but appears in no field (a phantom parameter cannot be disambiguated at a call site)
exit=1
```

#### Verdict

REJECTED — at the TYPE DECLARATION, before any trait or impl machinery.
`phantom_ty_var_error` (src/parser.rs:2512) is raised at src/parser.rs:3049
during generic-typedef prepass. The rejection is deliberate and documented
(src/parser.rs:2503-2510): instantiation dispatch disambiguates two
instantiations by the generated constructor's INPUT types alone, so two
instantiations agreeing on every input and differing only in a phantom output
type are unrepresentable. Candidate (a) is dead by design; it never reaches
header grounding, let alone the Option['U]-vs-i64 body mismatch the probe was
written to capture.

---

### P8-3b-generic — generic target `impl: Iterator for Range` over `Range['T] cur 'T limit 'T`

#### Hypothesis

The brief's real worry: a generic target makes the body generic over 'T with
no arithmetic trait to +1 it. Reduced row (same reason as P8-3a-reduced).

#### Fixture (/tmp/p8-probes/p8-3b/p8-3b-generic-target.sth)

```sth
\ P8-3b-generic: the brief's "generic target" candidate — bare ctor target
\ `impl: Iterator for Range` over a fully-generic Range['T] (cur 'T limit
\ 'T). The App arg in `next ( 'It['T] -- 'It['T] )` aliases the member local
\ to the target's own variable, so the body is generic over 'T with NO
\ arithmetic trait available to +1 it. Expected: the body dies on `1 add`
\ (or `lt`) unifying 'T against i64.
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;
import: core::option * ;

trait: Iterator['It: * -> *] :
  next ( 'It['T] -- 'It['T] ) ;
;

type: Range['T] cur 'T limit 'T ;

impl: Iterator for Range
  : next
    | r |
    &r &cur @ | cur0 |
    &r &limit @ | lim |
    cur0 lim lt
    ~[ cur0 1 add lim Range ]
    ~[ r ]
    if ;
;

: main ( -- ) 0 drop ;
```

#### Command

```text
./target/debug/sooth build /tmp/p8-probes/p8-3b/p8-3b-generic-target.sth --manifest tests/fixtures/sooth.pkg
```

#### Output (both runs identical)

```text
error: cannot borrow the local `r` of type `Range['ctor0]` in `next` (member of trait `Iterator` for `Range['T0]`) (line 21, col 5)
  only an aggregate (a struct, enum, array, or owning cell) is borrowable; `Range['ctor0]` is not
exit=1
```

#### Verdict

REJECTED — later than every other candidate: the target PARSES and GROUNDS
(bare `Range` pads to `Range['ctor0]`, a Generic pattern; S2-6 does not fire;
`ground_member_poly`'s App arm dissolves `'It['T]` against it cleanly), and
the impl registers. The member BODY then dies at the first field read: the D5
borrow rule in the generic checker admits only `PolyType::Array` or
`PolyType::Concrete(Struct|Enum|Array|OwnedCell)` locals as borrowable
(src/check/poly.rs:6572-6604, the `is_aggregate` match; message
`poly_borrow_of_non_aggregate_local_error` at src/check/poly.rs:11251). A
`Range['ctor0]` local is `PolyType::Generic` — not borrowable, so `&r &cur @`
is unavailable and the arithmetic gap (`1 add` on 'T) is never even reached.
Note the generic-target candidate is not killed by the S6 construction wall
(either) — it dies before construction, at the read.

---

### P8-3c — non-generic type as target `impl: Iterator for Count`

#### Hypothesis

The task predicts a kind error (`Iterator['It: * -> *]` needs `* -> *`; Count
is `*`). No kind check exists on impl targets: `var_kind` is only parsed and
annotation-conflict-checked (src/parser.rs:1931, 2029, 2315, 2380, 3517-3554);
nothing compares it to the target's kind.

#### Fixture (/tmp/p8-probes/p8-3c/p8-3c-reduced.sth)

```sth
\ P8-3c-reduced: non-generic type as target, row reduced to
\ `next ( 'It['T] -- 'It['T] )`. Count is a * (monomorphic struct, zero
\ type params); Iterator['It: * -> *] needs a * -> *. `impl: Iterator for
\ Count` — what fires: a kind check, or ground_member_poly's arity gate?
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;
import: core::option * ;

trait: Iterator['It: * -> *] :
  next ( 'It['T] -- 'It['T] ) ;
;

type: Count cur i64 limit i64 ;

impl: Iterator for Count
  : next | c | c ;
;

: main ( -- ) 0 drop ;
```

(The verbatim-row variant /tmp/p8-probes/p8-3c/p8-3c-nongeneric-target.sth was
also run; it reproduces the round-level S2-3 gate error at line 11 — same text
as P8-3a's, exit=1, both runs identical.)

#### Command

```text
./target/debug/sooth build /tmp/p8-probes/p8-3c/p8-3c-reduced.sth --manifest tests/fixtures/sooth.pkg
```

#### Output (both runs identical)

```text
error: trait member `next` of `Iterator` (line 17, col 5) applies the trait variable `'It`, but the impl target `Count` is concrete
  an application-headed member has no monomorphic representation (its applied arguments are member locals); implement the trait for a constructor target with a type variable instead
exit=1
```

#### Verdict

REJECTED — same S2-6 concrete fence, NOT a kind error. Bare `Count` (zero type
params) folds to a Concrete target pattern, `is_concrete()` is true, and the
fence at src/parser.rs:4314-4323 fires before `ground_member_poly`'s arity
gate (src/ast.rs:2378, `member_app_arity_error` at src/ast.rs:2051) could have
said "applies the trait variable to 1 type arguments, but the impl target
constructor `Count` declares 0". The predicted kind error does not exist in
the compiler; the S2-6 fence is the de-facto rejection site for every
non-generic target under an App-headed member.

---

### P8-3d — plain-word fallback (no trait): a working count-up `next` today

#### Hypothesis

A plain word `nextc ( Count -- Option[i64] Count )` with a full body (match on
`cur lt limit`; `Some(cur)` + advanced `Count`, or `None` + unchanged Count)
builds and runs a drain loop to exhaustion — proving the count-up iterator
exists OUTSIDE the trait system.

#### Fixture (/tmp/p8-probes/p8-3d/p8-3d-plain-word.sth)

```sth
\ P8-3d: plain-word fallback — NO trait at all. `nextc` is an ordinary word
\ ( Count -- Option[i64] Count ): match arms on cur lt limit; Some(cur) and
\ Count(cur 1 add limit) in the then-arm, None in the else-arm. Then drain,
\ a consuming loop until None. This proves a working count-up next exists
\ today OUTSIDE the trait system.
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;
import: core::option * ;

type: Count cur i64 limit i64 ;

: nextc ( Count -- Option[i64] Count )
  | c |
  &c &cur @ | cur0 |
  &c &limit @ | lim |
  cur0 lim lt
  ~[ cur0 Some cur0 1 add lim Count ]
  ~[ None c ]
  if ;

: drain ( Count -- )
  nextc swap
  ~[ ( Some ) Some> . drain ]
  ~[ ( None ) drop drop ]
  Option? ;

: main ( -- ) 0 3 Count drain ;
```

Fixture self-fix (my error, not a compiler behavior): the first draft's else
arm was `~[ None ]`, which leaves `Option[i64]` where the then arm leaves
`Option[i64] Count`; the compiler correctly said "the quotations passed to
`if` leave different stack shapes: an earlier one leaves `Option[i64] Count`,
this one leaves `Option[i64]` in `nextc` (line 19)" (exit=1, both runs). The
else arm was corrected to `~[ None c ]` (return the unchanged iterator — the
exhausted-case shape P8-1 will rule on); only the fixed version's results are
reported.

#### Command (build)

```text
./target/debug/sooth build /tmp/p8-probes/p8-3d/p8-3d-plain-word.sth --manifest tests/fixtures/sooth.pkg
```

#### Output (build; both runs identical)

```text
exit=0
```

#### Command (run)

```text
./target/debug/sooth run /tmp/p8-probes/p8-3d/p8-3d-plain-word.sth --manifest tests/fixtures/sooth.pkg
```

#### Output (run; both runs identical)

```text
0
1
2
exit=0
```

#### Verdict

GROUNDS and RUNS — a fully working count-up iterator as an ordinary word:
builds clean, drains 0 1 2 to exhaustion, exit 0, deterministic. The linear
protocol (consume Count, return element + remainder) needs no trait machinery;
the exhausted case surfaces to the caller as `None` + the same Count, which
the caller drops (one canonical drop site — P8-1's question, answered in
passing for the plain-word shape).

---

### Verdict summary (P8-3 candidate shapes)

| Shape | Result | Rejecting site |
|---|---|---|
| (verbatim row) `next ( 'It['T] -- Option['T] 'It['T] )` | REJECTED before any impl | S2-3 shape gate: `PolyType::Generic` not admissible — src/parser.rs:399-404, loop src/parser.rs:3952-3974, msg src/parser.rs:441 |
| (a) phantom `Range['U] cur i64 limit i64` | REJECTED at the typedef | phantom check src/parser.rs:3049 → msg src/parser.rs:2512 (deliberate, doc src/parser.rs:2503-2510) |
| `impl: Iterator for Range[i64]` (reduced row) | REJECTED — S2-6 fence | fence call src/parser.rs:4314-4323 → fence src/ast.rs:2134-2143 → msg src/ast.rs:2095; `is_concrete` src/ast.rs:2480-2483 |
| generic `impl: Iterator for Range` (reduced row) | grounds to registration; member body REJECTED | D5 borrow gate src/check/poly.rs:6572-6604, msg src/check/poly.rs:11251 (arithmetic gap never reached) |
| `impl: Iterator for Count` (reduced row) | REJECTED — S2-6 fence (no kind check exists) | same S2-6 sites as Range[i64]; arity gate src/ast.rs:2378/msg src/ast.rs:2051 never reached |
| (d) plain word `nextc`, no trait | GROUNDS + RUNS (`0 1 2`) | — |

Recommendation (recommendation only; the ruling is the user's): no traitful
Range shape works today without compiler change — the row needs ctor
applications admitted by the S2-3 gate, the target needs either fully-applied
ctor targets kept off the Concrete fold or the S2-6 fence lifted for them, and
a generic-target body needs the D5 borrow gate (and then an arithmetic trait)
before it can advance `cur`. Given P8-3d shows the plain-word count-up
iterator working end-to-end today, the lowest-risk S8 exit is the plain-word
fallback (or List-only goldens with a recorded ruling), with the traitful
Range shaped by a user ruling on which of those three compiler deltas to make.

---

## P8 probe log — p8-consumers (P8-4a, P8-4b, P8-6)

Date: 2026-09-05
HEAD: ae6fdd7 (locked worktree, no commits made; only pre-existing untracked file in tree: docs/roadmap/P7b/slice8-brief.md)
Binary: target/debug/sooth (called directly, no cargo build in the repo)
Fixtures dir: /tmp/p8-probes/ (all commands run with --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg)
Every command below was run twice; outputs and exit codes were identical both times. The two .ssa files were re-emitted via the irprobe crate and compared byte-for-byte (`cmp`) — identical.

Executive summary: the P8-4 headline ("List's next grounds TODAY with zero compiler changes") is **refuted as spelled** — the briefed `Iterator` trait cannot be declared at all, blocked by two parser fences. But the *mechanism* (destructure-only next over core List returning Option + remainder, and a self-recursive generic consuming loop) **does ground with zero compiler changes** as plain poly words; only the trait-member spelling and the poly-calls-poly next call are fenced. P8-6 IR facts recorded; no fusion verdict.

---

### P8-4a — the briefed trait: `Iterator` member signature

Hypothesis (from the task): `trait: Iterator['It: * -> *] : next ( 'It['T] -- Option['T] 'It['T] ) ; ;` + `impl: Iterator for List` with a destructure-only next grounds TODAY with zero compiler changes.

Fixture: /tmp/p8-probes/p8-4a-next-list.sth

```sth
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;
import: core::option * ;
import: core::list * ;
trait: Iterator['It: * -> *] :
  next ( 'It['T] -- Option['T] 'It['T] ) ;
;
impl: Iterator for List
  : next
    ~[ ( Nil ) None swap ]
    ~[ ( Cons ) Cons> ^> swap Some swap ]
    List? ;
;
: mkempty ( -- List[i64] ) Nil ;
: main ( -- )
  3 mkempty ^ Cons
  2 swap ^ Cons
  1 swap ^ Cons
  next swap
  ~[ ( Some ) Some> . ] ~[ ( None ) drop ] Option?
  next swap
  ~[ ( Some ) Some> . ] ~[ ( None ) drop ] Option?
  drop ;
```

Command (verbatim):

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth build /tmp/p8-probes/p8-4a-next-list.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (verbatim, stderr; stdout empty):

```text
error: trait `Iterator`'s member at line 7, col 3 has an unsupported signature shape (only concrete, array, and reference types over the trait's type variable -- plus a trait-var-headed application or an App-free quotation -- are supported)
exit=1
```

Verdict: **refuted** — the member signature is fenced before any impl or body is seen. Attribution (confirmed by grep, not guessed):

- `next`'s output `Option['T]` parses as `RawTy::Generic` (named ctor) whose arg is a variable; `raw_to_poly_type` only folds a named application to `Concrete` when *every* arg is concrete (src/parser.rs:5657-5679), so it stays `PolyType::Generic`.
- `member_shape_is_supported` rejects any `PolyType::Generic` slot outright (src/parser.rs:399-401, `PolyType::Generic { .. } => false`); the only App a member may carry is trait-var-headed (`App { head, .. } => *head == 0`, src/parser.rs:386-387, P7b.S2 S2-3).
- Enforcement is in `parse_trait_member_effect`'s sig walk (src/parser.rs:3947-3975); the message is `unsupported_trait_member_shape_error` (src/parser.rs:441-446).
So a member output of `Option['T]` (or `Step['T 'It['T]]`) is unrepresentable in a member signature today; a fence lift in these exact functions would be needed (the brief's P8-1/S7-coordination sites: `member_shape_is_supported` / the member parse path). Note the `'It['T]` input half alone IS the supported shape (existing Functor goldens).

### P8-4a2 — alternate spelling: two-var trait header

Hypothesis: moving the element type into the header (`'O: * -> *` for the step type) might dodge the member fence.

Fixture: /tmp/p8-probes/p8-4a2b-two-var-header.sth (member spelled `next ( 'It['T] -- 'O['T] 'It['T] )`)

Command (verbatim):

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth build /tmp/p8-probes/p8-4a2b-two-var-header.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (verbatim):

```text
error: trait `Iterator` names more than one type variable at line 6, col 29 (only single-type-variable traits are supported)
exit=1
```

Verdict: **refuted** — multi-var trait headers are fenced by design (P7.S3e R16, `multi_variable_trait_error`, src/parser.rs:485-491). Even without it, `'O['T]` would be an App with head != 0, also fenced (src/parser.rs:386-387).

### P8-4a3 — mechanism, zero compiler changes: `list-next` as a plain poly word

Hypothesis: the same destructure-only next body grounds as a plain poly word `( List['T] -- Option['T] List['T] )` (plain word sigs have no member-shape fence) and the two-next protocol runs.

Fixture: /tmp/p8-probes/p8-4a3-listnext-plain.sth

```sth
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;
import: core::option * ;
import: core::list * ;
: list-next['T] ( List['T] -- Option['T] List['T] )
  ~[ ( Nil ) drop None Nil ]
  ~[ ( Cons ) Cons> ^> swap Some swap ]
  List? ;
: mkempty ( -- List[i64] ) Nil ;
: main ( -- )
  3 mkempty ^ Cons
  2 swap ^ Cons
  1 swap ^ Cons
  list-next swap
  ~[ ( Some ) Some> . ] ~[ ( None ) drop ] Option?
  list-next swap
  ~[ ( Some ) Some> . ] ~[ ( None ) drop ] Option?
  drop ;
```

Commands (verbatim):

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth build /tmp/p8-probes/p8-4a3-listnext-plain.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth run /tmp/p8-probes/p8-4a3-listnext-plain.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (verbatim; build prints nothing, exit=0; run stdout):

```text
1
2
exit=0
```

Verdict: **confirmed at the mechanism level** — the destructure-only next over the real core List grounds and runs with zero compiler changes: two `list-next` calls print 1 then 2, remainder List[3] dropped. Notes extracted the hard way:

- The Nil arm must produce the two-value output `None` + empty remainder; it constructs a fresh nullary `Nil` (`drop None Nil`). This is enum-ctor construction over a var in a poly body and grounds — the S6 construction wall (`poly_bind_construction_arg`, src/check/poly.rs:6072 per slice6's recorded wall) is about Cons's self-reference FIELD, and Nil has no fields.
- Next's Cons arm is `Cons> ^> swap Some swap` (destructure Cons, unwrap the `^List` cell with `^>`, wrap the element in `Some`, order Option-below/remainder-on-top).

### P8-4a3b — Nil arm that keeps the matched Nil

Hypothesis: the empty List itself could serve as the exhausted remainder (`~[ ( Nil ) None swap ]`, no construction).

Fixture: /tmp/p8-probes/p8-4a3b-nil-keep.sth (same as p8-4a3 but Nil arm `None swap`)

Command (verbatim):

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth build /tmp/p8-probes/p8-4a3b-nil-keep.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (verbatim):

```text
error: an arm of `List?` leaves `List.Nil` on the stack in `list-next` (line 7)
  a variant-typed value is reachable only inside the arm that bound it; consume it there, or leave its fields instead
exit=1
```

Verdict: **refuted** — a matched variant cannot escape the dispatch that bound it (variant-escape check, poly arm at src/check/poly.rs:11544, mono twin src/check.rs:2835). The exhausted arm must construct its remainder; hence P8-4a3's `drop None Nil`.

### P8-4a4 — the task's literally-prescribed Nil arm (`drop None`)

Hypothesis (as briefed): "Nil arm drops the Nil and pushes None" — one output value.

Fixture: /tmp/p8-probes/p8-4a4-nil-drop-none-plain.sth (Nil arm `drop None`)

Command (verbatim):

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth build /tmp/p8-probes/p8-4a4-nil-drop-none-plain.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (verbatim):

```text
error: the quotations passed to `List?` leave different stack shapes: an earlier one leaves `Option['T]`, this one leaves `Option['T] List['T]` in `list-next` (line 8)
exit=1
```

Verdict: **refuted** — with next's two-output signature, a `drop None` Nil arm fails arm-shape parity (src/check.rs:2983). The exhausted case as prescribed is uncheckable today: either the remainder is produced (P8-4a3 shape) or the signature shrinks to one output (the P8-1 Step/None shape question). Evidence for the round's P8-1 ruling.

### P8-4b — generic consumer through a shared bound, looping on next

Hypothesis (from the task): `: consume['It: Iterator 'T] ( 'It['T] [ 'T -- ] -- )` looping on `next`, run over the List impl. Recursion in a generic word is available (P7.S3g delivers non-inline poly self-calls; docs/roadmap/P7/slice3g-spec.md; checker arm per slice3g-follow-spec.md:37).

Fixture: /tmp/p8-probes/p8-4b-consume-plain.sth — the closest reachable spelling (consume is poly over 'T, calling the plain poly `list-next`; the Iterator trait itself cannot exist per P8-4a):

```sth
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;
import: core::option * ;
import: core::list * ;
: list-next['T] ( List['T] -- Option['T] List['T] )
  ~[ ( Nil ) drop None Nil ]
  ~[ ( Cons ) Cons> ^> swap Some swap ]
  List? ;
: p ( i64 -- ) . ;
: consume['T] ( List['T] [ 'T -- ] -- )
  | q |
  list-next swap
  ~[ ( Some ) Some> q call q consume ]
  ~[ ( None ) drop drop ]
  Option? ;
: mkempty ( -- List[i64] ) Nil ;
: main ( -- )
  3 mkempty ^ Cons
  2 swap ^ Cons
  1 swap ^ Cons
  [ p ] consume ;
```

Command (verbatim):

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth build /tmp/p8-probes/p8-4b-consume-plain.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (verbatim):

```text
error: `consume` cannot call the polymorphic word `list-next` (line 13, col 3)
  returning the compound type `Option['T]` from a polymorphic word is not yet supported from a polymorphic body
  call `list-next` from a monomorphic word instead
exit=1
```

Verdict: **refuted / fenced, recorded as the round's second finding.** A poly body cannot call a *different* poly word whose signature returns a compound generic type: `poly_cross_call_unsupported_error` (src/check/poly.rs:4372-4380, P7.S3k's symbolic cross-call gate; only the SELF-call was lifted in P7.S3g). The task's prescribed fallback (an unrolled twice-style body) does NOT rescue this: it still calls `list-next` from a poly body and hits the same fence. So the shared-bound dispatch of P8-4 is **not measurable today** at any spelling: the trait is undeclarable (P8-4a) and even a bound-free poly consumer cannot call a poly `next`. A mono consumer CAN call `list-next` (the error's own remedy); `main` does exactly that in P8-4a3 and the chain below.

### P8-4b2 — the loop shape itself, zero compiler changes: self-recursive generic consume matching Cons/Nil

Hypothesis: a generic consuming loop over core List grounds if the body never calls another poly word — direct Cons/Nil dispatch with an S3g self-call in tail position.

Fixture: /tmp/p8-probes/p8-4b2-consume-loop.sth

```sth
import: intrinsics * ;
import: hosted::show | . | ;
import: core::prelude * ;
import: core::option * ;
import: core::list * ;
: list-next['T] ( List['T] -- Option['T] List['T] )
  ~[ ( Nil ) drop None Nil ]
  ~[ ( Cons ) Cons> ^> swap Some swap ]
  List? ;
: p ( i64 -- ) . ;
: consume['T] ( List['T] [ 'T -- ] -- )
  | q |
  ~[ ( Nil ) drop ]
  ~[ ( Cons ) Cons> ^> swap q call q consume ]
  List? ;
: mkempty ( -- List[i64] ) Nil ;
: main ( -- )
  3 mkempty ^ Cons
  2 swap ^ Cons
  1 swap ^ Cons
  [ p ] consume ;
```

Commands (verbatim):

```text
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth build /tmp/p8-probes/p8-4b2-consume-loop.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
cd /root/code/ordfruma/sooth-worktrees/p7b-s8 && ./target/debug/sooth run /tmp/p8-probes/p8-4b2-consume-loop.sth --manifest /root/code/ordfruma/sooth-worktrees/p7b-s8/tests/fixtures/sooth.pkg
```

Output (verbatim; build silent, exit=0; run stdout):

```text
1
2
3
exit=0
```

Verdict: **confirmed** — a generic ('T-polymorphic), self-recursive consuming word over the real core List grounds and runs, printing all three elements. The call-operand lesson en route (recorded for fixture authors): `call` consumes its inputs from directly beneath the quotation, so the arm is `Cons> ^> swap q call q consume` (element swapped under q), not `Cons> ^> q call ...` (that rejects: `` `call` expected `'T`, found `List['T]` ``). This is the closest today's compiler comes to the P8-4 loop; the Iterator-bound version waits on the two fences.

### P8-6 — fusion evidence (recorded; no verdict)

How captured: no CLI/env path keeps the .ssa — `sooth build` accepts no other flag (`parse_entry_and_manifest` rejects any `--flag` but `--manifest`, src/main.rs:38) and main.rs reads no env vars. The library path is public: `sooth::driver::emit_ssa_with_manifest(path, manifest)` → `check` → `ir::lower` → `backend::qbe::emit`, documented as "the exact bytes `build` hands to `qbe`" (src/driver.rs:886-907). A disposable crate at /tmp/p8-probes/irprobe (path dependency on the worktree, writes only under /tmp) emitted both fixtures through it; re-running reproduced byte-identical .ssa.

Fixtures: /tmp/p8-probes/p8-4b2-consume-loop.sth (loop alone) and /tmp/p8-probes/p8-6-chain-plain.sth (consume over L1 = 1,2,3, then a fresh L2 = 1,2,3 and one `list-next`, print, drop). Chain run output (verbatim):

```text
1
2
3
1
exit=0
```

Command (verbatim):

```text
cd /tmp/p8-probes/irprobe && cargo run --quiet
```

Output (verbatim):

```text
wrote /tmp/p8-probes/p8-4b2-consume-loop.ssa (27370 bytes)
wrote /tmp/p8-probes/p8-6-chain-plain.ssa (29531 bytes)
```

Facts from the .ssa (no interpretation beyond frames/calls):

1. **The consuming loop lowers to ONE frame.** `$sooth_mono_consume__m0__t0_i64(:List[i64], :Q0)` (p8-4b2 .ssa line 1278; chain .ssa line 1334) is the single monomorphized frame for the whole generic consumer. Its self-call is a loop back-edge, not a recursive call: the Cons arm ends `blit %v3, %v2, 24` / `jmp @blk1` back to the tag test. This is the poly self-tail-call → loop transform (named test `poly_self_tail_call_lowers_to_loop_back_edge`, src/ir/driver.rs:1093) applying to the match-arm tail self-call. No per-element frame comes from the iteration machinery itself.
2. **The element function is a real indirect call per element.** Inside the loop: `call %v19(l %v13, l %v21)` through the quotation's code pointer (the Q0 closure header stores `$main__quot0`, which tail-calls `$p__m0`, which calls the show `.`). So one frame sits between per element — the user's quotation — plus a real `call $sooth_free` per element where `^>` unwraps the Cons cell, and one `call $sooth_enum_drop_2` at the Nil arm (the exhausted-list destructor).
3. **`next`/`list-next` is a real call frame, not spliced.** The chain emits `$sooth_mono_list_next__m0__t0_i64(:List[i64]) -> :__ret_3` (chain .ssa line 1379), called once from `$sooth_main` (`call $sooth_mono_list_next__m0__t0_i64`, .ssa line 118). Its body is a tag branch returning a 40-byte pair (Option + remainder); the Nil arm constructs `None` + empty `Nil` inline (stores, no helper call beyond the enum drop of the consumed list).
4. **The Option dispatch after the next call inlines into the caller.** In `sooth_main`, the Some/None match after `list-next` is a local `jnz` on the tag with the Some arm's `.` call inline (@blk1/@blk2/@blk3, .ssa lines 119-140) — no separate match frame.
5. Chain total: two consuming operations lower to two dedicated frames (`consume`, `list-next`) plus the shared show/print frames; `sooth_main` calls each once. For the loop alone: one frame (`consume`).

No fusion verdict is drawn — per the brief the question stays open.

---

### Findings for the round (compiler-change ledger; nothing was changed)

- F1 (blocks P8-4a as briefed): trait-member signatures cannot name a constructor over the member's type variable (`Option['T]`, `Step['T 'It['T]]`): `member_shape_is_supported`'s `Generic { .. } => false` arm, src/parser.rs:399-401, enforced in `parse_trait_member_effect` src/parser.rs:3947-3975; the fold that would need an arm is `raw_to_poly_type`'s all-concrete collapse, src/parser.rs:5657-5679. Any S8 lift here touches S7's fence neighborhood — sequencing note in slice8-brief.md applies.
- F2 (blocks any two-var header alternative): `multi_variable_trait_error`, src/parser.rs:485-491 (P7.S3e R16, single-type-variable traits only).
- F3 (blocks the poly consumer calling a poly next, bound or not): `poly_cross_call_unsupported_error`'s compound-type case, src/check/poly.rs:4372-4380 (P7.S3k gate; only poly SELF-calls are delivered, P7.S3g).
- F4 (exhausted-case shape): with next's two-output signature, the Nil arm must produce the remainder; a bare `drop None` fails arm parity (src/check.rs:2983) and keeping the matched Nil fails the variant-escape rule (src/check/poly.rs:11544). The grounds-today shape is `drop None Nil` (construct the empty remainder).
- No repo files were created, edited, or deleted; no commits; `git status --porcelain` shows only the pre-existing untracked docs/roadmap/P7b/slice8-brief.md.
