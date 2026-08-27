# P7.S8 — nested inline-combinator splice-uid collision (spec)

Input: [`slice8-brief.md`](./slice8-brief.md), taken as authoritative discovery (written
and twice corrected by gdb-traced probe subagents). No generics are involved in the
defect; nothing here re-derives the diagnosis.

**Nature of the change: one lowering-time uid rule, plus the `lib/cmp.sth` flip that makes
it reachable.** No parser, AST, checker or backend change. The checker's uid numbering
(`INLINE_UID_STRIDE`, `src/check.rs:29`; the `word_idx` seed at `src/check.rs:1007`) is the
*correct* namespace and is untouched — lowering is what disagrees with it.

The slice ships four things:

1. `lower_resolved_word_call`'s combinator branch (`src/ir/func_builder/calls.rs:189`)
   lowers a spliced trait-member body under **that member's own** check-time uid
   namespace, not the enclosing caller's — and resets the *nested-splice minting counter*
   to that same namespace for the duration, not just the lookup stack (R1);
2. `self.trait_calls`'s span-keyed lookup (`src/ir/func_builder/calls.rs:243`) is gated off
   while a **member re-splice** (1)'s bracket opened is active — it must not answer for a
   second, differently grounded splice of the same source span reached from inside that
   re-seeded body (R1b);
3. `lib/cmp.sth`'s six surface comparisons (`eq`/`lt`/`gt`/`lte`/`gte`/`ne`) become
   `inline`, and the stale "they stay non-inline" rationale comment goes — the brief calls
   this a one-line follow-up, but it is the only thing that makes (1)/(2) reachable, so it
   lands in the same review, not as an untracked step;
4. the collateral the flip exposes in the existing suite (measured, section 4) — including
   two test *helpers* that are unsound once a comparison is spliced.

---

## 1. Verified anchors (HEAD `fcac96a`; two commits past `9c13878`, both docs-only —

`calls.rs`/`mod.rs`/`driver.rs` are byte-identical, so the line numbers below still hold
except where individually corrected against the earlier `9c13878` draft)

| What | Where (verified) |
| --- | --- |
| `lower_resolved_word_call` | `src/ir/func_builder/calls.rs:187` |
| the defect: the combinator branch reusing the caller's uid | `src/ir/func_builder/calls.rs:188-195`, uid read at `189` |
| the `.expect("checked user word exists")` panic site | `src/ir/func_builder/calls.rs:733` (a second one at `724`) |
| `splice_trait_calls` `(uid, span)` lookup that misses | `src/ir/func_builder/calls.rs:255` |
| `splice_records` `(uid, span)` lookup, same keying | `src/ir/func_builder/calls.rs:326-327` |
| the *ordinary* combinator splice: mint `inline_uid`, push/pop `splice_uid_stack` | `src/ir/func_builder/calls.rs:627-643` |
| `FuncBuilder::inline_uid` / `splice_uid_stack` / the two `(uid, span)` maps | `src/ir/func_builder/mod.rs:367`, `382`, `369-378`; init at `447-450` |
| `inline_uid_seed` parameter and its single assignment | `src/ir/func_builder/mod.rs:916`, `930` |
| `INLINE_UID_STRIDE = 1 << 20`, and its doc comment | `src/check.rs:29`, `23-28` |
| the checker's per-word seed, `word_idx as u32 * INLINE_UID_STRIDE` | `src/check.rs:1007` |
| the *correct* mirror of that seed on the lowering side | `src/ir/driver.rs:235` (comment `229-234`) |
| composed-instantiation seed hardcoded `0` | `src/ir/driver.rs:371` (rationale comment `366-370`) |
| REPL `lower_instantiation` seed hardcoded `0` | `src/ir/driver.rs:968` |
| REPL check-def seed hardcoded `0` (checker side matches, so consistent) | `src/ir/driver.rs:903` |
| `CombinatorEntry` (`terms`/`inputs`/`ambiguous` — carries **no** word index) | `src/check/combinators.rs:60-72` |
| `combinator_index`, keyed by `word.name` | `src/check/combinators.rs:80-102` |
| the module-order build site lowering uses | `src/ir/driver.rs:62` |
| other `combinator_index` callers (REPL, tail walk, drop-graph tests) | `src/repl.rs:176`, `src/check/combinators.rs:38`, `49`, `src/check/drop_graph.rs:1103`… |
| `resolve_splice_member_call` (check-time resolution of the bare member call) | `src/check/poly.rs:930` |
| `cross_calls_of`'s `is_combinator` branch (`self.compose(...)`) | `src/check/poly.rs:5655-5697`, `is_combinator` test at `5672` |
| `FuncBuilder::trait_calls` (`&HashMap<Span, String>`, per-composed-instantiation, no uid scoping) | `src/ir/func_builder/mod.rs:209` |
| the unconditional, unscoped `trait_calls` lookup | `src/ir/func_builder/calls.rs:243-247` |
| the doc comment `trait_calls` lookup carries today, correct about *what* it resolves, silent on nesting | `src/ir/func_builder/calls.rs:238-243` |
| composed cross-call routing: `poly_calls` checked *before* the combinator-splice branch, so an inline callee with a `Bound::User` calls a real `IrFunc` | `src/ir/func_builder/calls.rs:316` vs `619`; the correct doc comment stating this | `src/check/poly.rs:5657-5663` |
| the six comparisons, and the stale non-inline rationale | `lib/cmp.sth:139`, `146`, `161`, `168`, `175`, `182`; comment `lib/cmp.sth:11-16` |

### The defect, re-measured

Confirmed at HEAD with the six comparisons flipped to `inline` and a **fully concrete**
fixture (no generics anywhere): a `type: Point`, an `impl: Ord for Point` whose `cmp` body
calls `lt`/`gt`, and `: main ( -- ) 3 Point 7 Point lt ~[ 1 ] ~[ 0 ] if . ;` panics at
`src/ir/func_builder/calls.rs:733:22: checked user word exists`. `mymax` is not required,
as the brief says. A self-tail-recursive variant (`countdown` comparing two `Point`s in its
loop condition) panics at the identical site, also as the brief says: the same bug reached
during straight-line lowering, not a back-edge hazard. **R1 alone fixes this concrete case**
(verified directly: build+run, correct output), but is not sufficient for the generic path.

