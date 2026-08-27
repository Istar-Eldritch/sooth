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

## 0. Entry state, and one blocking precondition

**Verified against source at `03f56f5`, not assumed from roadmap prose.**

### 0.1 The load-bearing assumption holds, but not for free

The slice rests on: *an `inline` impl member's `cmp`-shaped body is reachable
when called from another `inline` word bounded `'T: Ord`.* This was probed
directly by hardcoding `declares_inline: true` at both `parse_impl_member_body`
sites and building against this repo's own `lib/`:

- The **checker** accepts it unchanged. `reject_user_bound_on_combinator` is
  already dead code (`#[allow(dead_code)]`, `src/check/poly.rs:7515`); S3o
  removed its call site at `src/check.rs:941`, and the per-splice
  `splice_trait_calls` map resolves a bare `cmp` inside a spliced `lt` to the
  `impl:` body's synthesized symbol.
- **Lowering panics.** `src/ir/func_builder/calls.rs:235`,
  `checked resolved call exists`. Both resolved-trait-call paths
  (`trait_calls` at `calls.rs:278`, `splice_trait_calls` at `calls.rs:291`)
  funnel into `lower_resolved_word_call`, which does `self.env.get(sym_name)`
  -- and a combinator mints no `IrFunc` and therefore no `env` entry
  (INV-INLINE-COMBINATOR, `calls.rs:640`).

A probe patch adding a combinator-splice fallback inside
`lower_resolved_word_call` made a one-comparison program build and run
correctly. A two-comparison program (`lt` and `gt` in one `main`) then failed
with a *second* panic, `checked user word exists: cmp`, because the probe
allocated a fresh `self.inline_uid` for the member splice: lowering's uid
counter drifted out of step with the checker's, so the next splice's
`(uid, span)` key missed `splice_trait_calls` entirely. Reusing the enclosing
splice's uid instead of minting one fixed it, and both programs, `two.sth` (a
bounded inline word calling `cmp` twice), and `examples/traits.sth` then built
and produced correct output.

**Therefore this slice is not parser-only.** It needs a lowering change
(section 3), and the uid discipline is the sharp edge of it.

### 0.2 Precondition: `main` is red at `03f56f5`, and it is not golden drift

`a640c13` (`Merge branch 'slice3o-impl'`) is green. The commit on top of it,
`03f56f5 Make cmp words inline` -- landed by a concurrent session while this
spec was being written -- makes the six surface comparisons in `lib/cmp.sth`
`inline`, and turns 17 tests red:

```text
a_generic_body_compares_its_own_variable_through_an_imported_generic_word
a_mutual_non_growing_generic_pair_compiles_runs_and_terminates
an_ord_bounded_generic_word_instantiates_over_a_user_struct
an_unsatisfied_ord_bound_names_the_missing_impl
corpus_qbe_stays_byte_identical_to_baseline
dup_quotation_self_tail_loop_runs_in_constant_stack
inline_mymax_mymax3_matches_noninline_baseline
ir::func_builder::calls::tests::each_lowers_to_a_loop_not_a_per_element_call
quotation_left_as_a_declared_output_is_error
self_tail_combinator_dups_an_inline_quotation_parameter
self_tail_combinator_dups_its_quotation_instead_of_binding_it
signed_vs_unsigned_compare_differ_on_same_bit_pattern
the_canonical_comparison_and_branch_costs_no_call
the_six_comparisons_are_library_words
times_carries_an_untouched_quotation_through_the_row
times_nested_inside_times_still_carries_a_row_quotation
while_carries_an_untouched_quotation_through_the_row
```

Some of these are expected golden drift (`the_six_comparisons_are_library_words`,
`the_canonical_comparison_and_branch_costs_no_call`,
`corpus_qbe_stays_byte_identical_to_baseline` all pin the *non-inline* cost
model this slice is deliberately undoing). Several are **not**. For example:

```text
times_carries_an_untouched_quotation_through_the_row
  build should succeed: "error: `call` in `main` (line 4) expects a quotation
  on the stack (a quotation cannot be a runtime value; a runtime quotation
  value is slice 7)"
```

