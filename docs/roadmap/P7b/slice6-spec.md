# P7b.S6 spec — container traits, linear merge, and `List['T]` in core

Scoped against worktree `p7b-s6`, HEAD `600bc1b`, baseline `cargo test
--no-fail-fast` green. Discovery input: the recon
[brief](./slice6-brief.md) (findings F1–F7, open questions Q1–Q4) and its verbatim
[probe log](./slice6-probes.md), plus **five spec-time measurements (M1–M5)** taken
while writing this document and re-verified line by line against source at HEAD for
this revision, recorded in full below. Two of those measurements are ICEs the recon
round never reached, and both sit directly on S6's exit criterion.

This is a **rewrite** after a three-reviewer spec-review round found the previous
draft's core diagnoses wrong in several load-bearing places. Every anchor below was
re-read against the actual source for this pass, not inherited from the review
reports; where the reviewers' own line numbers had drifted (the file has moved since
they wrote), the corrected HEAD anchors are used. The specific corrections folded in:

- The M3 unifier already has a `Quotation`/`Quotation` arm; the real defect is
  narrower (a cross-representation gap when the caller's quotation param has a
  *concrete* effect). The headline generic-effect forwarding case **already works at
  HEAD**.
- Mono `empty` is explicit-instantiation-only for this slice (Q1's post-review
  revision); there is no consuming-context lookahead tier. The S2-16 concrete-target
  guard must be carved for a zero-dispatchable-input member.
- The `List['T]` impl ICE has **two** twinned `unreachable!` arms (destructure and
  construction), both needing the same `OwnedCell(Generic)` arm.
- The `CtorImage` widening for array must also span `PolyType::Generic`'s identity
  triple and `GenericId`, because grounding reads `PolyType::Generic`, not
  `CtorImage`.
- List trait impls carry **no** recursion wall (a follow-up probe settled this); they
  are simply non-inline, the natural default.

The roadmap's S6 framing
([P7b-higher-kinded-types.md](../P7b-higher-kinded-types.md) line ~168) calls this
"the tier-1 library slice, unblocked by S4". **That framing is wrong, and this spec
says so up front.** The recon round tested every member shape from a *monomorphic*
call site, where everything works. The exit criterion asks for dispatch *through
shared bounds* — a polymorphic body — and calling a quotation-taking member from one
**panics the compiler** in a narrow but on-path case (M3). Separately, giving core's
`List['T]` any trait impl at all panics the compiler (M4). S6 is a compiler slice
with a library payload, not a library slice.

Two decisions arrived settled from the user interviews (2026-09-05, and a post-review
follow-up the same day) and are treated here as requirements, not open questions:

- **Q1 — mono `empty` via explicit instantiation only.** `empty[i64]`-style
  dispatch from a mono body is in scope; bare `empty` with no instantiation is a
  located error, full stop. Consuming-context grounding is an explicit *future*
  follow-on slice, not attempted-and-degraded here. Ruled in R4/R5.
- **Q2 — S6 owns the array-as-constructor widening**, last and fenced. Ruled in R8;
  R8.4's ordering and budget fence are **settled fact**, not an open invitation to
  re-decide.

## Spec-time measurements (M1–M5)

All five ran against the built binary at HEAD with the probe round's manifest
(`/tmp/p7bs6-probes/sooth.pkg`); `src/` was never edited. Trait members are spelled
`name ( … ) ;` for the first member and `: name ( … ) ;` thereafter (the `trait: T :`
header consumes the first `:`).

### M1 — `empty` already grounds in a *poly* body; only the mono site is blocked

```sth
trait: Monoid['T] :
  empty ( -- 'T ) ;
  : combine ( 'T 'T -- 'T ) ;
;
impl: Monoid for i64
  : empty 5 ;
  : combine add ;
;
: mzero['T: Monoid] ( 'T -- 'T ) empty combine ;
: main ( -- ) 7 mzero . ;      \ prints 12
```

`empty` dispatches, off the caller's `'T: Monoid` bound, with zero operands. This is
P7.S3t's documented mechanism, whose rationale is written into
`member_binds_trait_var`'s doc comment (`src/check/declarations.rs:373`): a nullary
member is admitted at declaration time precisely because *the wrapping generic word*
can supply `'T`. The impl body returning `5` rather than `0` proves the dispatch is
real (`7 + 5 = 12`).

**This corrects F4/F5.** `Monoid.empty` is not "structurally unreachable" — it is
reachable through the bound path and unreachable through the mono path
(`resolve_mono_member_call`, `src/check/poly.rs:2165`).

### M2 — `combine` needs nothing

`: main ( -- ) 7 5 combine . ;` prints `12` from a monomorphic body. The brief's
argument-by-analogy (F3, Q3) holds by direct measurement for the numeric case.

### M3 — a concrete-effect quotation operand at an HKT member ICEs; a generic-effect one already works

This is the headline finding and it is not in the brief. **It is narrower than the
previous draft claimed**, corrected here against source.

What already works at HEAD (re-verified): a forwarded quotation **parameter** whose
effect is written with the member's own type variables (a *generic* effect) unifies
cleanly through `unify_member_operand`'s existing `Quotation`/`Quotation` arm
(`src/check/poly.rs:1370-1385`) and runs:

```sth
: bump['F: Functor 'A] ( 'F['A] [ 'A -- 'A ] -- 'F['A] ) map ;
: main ( -- ) 3 Some [ 1 sub ] bump showopt ;   \ prints 2 at HEAD
```

What panics: the same forwarded parameter when its effect is written **concrete**
(`[ i64 -- i64 ]` rather than `[ 'A -- 'A ]`). The operand slot folds to
`PolyType::Concrete(Type::Quotation(..))`, while the *declared* member input stays a
`PolyType::Quotation(..)`. That pair matches neither the `Quotation`/`Quotation` arm
(the found side is `Concrete`, not `Quotation`) nor any other arm, so it falls
through `unify_member_operand`'s catch-all `_ => declared == found`
(`src/check/poly.rs:1397`), reports a mismatch, and the diagnostic render then
panics:

