# P7b.S3 spec — inline + HKT bounds, the zero-cost splice

Technical specification for compiler slice P7b.S3, implemented in full. Scope input was
the recon [brief](./slice3-brief.md) and [probe log](./slice3-probes.md); exit criteria
are from the [phase doc](../P7b-higher-kinded-types.md).

## Exit criteria (from the phase doc)

`Functor.map` called through a bound on an **inline** word splices to the same IR as a
hand-written inline `map` would produce; no call frame; no runtime dispatch.

## Shape

An `inline` trait member is spliced by `inline_combinator`, the same mechanism
`core::cmp`'s `lt` already uses for ordinary combinators — there is no second splice
mechanism. Splicing is gated on the **member's** own `inline` keyword, never on the
caller's: an `inline` member splices from a mono, non-inline-poly, or inline caller
alike; a non-`inline` member monomorphizes from every caller.

`WordDef::is_trait_member: bool` (`ast.rs:1846`) marks a member word at synthesis
(`parser.rs:4128`) and discriminates it from an ordinary combinator at both remaining
gate sites.

### Caller-side resolution

Six gates fire in order for the exit shape (outermost first); their anchors:

| Gate | What it does | Anchor |
| --- | --- | --- |
| G1 | Standalone check seeds an Arrow-kinded variable with `Type::CtorImage(g)`, `g` the first declared `impl:`'s constructor, instead of an `i64` stand-in that can't represent an App-headed type. A grounding failure or operand mismatch against the stand-in's constructor is rescued (standalone check skipped); every other failure (arity, linearity, unknown word, borrow/move, undischarged bound) stays a hard error. No impl in the program, or an Arrow variable with no user bound: standalone check is skipped outright (checked only at splice sites). | `check/poly.rs:552-556` (seed), rescue at `check_poly_combinator_standalone` |
| G2 | Member-slot grounding goes through the caller's θ (S2-6 leading-slot grounding) instead of a single-`Type` `try_ground_member_type`; `splice_member_hkt_error` and both candidate-disambiguation `continue`s (App-headed skip, `try_ground_member_type`-`None` skip) are gone. A θ that fails to ground a candidate is a located error naming the candidate(s) and the unbound variable, never a panic or silent skip. `splice_member_ctor_image_error` survives, narrowed to the case θ still leaves an unbound `CtorImage`. | `check/poly.rs` member-slot grounding path |
| G3 | The splice path's impl lookup goes through `find_bound_impl` (with the live `arrays`/`cells`/`refs`/`generics` registries) instead of a bare `find` with empty registries, so a `Generic` target pattern matches a monomorph operand, `where`-bound discharge and most-specific selection apply, and the splice path and `resolve_user_bound` agree on which impl wins. Returns `(impl idx, Subst)`; the `Subst` feeds G6's `inline_combinator` as its `seed`. | `check/poly.rs:7654` |
| G4 | R3's refusal of a top-level `PolyType::Generic`/`GenericVariant` input is skipped when `word.is_trait_member`; unchanged for every other word. | `check/poly.rs:566-571` |
| G5 | The poly pre-pass records member combinators (per `is_trait_member`), so `word_sig_of` hits for them. | `check.rs:837` |
| G6 | A member call at a splice site: if `words[idx].declares_inline` (via `TraitResolveCtx::words`, never `word_symbols[idx]`, which `overload_symbols`' `$$`-suffixing can desync from the word's own name), look the member up in `poly.combinators` by its synth name and call `inline_combinator` with the caller's live stack and G3's `Subst` as `seed`, instead of writing a bare-symbol resolution. `splice_trait_calls[(uid, span)]` still gets written — with the member's synth name, which is lowering's routing record into the splice bracket, not an impl-symbol resolution. | `check/poly.rs` splice-caller hop |

### Instantiation-emission diversion (the non-inline-poly-caller half)

A non-inline poly caller resolves a member call through `resolve_user_bound`'s
post-hoc fixpoint, which already walks the member body per-θ and produces a correct
`CallInst`; the miss was that lowering's dedup/emission stretch minted a real `IrFunc`
for it regardless. A pre-pass, placed above the lowering env build and above both the
per-word `FuncBuilder` pass and the (alphabetically sorted) dedup/emission loop, walks
the same `instantiations`/`transitive_instantiations`/`splice_records` chain and diverts
every `is_combinator` instantiation's `(symbol, CallInst)` into
`combinator_instantiations: HashMap<String, CallInst>`, threaded into every
`FuncBuilder`. The `Arity` env-insert loop and the emission loop skip those symbols (no
`IrFunc`, no env entry). `lower_resolved_word_call`, on a `combinators` miss, consults
`combinator_instantiations`; a hit recovers the member and splices through the existing
P7.S8 R1 uid bracket, with that instantiation's own `enum_words`/`trait_calls` installed
for the bracket's duration. `member_uid_seeds` stays name-keyed and is consulted by the
recovered `inst.callee`, never by the instantiation symbol.

