# P7.S3p brief — A trait member is only dispatchable through a bound if its *last* declared input is the bound variable

## Problem, confirmed live against current `main`

`receiver_ty_var` (`src/check/poly.rs:798-807`) is the single gate every bound-directed
dispatch goes through:

```rust
fn receiver_ty_var(stack: &[PolySlot]) -> Option<u32> {
    match &stack.last()?.pt {
        PolyType::Var(v) => Some(*v),
        PolyType::Ref(referent, _) => match referent.as_ref() {
            PolyType::Var(v) => Some(*v),
            _ => None,
        },
        _ => None,
    }
}
```

It inspects only `stack.last()` — the top-of-stack operand — and answers "is this a bound
type variable at all", never "which declared input position is the bound variable at".
`poly_trait_member_call` (`poly.rs:835-954`) calls it once, at the top of
`poly_call_term` (`poly.rs:1021`, ahead of every other dispatch arm per S3e review finding
3), and if it returns `None` the call falls straight through the rest of `poly_call_term`
to ordinary `env`/builtin dispatch or `unknown_word_error`.

Consequently a trait member whose bound variable is *not* the last declared input can
never be reached through this mechanism, regardless of how the trait is declared, because
nothing after `receiver_ty_var` ever looks past the top slot for a candidate variable. Three
shapes are affected:

- **A receiver elsewhere in the input list.** `at ( &'T i64 -- i64 )` (an index/lookup
  shape) — the bound variable is `inputs[0]`, not `inputs.last()`.
- **A zero-input member.** `tag ( -- i64 )` — a constructor or nullary accessor has no
  operand at all to inspect.
- **Any shape with a trailing non-`'T` input**, e.g. `insert ( &!'T i64 'V -- )` where the
  bound variable sits under an unrelated argument on top.

S3e did not attempt real dispatch for these; it added a **declaration-time rejection**
instead (`member_ends_in_trait_var`, `src/check/declarations.rs:361-364`, wired into
`check_trait_decls` at `:341-345`), so today a `trait:` block naming any such member is
rejected outright with `non_trailing_receiver_error` (`declarations.rs:367-373`), which
names this slice by number in its own message text. This is a closed door, not a silent
mis-dispatch — the danger S3e's own comment (`declarations.rs:335-340`) worried about
(falling through to an unrelated same-named concrete word, or a confusing "unknown word")
is avoided by never letting such a trait declare in the first place.

## Existing precedent (what's already there to build on)

**The declaration-time rejection is the only thing gating this today.** There is no
partial support, no other code path that tries and fails — `member_ends_in_trait_var` is a
pure syntactic check over `member.sig.inputs.last()` and runs once, at `trait:` checking,
long before any call site exists. Lifting it is purely additive: nothing downstream
currently depends on "every trait member's receiver is on top of the stack" except
`receiver_ty_var` itself and its one caller.

**`poly_trait_member_call`'s substitution machinery already generalizes cleanly past the
top slot.** Once a candidate variable `var` and its bound traits are known, the rest of
the function (`poly.rs:882-951`) already walks the *member's full declared input list*
(`inputs: Vec<PolyType>`, `poly.rs:882-887`, built via `substitute_member_var` against
every declared input, not just the last) and checks each one against the stack window at
`base = stack.len() - inputs.len()` (`poly.rs:895-905`). This part has no positional
assumption baked in — it already handles `at`-shaped members correctly *if* it is ever
reached with the right `var`. The entire gap is upstream, in how `var` gets discovered.

**The three-barrier partition (S3e, R7) is receiver-position-agnostic and needs no
change.** `poly_var_to_concrete_error`, `poly_delegate_op`'s concrete-suffix truncation,
and `poly_op_on_variable_error` (`slice3e-spec.md` design ruling 7) are the fallback paths
a call takes when bound dispatch declines; they are unaffected by *where* the receiver
sits, only by *whether* dispatch found one. No change needed there.

## The actual gap: finding a candidate variable when it isn't on top

`receiver_ty_var` has no way to answer "does *any* operand in the call's operand window
carry one of this body's bound type variables" — it was written for, and only tested
against, the single case where the receiver is `inputs.last()`. Generalizing it requires
knowing the *member's declared shape* before the operand window can even be sized, which
today's call order doesn't have: `poly_trait_member_call` currently finds `var` first
(from the stack), then looks up which traits/members are bound to it and match `name`
(`poly.rs:865-880`). To search a non-trailing position, the order of operations has to
partially invert — either:

- **(a)** for each of this body's `Bound::User` traits, look up whether it declares a
  member named `name`, then use *that member's declared input list* to know how many
  operand slots to inspect and at which offset the bound variable is declared to sit, or
- **(b)** scan the operand window (sized somehow — from what, if not the member's own
  signature?) for any slot whose `PolyType` is one of this body's bound variables, then
  disambiguate against `name` the way `matched` already does.

(a) is the more principled read: a trait member's own declared signature already commits
to where its receiver sits, per-member, so grounding the search in that signature (name
lookup first, ahead of any stack read at all, mirroring how `env.get(name)` lookups for
ordinary dispatch elsewhere in `poly_call_term` are name-first) sidesteps needing to know
the window size before finding the member declaration. **Not yet recon'd or designed which
approach is right, nor whether (a) collides with the ordering S3e review finding 3 locked
(bound dispatch must run *ahead of* every name-based special case, `poly.rs:1008-1020`) —
a name-first lookup for trait members is exactly a name-based special case, so this needs
to be re-examined against that ordering rule, not assumed compatible with it.**