The generic exit criterion, `an_ord_bounded_generic_word_instantiates_over_a_user_struct`
(`mymax` instantiated at both `Point` and `i64` in one program), needs R1b. **R1 and R1b fix
two separate failure modes, not a sequential cause-and-effect** — verified directly: R1b's
`member_splice_depth` gate alone (with none of R1's uid/`inline_uid` changes) already makes
the generic fixture print the correct `7`/`9`, while the same partial patch still panics on
the concrete `Point`-only fixture at `calls.rs:733`. So R1b's fix is not conditional on R1
having landed first; R1 is what the *concrete* and self-tail paths need (Bug A, the uid
mismatch), and R1b is what the *composed/generic* path needs (Bug B, the stale span-keyed
entry) — both bugs pre-exist independently in `lower_resolved_word_call` and its callers,
and happen to share one panic site once both are triggered through a member re-splice.
Both are required for every exit criterion to pass, but neither is a precondition for the
other to be correct or testable in isolation (mutation tests 1 and 3 below rely on this).
`mymax`'s
call to `gt` is composed against θ=`Point` (`cross_calls_of`'s `is_combinator` branch,
`src/check/poly.rs:5655-5697`) by grounding `gt`'s own obligations and splicing its body
directly into `mymax[Point]`'s function — `gt`'s internal `cmp` call resolves through
`self.trait_calls`, a **bare-span-keyed** table with no nesting awareness, recording
`[span-of-that-call] -> cmp-for-Point`. Once R1 makes `cmp`-for-`Point`'s own body reachable
(it calls `lt`/`gt` again, now grounded at `i64`, the struct's field type), that *second*
splice of `gt` hits the *same source span* for its internal `cmp` call — and
`self.trait_calls` blindly returns the stale `cmp`-for-`Point` answer again, instead of
falling through to the uid-scoped `splice_trait_calls` table R1 now populates correctly.
That re-enters `cmp`-for-`Point`, which calls `lt`/`gt` again, which hits the same stale
entry again: unbounded recursion. Traced directly (instrumented, depth-capped probe): the
call chain cycles `cmp`-for-`Point` → `gt`(i64, spliced) → `cmp`-for-`Point` (wrongly, via
the stale span-keyed entry) → …

A first-pass fix (gate the lookup on `splice_uid_stack.is_empty()`) was probed and **rejected**:
it is over-broad. `splice_uid_stack` also grows for an *ordinary* combinator splice unrelated
to any re-grounding (e.g. a bound member call sitting inside an `if`'s quotation argument, in
a generic body); gating on "any splice active" breaks that case, which builds and runs
correctly today and has no fallback (`splice_trait_calls` has no entry for it, since
`resolve_splice_member_call`, `poly.rs:930`, needs `poly.combinator_sig`, which this call
shape doesn't have). Reproduced directly: a `myeq` generic word calling `cmp` inside `if`'s
quotation, instantiated at `Point` and `i64`, prints correctly on pristine HEAD but panics
at `calls.rs:733` with the blanket gate alone. See R1b below for the corrected, narrower fix.

### Why the member's own namespace is reconstructible

The checker checks each `impl:` member as an ordinary top-level word with
`inline_uid` seeded at `word_idx * INLINE_UID_STRIDE` (`src/check.rs:1007`), so every
combinator splice inside that member's body carries a uid in `[seed, seed + stride)` —
the traced pair in the brief (`1048577` observed vs `1048576` wanted) is exactly
"lowering's outer counter" vs "`1 * INLINE_UID_STRIDE`". `src/ir/driver.rs:235` already
mirrors that seed for a top-level word, over the same `module.words.iter().enumerate()`
order used at `src/ir/driver.rs:62`. So the datum needed at `calls.rs:189` is the member
word's index in `module.words` — available at the index-build site, absent from
`CombinatorEntry`.

---

## 2. Requirements

### R1 — a spliced member body lowers under its own check-time uid namespace

At `src/ir/func_builder/calls.rs:188-195`, splicing a resolved member body must, for the
duration of that body's lowering:

- push the **member's own** seed onto `splice_uid_stack` (so a nested bare-member call in
  the body finds its `splice_trait_calls[(uid, span)]` entry, `calls.rs:255`, and a nested
  splice finds its `splice_records` entry, `calls.rs:327`), and
- reset **`self.inline_uid`** (the counter, not just the lookup stack) to that same seed for
  the duration, restoring the caller's counter value on the way out. This is not optional:
  a nested combinator splice *inside* the re-seeded body mints its own fresh uid via
  `calls.rs:645-646`'s `self.inline_uid; self.inline_uid += 1`, which is a single counter
  shared across the whole program's lowering. Pushing the seed onto `splice_uid_stack`
  alone (without resetting this counter) leaves that nested mint reading whatever value the
  *caller's* splice chain left the counter at — verified directly: with only the
  `splice_uid_stack` push applied, the concrete `Point`/`lt`/`gt` fixture still panics, one
  level deeper than before, at the *second* nesting level (`cmp`-for-`Point` calling
  `lt`(i64)) instead of the first.

The exact starting value and whether the member's *own top-level* dispatch uses `seed` or
`seed + k` is a **measured** quantity, not a guess: phase 2 reads the checker's numbering
for the member word (the first uid `check_word` mints inside it) and makes lowering agree,
then proves agreement by the fixtures in R6/R7 rather than by inspection. The current
"reuse the enclosing splice's uid" rule is deleted, and the doc comment at
`calls.rs:165-176` that states it (the `57943bb` rationale) is rewritten to state the new
rule (both the stack push and the counter reset) and why the old one only worked one level
deep.

The alpha-renaming rule is unchanged: `alpha_rename_member_locals` (`calls.rs:190`) keeps
its disjoint suffix, because the member seed is still not unique across two splices of the
same member at the same type, and the reason recorded in the existing doc comment (a
member `| x |` and an enclosing `| x |` colliding into one name-keyed local) still holds.

### R1b — `trait_calls`'s span-keyed lookup is gated off during an active member re-splice

`self.trait_calls` (`src/ir/func_builder/mod.rs:209`) resolves a *composed* poly-callee's
own internal trait-member call (e.g. `mymax`'s composed call to `gt` grounds `gt`'s own
`cmp` obligation against `mymax`'s θ and records `trait_calls[span] = cmp-for-θ`, per
`cross_calls_of`'s `is_combinator` branch, `src/check/poly.rs:5655-5697`). This composed
cross-call routes through `poly_calls`, checked *before* the combinator-splice branch
(`calls.rs:316` vs `619`), so `gt`'s composed instantiation lowers as its **own real
`IrFunc`** (`src/ir/driver.rs:340-372`, its own `trait_calls`), matching the existing, correct
doc comment at `src/check/poly.rs:5657-5663`. **Composition calls; it does not itself splice.**
(An earlier draft of this requirement claimed composition splices and asked for the doc
comment at `calls.rs:165-176` to be rewritten to say so; that claim was checked and found
false — no such claim exists at that anchor, and the real, correct comment at `poly.rs:5657-5663`
states the opposite. `calls.rs:165-176` needs no correction for this reason.)

The collision happens **inside** that composed `gt[Point]` `IrFunc`'s own body: R1 makes
its internal `cmp`-for-`Point` reachable, which calls `lt`/`gt` again, now grounded at
`i64`. That second splice of `gt` hits the *same source span* for its internal `cmp` call
as the first grounding did, and `self.trait_calls` — keyed by bare span with no nesting
awareness, checked first at `calls.rs:243-247` — blindly returns the stale first-grounding
answer instead of falling through to the uid-scoped `splice_trait_calls`.

Fix: **not** a blanket "any splice is active" gate. A first-pass version of that (`if
self.splice_uid_stack.is_empty()`) was probed and rejected (see above): it also disables
`trait_calls` for an ordinary, unrelated combinator splice (e.g. a bound member call inside
an `if`'s quotation, in a generic body), which builds and runs correctly today and has no
`splice_trait_calls` fallback for that shape. The correct gate tracks *member re-splice*
nesting specifically, distinct from `splice_uid_stack`:

```rust
// FuncBuilder field, alongside splice_uid_stack:
/// Nesting depth of an active *member re-splice* (R1's bracket in
/// `lower_resolved_word_call`'s combinator branch) specifically — not
/// `splice_uid_stack.is_empty()`, which also counts an ordinary combinator
/// splice unrelated to any re-grounding. `trait_calls` (span-keyed, one
/// grounding per FuncBuilder session) stays valid at any depth of ordinary
/// splice nesting; it only goes stale once a member re-splice reintroduces a
/// second grounding within the same session.
member_splice_depth: u32,
```

Incremented/decremented exactly by R1's push/pop bracket in `lower_resolved_word_call`
(`calls.rs:188-195`, alongside the `splice_uid_stack` push and `inline_uid` reset), never by
the ordinary combinator-splice path (`calls.rs:627-643`). The lookup becomes:

```rust
if self.member_splice_depth == 0 {
    if let Some(sym_name) = self.trait_calls.get(&span).cloned() {
        self.lower_resolved_word_call(&sym_name);
        return;
    }
}
```

Verified directly, in order: (1) the rejected blanket-gate counter-example — a bound member
call inside an `if`'s quotation in a generic body — passes with the narrowed gate (matches
pristine-HEAD output); (2) the generic exit criterion
(`an_ord_bounded_generic_word_instantiates_over_a_user_struct`) passes with correct printed
output (`7`/`9`), not just "doesn't crash"; (3) the concrete R6/R7-style fixtures are
unaffected (`member_splice_depth` stays `0` throughout, since neither ever exercises R1's
bracket while `trait_calls` is non-empty); (4) a full `cargo test --no-fail-fast` with R1 +
this corrected R1b + the `lib/cmp.sth` flip applied together shows exactly the spec's
section-4 table minus the one entry this fix resolves — no new failures.

This was not visible without R1: before R1, no code path could splice the same combinator
twice at different groundings within one `FuncBuilder` session, so `trait_calls`'s flat
span-keying was sound by omission. R1 making a member's own body reachable is exactly what
breaks that invariant, so R1b is inseparable from R1, not a follow-on.

### R2 — the seed reaches lowering without polluting `CombinatorEntry`

`CombinatorEntry` (`src/check/combinators.rs:60-72`) gains **no** uid field. A word index is
meaningless for four of `combinator_index`'s six callers (`src/repl.rs:176`,
`src/check/combinators.rs:38`, `49`, `src/check/drop_graph.rs:1103`…), which pass filtered
or session-derived iterators; an `Option<u32>` there would be a hedge that reads as data
and behaves as a fallback.

Instead, thread a separate `&HashMap<String, u32>` (member word name → check-time uid seed)
into `FuncBuilder` alongside `splice_records`/`splice_trait_calls`, built in `ir::lower`
from `module.words.iter().enumerate()` — the same enumeration `src/ir/driver.rs:62` and
`src/check.rs:1007` walk, so the two sides agree by construction rather than by copying,
the property `src/ir/driver.rs:229-234` already relies on. Keys are `word.name`, matching
`combinator_index`'s own keying (`src/check/combinators.rs:92`), so the existing
`self.combinators.get(sym_name)` lookup and the new seed lookup use one key.

A member name absent from the map keeps today's behaviour (reuse `.last()`), and that is
the REPL's state, not a silent fallback for the native path: the REPL passes
`empty_splice_trait_calls()` already (`src/ir/driver.rs:895-903`, `961-968`), so no member
splice on that path has an entry to miss. Say so in the map's doc comment; do not add a
REPL story this slice.

### R3 — rule on the composed-instantiation `0` seed by measurement, not by hedging

`src/ir/driver.rs:371` and `:968` hardcode `inline_uid_seed = 0` with a rationale comment
(`366-370`) that is true in isolation and, per the brief, false the moment a composed body
transitively splices a concretely-checked member body. R1 fixes the transitive case at its
source: after R1 the spliced member body no longer inherits *any* caller counter, composed
or not, so a `0`-seeded composed instantiation cannot collide through that path.

The ruling: **do not pre-emptively give composed instantiations a third stride.** Land R1
and the corrected R1b together, then run
`an_ord_bounded_generic_word_instantiates_over_a_user_struct`
(`tests/phase7_slice3s_flip.rs:113`, the generic path) and
`inline_mymax_mymax3_matches_noninline_baseline` (`tests/phase7_slice3s_oracle.rs:224`, once
section 3b's rewrite lands). Measured with R1 + the `member_splice_depth`-gated R1b together:
the generic test passes with correct output; the two sites collapse into one fix and
`src/ir/driver.rs:366-370`'s comment is amended to name R1/R1b as the reason `0` is safe.
The comment must stop asserting a reason that is no longer the operative one. **Re-measure
against the landed code in phase 2** — this section's evidence came from a throwaway clone,
not the committed tree.

This is a two-command check inside the fix phase, not an investigative phase of its own
(see section 5).

### R4 — the six comparisons become `inline`, and their rationale comment goes

`lib/cmp.sth:139`, `146`, `161`, `168`, `175`, `182` gain `inline`. Only the *false* part of
the comment at `lib/cmp.sth:10-16` is deleted — lines 10-13a ("`cmp` is an `inline` trait
member … every `impl: Ord` body is spliced at its call sites instead of costing a call
frame…") document `cmp`'s own inlining, which this slice does not change, and stay. Only
the clause starting mid-sentence at 13b ("The six comparisons themselves stay ordinary
non-inline words — splicing them inside a quotation-carrying combinator perturbs quotation
provenance tracking (P7.S3s Phase 1)…") through line 16 is deleted, since only that part is
now false. Re-read the exact line range against the landed file before deleting — a literal
"delete 11-16" instruction truncates a still-true sentence. The replacement text states the
current design (the six are spliced) and does not narrate the reversal.

### R4b — the REPL regression is scoped and ruled on, not silently shipped

The flip turns currently-`#[ignore]`d REPL tests from a clean diagnostic (`error: unknown
word \`lt\`/\`gt\`/\`eq\``) into a compiler panic (`src/check/poly.rs:976:26: index out of
bounds: the len is 1 but the index is 1`) the moment a REPL session imports the flipped
`lib/cmp.sth` and calls one of the six. The affected set is every test whose `#[ignore]`
reason chains to the "same REPL non-inline-poly-word gap" root notes — measured at spec
time as 9 (`grep -rn '#\[ignore' tests/phase1.rs tests/phase4_combinators.rs
tests/phase4_slice10c_tail_splice.rs`, filtered to the`sign_definable_and_callable_in_repl`
and `repl_while_define_runs_to_fixpoint`root notes and their cross-references: 2 root
ignores + 7 cross-referencing ones). **Re-run that grep against the landed code before
rewriting reason strings** — an earlier spec draft hand-copied a shorter line-number list
that undercounted the true set; do not repeat that. This is **checker-side**
(`check/poly.rs`), outside R1/R1b's lowering-only scope, so R1/R1b cannot fix it, and it is
invisible to exit criterion 1 (`cargo test`) since all affected tests are ignored.

Root cause (scoped, not fixed, this slice): `check.rs:1293`/`:1387` hardcode
`TraitResolveCtx::scratch()` for REPL-line checking. Its premise ("a session declares no
`trait:`, so no `Bound::User` reaches a REPL body") predates `Ord` becoming an ordinary
library trait and is false the moment a session imports an inline `Bound::User`-bounded
combinator. A real fix needs a `Session`-level `traits`/`impls` accumulation table (Session
has none today, unlike its existing `structs`/`enums` tables) threaded through both check
sites — comparable in size to the prior struct/enum REPL work, not a one-liner.

**Ruling: out of scope for this slice.** Fixing the REPL's trait/impl accumulation is its
own slice. This slice must: rewrite every affected `#[ignore]` reason string (the two root
notes and every test that cross-references them — re-grep to get the current, complete
set) to state the true, current reason (REPL trait/impl checking is unimplemented, causes
an ICE once a session reaches a `Bound::User` call, tracked as a named follow-up slice); and
record the follow-up in the slice's exit notes and `ROADMAP.md`'s P7 status, alongside the
unsatisfied-`Ord` attribution regression (section 3).

### R5 — the flip's collateral is migrated against the live mechanism, never weakened

Measured, not estimated: with the flip applied at HEAD, `cargo test --no-fail-fast` fails
**12 tests across 6 binaries**. Each is listed in section 4 with its required treatment.
Two of them are helper-level unsoundness, not assertion drift, and matter most:

- **`back_edges`** (`tests/phase4_slice10c_tail_splice.rs:66-71` and its copy at
  `tests/phase4_slice10c_row_gate.rs:44-49`) counts *any* block whose `Jmp` target id is
  `<=` its own id. A spliced comparison body's `Ordering?` eliminator allocates its join
  block before its arm blocks, so an arm's jump to the join is counted as a back-edge. The
  regenerated `examples/gcd.sth` baseline shows the shape directly (`@blk7` jumping to
  `@blk4` inside an inline-`eq` body). The helper must identify the *loop's* back-edge (the
  jump to the block `begin_loop` opened, which `opens_a_loop_header`,
  `tail_splice.rs:78-80`, already locates) rather than any backwards jump. The five
  affected assertions keep their present expectations (`1`, `1`, `0`, `0`, `1`) — the
  helper is what was wrong, and the two one-million-iteration constant-stack tests
  (`spliced_self_tail_runs_one_million_iterations_in_constant_stack` and its `myif` twin)
  pass **with the flip and without the fix**, which is the independent evidence that the
  loop transform itself is unaffected.
- **`inline_mymax_mymax3_matches_noninline_baseline`**'s discriminator
  (`tests/phase7_slice3s_oracle.rs:263-268`) asserts a dispatch target starting with
  `sooth_mono_gt`, whose stated purpose is that "a `gt` -> `lt` swap in the source is
  invisible to this diff" otherwise. Once `gt` is `inline` it mints no monomorph and the
  candidate side finds no such target, so the assertion cannot be repaired by relaxing it
  without deleting the guard. Replace it with a **swap control**: build a third source with
  `gt` replaced by `lt` and assert its stdout differs from the baseline's, restoring the
  guard's discriminating power without grepping for a symbol that inlining removes. This
  is the same blindness recorded for this harness before; do not re-introduce a
  symbol-name discriminator over an inlinable word.

No test in this slice is deleted, retargeted or relaxed to pass. Where an assertion
encoded the retired rationale (the three in `tests/phase4_slice10c_primitives.rs`), it is
**inverted to assert the new fact** and its doc comment rewritten to the current design —
the subject (a library comparison's cost against the raw primitive) stays under test.

### R6 — a `mymax`-free regression test

New test, in `tests/phase7_slice3s_flip.rs` (it already owns the `Ord`-flip goldens and the
`POINT_IMPL` fixture and builds through the real binary against this repo's `lib/`):
an `impl: Ord for Point` whose `cmp` body calls `lt`/`gt`, called from a concrete `main`
with **no generic word anywhere in the call chain**, builds and runs. This is the fixture
that reproduces the panic today; `an_ord_bounded_generic_word_instantiates_over_a_user_struct`
(which does involve `mymax`) is kept as the generic-path companion, and both are exit
criteria.

### R7 — a self-tail-recursive regression guard

New test in the same file: a self-recursive concrete word whose loop condition compares two
`Point`s through the user `impl: Ord` (a member splice inside the loop body), built and
run for its printed answer. Per the brief this shape is a resolved non-issue — uid minting
is static and `emit_back_edge` (`src/ir/func_builder/calls.rs:746`) never reads
`splice_uid_stack` — so this is a cheap guard, not new investigation. It does panic today
at `calls.rs:733`, verified, so it is a real witness for R1 and not a placebo.

### R1c — a committed regression test for R1b's counter-example

Section 1's counter-example (a bound member call, e.g. `cmp`, inside a combinator's
quotation argument — `if`'s branches, or `Ordering?`'s eliminator arms — in a generic body
like `myeq`, instantiated at two distinct types) is currently only a mutation-test witness
(test 3b). It must also be a **committed** test in `tests/phase7_slice3s_flip.rs`, asserting
printed output, not just "doesn't crash" — mutation test 3b then reverts *this* test's
fixture rather than one written ad hoc during the mutation pass. Without a committed
fixture, a future change could re-introduce the rejected blanket gate and nothing in
`cargo test` would catch it, since the existing suite (verified at spec time) is fully
green under that broken gate.

### R8 — the QBE baseline is regenerated deliberately

`tests/qbe_baseline.rs`'s corpus snapshots drift by design: `examples/gcd.sth` loses its
`call $sooth_mono_eq__m3__t0_i64` and the whole `sooth_mono_eq` function, inlining the
comparison into `gcd__m0`'s loop. Regenerate with `REGEN_QBE_BASELINE=1` and **review the
diff** as the test's own doc comment (`tests/qbe_baseline.rs:7-9`) demands: every changed
snapshot must differ only by a comparison monomorph disappearing into its caller. A
snapshot that changes in any other way is a finding to report, not a regeneration.

---

## 3. Ruling on the diagnostic that changes text

`an_unsatisfied_ord_bound_names_the_missing_impl` (`tests/phase7_slice3s_flip.rs:141`)
asserts that an unsatisfied `Ord` names **`lt`**, the word the user wrote, and its doc
comment says so explicitly because `lt` is non-inline. With the flip, the measured
diagnostic becomes:

```text
cannot instantiate `'T` of `cmp` with `Vec2` in `main` (line 147, col 3)
  `Vec2` does not satisfy `Ord`: no `( Vec2 Vec2 -- Ordering )` found
```

— it now names `cmp`, the spliced member, and reports the *library's* line, not the user's.
The second line (the useful one) is unchanged.

**Ruling: accept the new wording, update the assertion and its doc comment, and record the
attribution loss as a named follow-up.** Restoring "name the word the user wrote" means
carrying a splice-origin span through unsatisfied-bound reporting, which is a diagnostics
feature with its own design surface, not a uid fix. This is a genuine, if small,
regression in error quality on the language's most common bound; it must appear in the
slice's exit notes and in `ROADMAP.md`'s P7 status, not be buried in a test edit. Do not
soften the assertion to a substring that passes either way.

### 3b. Ruling on the oracle's now-unsatisfiable dispatch-target assertion

`inline_mymax_mymax3_matches_noninline_baseline` (`tests/phase7_slice3s_oracle.rs:224`)
asserts `baseline_targets == candidate_targets` (`:262`) as well as the `sooth_mono_gt`
reachability check R5 already replaces with a swap control. That equality assertion is a
**second, separate** failure, and it is not patchable: measured directly, the baseline
(`mymax` non-inline) still mints two `sooth_mono_gt__*` symbols — its cross-call to `gt`
composes into a real separate function regardless of `gt`'s own inline-ness (see R1b) —
while the candidate (`mymax` inline too) mints **zero**: once the caller is itself a
combinator, `gt`'s call collapses to a pure lowering-time splice with no symbol at all. So
`baseline_targets == candidate_targets` is permanently false once the library inlines, not
an edge case the swap control's rewrite happens to expose.

**Ruling: drop the dispatch-target-set-equality assertion entirely; keep the stdout-identity
assertion (unaffected, already catches a wrong-direction `gt`/`lt` swap directly); add a
swap control on *each* side** (baseline and candidate), reusing R5's already-approved
mechanism, rather than inventing a disassembly-based codegen check. The test's subject
becomes "inline and non-inline `mymax` produce identical program behaviour," not "identical
dispatch targets" — the latter claim stops being meaningful the moment either side can
inline its own cross-calls away. Rename the test if the current name asserts symbol
identity it no longer checks.

---

## 4. Measured collateral inventory

With the six comparisons flipped to `inline` at HEAD `9c13878`, and **before** R1:

| Test | Site | Cause | Treatment |
| --- | --- | --- | --- |
| `an_ord_bounded_generic_word_instantiates_over_a_user_struct` | `phase7_slice3s_flip.rs:113` | the defect (infinite recursion/stack overflow without R1b, panic at `calls.rs:733` without R1) | fixed by R1+R1b together; exit criterion |
| `an_unsatisfied_ord_bound_names_the_missing_impl` | `phase7_slice3s_flip.rs:141` | diagnostic now names `cmp` | section 3 ruling |
| `the_six_comparisons_are_library_words` | `phase4_slice10c_primitives.rs:265` | asserts `!declares_inline` | invert; assert the six *are* `inline` and stay polymorphic with a `'T: Ord` bound |
| `the_canonical_comparison_and_branch_costs_no_call` | `phase4_slice10c_primitives.rs:366` | asserts an `eq`/`cmp` `IrFunc` is minted and `w` contains a `Call` | invert: no comparison monomorph is minted and `w` is call-free; the test name becomes true again |
| `the_library_eq_costs_a_call_the_branch_primitive_does_not` | `phase4_slice10c_primitives.rs:438` | asserts the library form shows a `call` | invert to the pre-P7.S3s claim it replaced: the library `eq`+`if` form and raw `ueq`+`branch` fold to call-free machine code (assert both call-free; assert equality only if measured, else state the residual difference) |
| `spliced_self_tail_through_shape_changing_myif_lowers_to_a_back_edge` | `phase4_slice10c_row_gate.rs:140` | `back_edges` heuristic (2 ≠ 1) | R5 helper repair |
| `spliced_self_tail_lowers_to_a_back_edge` | `phase4_slice10c_tail_splice.rs:139` | same (2 ≠ 1) | R5 |
| `discard_after_the_parameter_call_stays_ordinary_recursion` | `phase4_slice10c_tail_splice.rs:173` | same (1 ≠ 0); `self_calls == 1` still passes, so the negative shape is intact | R5 |
| `forwarded_recursion_through_a_mid_body_bind_declines_the_loop_but_still_checks` | `phase4_slice10c_tail_splice.rs:216` | same (1 ≠ 0) | R5 |
| `linear_value_forwarded_into_the_spliced_back_edge_is_ok` | `phase4_slice10c_tail_splice.rs:303` | same (2 ≠ 1) | R5 |
| `inline_mymax_mymax3_matches_noninline_baseline` | `phase7_slice3s_oracle.rs:263` | `sooth_mono_gt` no longer exists on the candidate side, **and** `baseline_targets == candidate_targets` is now permanently false | section 3b ruling: drop the equality assertion, keep stdout-identity, swap control both sides |
| `corpus_qbe_stays_byte_identical_to_baseline` | `qbe_baseline.rs:91` | intended codegen drift | R8 regeneration + diff review |

Everything else in the suite (1705 lib unit tests and ~40 other integration binaries) is
green **with the flip and without the fix**, so this table is the whole blast radius for
`cargo test` (non-ignored tests). The nine `#[ignore]`d REPL tests are a **separate**,
checker-side regression not visible to this table — see R4b.

---

## 5. Phase sequencing rationale

The brief is right that the change is architecturally narrow — one uid rule, one library
edit, no rejection path, no generics machinery. It is **not** a one-commit change, though,
for a reason the brief did not have measured: two test helpers are unsound once a
comparison is spliced, and their repair is verifiable *before* the flip, against the
current mechanism.

So: **two phases**, split on "what can be proven green without the flip".

- Phase 1 repairs `back_edges` in both files and replaces the oracle's symbol-grep
  discriminator with a swap control. Both must be green on unflipped `main`, and both must
  be mutation-tested there (a repaired `back_edges` that no longer detects a missing loop
  is worse than the false-positive version it replaces). Doing this first means phase 2's
  red-to-green is attributable to the fix instead of tangled with harness repair — the
  ordering CLAUDE.md's "migrate against the live mechanism first" habit already implies.
- Phase 2 is the fix, the flip, R3's measurement, the assertion inversions, the two new
  regression tests and the baseline regeneration. These cannot be split further: the fix is
  unreachable without the flip, the flip is not green without the fix, and the inverted
  assertions are false in either half alone.

Two things explicitly **not** phased out:

- **No investigative phase for R3.** Deciding whether `src/ir/driver.rs:371`/`:968` need
  their own stride is two `cargo test --test` invocations after R1 lands, with the witness
  fixture already written. A phase whose deliverable is "run two tests" is process for its
  own sake.
- **No phase for the `lib/cmp.sth` flip.** The brief calls it a separate tiny commit; that
  would leave an untracked step whose failure mode is "the fix ships and nothing exercises
  it". It belongs in phase 2.

Every phase leaves `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green.

---

## 6. Test plan

Per CLAUDE.md: unit tests beside the stage code, the exit criteria as goldens,
`thing_condition_expected` naming.

New:

- `tests/phase7_slice3s_flip.rs`: the R6 `mymax`-free concrete `impl: Ord` fixture
  (`a_concrete_impl_ord_delegating_to_lt_builds_and_runs`), the R7 self-tail one, and R1c's
  committed counter-example (a bound member call inside a combinator's quotation argument in
  a generic body, instantiated at two types)
  (`a_self_tail_word_comparing_a_user_struct_in_its_loop_builds_and_runs`), both through
  the real binary against this repo's `lib/`, both asserting printed output.
- `src/ir/func_builder/` unit test for R1's uid rule: a lowered fixture where a spliced
  member body itself splices a combinator, asserting the nested `(uid, span)` lookup
  resolves — i.e. that lowering succeeds where it panics today. If a unit-level fixture
  cannot reach the shape without the real `lib/` (`check()` does not run the trait/impl
  pre-passes, a known constraint), say so in the phase report and let the two integration
  fixtures carry it rather than building a fake.

Mutation tests, before the phase is called done (this project has shipped placebo tests
repeatedly):

1. revert R1 alone (keep R1b), with the flip in place → the R6, R7 and R1c (below)
   fixtures must all fail at `calls.rs:733`. Already verified by hand at spec time for R6/R7;
   re-verify against the landed code and confirm R1c too.
2. revert R1's `self.inline_uid` reset specifically, keeping the `splice_uid_stack` push →
   the R6 concrete fixture must still fail (one level deeper than a full R1 revert, at the
   second nesting level, not the first). This is the exact half-fix that was tried and shown
   insufficient at spec time; the mutation proves the counter reset is load-bearing, not the
   stack push alone.
3. revert R1b alone (keep R1), with the flip in place →
   `an_ord_bounded_generic_word_instantiates_over_a_user_struct` must fail (a stack
   overflow: `cargo test --test phase7_slice3s_flip -- --exact
   an_ord_bounded_generic_word_instantiates_over_a_user_struct` aborts the test binary with
   "has overflowed its stack"/a non-zero signal exit — do not attempt to cap it with an
   in-process depth guard, since a lowering-time stack overflow aborts the whole process,
   not a single assertion). The R6/R7 concrete fixtures must still pass (R1b's absence only
   matters once a composed cross-call and a member re-splice share one `FuncBuilder`
   session).
3b. widen R1b's gate back to `splice_uid_stack.is_empty()` (the rejected first-pass version),
   keeping `member_splice_depth` unused → a bound member call inside an `if`'s quotation
   argument in a generic body (the counter-example in section 1: e.g. a `myeq` word calling
   `cmp` inside `Ordering?`'s eliminator quotations, instantiated at `Point` and `i64`) must
   fail at `calls.rs:733`, though it builds and runs correctly on pristine HEAD and with the
   correct, narrow gate. This is the guard against re-introducing the over-broad gate that
   was probed and rejected during spec work.
4. replace R2's measured seed formula (`word_idx * INLINE_UID_STRIDE`) with a constant `0`
   for every member, and separately with `seed + 1` → run against R6/R7/the generic exit
   criterion **and measure which actually fails**; do not assume the single-member
   `Point`+`i64` fixtures discriminate a wrong formula — that is unverified as of spec time,
   not a fact. If neither wrong formula breaks any existing fixture, add the two-distinct-
   member fixture below (`Point` and a second struct `Point3`, not `Point` and `i64`, since
   a primitive-typed comparison may not exercise the member-keyed map the same way) and
   confirm *that* the wrong formulas actually fail before treating the mutation as done.
5. revert R5's `back_edges` repair, with the flip in place → the five loop assertions must
   fail. And on unflipped `main`, break the loop transform (or point the back-edge at the
   wrong block) → the repaired helper must still fail, or it has become a placebo. Note:
   `back_edges == 0` is implied by `!opens_a_loop_header` for the two 0-expecting
   assertions, so this half must break a case where the header check itself would pass
   (i.e. `blocks[0]` is a `Jmp`) but the count should still be `0`.
6. revert R5's swap control (both sides, per section 3b) →
   `inline_mymax_mymax3_matches_noninline_baseline` must fail on the `gt`→`lt` swapped
   source, on whichever side(s) the control was removed from.
7. per R3's outcome: if a composed-instantiation stride is added, revert it and name the
   failing test; if it is *not* added, record which two tests were run to prove `0` is
   still safe.

Goldens: `examples/gcd.sth`, `examples/factorial.sth`, the `tests/corpus_stdout/*.txt`
program-output corpus and the regenerated `tests/qbe_baseline/` snapshots.

---

## 7. Exit criteria

1. `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green with
   `lib/cmp.sth`'s six comparisons `inline` and the stale rationale comment gone.
2. `an_ord_bounded_generic_word_instantiates_over_a_user_struct` passes (the generic path).
3. The R6 `mymax`-free fixture — a concrete `impl: Ord for Point` delegating to `lt`/`gt`,
   no generics in the call chain — builds and runs.
4. The R7 self-tail-recursive fixture with a member splice inside its loop body builds and
   runs.
5. `lower_resolved_word_call` no longer reads `splice_uid_stack.last()` for a member
   splice, resets `self.inline_uid` alongside `splice_uid_stack` for the duration, and its
   doc comment states the current rule (both halves) with no narration of the old one.
5b. `self.trait_calls`'s lookup (`calls.rs:243-247`) is gated on a new `member_splice_depth`
   field (incremented/decremented only by R1's own bracket) being `0`, not on
   `splice_uid_stack.is_empty()`; the counter-example (a bound member call inside a
   combinator's quotation argument in a generic body) still passes; the doc comment at
   `calls.rs:238-243` states the narrower rule ("valid except during an active member
   re-splice", not "valid only outside all splicing").
6. `src/ir/driver.rs:366-370`'s seed rationale states the operative reason (R3), whether or
   not a composed-instantiation stride was added.
7. `back_edges` identifies the loop's back-edge in both copies, and the five loop
   assertions keep their original expected counts.
8. The oracle's inline/non-inline diff has a discriminator that survives inlining, proven
   by a `gt`→`lt` swap control.
9. Every regenerated QBE snapshot differs only by a comparison monomorph folding into its
   caller, reviewed and stated.
10. The unsatisfied-`Ord` attribution regression (section 3) **and** the REPL trait/impl
    ICE follow-up (R4b) are both recorded in the slice's exit notes and `ROADMAP.md`'s P7
    status; the nine affected `#[ignore]` reason strings state the true, current cause.
11. All eight mutations in section 6 (1, 2, 3, 3b, 4, 5, 6, 7) reported with observed
    result, not just R1/R5.
12. R1c's counter-example fixture is a committed test (not only a mutation-test-time
    fixture), and mutation test 3b reverts to it.
13. CLAUDE.md's five split signals (import divergence, X/Y/Z-in-one-file, mixed
    high/low-level code, non-calling functions in one file, a would-be circular
    dependency) each answered yes/no against `src/ir/func_builder/calls.rs` post-change; 2+
    yes means split or record why not — no default expectation either way.
14. `inline_mymax_mymax3_matches_noninline_baseline`'s dispatch-target-equality assertion is
    removed per section 3b, its stdout-identity assertion and both-side swap controls are
    in place, and it is renamed if its name still claims symbol-target identity.

---

## 8. Risks

- **Guessing the seed instead of measuring it.** R1's whole content is "agree with the
  checker's numbering". An off-by-one that happens to work for a one-deep splice is
  exactly the bug being fixed, one level down. The R6 and R7 fixtures plus a two-deep
  nesting are the only proof; inspection is not.
- **R1 without R1b looks done but isn't.** R1 alone fixes every fixture that never mixes a
  composed cross-call (a poly word calling a bound combinator) with a member re-splice in
  the same lowering pass — which includes both new regression tests (R6, R7) if they are
  read carelessly as "the fix", since neither happens to exercise a composed cross-call.
  Only the generic exit criterion (`an_ord_bounded_generic_word_instantiates_over_a_user_struct`)
  exercises the combination, and its failure mode without R1b is a genuine stack overflow,
  not a clean panic — a phase that stops at "R6/R7 pass" without running the generic test to
  completion would ship an infinite-recursion regression believing itself done. Mutation
  test 3 above is the guard against exactly this.
- **R1b's gate must not be "any splice is active".** That version was probed directly and
  found over-broad: it silently breaks a bound member call inside a combinator's quotation
  argument in a generic body, a shape that works today and has no test coverage before this
  slice. `member_splice_depth`, gated specifically on R1's own bracket, is the requirement;
  mutation test 3b is the guard against silently regressing to the broad version, since
  `cargo test --no-fail-fast` was fully green on the over-broad gate in isolation — the
  existing suite has no fixture that would catch it.
- **Composition calls a real function; it does not splice.** `src/check/poly.rs:5657-5663`
  already says so correctly. Nothing in this slice should assert or imply otherwise — an
  earlier draft of R1b did, and that claim was checked and refuted directly; the actual
  stale-entry recursion happens inside the composed callee's own `IrFunc` body, not because
  composition itself splices.
- **The `back_edges` repair becoming a placebo.** Its current form is over-sensitive; a
  repair that is under-sensitive silently retires five loop-transform guards. Mutation
  test 2's second half is the guard on the guard.
- **The oracle losing its subject quietly.** The natural "fix" is to drop the
  `sooth_mono_gt` assertion. That leaves a test that diffs stdout against stdout of the
  same program. The swap control is not optional.
- **Baseline regeneration as a rubber stamp.** `REGEN_QBE_BASELINE=1` will make the test
  green regardless of what changed. R8's diff review is the only thing standing between
  that and a silent codegen regression across the whole committed corpus.
- **REPL untouched, and it should stay that way.** The REPL passes empty splice tables, so
  no member splice there has an entry to miss; the seed map being empty on that path is a
  documented no-op, not a fallback. Resist widening this slice into REPL member splices
  (the REPL cannot link a materialized quotation either — a separate known blocker).

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Test-harness soundness, provable on unflipped main. (a) Repair `back_edges` in tests/phase4_slice10c_tail_splice.rs:66-71 and its copy in tests/phase4_slice10c_row_gate.rs:44-50 so it counts the loop's back-edge rather than any block whose Jmp target id is <= its own -- today a spliced eliminator's join block is miscounted, which is why five assertions break the moment a comparison is spliced. `opens_a_loop_header` (tail_splice.rs:78-80) is a bool predicate over blocks[0] only (`matches!(f.blocks[0].term, Terminator::Jmp(_))`) and does NOT locate the header id; destructure blocks[0].term's Jmp(target) yourself to get it, then count blocks 1.. whose Jmp target equals it, returning 0 when blocks[0].term is not a Jmp. Keep all five expected counts (1, 1, 0, 0, 1) unchanged. Also land a witness BEFORE the flip: cmp is already inline today (lib/cmp.sth:39), so a self-tail loop splicing an eliminator-carrying inline word already produces the false-positive join-block shape without the flip -- assert the OLD helper counts 2 where the NEW helper counts 1 on that witness, proving the repair fixes a real false positive rather than just staying green. (b) In tests/phase7_slice3s_oracle.rs, replace the `sooth_mono_gt` reachability assertion at 263-268 AND the separate `assert_eq!(baseline_targets, candidate_targets)` at :262 (this second assertion is a distinct failure -- the equality becomes permanently false once the library inlines, since the candidate mints zero gt symbols while the baseline still mints two, not an edge case the first assertion's fix happens to cover) with a `gt`->`lt` swap control applied to BOTH the baseline and candidate sides: build swapped variants of each and assert each swapped stdout differs from its own unswapped baseline. Keep the existing stdout-identity assertion between baseline and candidate (unaffected, still catches a wrong-direction swap). Rename the test if its name still implies dispatch-target-set identity. Do not touch lib/cmp.sth or src/ in this phase. Mutation-test both: break the loop transform (or misdirect the back-edge) and confirm the repaired `back_edges` still fails, AND confirm a case where blocks[0] IS a Jmp but the true back-edge count should still be 0 still passes (back_edges==0 must not just be implied by !opens_a_loop_header on the two 0-expecting assertions); revert each swap control and confirm the corresponding oracle assertion fails on its swapped source. Green on `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.",
      "difficulty": "medium"
    },
    {
      "phase": 2,
      "focus": "The uid fix (R1+R1b), the flip, and the collateral. (1) R1/R2: build a `HashMap<String, u32>` (member word name -> `word_idx * crate::check::INLINE_UID_STRIDE`) in `ir::lower` from module.words.iter().enumerate(), the same enumeration src/ir/driver.rs:62 and src/check.rs:1007 walk; thread it into FuncBuilder beside splice_records/splice_trait_calls (src/ir/func_builder/mod.rs:369-378) -- name every FuncBuilder construction site that must receive the real populated map (the per-word native path, src/ir/driver.rs:235 area, AND the composed/transitive instantiation path, src/ir/driver.rs:371 area) versus an empty map (both REPL paths, src/ir/driver.rs:903 and :968, matching their existing empty_splice_trait_calls() pattern) -- a composed instantiation silently getting an empty map instead of the real one would leave R1 inert on exactly the path the generic exit criterion exercises. In lower_resolved_word_call's combinator branch (src/ir/func_builder/calls.rs:188-195) push the member's own seed onto splice_uid_stack AND reset self.inline_uid to that same seed for the duration AND increment a new member_splice_depth: u32 field on FuncBuilder (decrement/restore all three on exit) -- resetting inline_uid is required in addition to the stack push, verified insufficient otherwise. Measure the checker's actual first-uid for a member word rather than assuming it (assert the map's seed equals the first uid check_word actually mints for that member, don't just trust the formula); add no field to CombinatorEntry; keep alpha_rename_member_locals as is; rewrite the calls.rs:165-176 doc comment to the new rule (uid push + inline_uid reset + member_splice_depth) with no history. (1b) R1b: gate self.trait_calls's lookup at calls.rs:243-247 behind `if self.member_splice_depth == 0` (NOT splice_uid_stack.is_empty() -- that blanket version is WRONG, it breaks a bound member call inside a combinator's quotation argument in a generic body, a case that works today with no splice_trait_calls fallback). Correct the doc comment at calls.rs:238-243 to state the narrower rule. Do NOT touch or claim anything false about src/check/poly.rs:5657-5663's doc comment (it already correctly states composition calls a real IrFunc; do not contradict it). Verify against: the generic mymax-over-Point-and-i64 fixture (must pass with correct output 7/9, not just avoid crashing); AND a counter-example fixture (a bound member call, e.g. cmp, inside an if's quotation argument in a generic body, instantiated at two types) which must still pass since it works on pristine HEAD. (2) R4: add `inline` to lib/cmp.sth:139/146/161/168/175/182; delete ONLY lib/cmp.sth's now-false clause (the sentence fragment starting mid-line-13, \"The six comparisons themselves stay ordinary non-inline words...\", through line 16) -- lines 10 through mid-13 document cmp's own pre-existing inlining and must NOT be deleted, re-read the exact current line range before deleting. (3) R3: run `--test phase7_slice3s_flip` and `--test phase7_slice3s_oracle`; both should pass with R1+R1b together; leave src/ir/driver.rs:371 and :968 at 0 and amend the rationale comment at 366-370 to name R1/R1b; if either test still fails, give the composed instantiation a callee-word_idx-derived seed in the same stride scheme and name the witness test. (4) R6/R7/R1c: add the mymax-free concrete `impl: Ord for Point` fixture, the self-tail-recursive one, AND R1c's counter-example (a bound member call inside a combinator's quotation argument in a generic body, instantiated at two types) to tests/phase7_slice3s_flip.rs, all asserting printed output (R6/R7 panic at calls.rs:733 today -- verified; R1c passes on pristine HEAD and must keep passing -- so all three are real witnesses; note none of the three alone exercises R1b's stale-trait_calls failure mode by itself, only the generic fixture in (3) does, since R1c's shape only breaks under the REJECTED blanket gate, not under a missing R1b). (5) R5/section 3/section 3b: invert the three assertions in tests/phase4_slice10c_primitives.rs (265, 366, 438) to the new fact with rewritten doc comments (for :438 specifically: measure whether the library eq+if form and raw ueq+branch produce identical call-free code or merely both call-free code with a residual instruction-count difference, and assert the measured outcome, not an unmeasured guess), keeping the cost-comparison subject alive; update an_unsatisfied_ord_bound_names_the_missing_impl (141) and its doc comment to the measured `cmp`-naming wording without softening it to a both-ways substring; drop inline_mymax_mymax3_matches_noninline_baseline's assert_eq!(baseline_targets, candidate_targets) at :262 entirely (permanently false once the library inlines, not just the sooth_mono_gt sub-assertion) and rename the test if needed. (6) R4b: re-grep `#[ignore]` across tests/phase1.rs, tests/phase4_combinators.rs, tests/phase4_slice10c_tail_splice.rs for the `sign_definable_and_callable_in_repl` and `repl_while_define_runs_to_fixpoint` root notes and every test that cross-references them (measured at spec time as 9: 2 roots + 7 cross-refs -- do NOT reuse a hardcoded line list, re-derive it against the landed code) and rewrite each reason string to state the true current cause (REPL trait/impl checking is unimplemented -- TraitResolveCtx::scratch() at check.rs:1293/1387 -- causing an ICE at check/poly.rs:976 once a session reaches a Bound::User call; tracked as a named follow-up slice, NOT fixed here). (7) R8: regenerate tests/qbe_baseline with REGEN_QBE_BASELINE=1 and review every diff, confirming each changes only by a comparison monomorph folding into its caller. Then run all eight mutation tests from section 6 (1: R1's stack push, 2: R1's inline_uid reset, 3: R1b's member_splice_depth gate, 3b: R1b's blanket-gate regression guard against R1c, 4: R2's wrong-seed-formula guards -- measure, don't assume, which fixture catches them -- 5: R5's back_edges repair, 6: R5's swap controls, 7: R3's outcome) with observed results, re-run CLAUDE.md's five split signals against src/ir/func_builder/calls.rs and record yes/no for each, and record BOTH the unsatisfied-Ord attribution regression AND the REPL trait/impl ICE follow-up in the slice notes and ROADMAP.md's P7 status.",
      "difficulty": "hard"
    }
  ]
}
```