Both check-side (G6) and lowering-side member splices mint their own fresh uid per call
site (the ordinary `prov.inline_uid` discipline, mirrored on the lowering side), so two
splices of one member body never collide on `(uid, span)`.

### `splice_enum_words`: per-splice enum resolution

`Module.splice_enum_words: HashMap<(u32, Span), EnumId>` gives a spliced body's own enum
construction/elimination sites somewhere per-splice to record their resolution (a poly
*call* inside a splice already carries this in its own `CallInst`; this covers the
splice's *own* enum sites). Written in the concrete path-A walk (`check_terms_relaxed`'s
arms), gated on `prov.splice_uid` being set **and** the chosen candidate being a
generated enum word (membership in `enum_generated_sigs`/`variant_generated_sigs`, not
sig shape): the candidate arm writes `(uid, span) -> EnumId` instead of the span-keyed
`builtin_overloads` insert, and `check_eliminator_call` records the operative id off the
live scrutinee. Non-enum records at the same sites (the Slice-10c combinator-name-collision
record, the D7/R5 struct ctor/accessor record) are untouched even inside a splice.
`lower_call`'s enum-word arm consults `splice_enum_words` first, then the span-keyed
`enum_words`, then the bare-key monomorphic path.

`discover_transitive_instantiations` threads `splice_records` (it walks them to mint
further monomorphs) but has no `splice_enum_words` companion: that map is terminal
`(uid, span) -> EnumId` data, read only by lowering.

### R1.5, re-sited

`is_combinator_splice` (the gate that refuses a generic enum construct/eliminate inside
a *non-member* combinator body, since the standalone check's `i64`/CtorImage stand-in
can't ground the enum's own type arguments) is unchanged for ordinary combinators. For a
member combinator (`word.is_trait_member`) the pre-pass sets it `false`: the pre-pass
walk still runs as a soundness check, but a member body is re-walked per splice with a
real θ, so a definition-time refusal no longer applies. The one pre-existing pin on
R1.5's text (`tests/phase7_slice12.rs:235`) uses a non-member combinator fixture and is
unaffected.

### Row 7 HKT (non-inline HKT member, inline caller)

A *non-inline* HKT member reached through an inline caller still rejects: grounding its
output-only variable against the CtorImage-bound head has no recovery path outside G6's
seeded hop. `P#3`'s twin pins this as a rejection, not a splice.

## Deliberate limitations (ledger)

1. A non-inline member still monomorphizes, from every caller flavour (C#1 pins the
   monomorph is still minted).
