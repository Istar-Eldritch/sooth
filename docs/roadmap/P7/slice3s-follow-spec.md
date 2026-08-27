# P7.S3s-follow -- Trait member declaration syntax, and an `inline` trait member

Two changes that ship together, because the second has nowhere to live without
the first:

1. A trait member is declared `: name ( sig ) ;`, the same form
   `parse_worddef` uses, replacing the bespoke bare `name ( sig )`.
2. An optional `inline` keyword sits in that form's one free slot (between the
   member name and its `(`), recorded on `TraitMember`, and read by
   `parse_impl_member_body` so every `impl:` body satisfying that member is
   spliced at its call sites instead of costing a call frame.

`inline` is a property of the member's declaration in `trait: ... ;`, never of
an individual `impl:` block: a trait's contract does not let one conforming
type opt out of a cost another pays. (`impl:` bodies could not carry it anyway
-- an `impl:` member restating anything of its signature is already a parse
error, `impl_member_restated_signature_error`.)

---

## Entry state and blocking precondition

**Verified against source at `03f56f5`, not assumed from roadmap prose.**

### The load-bearing assumption

The slice rests on: *an `inline` impl member's `cmp`-shaped body is reachable
when called from another `inline` word bounded `'T: Ord`.* Probing proved the
checker accepts this unchanged, but lowering panics: both resolved-trait-call
paths funnel into `lower_resolved_word_call`, which expects an `env` entry that
a combinator never has (it mints no `IrFunc`).

A probe patch adding a combinator-splice fallback made a one-comparison program
build and run correctly, but a two-comparison program hit a second panic caused
by uid drift: the probe minted a fresh `self.inline_uid` for the member splice,
desynchronizing from the checker's counter so the next splice's key missed the
`splice_trait_calls` map. Reusing the enclosing splice's uid instead fixed it.

**Therefore this slice is not parser-only.** It needs a lowering change, and
the uid discipline is the sharp edge of it.

### Precondition: `03f56f5` makes 17 tests red

`03f56f5 Make cmp words inline` landed mid-spec from a concurrent session,
making the six surface comparisons in `lib/cmp.sth` `inline` and turning 17
tests red. Some are expected golden drift (they pin the non-inline cost model
this slice deliberately undoes). Several are not — e.g. a functional regression
where splicing `lt` inside a `times` row perturbs quotation provenance tracking.

**Ruling:** `03f56f5` is inside this slice's blast radius, so this slice adopts
it rather than routing around it. The triage phase must classify each of the 17
failures, fix genuine regressions, and re-cut goldens that only pin the retired
cost model. If triage shows the regressions are not fixable within this slice's
surface, the fallback is to revert `03f56f5` and re-land the six comparisons'
`inline` last, where the lowering splice path already exists to support it.

### `docs/book/` is out of scope

The book teaches no trait syntax at all (`grep -rn "trait" docs/book/` returns
nothing), so it needs no migration here.

---

## The grammar change

The member declaration form changes from bare `name ( sig )` to
`: name inline? ( sig ) ;`, matching `parse_worddef`'s shape:

```sooth
trait: Ord 'T
  : cmp inline ( 'T 'T -- Ordering ) ;
;
```

The `inline` slot mirrors `parse_worddef` exactly: one peek for
`Token::Word("inline")` between the name and the `(`, consuming at most one.
No global reservation is needed — the name is already consumed, so
`: inline ( 'T -- ) ;` still declares a member *named* `inline`, and a second
`inline` falls through to the `(` and fails there.

`parse_trait_member_effect` is unchanged; only what wraps it changes. Every
existing member gate stays in the same order with the same span.

The `declares_inline` flag is added to `TraitMember` and read by
`parse_impl_member_body` in **both** branches (concrete and generic target).
Each branch gets its own test because the twinned-guard failure mode — a pair
where only one half is covered — has shipped in this repo before.

`is_combinator` is exactly `word.declares_inline`, so a synthesized member word
joins the combinator index with no further plumbing and mints no `IrFunc`.

---

## Lowering: a resolved trait member that is a combinator

Both resolved-trait-call paths call `lower_resolved_word_call`, which expects
an `env` entry a combinator never has. The function gains a splice path taken
when the resolved symbol names a combinator, mirroring the ordinary combinator
splice: alpha-rename the body, lower its terms, truncate `self.locals`.

**The uid rule.** The checker does not splice a resolved trait member — it
checks it as an ordinary call against the member word's grounded signature — so
no uid was allocated for it. Minting a fresh one desynchronizes the counters and
the next splice's lookups miss; the observed symptom is a downstream
`checked user word exists: cmp` panic one comparison later, which reads like an
unrelated bug. The member splice therefore **reuses the enclosing splice's uid**
(`splice_uid_stack.last()` or the top-level default) and pushes nothing.

Two consequences pinned with tests:

- Two member splices under one enclosing uid alpha-rename to the same local
  names. This is correct today because the splice truncates `self.locals` to
  its entry depth and the resolver is scope-bounded — but it is correct by a
  property easy to break, so it gets a dedicated value-checking test.
- The member splice passes `tail = false`. A trait member body is not the
  enclosing word's tail; threading the caller's `tail` would let the member's
  terms back-edge into a loop they do not belong to.

---

## Diagnostic: the retired bare-member form

Diagnostics are behaviour, so this is specified. In the member position, a
`Token::Word` that is not `:` is overwhelmingly the old grammar, so the error
names the new form directly rather than reporting a token mismatch:

