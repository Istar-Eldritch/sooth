# P7b.S6 brief — container traits and linear merge (recon round)

Scope input for the S6 spec. Produced by a recon round against the clean tree
(worktree `p7b-s6`, HEAD `600bc1b`): a read-only machinery map plus twelve live
probes under `/tmp/p7bs6-probes/` (verbatim log:
[slice6-probes.md](./slice6-probes.md)). Repo untouched throughout
(`git status --porcelain` empty at finish). Baseline `cargo test
--no-fail-fast` at HEAD is green.

S6's scope (from the [phase doc](../P7b-higher-kinded-types.md), line ~168)
is: `Functor`/`Bifunctor`/`Foldable` over the real `core::option`/`core::result`,
`Semigroup`/`Monoid` as linear merge, promoting `List['T]` into core, and an
array-as-`'F: * -> Len -> *` probe.

## What the round established

**Functor/Foldable/Bifunctor over the real lib types already ground — S4
unblocked exactly what it says it did.** `map` over real `Option[i64]` was
already an S4 golden (re-confirmed as a control, unchanged). `fold` (a new
member shape, single-argument accumulator threading) and `bimap` (a new
*two-argument* constructor kind, `Result['A 'B]`) both dispatch and produce
correct output on the first working fixture, once probe iteration found the
right stack-shape spelling. Nothing in the constructor-keyed dispatch or
member-grounding machinery needed to change for either.

