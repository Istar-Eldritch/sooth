# Slice 7b Q1 Probe — capturing quotation materialization

Working copy: `/tmp/q1probe` (throwaway, `.git` removed). Real repo untouched, read-only.
Compiler invocation: `cargo run -q -- run FILE.sth` (no `-o`; `run` = compile+exec). A compile
error prints to stderr (exit 1); a compiler *panic* is exit 101; a clean run is exit 0.

**Headline results (measurement, not argument):**

1. With identity intact, 6f + linear move-tracking already enforce everything 7b's *checking*
   needs for the non-erased case (P1). It all keys off `Some(QuotRef::Known(id))`.
2. Lift the guard and the checker **accepts a capturing quotation at all four boundaries**;
   lowering then panics because there is *zero* capture plumbing (P2).
3. **The checker does not distinguish the unsound escape from the sound one** once the guard is
   lifted (P3, make-a vs make-b: both accepted). A *linear* referent is caught, but by the
   pre-existing linear-leak / move rules, not by anything capture-aware.
4. For the two motivating programs the captured names are **already dead at the call site** under
   the existing 6f scan, because erasure sets `quot = None` and drops them from the walk (P4).

Consequence for the decision: direction (b) "rely on 6f's existing liveness" cannot be
implemented as stated. Evaluated *at the boundary*, 6f liveness is TRUE for the unsound escape
(make-a) → it would accept it. Evaluated *at the call site*, it is FALSE for the motivating
programs → it would reject them. There is no single existing query that says yes to sound and no
to unsound. Both directions need new machinery; (b)'s "no new machinery" premise is false. See
the closing section for the one existing analysis (`owned_root`/`outer_locals`) that *does*
separate them and what wiring it still lacks.

Boundaries and the guard (confirmed by reading `src/check.rs`):

- word output (return): materialize loop ~line 3675 → `materialize_quotation_at_boundary`.
- `!`/`+!` store through `&!Quotation` (array elem / struct field via ref): ~line 6912 → same fn.
- quotation parameter / struct constructor / setter: ~line 7038 → same fn.
- differing-arm `if` join: inline `body_captures_enclosing` guard ~line 7204 (separate copy).

`materialize_quotation_at_boundary` (6221) ran `body_captures_enclosing` (6238) then
`check_literal_against_declared_effect` (6067). That effect-check holds the D3 R12 guards: a
*linear* enclosing local consumed by the body (6101), and a borrow of an enclosing place *left on
the literal's exit row* (6108). A borrow taken and **consumed inside** the body leaves nothing on
the exit row, so D3 does not see it — `body_captures_enclosing` was the only thing catching that
shape at a boundary.

The P2 edit (smallest possible): short-circuit both guard copies.

- `materialize_quotation_at_boundary`: deleted the `if body_captures_enclosing {...} return Err`.
- if-join (~7204): deleted the inline `for id in [a,b] { if body_captures_enclosing {...} }`.
Nothing in lowering was touched; `env` stays hardcoded null (`Const(env,0)`, `ir.rs:4188`).

---

## P1 — what 6f already catches, identity intact (UNMODIFIED compiler)

### Measurements

| # | program | what it does | result |
|---|---|---|---|
| P1a | p1a.sth | `10 \| x \| [ x + ] \| q \| 5 q call .` | compiles, **prints 15** |
| P1e | p1e_refcap.sth | capture ref *local* `r=&arr` in `q`; `&!arr bump`; `q call` | **rejected** (`&!arr` conflicts with live borrow of `arr`) |
| P1e2 | p1e2_baseline.sth | same, no quotation: `r @ .` then `&!arr bump` | compiles, prints 0 |
| P1i | p1i_lin.sth | capture ref local `r=&c` to **linear** `^i64`; consume `c`; `q call` | **rejected** (cannot consume borrowed `c`; shared borrow live) |
| P1j | p1j_lin_base.sth | same, no quotation | compiles, prints 42 |
| P1k | p1k_inbody.sth | borrow linear `c` **inside** q body; consume `c`; `q call` | **rejected** (use-after-move: c read by q, moved before call) |
| copytest/2 | consume/drop `[i64 4]` twice | **both compile** → `[i64 N]` is **Copy** |
| P1b/c/d/f/h | move/drop/mutate a `[i64 4]` (Copy) while a capturing q is live | **all compile & run** |

Exact text — P1e: `error:`&!arr` conflicts with a live borrow of `arr`... the shared borrow
taken at line 4 is still live`. P1i: `cannot consume the borrowed local`c` of type `^i64`...
the shared borrow taken at line 3 is still live`. P1k: `use after move ...`c` ... was moved at
line 4; `^i64`is linear`.

### Interpretation

- A **Known** capturing quotation is spliced at the call site; its captured borrows do not exist
  until the call, so there is no def-to-call aliasing window to police (P1a). Only the referent's
  validity *at the call* must hold.
- Captured **reference local**: 6f keeps its borrow live for as long as `q` survives, so a
  conflicting `&!` (P1e) or a consume of a linear owner (P1i) between def and call is rejected
  where the same code without `q` is accepted (P1e2/P1j). 6f working exactly as designed.