That is a functional regression: splicing `lt` inside a `times` row perturbs
quotation provenance tracking. `signed_vs_unsigned_compare_differ_on_same_bit_pattern`
and the three `self_tail_combinator_dups_*` failures are in the same
suspicious class.

**Ruling.** `03f56f5` is inside this slice's blast radius (it is exactly the
`lib/cmp.sth` consumer migration this slice owns), so this slice adopts it
rather than routing around it. Phase 1 is a triage phase that must land green:
classify each of the 17, fix the genuine regressions, and re-cut the goldens
that only pin the retired cost model. **No later phase starts until Phase 1 is
green.** If triage shows the regressions are not fixable within this slice's
surface, the correct move is to revert `03f56f5` and re-land the six
comparisons' `inline` last, alongside `cmp`'s own in Phase 5, where the
lowering splice path (Phase 4) already exists to support it.

### 0.3 `docs/book/` is out of scope, confirmed

`grep -rn "trait" docs/book/` returns nothing. The book teaches no trait syntax
at all, so it needs no migration here. (Its known divergence on `if`/`else`/`end`
is a separate, pre-existing matter.)

---

## 1. The grammar change

### 1.1 Current shape

`parse_trait_decl` (`src/parser.rs:2258`) loops on `self.peek()`:
`Token::Semicolon` ends the trait; `Token::Word(_)` is taken as a bare member
name, followed by `expect(Token::LParen)`, `parse_trait_member_effect`,
`expect(Token::RParen)`, and no terminator.

### 1.2 New shape

```sooth
trait: Ord 'T
  : cmp inline ( 'T 'T -- Ordering ) ;
;
```

The member loop becomes:

