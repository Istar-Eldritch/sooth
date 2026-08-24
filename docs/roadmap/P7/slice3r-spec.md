## Rulings

- **R1, one spelling.** The binding form's parse path was deleted, not deprecated. Withdrawn
  with it: "one word satisfies two traits' members" (no in-tree consumer; a forwarding body
  covers a future need) and binding a member to a pre-existing operator-named word.
- **R2, signature inheritance.** The synthesized effect is the member signature with the
  trait's type variable substituted by the `for` type, grounded to a concrete `StackEffect`
  (`poly: None`), byte-identical to what `check_impl_decls` computed. Restating a signature in
  a body is rejected. `ground_member_type` moved from `check` to `src/ast.rs` (the lowest
  common ancestor of `parser` and `check`), behaviour unchanged.
- **R3, readable rendering.** A synthesized name must never leak verbatim into a diagnostic;
  it renders as `` `cmp` (member of trait `Order` for `Point`) ``.
- **R4a, self-recursion.** While desugaring, the parser rewrites every call token equal to the
  member's own name (nested quotations included) to the synthesized name. A `| cmp |` binder
  inside `cmp`'s own body is a located error, since the rewrite is unconditional token equality
  and silent shadowing is refused.
- **R4, builtin-spelled members rejected at `parse_trait_decl`**, not at the impl body (the two
  sites would be mutually exclusive, and an unreachable guard is a placebo). Three categories,
  three messages: a name-dispatched builtin (`is_name_dispatched_builtin`, which claims the
  `>`-prefixed conversions the `BUILTIN_WORDS` const omits) gets new text; `@`/`!`/`+!` reuses
  `shadowed_access_word_error`; caret and reserved-ref names reuse `reject_reserved_name`. The
  six surface comparisons (`eq`/`lt`/`gt`/`lte`/`gte`/`ne`) stay legal: they are `lib/` words,
  so an `eq` member shadows a user word, which the construct-scoped exception already admits.
  Rejecting on the raw const would have killed the `Eq` trait and the planned `Map` consumer.
  `BUILTIN_WORDS` and both predicates were elevated to `src/ast.rs` so `parser` reaches them
  without depending on `check`.
- **R5**, the trailing `; ;` is accepted as-is.
- **R6**, a non-member body (`: bogus ... ;`) is a located parse error, carrying the retired
  check-time `unknown_member` guard's intent.
- **R7**, no sibling-member access. A sibling call resolves by ordinary lookup and silently
  binds a same-named library word; pinned by `impl_body_sibling_call_does_not_reach_the_sibling`.
- **R8**, `impl:` / `trait:` get located REPL rejections, mirroring `export:` / `global:`.

## The synthesized name

`member;Trait;trait-module;Type`, e.g. `cmp;Order;0;Point`. Unforgeable: `;` is a hard lexer
delimiter, so no new name-rejection rule was needed. `qbe_name` escapes it to `.3b.`
injectively at definition and call site.

The trait component carries `TraitDecl::module` as well as the name: two same-named traits from
different modules may both be implemented for one type in one module, and the bare name would
collide. The `Type` component is only the rendered type name, so two same-named types from
different modules are separated by the ordinary overload-suffix path instead; that works because
a member must take `'T`/`&'T` as *some* input (`member_binds_trait_var`), so every grounded
signature mentions the `for` type. P7.S3p relaxed that rule from the last input to any input,
which leaves the guarantee intact; only admitting a member that binds `'T` in no input at all
(P7.S3t) would make the `Type` component need its own module id.

## Deleted, and where the intent went

Five `check_impl_decls` guards went vacuous under inheritance and were deleted with the binding
form rather than migrated (a test that can no longer fail is a placebo defect):
`signature_mismatch` (**relocated**: a wrong body now fails ordinary in-body stack-effect
checking, located in the body, naming the readable member), `does_not_bind_a_word_from_another_module`,
`polymorphic_member`, `polymorphic_member_with_a_zero_slot_member`, `drop_overload_member` (all
four gone with the feature: there is no separate word to bind, and a body is concrete by
construction). Also deleted: the odd-binding-token parse error, the check-time `unknown_member`
guard (intent re-pinned at parse time per R6), and `a_resolved_trait_call_carries_the_overloaded_members_suffixed_symbol`
(a synthesized name is never `$$N`-suffixed, so the scenario cannot recur).

R15's `trait_calls` conjunct in `uncalled_operator_overloads` (`src/ir/driver.rs`) existed only
for the binding form, where a bound operator-named word never appeared as a literal `Call`.
Every body-form member is a real `Call`, so `called` covers it; mutation-tested green at Phase
3's HEAD and deleted in Phase 4.