Two follow-on questions the design will need to answer once the search order is settled:

1. **Ambiguity when two different bound traits declare the same member name at different
   input positions.** `matched`'s existing ambiguity check (`poly.rs:906-914`,
   `ambiguous_trait_member_error`) assumes one shared position; a genuinely positional
   search may need to widen that check rather than reuse it unchanged.
2. **A zero-input member (`tag ( -- i64 )`) has no operand to find `var` from at all.**
   The call site carries no stack evidence of *which* concrete type's `tag` is meant —
   dispatch would need some other signal (an explicit type argument? a surrounding
   context binding?) that doesn't exist anywhere in the language today. This may be
   legitimately out of scope even after this slice lifts the positional restriction; the
   exit criteria below should be explicit about whether zero-input members are addressed
   or explicitly deferred again.

## Probe result (2026-08-23): approach (a) works, with one caught adjustment

A hand-patched build (`receiver_ty_var`'s call site replaced with a name-first,
position-aware lookup; the S3e declaration-time rejection temporarily disabled;
reverted after, `git diff` clean) confirmed approach (a) is viable and does **not**
collide with S3e review finding 3's ordering invariant:

- **Non-trailing receiver dispatches correctly.** `at ( &'T i64 -- i64 )` (receiver
  *first*, not last) on an `impl: Indexable for Pair` correctly returned `p.a`/`p.b`
  for index 0/1 through a bound-directed call in a generic body, with the ordinary
  `eq`/`if`/`bool` dispatch in the same bodies unaffected — no interception collision.
  `poly_trait_member_call` still runs first, unconditionally, exactly where S3e's review
  finding 3 put it; only *how* it finds the candidate variable changed (name-first: look
  up which of this body's bound traits declares a member named `name`, then use that
  member's own declared input list to find where its receiver sits, rather than reading
  `stack.last()` unconditionally). The ordering concern in the design section above does
  not materialize — the fix is internal to `poly_trait_member_call`, not a change to
  where it's called from.
- **Caught and fixed during the probe: a naive "exact structural match" candidate
  filter silently swallowed the operand-mismatch diagnostic.** The first probe cut
  required the stack slot at the receiver position to *structurally* match the member's
  declared receiver type (`Ref(Var(0))` vs `Ref(Var(0))`) to even select the candidate;
  passing a bare `'T` where `&'T` was declared then failed to match, so the call fell
  through to ordinary dispatch and reported `unknown word` instead of the located
  mismatch — regressing `trait_member_operand_mismatch_is_located` (confirmed by running
  the existing unit-test suite against the probe branch before fixing). Fix: unwrap
  through any `Ref` to find the *underlying* variable when selecting a candidate (ignore
  ref-vs-bare shape at that stage), and let the existing per-input structural check
  (unchanged, further down `poly_trait_member_call`) catch and report the actual
  ref/bare mismatch. After the fix, all 34 `--lib` trait-related tests passed, including
  that one. **This is the concrete shape of the risk the brief's ambiguity/diagnostic
  question named** — the spec phase must design the candidate-selection step to select
  loosely (by variable identity, not full type match) and defer precision to the existing
  downstream operand check, not the other way around.
- **Not probed:** the multi-position ambiguity case (two bound traits declaring the same
  member name at *different* input positions) and the zero-input-member case (`tag`).
  Both remain open per the brief above; the probe only exercised a single, unambiguous
  non-trailing-receiver member.

## Exit criteria (from the roadmap, unchanged)

- A trait member may declare its bound type variable at any input position, not only
  last, and a call to it dispatches correctly regardless of position.
- The S3e declaration-time rejection for a non-trailing `'T` (`member_ends_in_trait_var`,
  `non_trailing_receiver_error`) is lifted for the positions this slice supports.

## Sizing

Smaller than S3k in mechanism count, and the core design question (search order) is now
probe-validated rather than open. Recommend spec-writer:

1. Spec the name-first, loose-match candidate selection (see "Probe result" above) as the
   mechanism, not a restated open question.
2. Decide explicitly whether the zero-input-member case (`tag`) is in scope or is a
   named, separately-tracked deferral — don't let it fall out unaddressed.
3. Design the multi-position ambiguity check properly rather than assuming
   `ambiguous_trait_member_error`'s current single-position shape carries over unchanged.

## Ready to spec: yes, now that (a) is probe-validated

Approach (a) (name-first lookup, receiver position read from the matched member's own
declared signature) is confirmed compatible with the existing dispatch ordering (S3e review
finding 3) and is the design to spec — see "Probe result" above. One instruction for
spec-writer: design the candidate-selection step to match loosely (receiver-variable
identity only, ignoring ref-vs-bare shape) and let the existing per-input operand check
downstream catch and report a real type/shape mismatch — get this backwards and a
mismatched call silently degrades to "unknown word" instead of a located diagnostic, which
is exactly what the probe caught and had to fix. Verify every citation above against live
`main` before writing — `poly.rs`/`declarations.rs` are active files other in-flight slices
also touch.