**The linear spine already enforces single-consumption inside a fold-style
quotation body — there is no new checker work here.** Every attempt to
misuse the accumulator or the extracted payload (forget it, double-call it,
leave it unconsumed) was caught by pre-existing, general-purpose checks
(arm-shape parity on `Option?`, ordinary arity underflow on `call`, the
per-arm variant-consumption rule) — none of which are Foldable-specific.
`Semigroup.combine` (total consumption of two operands) rides the same rails
`Bifunctor`/`Functor` already exercise, so this round expects no new finding
there either (not separately probed as a standalone trait — the mechanism is
identical to `bimap`'s two-quotation consumption, already demonstrated).

**`Monoid.empty`'s return-type polymorphism is genuinely new — the existing
"grounds from context" precedent (`None`) does not transfer.** A zero-arity
*variant constructor* already grounds its type parameter purely from a
consuming declared type, with zero operands — but that mechanism is entirely
separate from trait dispatch (it never touches `resolve_mono_member_call`).
A trait member with the same *shape* (`empty ( -- 'T )`) is rejected outright,
with or without an explicit instantiation, because `resolve_mono_member_call`
structurally requires a dispatchable *input* operand to find any candidate at
all (`dispatchable_input_pos` returns `None` for a zero-input signature, so
`viable` is always empty). `Monoid`/`Semigroup` cannot be speced as "library
work over an existing mechanism" the way `Functor`/`Bifunctor`/`Foldable`
could; `empty` needs either a new dispatch entry point (grounding a trait
member from the *output* context alone) or a design ruling that `empty`
cannot be a bare trait member and must be spelled some other way (e.g. an
explicit-instantiation-only form, or piggybacking on the same variant-ctor
machinery `None` already uses, which would mean `Monoid`/`Semigroup` are not
ordinary traits at all).

**`List['T]`'s two probe questions both ground clean; no new machinery
needed for the exit criterion's core.** Self-reference (`^List['T]`) already
works under a generic declaration (this is the *same* mechanism S4's `L['T]`
golden already pinned, just renamed and un-cross-module'd). The "fused
iterative destructor drops per-instantiation payload" question resolves for
free: the destructor synthesizer already operates on concrete, already-
monomorphized `IrType::Struct`/`IrType::Enum` ids, so each `List[T]`
instantiation simply gets its own synthesized destructor the same way any
other recursive generic type would — there is no separate "does this apply
to `List` specifically" question, because the mechanism was never type-
specific to begin with.

**The array-as-constructor probe hits a structural wall, not a narrow kind
gap.** The `Kind` enum, the parser's `* -> Len -> *` spelling, and a trait
declaration using it all already work — S6a/S1 built exactly the kind
machinery this needs. But `impl:`-ing a trait *for* `array` has no path
forward: the sole "this target is a constructor" representation the member-
grounding code understands is `Type::CtorImage(GenericId, ..)`, minted only
from a user `type:`/`variant:` header; `array` is a distinct built-in `Type`
variant that is never wrapped as a `CtorImage` anywhere in the tree, and the
one impl-target parse path that gets structurally close (the named-array
reader, already used for non-HKT traits like `Show`) discards all kind
information regardless (`ty_kinds` is hardcoded to `Kind::Star` for every
impl target, unconditionally). This is a design-sized question — extend
`CtorImage` to cover builtin constructors, or find another mechanism
entirely — not a probe or spec-writing exercise.

| # | Finding | Probes |
| --- | --- | --- |
| F1 | **`Functor.map`/`Foldable.fold` over the real `core::option` both ground with no new mechanism.** `map` reconfirms the S4 golden; `fold` (a new member shape: accumulator-threading, single-argument dispatch) works on the first fixture that gets the stack-shape right. | p1-functor (control), p1-foldable |
| F2 | **`Bifunctor.bimap` over the real `core::result` grounds, including the two-argument constructor kind (`* -> * -> *`) previously untested over a real lib type.** Both branches (`Ok`/`Err`) dispatch their respective quotation correctly. | p2-bifunctor-ok, p2-bifunctor-err |
| F3 | **Linear single-consumption inside a fold-style quotation body needs no new S6 machinery.** Forgetting/double-using/leaving-unconsumed are each caught by a pre-existing, Foldable-agnostic check (arm-shape parity, arity underflow, variant-consumption rule). | p3-forget, p3-double |
| F4 | **`Monoid.empty`'s return-type polymorphism is structurally unreachable through the current mono-dispatch entry point.** `resolve_mono_member_call` requires a dispatchable input operand (`dispatchable_input_pos`, `poly.rs:1444`) to find any viable impl; a zero-input trait member always fails with `mono_member_no_dispatch_error`, even with an explicit type-argument list. | p4-monoid-empty |
| F5 | **The zero-arity-variant-ctor precedent (`None` grounds from a declared consumer type) does not transfer to trait members** — it is a wholly separate mechanism (variant construction, never touches trait dispatch) from what `Monoid.empty` would need. | p4-none-ctx |
| F6 | **`List['T]` self-reference and per-instantiation destructor synthesis both ground with no new work.** Self-reference is the same mechanism S4's `L['T]` golden already pinned; the fused destructor already operates per-concrete-instantiation because it is built from already-monomorphized IR types, never type-specific to `List`. | p5-selfref, p5-str-payload |
| F7 | **The array-as-constructor path is structurally absent, not merely under-sized.** The kind machinery (`* -> Len -> *`) already works at the trait-declaration level; `impl: ... for array[...]` fails because `array` has no `CtorImage` representation, and the impl-target parser that comes closest discards kind information (`ty_kinds` hardcoded to `Star`) regardless. | p6-trait-decl, p6-impl |

## Machinery map (verified anchors, this tree, HEAD `600bc1b`)

- **Real lib types.** `lib/core/option.sth` (`type: Option['T] | None | Some 'T ;`),
  `lib/core/result.sth` (`type: Result['T 'E] | Ok 'T | Err 'E ;`) — both
  single-line declarations plus exports, unchanged shape since before S4.
- **Constructor-keyed dispatch entry (mono, non-inline).**
  `resolve_mono_member_call` (`src/check/poly.rs:2165-2428`): candidate
  collection over the whole-program trait registry (no module filter),
  viability via `dispatchable_input_pos` (`poly.rs:1444-1450`) + `find_bound_impl`
  (`poly.rs:8235`), then a concrete-target branch or a generic-target branch
  (S4/S5 already documented this path in detail; unchanged this round).
- **The dispatchability gate `Monoid.empty` fails.** `dispatchable_input_pos`
  (`poly.rs:1444-1450`) scans `sig.inputs` for one whose head is the trait's
  own variable (`PolyType::Var(0)` or `App { head: 0, .. }`); a zero-input
  signature (`empty ( -- 'T )`) always returns `None`. The call site
  (`poly.rs:2225`) skips any member for which this is `None`, so `viable`
  stays empty and `mono_member_no_dispatch_error` (`poly.rs:2256`) fires
  unconditionally — the same error S5 traced for a structurally different
  reason (cross-module instantiation), here firing for lack-of-operand
  instead.
- **Zero-arity variant ctor grounding (the `None` precedent).**
  Documented at the standing memory note
  `project_zero_arity_variant_ctor_collides_across_monomorphs`; this round's
  `p4-none-ctx` probe confirms it is live and unrelated to trait dispatch —
  it never enters `resolve_mono_member_call` at all, since `None` is an
  ordinary variant constructor, not a trait member call.
- **Fused iterative destructor.** `src/ir/destructors.rs`: struct case
  (`synthesize_struct_destructor`, doc comment at `destructors.rs:298-299`,
  "disposed by one fused loop that walks the whole route"); enum case
  (doc comment at `destructors.rs:409`, "the whole destructor becomes one
  fused [loop]"). Both build from an already-concrete `IrType::Struct(id)`/
  `IrType::Enum(id)` — the synthesis is per-monomorphized-id by construction,
  with no type-specific carve-out for `List` or any other recursive generic.
- **`Kind` enum and the `* -> Len -> *` spelling.** `Kind::Arrow { domains,
  result }` (`src/ast.rs:552-556`); parser support at
  `parse_kind_expr`/`parse_kind_atom` (`src/parser.rs:2391-2429`); an existing
  unit test already exercises exactly this spelling
  (`src/parser.rs:13863-13870`, `header_bracket_vars("['F: * -> Len -> * 'T]")`).
  This machinery is orthogonal to the array-as-constructor wall below — it
  works.
- **The array-as-constructor wall.** `Type::CtorImage(GenericId, &'static str)`
  (`src/ast.rs:3115`) is the sole representation `ground_member_poly`'s App
  arm (around `ast.rs:2352`, raising `member_app_abstract_target_error` when
  the target isn't one) accepts as "a constructor"; minted only from
  `GenericId` (a user `type:`/`variant:` generic header index). `array` is
  `Type::Array(ArrayId, name)`, structurally distinct, never wrapped. The
  named-array impl-target reader (`parse_impl_target_named_array_parses`,
  `src/parser.rs:13796-13812`, already exercised for `Show`) parses
  `array['T 'N]` as a target pattern but `parse_impl_target`
  (`src/parser.rs:4071-4103`) unconditionally sets
  `ty_kinds: vec![Kind::Star; builder.ty_names.len()]` (`parser.rs:4097`) —
  the parsed target carries no real kind information regardless of which
  variables are type-kinded vs. `Len`-kinded.

## Open questions and scope recommendation

**Decided (2026-09-05, user interview), revised (2026-09-05, post spec-review
round 1) — Q1: S6 owns `empty` via explicit instantiation only.** The
original decision ("grounds from consuming context alone", not the narrower
explicit-instantiation option) is reversed after a 3-reviewer spec-review
round found the context-grounding tiers (Tier A/B as specced) cannot ground
the slice's own canonical shapes (`empty combine`, `mconcat`) without real
deferred-slot inference — a single-pass-checker redesign the spec itself
ruled out of budget. Revised scope: `empty[i64]`-style explicit
instantiation dispatches from a mono body (requires carving the existing
concrete-target `no_type_arguments_error` guard to admit a zero-dispatchable-
input member, since an explicit argument is meaningful there unlike the case
the guard was written for); bare `empty` with no explicit instantiation is a
located error. Deferred-slot context inference is recorded as an explicit
future follow-on, not silently dropped. See Q1 below for detail.

**Decided (2026-09-05, user interview) — Q2: S6 owns the `CtorImage`
widening.** `array` becoming a valid `impl:` target (with real kind info
threaded through, not the hardcoded `Star`) is in scope this slice, not
carved out to a future one; see Q2 below for the audit this commits to.

**Q1 — does S6 own `Monoid`/`Semigroup.empty`'s new dispatch entry point, or
is `empty` out of scope this slice?** F4/F5 show this is not library work
riding an existing mechanism the way `Functor`/`Bifunctor`/`Foldable` are —
it needs either (a) a new mono-dispatch path that grounds a trait member from
consuming context alone (no operand), sized against the existing
`resolve_mono_member_call`/`dispatchable_input_pos` machinery, or (b) a
ruling that `Monoid.empty` is expressed some other way (explicit
instantiation only, e.g. `empty[i64]` made to work by widening the
*explicit-type-args* path specifically rather than the operand-driven one;
this round did not probe whether that narrower fix is easier — `empty[i64]`
was rejected by the same zero-viable-candidates gate before the
explicit-args path was ever reached, so it is untested whether widening
just the explicit-instantiation case is a smaller, sufficient fix). `combine`
needs no such new machinery (it has a dispatchable operand, same shape as
`bimap`'s two-operand consumption); only `empty` is blocked. Recommend the
spec size (a) vs (b) explicitly, since S6's own text ties `empty` and
`combine` together as one trait pair but only one half has a wall.

**Q2 — does S6 own the array-as-constructor ruling, or does it defer to a
later slice?** (Decided: yes, S6 owns it — see above.) F7 shows this is not a probe-sized question — it is "does
`Type::CtorImage` grow a builtin-constructor variant" or "is there a wholly
separate desugar path for builtin-type trait impls," either of which is a
design decision with its own blast radius (every place that pattern-matches
`Type::CtorImage` today assumes a `GenericId`; widening that assumption needs
its own audit). The phase doc frames this as "either it grounds and arrays
become Functor/Foldable instances, or the kind story gets a ruling" — this
round's finding is the *second* branch: the kind story itself (the `Kind`
enum, the parser) is fine, but the *target representation* story needs a
ruling that is independent of kinds. Recommend the spec record this as a
ruling ("array does not get Functor/Foldable this slice; `CtorImage`
widening is carved out to a future slice") rather than attempting to close
it in S6's implementation budget, unless the spec explicitly wants to size
the `CtorImage` widening as its own phase.

**Q3 — scope fence: `Semigroup`/`Monoid` for which concrete types?** The
phase doc names "StrBuf concat, list append, numeric add" as `combine`
examples. This round did not probe `combine` standalone (its consumption
shape is provably identical to `bimap`'s two-operand pattern, already
demonstrated grounding), but did not probe *StrBuf* or *list-append*
specifically — only `i64` (implicitly, via the blocked `empty` fixture) and
`Result`/`Option` (via `bimap`/`fold`). Recommend the spec's goldens
enumerate the concrete `combine` instances it wants (numeric add is trivial;
StrBuf concat and list append both need the impl body to actually consume
and merge a heap-owning value, not just re-tag a variant) as a checklist,
since this round's evidence for `combine` is by analogy, not direct
measurement.

**Q4 — `List['T]`'s exit-criterion destructor claim needs no further probe,
but the round did not measure it under S6's actual expected shape (a shared
`Foldable`/`Functor` bound over `List`, not just raw `Cons`/`Nil`).** F6
grounds the self-reference and the destructor separately; this round did not
write a `Functor for List` or `Foldable for List` impl (only `Option`/
`Result` were probed for those traits) — recommend the spec's own dogfood
golden be the first place this combination is actually built, since a
`List`-specific eliminator pattern (walking `Cons`/`Nil` recursively inside
an impl body, rather than a flat `Option?`-style two-arm dispatch) was not
exercised this round and could still surface a shape this round's simpler
fixtures did not.