Guards that stayed (their subject is the trait decl or the registry, so only their fixtures'
syntax migrated): duplicate `(Trait, Type)`, missing member, the orphan rule, static/trait
collision, export/selective-import, and the P7.S3p receiver rules.

## Phases

1. **Body-form parse + desugar** (hard), coexisting with the binding form. New rejections:
   restated signature, non-member body, self-shadowing binder, and the three R4 categories at
   `parse_trait_decl`. Two in-tree casualties took opposite fixes: `tests/phase7_slice3e.rs`'s
   `tag` member renamed; `src/check/poly.rs`'s `bound_dispatch_and_a_builtin_named_member_coexist`
   deleted, since its subject was the operator-spelled member itself and its successor (member
   `eq`) carries the surviving half. Goldens: `impl_body_form_builds_and_runs` (asserts the
   linked symbols via `nm`), `impl_body_restated_signature_is_rejected`,
   `impl_body_non_member_is_rejected`, `impl_body_binder_named_after_the_member_is_rejected`,
   `trait_member_named_after_a_builtin_is_rejected` (each category asserting its own message),
   `trait_member_named_after_a_comparison_is_accepted` (the `Eq`-trait regression guard),
   `impl_body_member_calls_itself_recursively`,
   `impl_body_trait_qualifier_disambiguates_shared_member_name`,
   `impl_body_disambiguates_same_named_traits_from_two_modules` (the only test that discriminates
   the module id), and the two unterminated-block diagnostics
   (`..._at_eof_is_error` / `..._absorbs_next_decl`: an unterminated block either hits the EOF
   error or absorbs the following declaration as an attempted member, and nothing is silently
   swallowed either way).
2. **Readable rendering (R3).** `render_word` / `render_call` in `src/resolve.rs` are
   display-only siblings of `demangle_word` / `demangle_call`, which stay bare because
   eliminator- and operator-name equality tests run through them. The four-component split drops
   the module id. The rendering carries its own backticks, so templates interpolate it bare;
   the ~100 in-body diagnostics that read the enclosing word out of `Ctx` go through
   `Ctx::rendered_word_or`. R3 is not one message, so one golden per rendering route:
   `impl_body_wrong_effect_names_readable_member`, `..._wrong_effect_type_...`, `..._underflow_...`,
   `..._unknown_word_...`, and `..._ungated_intrinsic_...` (the accessor path the others miss).
3. **Migration.** `examples/traits.sth`'s two `impl:` lines fold into blocks (`point-cmp` /
   `point-show` and their restated signatures vanish), plus the Phase-3-classified fixtures across
   six files. The two operator-named fixtures took different migrations, which is load-bearing:
   `src/ir/driver.rs`'s two-input case became a forwarding body `: get | a b | a b max ;`
   (assertion now pins overload resolution); `tests/phase7_slice3e.rs`'s one-input case could not,
   since a one-input word named `max` is unreachable by name, so its body was inlined and the
   `: max` word deleted.
4. **Delete the binding form**, its parse branch, the vacuous guards, and the Phase-4-classified
   fixtures. Replacement golden `impl_body_wrong_effect_is_rejected_in_body` asserts the rejection
   is located inside the body (Phase 2's golden asserts the rendering).
5. **REPL rejections.**

```text
error: `impl:` has no meaning at the REPL (line L, col C)
  note: a live session has no module to attach a trait implementation to

error: `trait:` has no meaning at the REPL (line L, col C)
  note: a live session declares no trait to satisfy
```

Growth-structure re-check at Phase 1 and Phase 4 exit: `parser.rs` grew past 8000 lines but the
new code sits beside the declaration parsers it extends and no split signals fire together. Both
new cross-module needs (`ground_member_type`; the `BUILTIN_WORDS` family) were resolved by
elevating to `ast`, so `parser` gains no non-test dependency on `check`.

## Out of scope

- General / builtin shadowing beyond R4's construct-scoped rejection.
- Any change to bound-directed dispatch, the impl registry, monomorphization, or lowering.
- P7.S3o and P7.S3n.

## Phases (machine-readable)

```json
{
  "phases": [
    { "phase": 1, "focus": "body-form impl parse and desugar coexisting with binding form", "difficulty": "hard" },
    { "phase": 2, "focus": "readable rendering of synthesized impl member names in diagnostics", "difficulty": "standard" },
    { "phase": 3, "focus": "migrate sth sources and mechanical fixtures to the body form", "difficulty": "standard" },
    { "phase": 4, "focus": "delete the binding form and its vacuous check guards", "difficulty": "standard" },
    { "phase": 5, "focus": "repl located rejection for impl and trait declarations", "difficulty": "standard" }
  ]
}
```