```text
thread 'main' panicked at src/check/poly.rs (poly_type_str, ~11617):
index out of bounds: the len is 1 but the index is 1
```

Diagnosed to two distinct defects on the bound-dispatch path
(`poly_trait_member_call`, the resolver around `src/check/poly.rs:2649-2700`):

1. **The real rejection — a cross-representation gap.** `unify_member_operand` has
   no arm bridging a declared `PolyType::Quotation(..)` against a found
   `PolyType::Concrete(Type::Quotation | Type::InlineQuotation | Type::OwningQuotation)`.
   When the operand's effect is fully concrete it arrives folded to `Concrete`, and
   the structural row unification never runs. Its second call site
   (`src/check/poly.rs:1271`, the `.all(..)` viability filter) drops the candidate on
   the same failure and is part of the same fix.
2. **The ICE.** The failure path calls `trait_member_operand_error`
   (`src/check/poly.rs:10806`), rendering
   `substitute_member_var(declared, var)` with `poly_type_str(…, sig)` where `sig` is
   the **caller's** `PolySig` (the call site at `src/check/poly.rs:2680-2686`).
   `substitute_member_var` (`src/check/poly.rs:1464`) rewrites *every* bare `Var` to
   the caller's index and head-0 App heads to the caller's var, but leaves a
   `Quotation`'s interior vars untouched (`other => other.clone()`); a member-local
   `Var(1)` (`map`'s `'U`, in the member's own table) then indexes off the end of the
   caller's `ty_var_names`. A diagnostic-rendering bug sitting on top of a real
   rejection.

M3's three variants are **not** identical — the previous draft's "three variants, all
panic" claim was wrong:

- a **concrete-effect quotation parameter** (above) — **panics**; this is the real
  bug Phase 2 fixes.
- a **generic-effect quotation parameter** — **already works** (prints 2 at HEAD), as
  shown above.
- a written **quotation literal** (`[ 1 sub ] map`) — already produces a **clean
  located error** today (the `QuotLit` marker carries no effect to unify and lands at
  Star kind without a panic), e.g. in the `mconcat` shape:
  ``error: `fold` of `Foldable` in `mconcat` … expects `[ 'T 'T -- 'T ]`, found `a
  quotation literal` in operand slot 2``. This needs **no Phase 2 work**; it is
  already acceptable behaviour.

Consequence: the S6 exit criterion's shared-bound map/fold is reachable at HEAD for
the generic-effect spelling and panics only for the concrete-effect one; Phase 1
removes the panic (rendering), Phase 2 removes the rejection (the bridge arm).

### M4 — `impl: <trait> for List` ICEs on the self-reference

```sth
type: List['T] | Nil | Cons 'T rest ^List['T] ;
impl: Foldable for List : fold … ;
```

Eliminating the variant panics in the destructure walk:

```text
thread 'main' panicked at src/ast.rs:2708:
internal error: entered unreachable code: a generic enum variant field is never
OwnedCell(Generic { .. }): `array['A N]`, `Inner['A]`, `&'A` and `^'A` are all parser
rejections for a variant field
```

The `unreachable!`'s own premise is false: `^List['T]` **is** a parser-accepted
variant field (the recon round's `p5-selfref` probe builds and runs it), so a generic
enum variant field can be `OwnedCell(Generic { is_enum: true, .. })`. This ICE has a
**twin on the construction side**: `poly_bind_construction_arg`
(`src/check/poly.rs:5769`) has the identical `other => unreachable!("a generic`type:`field is never {other:?}")` catch-all (`src/check/poly.rs:5868`), with the
same missing `OwnedCell(Generic)` arm; constructing `Cons 'T rest ^List['T]` reaches
it. The doc on `substitute_generic_variant_field` (`src/ast.rs:2690`, doc at
`:2684-2707`) explicitly calls the pairing out ("a field shape one accepts and the
other rejects is a defect"), so both arms must be fixed together. F6 measured
`List['T]` *without* any impl; the wall is the combination.

### M5 — the array-as-constructor gap is about twice the brief's size

Beyond F7's two items (`array` has no `CtorImage`; `parse_impl_target` hardcodes
`ty_kinds: vec![Kind::Star; …]` at `src/parser.rs:4097`), a third and larger one:

**`PolyType::App { head: u32, args: Vec<PolyType> }` (`src/ast.rs:2589`) carries no
`len_args`** — unlike its sibling `PolyType::Generic`, which has carried
`len_args: Vec<Len>` since P7.S6a (`src/ast.rs:2610`). So in `'F['T 'N]` under
`'F: * -> Len -> *`, `'N` is parsed as a **type** variable:

```text
error: variable `'N` … is used as both a type variable and a length variable;
these are two different variables
```

Kind **arity** is checked; per-argument kind is not, because there is no
representation in which it could differ. The recon round's `p6-trait-decl` "grounds"
verdict is a false positive: the trait declaration parses, but its `Len` domain is
decorative.

A further correction the brief and previous draft both missed:
`ground_member_poly`'s App arm (`src/ast.rs:2388+`) destructures its **target** as
`PolyType::Generic { is_enum, idx, module, args, len_args, name }` directly and
raises `member_app_abstract_target_error` if the target is not one. It does **not**
read `Type::CtorImage` — `CtorImage` is used only as the App-head *binding* on the
construction/unification side (`poly_bind_construction_arg`, `src/check/poly.rs:5847`,
mints `Type::CtorImage`). So making `array` a groundable impl target spans
`PolyType::Generic`'s identity triple `(is_enum, idx, module)` and `GenericId` (whose
doc, `src/ast.rs:2516`, says it mirrors that triple) **as well as** `CtorImage`. This
enlarges Phase 6, which is why R8.4's fence stays.

## Rulings

### R1 — the exit criterion is amended, and the amendment is the slice (M3, M4)

The roadmap's S6 exit text stands as the goal, but the phase doc's *characterisation*
of S6 as a library slice is edited to current state (no history, per
[[feedback_roadmap_design_no_history]]): S6's substance is three compiler fixes
(M3's cross-representation unifier arm + its diagnostic, M4's twinned variant-field
arms, R8's constructor widening) with the library and goldens riding on top. Phase 5
owns the roadmap edit.

### R2 — quotation-taking members callable from a poly body: fix the render, then bridge the representation gap (M3; Phases 1–2)

**R2.a — the diagnostic first (Phase 1).** `trait_member_operand_error`
(`src/check/poly.rs:10806`) currently takes one `sig` used to render both `expected`
and `found`. Split it into two sigs — `expected_sig` and `found_sig` — and render by
provenance:

- At the erroring bound-dispatch site (`src/check/poly.rs:2680`), pass the *raw*
  member input `declared` (which is entirely in the member's own variable space,
  taken straight from `member_decl.sig.inputs`) as `expected` with
  `&member_decl.sig` as `expected_sig`, and the caller-space slot as `found` with the
  caller `sig` as `found_sig`. Drop the `substitute_member_var(declared, var)` call
  entirely. Because `declared` is pure member-space and `found` is pure caller-space,
  there is no mixed-provenance term to mis-render, and no index can run off either
  table: this sidesteps the reviewer's two-caller-var counter-example rather than
  papering over it.
- The two other call sites (`src/check/poly.rs:1837`, `:2371`) pass `Concrete`
  expected/found (a monomorphic mismatch), so they pass the same sig for both
  params; record a per-site verdict confirming this in the phase.

Land this first so Phase 2 debugs against an error message, not a panic. Regression:
a `#[test]` asserting the M3 concrete-effect message text, plus a unit test beside
`poly_type_str` proving a member-local `Var(n)` with `n >= caller ty_var_names.len()`
renders (against the member sig) rather than panics.

**R2.b — the cross-representation bridge (Phase 2).** `unify_member_operand`
(`src/check/poly.rs:1335`) gains **one new arm** — not a `Quotation` arm, which
already exists (`:1370-1385`). The new arm bridges a declared
`PolyType::Quotation(dins, douts, dinline, ..)` against a found
`PolyType::Concrete(Type::Quotation(eff) | Type::InlineQuotation(eff) |
Type::OwningQuotation(eff))`: reconstruct the concrete quotation's own input/output
rows from its effect and unify them structurally against `dins`/`douts`, binding
member-local variables into `bindings` exactly as the existing arm does, and honour
the inline/owning flavour match. Both call sites are covered: the erroring one
(`:2679`) and the silent viability filter (`:1271`).

Rulings on the two non-parameter operand shapes:

- A **written quotation literal** (`QuotLit`) already gets a clean located error at
  HEAD (M3); it is **out of scope** for Phase 2. No materialization at a poly member
  call site is attempted this slice — that is the mono path's behaviour and importing
  it is deferred, consistent with
  [[project_no_loop_combinator_in_a_poly_body]] and
  [[project_splice_route_ignores_quotation_flavour]]. A located rejection is the
  spec'd outcome for a literal, not a defect.
- A **generic-effect quotation parameter** already works (M3); it gets a
  non-regression golden, not new code.

Non-goal, explicitly: nothing here relaxes "a quotation in a generic body is spliced
where it is written" for *combinators*. The change is confined to trait-member
operand unification.

**Mutation check (R2.b).** The `Quotation`/`Quotation` arm predates this slice, so
"revert the arm, confirm goldens fail" is not executable against it. The mutation
check targets the **new** cross-representation bridge arm specifically: revert only
that arm and confirm the concrete-effect golden regresses (to the Phase-1 located
error, no longer a value), per [[workflow_mutation_test_the_guards]].

### R3 — `List['T]` gets a trait impl: fix both twinned arms (M4; Phase 3)

Both `unreachable!` catch-alls gain an `OwnedCell(payload)` arm, by **plain
substitution**, matching `substitute_generic_variant_field`'s contract (`src/ast.rs`,
doc at `:2698-2707`: it interns nothing and does no grounding — grounding is
`apply_subst`'s job at Phase 3). The wording is "recurse into the cell's payload and
re-wrap the `OwnedCell`", *not* "ground the inner Generic":

- **Destructure side** (`src/ast.rs:2708`): add `PolyType::OwnedCell(payload) =>
  PolyType::OwnedCell(Box::new(substitute_generic_variant_field(payload, args)))`,
  and correct the false-premise doc.
- **Construction side** (`src/check/poly.rs:5868`, `poly_bind_construction_arg`): add
  the analogous `OwnedCell` arm binding through the payload, so building
  `Cons 'T rest ^List['T]` no longer hits the catch-all.

Both are load-bearing for `impl: Foldable for List` (destructure) and for building a
`List` value at all (construction); neither is scope-fenced away.

Per [[workflow_twinned_guard_only_one_half_tested]], the struct twin
(`substitute_generic_field`, which already carries a wider arm set) is checked: if a
generic *struct* field can reach an equivalent gap via `^Self['T]`, it gets its own
fixture in the phase, or a recorded "structurally unreachable" verdict with the parse
error that proves it.

**No recursion wall.** A follow-up probe settled that a **non-inline** trait member
over `List['T]` with ordinary recursive self-calls (tail or non-tail) mints a real
`IrFunc` and lowers as an ordinary `Call` — it runs today. Only `inline` members are
combinators subject to splice-budget limits, and non-tail self-recursion in those is
cleanly rejected at *check* time (`check_combinator_cycles`,
`src/check/combinators.rs`) before lowering. So List trait impls in this slice are
simply **non-inline** (the natural default shape for these members), and this carries
no special risk. The previous draft's "unmeasured recursion wall" framing is dropped
entirely.

### R4 — mono `empty`: explicit instantiation, carving the S2-16 guard (Q1; Phase 4)

Q1 was revised post-review to **explicit-instantiation-only** for this slice.
`resolve_mono_member_call` (`src/check/poly.rs:2165`) gains a **zero-dispatchable-input
branch**: when the ordinary `viable` loop finds no candidate (every candidate's
`dispatchable_input_pos` returned `None`, so all were `continue`d at `:2226`) *and*
the call carries an explicit `type_args` list *and* exactly one candidate has no
dispatchable input, ground that candidate's trait variable from `type_args`, run
`find_bound_impl` on the resulting concrete type exactly as the operand path does, and
proceed into the existing concrete/generic branches.

**The S2-16 guard carve-out (P0-B).** The concrete branch's guard at
`src/check/poly.rs:2303` rejects *any* explicit type/length list against a concrete
target ("a concrete target's member sig has no free variables to bind, so an explicit
list is provably meaningless"). For a **zero-dispatchable-input** member the explicit
list is the *only* way to ground `'T` and is therefore meaningful — the exact case the
guard's rationale does not cover. Carve a narrow exception: the guard fires unless the
member has no dispatchable input.

This carve-out is **disjoint from the guard's pinning test**
(`mono_concrete_member_call_with_explicit_type_args_is_error`,
`src/check/poly.rs:12071`), verified against source: that test's member is
`size ( 'F -- i64 )`, which *has* a dispatchable input, so the exception never
touches it and no ruling to amend an S2-16-pinned test is needed. The sibling
collision test (`mono_word_colliding_with_member_name_rejects_explicit_type_args`,
`:12089`) rides the env route and is likewise untouched. Both must still pass
unchanged.

Scope fence: the new branch fires **only** when no candidate has a dispatchable input;
an operand-carrying member keeps today's behaviour byte-for-byte.

### R5 — bare `empty` in a mono body is a located error, full stop (Q1; Phase 4)

Per Q1's revision, there is **no** consuming-context tier and **no** lookahead
machinery. Bare `empty` with no explicit instantiation in a mono body is a located
error naming the remedy (`` write `empty[i64]` ``). Context-alone grounding
(deferred-slot inference) is an explicit **future follow-on slice**, recorded as
deferred, not attempted-and-degraded here.

Explicitly rejected alternatives, with the reason (kept because they are the shapes a
future slice must weigh, and rejecting them now prevents a silent-unsound shortcut):

- **Sole-impl grounding** ("only one `impl: Monoid` exists, so use it") — silently
  correct today and silently wrong the moment a second impl lands
  ([[feedback_magicless_beats_measured_convenience]]).
- **Mirroring the nullary-variant-ctor path** (`None`'s "grounds from context") —
  ruled out on **separate-mechanism** grounds: it is variant construction, never
  entering `resolve_mono_member_call` at all, so there is nothing to reuse here. (The
  standing note [[project_zero_arity_variant_ctor_collides_across_monomorphs]] adds
  that the path is also a first-minted-monomorph pick and would import that bug if
  copied; this spec does not rest the rejection on that citation alone, since the
  separate-mechanism ground is sufficient and independently verifiable — `None` never
  reaches `resolve_mono_member_call`.)
- **Full deferred-slot inference** — correct, and out of a slice's budget: the mono
  checker is a single forward pass with no unification variables on the stack. This is
  the future follow-on's job.

### R6 — the library surface, enumerated (Q3; Phase 5)

`combine`/`empty` impls, as a checklist, each with a golden.

| Instance | Shape | Risk |
| --- | --- | --- |
| `Monoid for i64` | `combine` = `add`, `empty` = `0`/`5` | measured (M1/M2) |
| `Monoid for List['T]` | list append, consumes both spines | **unmeasured**; depends on R3 |

The `Semigroup for str`/StrBuf row from the previous draft is **dropped**, premise
corrected: `lib/core/show.sth`'s `StrBuf` is `type: StrBuf data array[u8 64] len
usize ;` — a fixed 64-byte inline buffer, so `combine` over it is a by-value struct
merge with a capacity-overflow failure mode, **not** "the first real linear merge" it
was billed as. Separately, a `Semigroup for str` doing real string concatenation needs
allocation, which conflicts with `core`'s `no_std` layering (CLAUDE.md) if placed in
the `core` module R7 targets. The real linear-merge witness this slice keeps is
`Monoid for List['T]` (heap-owning spines, total consumption). If it does not ground,
the phase records the wall with its exact error text and drops the instance from the
goldens rather than papering over it.

`mconcat` is a Phase 5 deliverable (see R6a) and depends on R2's bridge arm; the
two-bound bracket spelling is `['F: Foldable 'T: Monoid]` (comma-separated is a parse
error).

### R6a — `mconcat` takes the merge quotation as a bound parameter (P0-2; Phase 5)

The previous draft's `mconcat` spelling
(`… ( 'F['T] -- 'T ) empty [ combine ] fold ;`) passes a written quotation **literal**
to `fold`, which per M3 gets a clean located rejection, not materialization — and its
signature has no quotation parameter to bind the literal to, so R5's remedy does not
apply. **Ruling: give `mconcat` a bound-quotation-parameter spelling** rather than
making literal-materialization a mandatory Phase 2 requirement:

```sth
: mconcat['F: Foldable 'T: Monoid] ( 'F['T] [ 'T 'T -- 'T ] -- 'T ) empty swap fold ;
: main ( -- ) someList [ combine ] mconcat . ;
```

Inside `mconcat` the merge quotation is a **bound parameter** with a generic effect
(`[ 'T 'T -- 'T ]`), forwarded to `fold` — exactly the shape that already unifies
through the existing `Quotation`/`Quotation` arm (M3's already-working case). `empty`
is a nullary member reached through the caller's `'T: Monoid` bound (M1). The literal
`[ combine ]` is written at the **mono** call site, where materialization already
works. This keeps Phase 2 scoped to the concrete-effect bridge arm only and needs no
poly-site literal materialization. If, contrary to this analysis, `fold`'s HKT
dispatch cannot forward the bound parameter in practice, Phase 5 records the wall and
`mconcat` is dropped from the goldens with its error text — it is not silently
downscoped.

### R7 — `List['T]` promotion into core (Phase 3)

`lib/core/list.sth`: `type: List['T] | Nil | Cons 'T rest ^List['T] ;` plus
`export: List ; export: Nil ; export: Cons ;`, mirroring
[lib/core/option.sth](../../../lib/core/option.sth). `list` is added to the `module:`
line of [lib/core/sooth.pkg](../../../lib/core/sooth.pkg). Whether it also joins
`prelude`'s re-export hub is ruled **no**: a hub re-export carries words only, not
type names or operators ([[project_hub_reexport_carries_words_only]]), so it would be
a partial and misleading surface.

The destructor half of the exit criterion needs no new machinery (F6, unchallenged),
but it needs a *witness*: a golden building a multi-element `List[str]` and dropping
it, so the per-instantiation payload drop is pinned rather than asserted.

### R8 — array as a constructor: the widening, sized honestly (Q2; Phase 6)

Three separable pieces, in dependency order, sized with honest **occurrence** counts
(re-run `grep -rho … | wc -l` at HEAD for this pass; the previous draft mixed a
line-count with an occurrence-count).

**R8.1 — `PolyType::App` carries length arguments.** Add `len_args: Vec<Len>`,
mirroring `PolyType::Generic`'s field (`src/ast.rs:2610`) and its `Len::Var` indexing
convention. Fan-out: **87 `PolyType::App` occurrences** across `src/`
(`check/poly.rs` 33, `parser.rs` 25, `ast.rs` 14, `ir/driver.rs` 5,
`check/declarations.rs` 4, `check/audits.rs` 3). The `ir/driver.rs` and
`check/audits.rs` sites are the surface the previous draft never named:
`check/audits.rs` is the fail-closed quotation-registry audit, where a new `len_args`
field reaching a wildcard `..` match is exactly the failure mode
[[workflow_enumerate_wildcards_when_adding_a_variant]] warns against — enumerate those
arms explicitly. The parser's application fold routes a bracket argument to `len_args`
when the corresponding kind domain is `Kind::Len`, which makes the kind annotation
load-bearing instead of decorative.

**R8.2 — a constructor identity for `array`.** This spans three registries, not just
`CtorImage`:

- `Type::CtorImage(GenericId, &'static str)` (`src/ast.rs:3115`) — **127**
  occurrences (`check/poly.rs` 78, `ast.rs` 22, `ir/driver.rs` 11, `parser.rs` 5,
  `ir/types.rs` 4, `check/builtins.rs` 4, `check/declarations.rs` 3).
- `PolyType::Generic`'s identity triple `(is_enum, idx, module)`, which
  `ground_member_poly`'s App arm (`src/ast.rs:2388+`) destructures **directly** — this
  is what impl-target grounding actually reads, so a `CtorImage`-only widening would
  still reject an array target with `member_app_abstract_target_error`.
- `GenericId` — **31** occurrences; its doc (`src/ast.rs:2516`) says it mirrors
  `PolyType::Generic`'s triple.

Widen `GenericId`'s discriminant (or the identity it wraps) to distinguish a user
header from the builtin array constructor, rather than adding a parallel `Type`
variant. Per [[workflow_widening_a_shared_predicate_inventory_every_consumer]], the
phase's first task is a written inventory of the `CtorImage`/`GenericId`/`Generic`-triple
read sites classified read-the-id vs. match-the-shape, including the `ir/*.rs` and
`check/audits.rs` sites; the inventory is a deliverable, not scaffolding.

**R8.3 — impl targets carry real kinds.** `parse_impl_target`
(`src/parser.rs:4071-4103`) replaces `ty_kinds: vec![Kind::Star; …]`
(`src/parser.rs:4097`) with the target's actual per-variable kinds, and publishes the
constructor's own kind so the trait-vs-impl kind check compares `* -> Len -> *`
against the target. The named-array reader
(`parse_impl_target_named_array_parses`, `src/parser.rs:13796`) is the path to extend.

**R8.4 — the budget checkpoint, settled as last and fenced.** Q2 arrived settled and
was **re-confirmed** post-review as "last and fenced"; this is stated here as fact,
not reopened. Phase 6 is the largest item in the slice (87 + 127 + 31 sites plus a
parser change) and the furthest from the exit criterion, which names arrays only as a
probe with "or the kind story gets a ruling" as an accepted outcome. **Phase 6 runs
last, after every exit-criterion phase is green and committed.** If R8.1's fan-out
exceeds the phase, the slice ships without it and the recorded ruling is "arrays do
not become Functor/Foldable instances in S6; the App-len-args widening and the
constructor-identity widening carve out to S6b" — which is a *ruling* and satisfies
the exit criterion's own wording. This ordering is insurance that M3/M4 (on the
critical path, unknown when Q2 was decided) cannot be starved by it.

**R8.4's verdict, exercised (Phase 6): carved out to S6b.** The Phase 6 section below
carries the inventory this ruling is grounded in, and the measured architectural
conflict the fence caught before any registry edit was made.

### R9 — no new machinery for linear consumption inside fold bodies (F3)

F3 is accepted as measured, with the previous draft's fixture count corrected against
the probe log: only **two** of the three probe fixtures actually reject. The third
(`p3_fold_forget_payload.sth`, a forgotten payload disposed via `drop`) **runs clean
and prints 10** — as the probe log itself states, "forgetting is an error, not
discarding via `drop`", so a `drop`ped payload is legal and is not a linearity
violation. There is **no** Foldable-specific linearity rule, and S6 adds none.

Phase 5 lands the **two real rejecting goldens**, attributed correctly to the general
mechanisms that fire (arm-shape parity on `Option?`; ordinary `call` arity underflow),
**not** to a new linearity rule. The mutation check as "delete the guard, confirm the
golden fails" is **not** executable against those general arm-parity/arity checks
(their deletion breaks the whole suite and cannot discriminate a Foldable-specific
rule, because there isn't one); so the two goldens are landed as behaviour witnesses
without a per-rule mutation claim. The phase doc's "forgetting one is a compile error"
clause is **left unwitnessed with a note**: S6 does not make bare forgetting-via-`drop`
illegal (it is legal by DESIGN.md's explicit-destructor rule), and no golden claims
otherwise.

## Phased delivery

Each phase is green (`cargo fmt --check && cargo clippy -- -D warnings && cargo
test`) at its exit; goldens are diagnostic-text or stdout assertions per CLAUDE.md
(`thing_condition_expected` naming, happy path plus at least one error case), and
**every phase adds unit tests beside the mechanism it edits**. New goldens land in
`tests/phase7b_slice6.rs`.

### Phase 1 — the M3 diagnostic, ICE first (R2.a)

- Split `trait_member_operand_error` (`src/check/poly.rs:10806`) into
  `expected_sig`/`found_sig`; at the bound-dispatch site (`:2680`) render `declared`
  against `&member_decl.sig` and `found` against the caller `sig`, dropping the
  `substitute_member_var(declared, var)` call. Record a per-site verdict for the two
  other sites (`:1837`, `:2371`) confirming their `Concrete` args make the split a
  no-op there.
- Unit test beside `poly_type_str`: a member-local `Var(n)` with `n` past the
  caller's `ty_var_names` renders (against the member sig) rather than panics.
- Golden: `poly_body_concrete_effect_quotation_operand_is_located_error` — the M3
  concrete-effect fixture produces a located message, not a panic. (Renamed in
  Phase 2 once the capability lands, per its exit note.)

Exit: no fixture in the M3 family panics; all three call sites carry a verdict.

### Phase 2 — the cross-representation quotation bridge (R2.b)

- `unify_member_operand` (`src/check/poly.rs:1335`) gains the bridge arm: declared
  `PolyType::Quotation` vs found `PolyType::Concrete(Type::Quotation |
  InlineQuotation | OwningQuotation)`, unifying the reconstructed rows structurally
  and binding member-local vars, honouring inline/owning flavour. Both call sites
  covered (`:2679` erroring, `:1271` silent viability filter).
- Headline golden `poly_body_forwards_a_concrete_effect_quotation_parameter_to_a_member`:
  the concrete-effect forwarding case now returns a value (rename the Phase-1
  located-error golden's assertion to the accept case, since the capability now
  exists — it must not be left asserting a dead criterion).
- Non-regression golden `poly_body_forwards_a_generic_effect_quotation_parameter`:
  the already-working generic-effect case still runs (pins the existing arm).
- The written-literal shape stays a located error (M3); a golden pins that it does
  **not** panic and is not silently accepted.
- Unit tests beside `unify_member_operand`: a concrete-quotation found value bridges
  and binds; an arity-mismatched concrete quotation rejects; a flavour mismatch
  rejects.
- Mutation check per R2.b: revert **only the new bridge arm** and confirm the
  concrete-effect golden regresses to the Phase-1 located error.

Exit: a quotation-taking member is callable through a shared bound for both the
generic- and concrete-effect parameter spellings; the literal shape is a located
error, not a panic.

### Phase 3 — `List['T]` into core, and impls for it (R3, R7)

- Add the `OwnedCell` arm to **both** twinned catch-alls:
  `substitute_generic_variant_field` (`src/ast.rs:2708`, plain substitution + doc
  correction) and `poly_bind_construction_arg` (`src/check/poly.rs:5868`).
- Struct-twin verdict per R3.
- `lib/core/list.sth` + the `sooth.pkg` `module:` line.
- Goldens: `impl_foldable_for_list_dispatches` (the M4 fixture, non-inline body, now
  producing a value); `multi_element_list_of_str_drops_clean` (destructor witness,
  R7); `list_self_reference_builds_across_the_core_import` (promotion witness).
- Unit tests beside both new arms (destructure and construction).

Exit: `List['T]` lives in core, takes a non-inline trait impl, and drops linear
payloads per instantiation with a golden for each.

### Phase 4 — mono `empty` (R4, R5)

- The zero-dispatchable-input branch in `resolve_mono_member_call`
  (`src/check/poly.rs:2165`), grounding `'T` from `type_args`, plus the narrow S2-16
  guard carve-out (`:2303`) for a zero-dispatchable-input member.
- Golden `nullary_trait_member_grounds_from_explicit_instantiation`
  (`7 empty[i64] combine .` → `12`).
- Golden `bare_nullary_member_without_instantiation_is_located_error` (bare `empty`
  cites the `empty[i64]` remedy).
- Unit tests: a zero-input candidate with matching `type_args` grounds and dispatches;
  a zero-input candidate without `type_args` errors; an **operand-carrying** member
  never enters the branch (the scope-fence test).
- Non-regression: `mono_concrete_member_call_with_explicit_type_args_is_error` and
  `mono_word_colliding_with_member_name_rejects_explicit_type_args` still pass
  unchanged.

Exit: `empty[i64]` dispatches from a mono body; bare `empty` errors with a remedy;
operand-carrying members are byte-unchanged, including the two S2-16 pins.

### Phase 5 — the library surface, the goldens, and the roadmap edit (R1, R6, R6a, R9)

- `Monoid` impls per R6's checklist (i64 measured; `List` append unmeasured — lands or
  records its wall), each with a golden.
- `mconcat` per R6a's bound-parameter spelling, over `Option` and `List`.
- The exit-criterion dogfood: one program mapping and folding over `Option`,
  `Result`, and `List` through shared bounds.
- R9's **two** rejecting linearity fixtures as goldens, attributed to general
  arm-parity/arity checking (no per-rule mutation claim); the "forgetting-via-drop is
  legal" case is noted, not made an error.
- Rewrite the phase doc's S6 paragraph to current state (R1), no history.

Exit: the exit criterion is met and witnessed for its non-array clauses, or each unmet
clause carries a recorded ruling with its error text. (The array-kind ruling is
authored in Phase 6; Phase 5 does not claim it.)

### Phase 6 — array as a constructor (R8), last and fenced

- R8.2's inventory first, as a written deliverable, spanning `CtorImage` (127),
  `GenericId` (31), and `PolyType::Generic`'s triple, including `ir/*.rs` and
  `check/audits.rs`.
- R8.1 (`App.len_args`, 87 occurrences, wildcard/audit arms enumerated), then R8.2
  (the constructor-identity widening across all three registries), then R8.3
  (`parse_impl_target` kinds).
- Golden `impl_functor_for_array_dispatches`, plus the kind-error golden that a
  `* -> *` trait rejects an `array` target.
- Unit tests beside each edit.
- Per R8.4: if the fan-out exceeds the phase, stop, revert cleanly, and land the
  recorded S6b carve-out ruling instead.

Exit: `array` is a valid `impl:` target with real kinds and a dispatching golden,
**or** the carve-out ruling to S6b is recorded in the phase doc with the measured
reason.

**Outcome: carved out to S6b.** No `src/` edit was made; the worktree stays at the
Phase 5 commit. The counts re-verified clean against HEAD before any edit (`grep -ro`
over `src/`): `CtorImage` 127 (`check/poly.rs` 78, `ast.rs` 22, `ir/driver.rs` 11,
`parser.rs` 5, `ir/types.rs` 4, `check/builtins.rs` 4, `check/declarations.rs` 3),
`GenericId` 31 (`check/poly.rs` 13, `ast.rs` 11, `ir/driver.rs` 4, `check/builtins.rs`
1, `check.rs` 1, `parser.rs` 1), `PolyType::App` 87 (`check/poly.rs` 35, `parser.rs`
25, `ast.rs` 15, `ir/driver.rs` 5, `check/declarations.rs` 4, `check/audits.rs` 3) --
all within a percentage point of the spec's numbers; no drift changed the sizing call.

**R8.2's inventory (the deliverable).** `GenericId { is_enum: bool, idx: u32, module:
u32 }` and `PolyType::Generic`'s matching `(is_enum, idx, module)` triple are read at
every site in one of two ways:

- **Read-the-id** (mechanical, safe to widen): sites that only construct or
  destructure the triple/`CtorImage` to carry it through unchanged (e.g. clone-forward
  in `substitute_generic_field`/`substitute_generic_variant_field`, the diagnostic
  render at `ast.rs:2930` `c{idx}m{module}_{name}`, the `App` fold's argument-arity
  bookkeeping in `parser.rs`). These are the majority of the 127+31 and would take a
  third `is_enum`-shaped discriminant without incident.
- **Match-the-shape** (semantic, architecture-load-bearing): every site that uses
  `is_enum` as a **binary switch into a registry indexed by `idx`**:
  `ctor_image_type` (`ast.rs:3465-3470`, `generics.enums[idx]` / `generics.structs[idx]`
  for the display name), `substitute_generic_field`'s `CtorImage` arm (`ast.rs:975-978`,
  dispatches to `instantiate_enum`/`instantiate_struct`), and -- the two sites R8.2
  explicitly flagged as easy to miss -- `ir/driver.rs`'s `subst_polytype` `Generic` arm
  (`:683`, `lookup_enum`/`lookup_struct`) **and** its `App` arm (`:709-721`, the same
  `gid.is_enum` binary dispatch after destructuring `Type::CtorImage(gid, _)`).
  `check/audits.rs`'s 3 `PolyType::App` sites are exhaustive `match` arms in the
  quotation-registry audit (fail-closed per
  [[project_quotation_registry_audit_fails_closed]]); they do not read `is_enum`
  directly but do require a new arm the moment `App` gains `len_args`, so they are R8.1
  fan-out, not R8.2 fan-out, and were re-confirmed enumerable (not wildcarded) before
  the fence fired.

**The measured reason the fence fired.** The match-the-shape sites are not a wider
*count* of the same edit -- they are a different *kind* of edit, and this is what R8.2's
prose ("widen `GenericId`'s discriminant ... rather than adding a parallel `Type`
variant") did not weigh: `is_enum: false` routes to `generics.structs[idx]`, a
`GenericStructDecl` carrying `fields: Vec<(String, PolyType)>` that
`instantiate_struct` walks to substitute and mint a concrete `StructDecl`; `is_enum:
true` routes to the enum twin. `array` has no such declaration. It is a built-in
`Type::Array(ArrayId, ..)` whose registry (`arrays: &[ArrayDecl]`) is **content-addressed
by `(element, count)`** (`ir/driver.rs`'s `Array` arm, `subst_polytype`: `arrays.iter()
.position(|d| d.element == element && d.count == count)`), not header-indexed by
`(idx, module)` the way `lookup_struct`/`lookup_enum` are. Making `array` a third
`GenericId` case that flows through the existing `is_enum`-binary sites would require,
at minimum, a fourth registry (`GenericArrayDecl` or equivalent) paralleling
`GenericStructDecl`/`GenericEnumDecl` plus an `instantiate_array`/`lookup_array` pair
written from scratch to bridge the header-indexed identity onto the shape-indexed
`ArrayDecl` registry `Type::Array` already uses everywhere else in check and lowering
-- new machinery, not a widened match arm, at exactly the `ir/driver.rs` sites R8.2
flagged as easy to miss. That is the fan-out this spec's own effort/difficulty rating
("L/H") anticipated but did not size: R8.1 (87 occurrences) and the read-the-id half of
R8.2 are mechanical and were confirmed tractable within the phase; the match-the-shape
half of R8.2 is a new instantiation subsystem, which is what R8.4's fence exists to
catch before a partial widening ships half-wired.

**Ruling: arrays do not become Functor/Foldable instances in S6.** The App-len-args
widening (R8.1) and the constructor-identity widening (R8.2/R8.3) carve out to a future
S6b, scoped up front as: design the array-constructor registry bridge
(`GenericArrayDecl` or equivalent, plus `instantiate_array`/`lookup_array`) as its own
piece, before touching any `is_enum`-binary call site. R8.1's `App.len_args` field can
land independently of R8.2 in that follow-on (it has no architectural conflict, only
occurrence count), but landing it alone in S6 with no consumer (no golden needs `Len`
kind-checking without R8.2/R8.3 behind it) would be unmotivated churn against
[[feedback_phase_scope_discipline]], so it carves out with the rest. No `src/` file was
edited to reach this verdict.

## Signals re-check at phase exit (CLAUDE.md growth structure)

`src/check/poly.rs` is already large and a split is deferred at 3/5 signals with no
clean cut ([[project_poly_rs_split_deferred]]). Phases 1, 2, 3 (the construction-arm
twin), and 4 all add to it. `src/ast.rs` grows in Phases 3 (the destructure arm) and 6
(the constructor identity). Re-run the split signals at each phase exit against the
file as it then stands; do not preemptively split. Re-check `src/ast.rs` at Phase 6
exit in particular, since R8's widening is the change most likely to create import
divergence. (Phase 4 touches only `src/check/poly.rs`, not `src/check/terms.rs` — the
Q1 revision removed the lookahead tier that would have.)

## Open questions (to be closed by the phase that resolves them)

- **R3's struct twin** — reachable via `^Self['T]` in a generic struct field, or
  structurally unreachable. Closed by Phase 3, with the parse error if the latter.
  **Verdict (Phase 3): reachable, already covered pre-existing this slice.**
  `substitute_generic_field`'s `OwnedCell` arm (`src/ast.rs`) and the `Generic` arm's
  re-entry into `instantiate_struct` (memo-before-substitute, R6) together terminate it;
  see `instantiate_struct_pushes_memo_key_before_substituting_fields`, which already
  builds `type: L['T] v 'T next ^L['T] ;`. No new code needed.
- **R6a's `mconcat` forwarding** — **verdict (Phase 5): grounds.** The bound-parameter
  spelling forwards through `fold`'s HKT dispatch for both `Option` and `List`
  (`mconcat_over_option_dispatches`, `mconcat_over_list_dispatches`,
  `tests/phase7b_slice6.rs`); no wall.
- **R6's `List` append** — **verdict (Phase 5): recorded wall, not landed.** A real linear
  merge over two spines panics in `poly_bind_construction_arg`
  (`src/check/poly.rs:6072`) on a bare `PolyType::Generic` field the existing `OwnedCell`
  arm (Phase 3, M4/R3) does not cover — distinct from M4's twinned arms, and *not* about
  recursion: a single non-recursive `Cons` construction inside any trait-member body over
  `List` reproduces the identical panic (measured directly; see
  `monoid_for_list_append_construction_wall_is_recorded`,
  `tests/phase7b_slice6.rs`). The same wall blocks `Functor for List`'s `map` for the same
  reason (any reconstruction, not just `combine`'s). `Monoid for List` and `Functor for
  List` are dropped from the goldens; `Foldable for List` (destructure-only, no
  reconstruction) is unaffected and already landed in Phase 3. Fixing the wall is a future
  slice's job, not Phase 5's — its cause (a construction-arm gap distinct from Phase 3's)
  is out of Phase 5's scope (library work, not compiler work).
- **R8.4's budget** — Phase 6 lands, or the S6b carve-out ruling does. Ordering and
  fence are settled (last, fenced); only the land-vs-carve outcome is open, closed by
  Phase 6.

## Phases (machine-readable)

```json
[
  { "phase": 1, "focus": "M3's ICE: split trait_member_operand_error (poly.rs:10806) into expected_sig/found_sig and, at the bound-dispatch site (poly.rs:2680), render the raw member input against member_decl.sig and the caller slot against the caller sig, dropping substitute_member_var; per-site verdict on the two Concrete sites (1837, 2371); unit test for a member-local Var(n) past the caller's ty_var_names; located-error golden for the M3 concrete-effect fixture", "effort": "S", "difficulty": "M" },
  { "phase": 2, "focus": "R2.b: unify_member_operand (poly.rs:1335) gains a cross-representation bridge arm for declared PolyType::Quotation vs found PolyType::Concrete(Type::Quotation|InlineQuotation|OwningQuotation), covering both call sites (poly.rs:2679 erroring, poly.rs:1271 viability filter); concrete-effect parameter forwarding becomes a value-asserting golden (renamed from Phase 1); generic-effect parameter non-regression golden; written literal stays a located error; unit tests beside the arm; mutation check reverts only the new bridge arm", "effort": "M", "difficulty": "M" },
  { "phase": 3, "focus": "M4/R3/R7: add the missing OwnedCell(Generic) arm to BOTH twinned catch-alls -- substitute_generic_variant_field (ast.rs:2708, plain substitution + doc fix) and poly_bind_construction_arg (poly.rs:5868); struct-twin verdict; lib/core/list.sth plus the sooth.pkg module: line; goldens for a non-inline dispatching List impl, a multi-element List[str] destructor witness, and the core-import self-reference; unit tests beside both arms", "effort": "M", "difficulty": "M" },
  { "phase": 4, "focus": "R4/R5: a zero-dispatchable-input branch in resolve_mono_member_call (poly.rs:2165) grounding from type_args, plus a narrow carve-out of the S2-16 concrete-target guard (poly.rs:2303) for a zero-dispatchable-input member (disjoint from its pinning test at poly.rs:12071); explicit-instantiation golden; bare-empty located-error golden; scope-fence unit test that an operand-carrying member never enters the branch; both S2-16 pins unchanged", "effort": "M", "difficulty": "M" },
  { "phase": 5, "focus": "R6/R6a/R9/R1: Monoid impls per the checklist (i64 measured; List append unmeasured, lands or records its wall), mconcat with a bound-quotation-parameter spelling over Option and List, the exit-criterion dogfood mapping and folding over Option/Result/List through shared bounds, R9's two rejecting linearity goldens attributed to general arm-parity/arity checking (no per-rule mutation claim, forgetting-via-drop noted as legal), and the phase-doc S6 rewrite to current state", "effort": "L", "difficulty": "M" },
  { "phase": 6, "focus": "R8, last and fenced: the constructor-identity inventory (CtorImage 127, GenericId 31, PolyType::Generic triple, incl. ir/*.rs and check/audits.rs) as a written deliverable, then PolyType::App gains len_args (87 occurrences, wildcard/audit arms enumerated), then the constructor identity widens across all three registries, then parse_impl_target publishes real kinds instead of the hardcoded vec![Kind::Star] (parser.rs:4097); dispatching and kind-error goldens, or the recorded S6b carve-out ruling per R8.4", "effort": "L", "difficulty": "H" }
]
```