- `Token::Semicolon` -> break (unchanged; this is the trait's own terminator).
- `Token::Word(w) if w == ":"` -> consume it, then parse one member:
  name, optional `inline`, `(`, `parse_trait_member_effect`, `)`, `;`.
- anything else -> the new diagnostic (section 4).

`:` and `;` are `Token::Word(":")` and `Token::Semicolon` respectively (the
lexer emits `;` as its own token at `src/lexer.rs:98`; `:` is an ordinary
word, which is why `parse_worddef` opens with `self.expect_word(":")`). The
member's terminating `;` and the trait's terminating `;` are the same token but
never ambiguous: the member's arrives immediately after its `)`.

**Reuse, do not rewrite.** `parse_trait_member_effect` (`src/parser.rs:2547`)
is unchanged; only what wraps it changes. Every existing member gate stays,
in the same order and with the same span (`member_span` is still the name's):
`reject_reserved_name("word", ...)`, `ACCESS_WORDS`,
`is_name_dispatched_builtin` plus the `call`/`slice`/`subslice` trio.

### 1.3 The `inline` slot

Mirror `parse_worddef` (`src/parser.rs:2268`) exactly, including its reasoning:
one peek for `Token::Word("inline")` between the name and the `(`, consuming at
most one. This needs no global reservation -- the name is already consumed, so
`: inline ( 'T -- ) ;` still declares a member *named* `inline`, and a second
`inline` falls through to the `(` and fails there. The flag goes on
`TraitMember`:

```rust
pub struct TraitMember {
    pub name: String,
    pub sig: PolySig,
    pub declares_inline: bool,
}
```

`TraitMember` is constructed at one site (`src/parser.rs`, the member loop);
grep for other constructors before adding the field, and update
`src/ast.rs:1773`.

---

## 2. `parse_impl_member_body` reads the flag

`parse_impl_member_body` (`src/parser.rs:2696`) already looks the member up by
name to steal its `sig`. Widen that lookup to take `(sig, declares_inline)` in
one pass, and replace the two hardcoded `declares_inline: false` literals (the
concrete branch ~2749, the generic branch ~2790) with the member's flag.

Both branches, not one. The generic-target branch is the easier one to forget,
and the twinned-guard failure mode (a pair where only one half is tested) has
already shipped in this repo more than once: each branch gets its own test.

`is_combinator` (`src/check/combinators.rs:155`) is exactly
`word.declares_inline`, so the synthesized member word joins
`collect_combinators`/`combinator_index` with no further plumbing, and mints no
`IrFunc`.

---

## 3. Lowering: a resolved trait member that is a combinator

This is the phase the probe proved is required, and the one to be careful in.

Both resolved-trait-call paths call `lower_resolved_word_call`
(`src/ir/func_builder/calls.rs:229`), which `expect`s an `env` entry that a
combinator never has. `lower_resolved_word_call` gains a splice path taken when
the resolved symbol names a combinator, mirroring the ordinary combinator
splice at `calls.rs:649`: alpha-rename the body, lower its terms, truncate
`self.locals`.

**The uid rule, and why.** The ordinary splice site mints a fresh
`self.inline_uid` and pushes it on `splice_uid_stack`, because the checker
allocated a matching uid for that same splice and every inner poly call is
keyed `(uid, span)`. The checker does **not** splice a resolved trait member --
it checks it as an ordinary call against the member word's grounded signature
-- so no uid was allocated for it. Minting one here desynchronizes the two
counters and the *next* splice's `splice_records`/`splice_trait_calls` lookups
miss; the observed symptom is a downstream `checked user word exists: cmp`
panic, one comparison later, which reads like an unrelated bug. The member
splice therefore reuses the enclosing splice's uid
(`splice_uid_stack.last()`, or the top-level default) and pushes nothing.

Two consequences to pin with tests:

- Two member splices under one enclosing uid alpha-rename to the same local
  names. The probe showed this is correct today, because the splice truncates
  `self.locals` to its entry depth and the resolver is scope-bounded -- but it
  is correct by a property that is easy to break, so it gets a dedicated test
  (a bounded `inline` word calling `cmp` twice, checked for the right *value*,
  not merely for building).
- The member splice passes `tail = false`. A trait member body is not the
  enclosing word's tail, and threading the caller's `tail` would let the
  member's terms back-edge into a loop they do not belong to.

If a self-tail member body ever needs the loop form, that is not this slice;
`terms_tail_call_self` is the predicate that would decide it.

---

## 4. Diagnostic: the retired bare-member form

Diagnostics are behaviour here, so this is specified, not left to a generic
parse failure. In the member position, a `Token::Word` that is not `:` is
overwhelmingly the old grammar, so it names the new form directly:

```rust
/// P7.S3s-follow: the retired bare `name ( sig )` trait member form. A word
/// where `:` or `;` is expected is almost always the old grammar, so the
/// error names the replacement rather than reporting a token mismatch.
fn bare_trait_member_error(trait_name: &str, member: &str, span: Span) -> String {
    format!(
        "error: trait `{trait_name}` declares member `{member}` without a leading `:` at line {}, col {}\n  note: a trait member is declared `: {member} ( ... ) ;`, the same form as a word definition",
        span.line, span.col
    )
}
```

The non-word case (`(`, a literal, EOF) keeps the existing "expected a member
name or `;`" shape, reworded to name `:` instead of a member name. The unterminated-trait EOF path (`self.eof_error`) is unchanged.

Required tests, `thing_condition_expected`:

- `parse_trait_decl_bare_member_names_the_colon_form` -- the old
  `cmp ( 'T 'T -- Ordering )` spelling produces the message above, matched on
  its substance (the trait name, the member name, and the `: cmp ( ... ) ;`
  note), not merely `is_err()`.
- `parse_trait_decl_member_missing_terminating_semicolon_is_error` -- `: cmp
  ( 'T 'T -- Ordering )` with no `;` before the trait's own `;`.
- `parse_trait_decl_member_named_inline_still_parses` -- `: inline ( 'T -- ) ;`
  declares a member *named* `inline` with `declares_inline == false`, the
  member-side twin of `parse_worddef`'s own carve-out.
- `parse_trait_decl_member_double_inline_is_error` -- the second `inline`
  fails at the `(`.

---

## 5. Migration inventory (grep-driven, not list-driven)

Every existing fixture in the old grammar breaks at once, so the migration
lands in the *same* phase as the grammar change. Counted at `03f56f5` with:

```sh
git ls-files '*.rs' '*.sth' '*.md' | grep -v '^docs/roadmap' \
  | xargs grep -cE "trait: +[A-Za-z_][A-Za-z0-9_]* +'"
```

**172 declaration sites**: 94 in `src/`, 74 in `tests/`, 4 in
`examples/traits.sth` (2), `lib/cmp.sth` (1), `README.md` (1). Per file:

| count | file |
| --- | --- |
| 33 | `src/check/poly.rs` |
| 26 | `tests/phase7_slice3e.rs` |
| 25 | `src/parser.rs` |
| 23 | `tests/phase7_slice3r.rs` |
| 19 | `src/check/declarations.rs` |
| 14 | `src/driver.rs` |
| 13 | `tests/phase7_slice4.rs` |
| 7 | `tests/phase7_slice3t.rs` |
| 2 | `tests/phase7_slice3s_crosscall.rs`, `src/ir/driver.rs`, `examples/traits.sth` |
| 1 | `tests/phase7_slice3s.rs`, `tests/phase7_slice3p.rs`, `tests/phase7_slice3h.rs`, `src/repl.rs`, `README.md`, `lib/cmp.sth` |

Some `src/` sites are doc-comment prose (`parse_trait_decl`'s own header, the
`impl:`/`trait:` narration in `check/poly.rs` and `check/declarations.rs`).
Those are migrated too: a doc comment teaching the retired grammar is a stale
spec.

The parser's own 25 sites span 24 test functions (plus one in
`parse_extern_decl`); the task-brief list drifted from the file, so it is
superseded by this command:

```sh
grep -nE "trait: +[A-Za-z_][A-Za-z0-9_]* +'" src/parser.rs
```

**These are migrations, not additions.** Rewrite each fixture string in place;
do not leave an old-form copy behind, and do not add a new-form sibling next to
an unmigrated original. Exit check for the phase:

```sh
git ls-files | grep -v '^docs/roadmap' \
  | xargs grep -nE "trait: +[A-Za-z_][A-Za-z0-9_]* +'" -A2 \
  | grep -vE "^\S+[-:][0-9]+[-:]\s*(:|;|\\\\|\|)" | head
```

should surface nothing but the trait header lines themselves. A scripted
rewrite is fine and preferred, but verify the count of *changed* sites equals
the count of *found* sites: a scripted edit that silently matches nothing is a
failure mode this repo has hit.

---

## 6. Consumer migration

**`lib/cmp.sth`.** `cmp` becomes `: cmp inline ( 'T 'T -- Ordering ) ;` in
`trait: Ord 'T`. The header comment block (lines 10-15) still opens "The
comparisons are deliberately **not** `inline` here (P7.S3s R5)" while the six
words below it already are; it is rewritten to state the shipped design: `cmp`
is an `inline` trait member, every `impl: Ord` body is spliced, and the six
surface comparisons are spliced over it.

Note the brief's suggestion to "drop `inline` from each `impl: Ord for ...`
member body" is **not applicable**: an `impl:` member body has never been able
to carry `inline` (restating any of the inherited signature is
`impl_member_restated_signature_error`). The six `inline`s in that file are on
`eq`/`lt`/`gt`/`lte`/`gte`/`ne`, which are ordinary words, not impl members;
they stay.

**`examples/traits.sth`.** Both `trait:` blocks (`Order`, `Show`) migrate to
the new grammar. `Order`'s `cmp ( &'T &'T -- Rank )` is a *by-reference*
member and a good second shape for the `inline` path: it built and ran
correctly under the all-inline probe. Whether to mark it `inline` in the
example is a judgment call -- prefer marking exactly one of the two, so the
file demonstrates both member forms.

**`README.md`** line 124's `trait: Show 'T` block migrates.

---

## 7. Test plan

Beyond the migration and the four parser diagnostics tests above:

- **Parser, happy path.** `parse_trait_decl_records_an_inline_member` --
  `declares_inline == true` on the member; the existing
  `parse_trait_decl_records_its_members` keeps covering the non-inline default.
- **Parser, both impl branches.**
  `parse_impl_body_inherits_the_members_inline_flag` (concrete target) and
  `parse_impl_body_generic_target_inherits_the_members_inline_flag`. Each must
  fail if its own branch's flag is reverted to `false`; mutation-test both
  halves independently, since a twinned pair covered in one half only has
  shipped here before.
- **Lowering.** An `nm`-style symbol assertion is worthless here (a spliced
  member mints no symbol, but so does a poly word regardless of dispatch).
  Assert on QBE text instead: the `impl:` body's terms appear at the call site
  and no `call $...cmp...` does. Pair it with a *behavioural* golden -- an
  `Ord`-bounded program whose printed output is checked -- so a wrong splice is
  caught as a wrong answer, not just a shape change.
- **The two-splice test** from section 3.
- **The regression floor.** `examples/traits.sth` and every `phase7_slice3*`
  suite must stay green; these are the trait grammar's real goldens.

Mutation-test each new guard before declaring the slice done: delete or invert
what the test guards and confirm the test fails. Commit first -- a mutation run
that restores with `git checkout -- src` has wiped uncommitted work in this
repo before.

---

## 8. Exit criteria

1. `trait: Ord 'T : cmp inline ( 'T 'T -- Ordering ) ; ;` parses, with
   `declares_inline == true` reaching both `parse_impl_member_body` branches.
2. The bare `name ( sig )` form produces `bare_trait_member_error`, located,
   naming the `: name ( ... ) ;` replacement.
3. Each `impl: Ord for ...` block's `cmp` is spliced at every call site reached
   through a `'T: Ord` bound word, with no `Instr::Call` to a member symbol.
4. All 172 declaration sites migrated; no old-form site remains anywhere
   outside `docs/roadmap/` (where historical spec prose is left alone).
5. `lib/cmp.sth`'s header comment states the shipped design.
6. `cargo fmt --check && cargo clippy -- -D warnings && cargo test` is green,
   including the 17 tests currently red at `03f56f5`.

## 9. Residual risks

- **The uid rule is a landmine.** Reusing the enclosing uid is correct for the
  shapes probed; a nested combinator inside a spliced member body was not
  probed and could still collide. If a `checked user word exists` or
  `checked resolved call exists` panic appears anywhere downstream, suspect uid
  drift before suspecting the local call site.
- **The 17 red tests may not all be golden drift.** Phase 1 must classify each
  one; treating a genuine quotation-provenance regression as stale-golden noise
  and re-cutting it would bury a real bug under a rewritten expectation.
- **A trait member named `inline` is legal**, and so is a *word* named
  `inline`. The two carve-outs must not drift apart.
- **Concurrent sessions are committing to `main`.** `03f56f5` landed mid-spec.
  Re-check `git log` before starting each phase.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Triage the 17 tests red at 03f56f5: classify each as retired-cost-model golden drift or genuine regression from inlining the six comparisons, fix the regressions, re-cut the goldens, land green",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "Change parse_trait_decl to the `: name ( sig ) ;` member grammar with the bare_trait_member_error diagnostic, and migrate all 172 declaration sites across src, tests, examples, lib and README in the same phase",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "Add declares_inline to TraitMember, parse the optional inline keyword in the parse_worddef slot, and read it in both parse_impl_member_body branches"
    },
    {
      "phase": 4,
      "focus": "Give lower_resolved_word_call a combinator splice path that reuses the enclosing splice uid, so a resolved trait member that is inline is spliced instead of called",
      "difficulty": "hard"
    },
    {
      "phase": 5,
      "focus": "Mark cmp inline in lib/cmp.sth, rewrite its stale not-inline header comment, migrate examples/traits.sth, and add the lowering and two-splice behavioural tests"
    },
    {
      "phase": 6,
      "focus": "Remove the dead reject_user_bound_on_combinator and user_bound_on_combinator_error functions from poly.rs (S3o dead code whose call site was already removed; the inline member path now handles what it guarded), update the stale comment at poly.rs:4825 that references it, mutation-test every new guard, re-run the full green gate, and confirm no old-form trait member syntax remains outside docs/roadmap"
    }
  ]
}
```