2. An HKT-bounded inline word whose trait has no impl in the program, or whose bound's
   impls all arity-mismatch an Arrow variable's kind, is not checked standalone — only at
   its splice sites. The no-impl trigger is unwitnessable by construction (no impl means
   no call can discharge the bound, so the word is never spliced); the no-user-bound
   trigger is pinned rejecting at its first splice site (E#1).
3. Fixture twins stand in for `core::option`/`core::result`/`core::list` (P7b.S4's
   module-identity gap is still open).
4. The struct route to `Functor` (`Box[…]` on a generic type) stays closed, identically
   with and without `inline` (E#3).
5. The S2-8 tie rule's pin metric still counts top-level pins only; untouched by S3 (G3
   routes through `find_bound_impl`, not that selector).
6. Bound dispatch inside a materialized quotation stays rejected (P7.S3o R5): a
   materialized quotation lowers to its own `IrFunc` with an empty `splice_uid_stack`, so
   no `(uid, span)` key resolves (E#2).

## Golden / test inventory (`tests/phase7b_slice3.rs`, harness `single_file_hosted`)

Every **positive** golden asserts: stdout/exit code; `nm` shows no `<member>.3b.<Trait>`
symbol and no `sooth_mono_<member>_<Trait>` monomorph symbol; `objdump`/`call_graph` shows
no call edge to either, reachable from `sooth_main`; a non-inline twin in the same test
shows the symbol present with call edges; and, when the *caller* word itself declares
`inline`, the caller's own frame is absent too (checked via `nm`/`call_graph` on the
caller's own mangled symbol) — this last clause applies to P#1/P#3/P#4 only (not P#2, a
monomorphization golden; not P#5, a deliberately non-inline-caller golden; not P#6, which
doesn't exist).

- **P#1** `inline_member_splices_into_an_inline_bound_caller` — non-HKT (`Sized`/`Box`),
  `size inline`, inline caller.
- **P#2** `non_inline_member_on_a_generic_target_from_an_inline_caller` — `size` without
  `inline`, generic target, inline caller: accepts, runs, monomorphizes (G3's fix).
- **P#3** `hkt_member_splices_through_an_inline_bound_caller` — the exit criterion
  itself, `Functor`/`Opt`/`Res`, `twice inline`.
- **P#4** `two_splices_of_one_member_at_two_thetas_resolve_independently` — one impl, one
  inline caller, two θ, each eliminating the enum; the two-different-`EnumId`s claim is
  unit-tested (unobservable from a compiled binary), the golden carries stdout/`nm`/`objdump`.
- **P#5** `inline_member_on_generic_target_splices_from_a_non_inline_poly_caller` — the
  instantiation-diversion witness: same `inline` member, non-inline poly caller, monomorph
  symbol absent, caller keeps its frame (clause 5 exempt).
- **C#1** `non_inline_member_still_mints_its_monomorph_symbol` — presence control, makes
  the splice goldens' absence assertions non-vacuous.
- **C#2** `s2_shared_bound_golden_unchanged`, **C#3** `concrete_target_inline_member_unchanged`,
  **C#4** `core_cmp_lt_still_splices` — non-regression.
- **E#1** `unbounded_arrow_variable_inline_word_is_rejected_at_its_splice_site` — ledger
  item 2's witnessable half (plus a clean-build companion for the unreferenced case).
- **E#2** `bound_member_in_a_materialized_quotation_still_rejects` — ledger item 6.
- **E#3** `struct_route_to_functor_still_rejects_identically` — ledger item 4.
- **E#4** `generic_enum_in_a_non_member_combinator_body_still_rejects` — R1.5's surviving
  gate on non-member combinators.

There is no `P#6`: under the shipped R1.5 ruling, `tests/phase7_slice12.rs`'s existing
R1.5 construction fixture (`wrap_it`, a non-member combinator) still rejects unchanged, so
no new golden was needed for it.

**Mutation coverage (measured, held).** Stubbing the `splice_enum_words` lowering read
fails P#4's stdout clause (two θ collapse onto one bare-key layout) without touching P#3
(single-θ per family still resolves correctly on the bare key). Reverting the G6 hop
alone fails all five positive goldens (no per-splice re-walk, no `splice_enum_words`
entries). Reverting the instantiation-diversion pre-pass alone fails P#5's monomorph-
absence clause only. The m4 adversary (checker gates lifted, lowering forced onto the
non-splice arm for member calls) fails P#1/P#4/P#5 on their `nm`/`objdump` clauses with
stdout still passing, and fails P#3 at the stdout clause (an HKT member's output-only
variable has no grounding path outside the splice hop, so the non-splice arm can't even
build it) — one clause earlier than the other three, and still a kill.

## Anchors

| Anchor | At |
| --- | --- |
| `WordDef::is_trait_member` / `declares_inline` | `ast.rs:1846` / `ast.rs:1810` |
| `Module.splice_enum_words` / `splice_records` / `splice_trait_calls` | `ast.rs:108` / `:89` / `:99` |
| `TraitResolveCtx` (`words` field) | `check/poly.rs:122-133` |
| `is_combinator` / `collect_combinators` key | `check/combinators.rs:139-141` / `:119-121` |
| `inline_combinator` (uid mint, θ from live operands, body walk) | `check/combinators.rs:313` / `:503-504` / `:591` / `:543` |
| `find_bound_impl` | `check/poly.rs:7654` (candidate collection `:7676-7699`) |
| `resolve_user_bound` / `impl_mono_seed` (per-θ member walk) | `check/poly.rs:7771` / `:7086` (`enum_words` `:7158-7167`) |
| Poly pre-pass member recording / `is_combinator_splice` set site | `check.rs:837` / `:865` |
| Instantiation-diversion pre-pass / dedup-emission stretch it precedes | `ir/driver.rs` (above `:135`) / `:319-339` |
| `combinator_instantiations` consult on a `combinators` miss | `ir/func_builder/calls.rs:229` |
| `member_uid_seeds` / P7.S8 R1 seed unit test | `ir/driver.rs:67` / `:734-761` |
| R1.5 elimination / construction raise sites | `check/poly.rs:4151` / `:5611` |
| `nm`/`call_graph` helpers | `tests/common/mod.rs` |

## Growth-signal note

`check/poly.rs` is 20735 lines at slice exit (11565 source), 3-of-5 split signals
(unchanged from S2's measurement): import divergence does not fire, and S3 reused the
existing combinator splice path rather than adding a second mechanism. Deferred again;
re-run at P7b.S4's exit.

**MEMORY correction.** The prior note "an inline trait member panics at lowering" is
stale and retired: `is_trait_member` routes an `inline` member on a generic impl target
onto the ordinary combinator splice path at check time, per-θ instantiations are
diverted from lowering's ordinary emission and spliced at their dispatch sites, and enum
sites inside the splice resolve through `splice_enum_words`.