- Borrow taken **inside** the body of a **linear** referent: caught by ordinary move-tracking
  (P1k).
- P1b/c/d/f/h "accepted" are **not holes**: `[i64 N]` is Copy (copytest), cannot dangle inside
  its own frame, needs no linear tracking; a later-spliced read observing a mutation (P1c prints
  99) is the intended late-read, not a bug.

**P1 bottom line:** with identity intact the checker already refuses to let a capturing quotation
be called after its captured referent is invalidated. Enforcement keys entirely off
`Some(QuotRef::Known(id))`.

---

## P2 — lift the guard, blast radius across the four boundaries (PATCHED compiler)

### Measurements

| boundary | program | checker | lowering / run |
|---|---|---|---|
| word output (return) | p2/b1_return.sth | **accepts** | **panic** `ir.rs:3488 "checked user word exists"` (exit 101) |
| word/ctor parameter | p2/b3_ctor.sth | **accepts** | **panic** `ir.rs:3488` |
| if-join (differing arms) | p2/b4_join.sth | **accepts** | **panic** `ir.rs:3488` |
| array element (`!` store) | p2/b2_array.sth | **accepts** | **panic** `ir.rs:3488` |

(First b2_array attempt hit a *different* pre-existing guard, `a quotation cannot be stored
(escaping quotations are slice 7)`, because `fill` is not a materialization boundary and needs an
already-erased seed; fixed by seeding with a word returning an erased quotation, then it too
reached lowering and panicked.)

### Interpretation

At **every** boundary the check phase accepts a capturing literal once the guard is gone. Runtime
correctness is **unmeasurable**: lowering panics *before* emitting a binary, so the anticipated
"wrong value vs crash" never arises. The panic is earlier and harder than a null-env runtime bug:
the materialized body is lowered as a standalone `IrFunc` whose signature is only the quotation's
declared inputs (`lower_materialized`, `ir.rs:2370`), so a captured name has no local and falls
through to user-word resolution (`env.get(name).expect("checked user word exists")`, `ir.rs:3488`)
or to the borrow-operand assertion (`ir.rs:3577`, seen in P3). `env` is written null and **never
read** (`materialize_quot_value`, `ir.rs:4171-4189`). There is no capture plumbing at all; the
"env hardcoded to null" is not even reached as a runtime concern.

---

## P3 — does anything catch the unsound program? (PATCHED compiler)

### Measurements

| # | program | captures | checker | lowering |
|---|---|---|---|---|
| make-a (UNSOUND) | p3/make_a_unsound.sth | `&arr` of its own frame local (Copy `[i64 4]`) | **accepts** | panic `ir.rs:3577 "checked: a borrow's operand is a local"` |
| make-b (SOUND) | p3/make_b_sound.sth | `r`, a parameter `&[i64 4]` (caller frame) | **accepts** | panic `ir.rs:3488` |
| make-a2 (unsound, linear) | p3/make_a_linear.sth | `&c` of own frame `^i64` | **rejected**: `linear value`c`is never consumed` |
| (baseline) | p3/linear_leak_base.sth | linear `c`, no quotation at all | **rejected**: identical `never consumed` error |
| make-a4 (linear escape attempt) | p3/make_a4_lin_escape.sth | capture `&c`, then consume `c` to satisfy linearity, return q | **rejected**: `use after move ... c moved before q's read` |

### Interpretation (this is the single most valuable measurement)

- **The checker does not distinguish make-a from make-b.** Both pass the check phase; both only
  fall over in lowering. The unsound one (a closure over a Copy stack local of a frame that has
  returned → dangling pointer) is accepted with no diagnostic.
- The linear variants **are** caught, but by rules that are *not capture-aware*: make-a2 fails the
  linear-leak rule (`c` never consumed) — the *identical* error fires with no quotation present
  (linear_leak_base). Trying to satisfy linearity (make-a4) then trips move-tracking, because the
  Known closure still keeps `c` alive (6f), so consuming `c` before returning `q` is use-after-move.
  So a *linear* frame-local simply cannot escape: consume it and it's use-after-move; don't and
  it's a leak. That safety net is a side effect of linearity, and it evaporates for a **Copy**
  referent, which has neither obligation. make-a is exactly that gap.
- Direction (b) has **no existing way** to tell make-a's unsound escape from make-b's sound one.
  6f's liveness is *intra-frame*: at make-a's return term, `arr` is still live (it is read on the
  very last line), so "6f keeps the captured name alive" is TRUE for the unsound program.

---

## P4 — are the motivating programs reachable under (b)? (PATCHED compiler)

### Measurements

| # | program | checker | lowering |
|---|---|---|---|
| P4(i) dispatch table (array of capturing closures) | p4/dispatch.sth | **accepts** | panic `ir.rs:3488` |
| P4(ii) closure in struct field, mutate captured arr between store and call | p4/lateread.sth | **accepts** | panic `ir.rs:3488` |
| P4(ii)-contrast: SAME mutation, closure kept **Known** (not erased) | p4/lateread_known.sth | **rejected**: `&!arr` conflicts with live borrow of `arr` | — |