> `error: trait `{trait_name}` declares member `{member}` without a leading `:` at line {l}, col {c}`
> `  note: a trait member is declared `: {member} ( ... ) ;`, the same form as a word definition`

The non-word case (`(`, a literal, EOF) keeps the existing "expected a member
name or `;`" shape, reworded to name `:` instead of a member name. The
unterminated-trait EOF path is unchanged.

---

## Migration scope

Every existing fixture in the old grammar breaks at once, so the migration
lands in the same phase as the grammar change. There are **172 declaration
sites** across `src/` (94), `tests/` (74), and 4 in `examples/traits.sth`,
`lib/cmp.sth`, and `README.md`. Some `src/` sites are doc-comment prose teaching
the retired grammar; those are migrated too — a doc comment teaching a retired
grammar is a stale spec. These are in-place rewrites, not additions; no
old-form copy is left behind.

### Consumer migration

- **`lib/cmp.sth`:** `cmp` becomes `: cmp inline ( 'T 'T -- Ordering ) ;` in
  `trait: Ord 'T`. The header comment is rewritten to state the shipped design
  (it previously said the comparisons are "deliberately not inline"). The six
  `inline`s on `eq`/`lt`/`gt`/`lte`/`gte`/`ne` stay — those are ordinary words,
  not impl members.
- **`examples/traits.sth`:** Both `trait:` blocks (`Order`, `Show`) migrate.
  `Order`'s by-reference `cmp` member is a good second shape for the inline
  path; it built and ran correctly under the all-inline probe. Prefer marking
  exactly one of the two members `inline` so the file demonstrates both forms.
- **`README.md`:** The `trait: Show 'T` block migrates.

---

## Test plan

- **Parser diagnostics:** bare-member error names the `: name ( ... ) ;`
  replacement (substance-matched, not just `is_err()`); missing-terminating-`;`
  is an error; a member named `inline` parses with `declares_inline == false`;
  double `inline` fails at the `(`.
- **Parser, happy path:** `declares_inline == true` recorded on the member;
  existing non-inline default coverage stays.
- **Parser, both impl branches:** each branch independently inherits the
  member's flag; mutation-test both halves separately.
- **Lowering:** assert on QBE text (the `impl:` body's terms appear at the call
  site, no `call $...cmp...` does), paired with a behavioural golden whose
  printed output is checked. Plus the two-splice value test from the lowering
  section.
- **Regression floor:** `examples/traits.sth` and every `phase7_slice3*` suite
  must stay green.
- Mutation-test each new guard; commit before mutating (a `git checkout --
  src` mutation run has wiped uncommitted work before).

---

## Exit criteria

1. `trait: Ord 'T : cmp inline ( 'T 'T -- Ordering ) ; ;` parses, with
   `declares_inline == true` reaching both `parse_impl_member_body` branches.
2. The bare `name ( sig )` form produces `bare_trait_member_error`, located,
   naming the `: name ( ... ) ;` replacement.
3. Each `impl: Ord for ...` block's `cmp` is spliced at every call site reached
   through a `'T: Ord` bound word, with no `Instr::Call` to a member symbol.
4. All 172 declaration sites migrated; no old-form site remains outside
   `docs/roadmap/`.
5. `lib/cmp.sth`'s header comment states the shipped design.
6. `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green,
   including the 17 tests red at `03f56f5`.

---

## Residual risks

- **The uid rule is a landmine.** Reusing the enclosing uid is correct for the
  shapes probed; a nested combinator inside a spliced member body was not
  probed and could still collide. If a `checked user word exists` or
  `checked resolved call exists` panic appears downstream, suspect uid drift
  before the local call site.
- **The 17 red tests may not all be golden drift.** Triage must classify each
  one; treating a genuine regression as stale-golden noise would bury a real
  bug.
- **A trait member named `inline` is legal**, and so is a word named `inline`.
  The two carve-outs must not drift apart.
- **Concurrent sessions are committing to `main`.** Re-check `git log` before
  starting each phase.

---

## Implementation

| Area | Commit | Key files |
| --- | --- | --- |
| Triage 17 red tests at `03f56f5` | `34a0aca9` | `lib/cmp.sth` |
| Colon-syntax trait member grammar + migrate all 172 declaration sites | `63c90078` | `README.md`, `examples/traits.sth`, `lib/cmp.sth`, `src/check/declarations.rs`, `src/check/poly.rs`, `src/driver.rs`, `src/ir/driver.rs`, `src/parser.rs` |
| `declares_inline` on `TraitMember`, parse optional `inline` keyword, read in both `parse_impl_member_body` branches | `a66818da` | `src/ast.rs`, `src/parser.rs` |
| Combinator splice path in `lower_resolved_word_call` (reuses enclosing uid) | `de651503` | `src/ir/func_builder/calls.rs` |
| Mark `cmp` inline in `lib/cmp.sth`, rewrite header, migrate example, add lowering/behavioural tests, re-cut QBE baselines | `f3017267` | `examples/traits.sth`, `lib/cmp.sth`, `src/ir/func_builder/calls.rs`, `tests/phase7_slice3s_flip.rs`, `tests/phase7_slice3s_oracle.rs`, `tests/qbe_baseline/*.ssa` |
| Remove dead `reject_user_bound_on_combinator`/`user_bound_on_combinator_error`, mutation-test guards, re-green | `ea1df1fb` | `src/check.rs`, `src/check/poly.rs`, `tests/phase4_generics.rs`, `tests/phase4_slice10c_primitives.rs` |