### Interpretation

- The erasure gap is airtight. The identical mutation `&!arr ... 99 !` is **rejected** when the
  closure stays Known (`[ r @ ] | q |`, lateread_known) and **accepted** the moment it is erased
  into a struct field (`[ r @ ] Box`, lateread). Erasure sets the slot/field `quot = None`, so
  `capture_alive_names` (which only unions captures for slots/bindings whose `quot` is
  `Some(QuotRef::Known(id))`, `check.rs:1062-1085`) can no longer see that `bx` reads `r`. `r`'s
  last use in `Liveness::scan` is the store line; `Liveness::dead(r, call_site)` is therefore true
  (`last < at`). The name is **dead at the call site**.
- Same for P4(i): `a`/`b` are last used at their store lines; the array slots are erased
  (`quot = None`); at `h call` both are dead.
- So under direction (b) as stated ("materialize only where 6f keeps every captured name alive"),
  evaluated at the call site, **both motivating programs would be rejected** — their captured
  names have already died. 7b's headline capability is unreachable under (b) without changing what
  keeps a name alive.

---

## Verdict: does the evidence favour (a) or (b)?

The evidence **falsifies the premise of (b) as written** and thereby favours (a) — but names a
cheaper middle that neither option spelled out.

- **(b) "rely on existing 6f liveness" cannot separate sound from unsound.** 6f liveness is
  intra-frame and keys off `Known`. At the *boundary* it is true for the unsound escape (P3
  make-a), so it would admit it. At the *call site* it is false for the sound motivating programs
  (P4), so it would reject them. Same query, opposite failures. No single evaluation point works.
- **(a) "extend the value-level marker so a capture set survives erasure" is the only thing that
  restores the intra-frame protections** P1 measured — but it is strictly more than P2 touched:
  it needs lowering to actually build and read an env (today `env` is null and never read, and the
  materialized body panics on any captured name — P2). Direction (a) is unavoidably a lowering
  project, not just a checker one.

**What neither option anticipated (the cheap, sound core):** the one existing analysis that *does*
separate make-a from make-b is `Deriv.owned_root` + the `outer_locals` test D3 already uses for
its exit-row guard (`check.rs:6108`). make-a's `&arr` roots at an *owned local of the current
frame*; make-b's `r` roots at a *parameter* (caller frame). That distinction is computable at the
boundary from machinery that exists — but today it only inspects borrows *left on the exit row*,
and a closure escapes via the *env* (interior reads consumed inside the body), which that guard
never examines. So a genuinely minimal 7b could be: **at a materialization boundary, reject if the
literal's capture set includes any name whose reference roots at an owned local of the current
frame; accept parameter/global roots.** That is sound (it is exactly the make-a/make-b line),
admits make-b and rules out make-a — but it does *not* admit the P4 motivating programs, whose
captures root at owned locals of the *same* frame that later calls them (dispatch table and the
struct-field late-read are both single-frame). Those need the env to hold a live reference and the
checker to keep that reference's exclusivity honest past erasure — i.e. (a)'s surviving capture
set feeding `capture_alive_names`.

**Cost of being wrong:**

- Ship (b) as literally specified → either an unsound compiler (admits make-a; a dangling stack
  pointer with no diagnostic, P3) or a 7b that rejects its own motivating programs (P4). Both are
  observed here, not argued.
- Ship (a) → real lowering cost (env build + env-read in the materialized body; today entirely
  absent, P2) and the checker cost of carrying a capture set through erasure so
  `capture_alive_names` still fires. Larger, but it is the only path that both admits P4 and stays
  sound.

**Recommendation from the measurements:** (a) is required for the P4 capability; the `owned_root`
frame-escape check is a sound, small *floor* that could land first (admits make-b, rejects make-a)
even if the full P4 capability waits on (a)'s env plumbing.

## Things I could not measure / caveats

- **No runtime behaviour** for any materialized capturing quotation: lowering panics before a
  binary exists (no capture plumbing). "Wrong value vs crash at runtime" is therefore not
  observable; the measurement lives at the checker-accept / lowering-panic line.
- `[i64 N]` being **Copy** made the P3 unsoundness one of *frame lifetime* (a dangling pointer
  into a returned frame), not of linearity. I could not exhibit a *linear* frame-local escape at
  all — linearity's own rules block every attempt (P3 make-a2/make-a4). So the Copy case is the
  live danger; a hypothetical non-Copy, non-linear aggregate (none exists today) would be the
  sharpest test and I could not construct it.
- I did not run the full `cargo test` suite green: the guard-lift *intentionally* breaks the four
  capturing-rejection goldens in `tests/phase4_quotations.rs` (store, array element, join, nested)
  — verified they fail with the patch and pass without it, which is the point (it confirms the
  guard was load-bearing). Notably the 733 `--lib` unit tests all still pass with the guard lifted:
  the 7b rejection has **no unit-level coverage**, only integration goldens.
