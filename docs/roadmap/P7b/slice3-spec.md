# P7b.S3 spec — inline + HKT bounds, the zero-cost splice

Technical specification for compiler slice P7b.S3. Scope input is the recon
[brief](./slice3-brief.md) (findings F1–F9, machinery map, open questions Q1–Q5) and the
verbatim [probe log](./slice3-probes.md); exit criteria are from the
[phase doc](../P7b-higher-kinded-types.md). All anchors were re-verified against HEAD
`403618f` while writing this spec, and the gate inventory below was re-measured live (see
[Correction to the brief](#correction-to-the-brief-the-caller-side-gates)).

Diagnostic texts here pin **shape**, not wording; exact strings freeze in the goldens
(S12 precedent, as in S1/S2).

## Exit criteria (from the phase doc)

`Functor.map` called through a bound on an **inline** word splices to the same IR as a
hand-written inline `map` would produce; no call frame; no runtime dispatch.

Two of those three clauses are *negative* claims about the emitted binary, which is why
S3-13 makes symbol/call-count assertions a
hard requirement rather than a testing style note.

## What the brief established, and what it did not

The brief's two load-bearing results stand and this spec is built on them:

- **F1 — the lowering half already works.** A concrete-target `inline` member compiles,
  runs and genuinely splices (`nm` shows no `size;Sized;0;Boxi` symbol). The splice
  bracket at `src/ir/func_builder/calls.rs:170-228` is present and implements the P7.S8
  R1 uid rule. **S3 is not a lowering slice**; the standing MEMORY note that inline
  members panic at lowering is stale and is retired by this spec.
- **F9 — lifting the checker gates is not the same as splicing.** With all three of the
  brief's gates stubbed (m4), p2/p4 print correct values while `nm`/`objdump` show
  symbols and call counts *identical to the non-inline control*. The `inline` keyword
  produced no lowering difference at all. A stdout-only golden would certify this slice
  done with the criterion unmet.

### Correction to the brief: the caller-side gates

Every probe in the round used a **non-inline caller** (`: usesize ['S: Sized] …`,
`: twice['F: Functor 'T 'E] …`). The exit criterion is about an **inline** caller. Re-run
this session as a 2×2×2 matrix (member `inline`? × impl target generic? × caller
`inline`?), against HEAD `403618f`:

| member `inline` | target | caller `inline` | outcome |
| --- | --- | --- | --- |
| yes | concrete | yes | OK, member spliced (0 symbols) |
| yes | concrete | no | OK, member spliced (0 symbols) |
| yes | generic | yes | **reject** — R3 grounding refusal (brief gate 1) |
| yes | generic | no | **reject** — R3 grounding refusal (brief gate 1) |
| no | concrete | yes | OK, member called (`size;Sized;0;Boxi` present) |
| no | concrete | no | OK, member called |
| no | generic | yes | **reject** — `` `Box[i64]` does not satisfy `Sized` `` |
| no | generic | no | OK, member monomorphized (`sooth_mono_size_Sized_0_Box__T0___m0__t0_i64`) |

Row 7 is the one the brief could not see: an **inline caller over a generic impl target
fails even with a non-inline member**. And the HKT form of the exit shape (`: twice
inline['F: Functor 'T 'E] …`) fails earlier still, on the standalone check's `i64`
stand-in:

```text
error: type mismatch in `twice` (line 14)
  `twice` expected `'F['T 'E]`, found `i64`
```

So the brief's "three-gate checker chain" is the inventory for the *member* axis. The
exit criterion needs the *caller* axis too, which adds three more gates. S2 already knew
about two of them and said so in the tree: the unit test at `src/check/poly.rs:11552`
records that `splice_member_hkt_error` is source-unreachable precisely *because* "the
standalone combinator check grounds every declared variable at the `i64` stand-in, where
`apply_subst`'s App arm rejects exactly that slot". That comment is this slice's brief,
written a slice early.

## The gate inventory (six gates, verified anchors)

Firing order for the exit shape, outermost first.

| # | Gate | Anchor | Axis | Witness |
| --- | --- | --- | --- | --- |
| G1 | Standalone stand-in binds **every** ty var to `Type::I64`, including an Arrow-kinded head, so an App-headed declared input has no representable stand-in operand | `check/poly.rs:552-556`; raise via `poly_rendered_type_mismatch_error` `:9611` | caller | measured (`twice inline`) |
| G2 | `splice_member_hkt_error`: an App-headed member signature is refused on the splice path outright | `check/poly.rs:1514` (builder `:1633`) | caller | code-read; unreachable behind G1 (its own unit test says so, `:11552`) |
| G3 | The splice path's impl lookup is a bare `find` with **empty registries and `generics: None`**, so a `Generic` target pattern can never match a monomorph operand | `check/poly.rs:1588-1591` (cf. `find_bound_impl` `:7654`, which passes the live registries at `:7676-7699`) | caller | measured (matrix row 7) |
| G4 | R3's grounding refusal: a declared top-level `PolyType::Generic` input is refused at the combinator standalone check | `check/poly.rs:566-571` | member | brief p2/p3, m1 |
| G5 | The poly pre-pass skip: a member combinator carries no `Bound::User`, is never recorded, and `word_sig_of` misses | `check.rs:837` (rationale block `:822-838`) | member | brief m2/m3 |
| G6 | R1.5, both sides: a generic enum eliminated/constructed inside a combinator splice is refused because "each splice would need its own resolution, and none is recorded" | `check/poly.rs:4151` / `:5611` (text `:10637`) | member | brief m3 |

G5's rationale block in the tree ends "**Revisit the skip if that restriction is
lifted**", naming G4's lift as the trigger. This slice is that revisit.

## Rulings on the brief's open questions

Each of Q1–Q5 is ruled here, not carried forward.

### S3-1 (Q1) — what replaces R1.5: route members onto the existing splice path

**Ruled: an `inline` member is spliced by `inline_combinator`, the same mechanism
`core::cmp`'s `lt` already uses. R1.5's gates are re-sited, not replaced by new
machinery. The one genuinely new table is `splice_enum_words`.**

The earlier reading of R1.5 — "the checker walks a member body exactly once, so a
per-splice record needs a new derivation pass" — is wrong about the tree. A combinator
body is walked **once per call site**, not once per word, and that walk already writes
per-`(uid, span)` records. What members lack is not a derivation pass; it is a *route to
that walk*.

**S3-1.a — path A, the mechanism that already works.** `lib/core/cmp.sth:145`'s
`lt inline ['T: Ord]` splices today through this chain:

| Step | Anchor | What it establishes |
| --- | --- | --- |
| `poly.combinators.get(name)` hit in `check_term`'s Call arm | `check/terms.rs:787` | the call is intercepted before ordinary dispatch |
| `check_poly_combinator_args` | `check/combinators.rs:591` | θ derived **fresh from this call site's live operand stack** (`unify_poly_input`) |
| `let uid = prov.inline_uid; prov.inline_uid += 1` | `check/combinators.rs:503-504` | a **fresh uid per call site** |
| `prov.splice_uid = Some(uid)` | `:510` | inner writes key on this uid |
| `poly.combinator_sig`/`combinator_subst`/`combinator_name` set | `:515-520` | the body's bare member calls resolve at this θ |
| `alpha_rename_locals(comb.terms, uid)` | `:521` | the body's locals are uid-disjoint |
| `check_terms_relaxed(&renamed, …)` | `:543` | **the body walk**, under this uid and this θ |
| restore `splice_uid`/`combinator_*` | `:565-570` | nested splices resolve at their own uid |

Inside that walk, `check_poly_call` writes `splice_records[(uid, span)] = CallInst` (with
its own `enum_words`) at `check/poly.rs:6758`, and a bare member call writes
`splice_trait_calls[(uid, span)] = symbol` at `:1619`.

So multi-instantiation safety is already **structural**: two splices of one combinator at
two θ mint two uids and re-derive θ twice, producing two disjoint `(uid, span)` key
families. No memo, no worklist, no uid-allocation window, no second walk. The prior
draft's `member_splices` / `member_splice_memo` / `MEMBER_SPLICE_UID_BASE` /
`derive_member_splices` design solved a problem the tree does not have, and is deleted
from this spec.

**S3-1.b — why a member never reaches path A: a name-keying miss, not an architecture
divergence.**

- `is_combinator(word)` is exactly `word.declares_inline` (`check/combinators.rs:139-141`).
- `parse_impl_member_body` **does** propagate the trait member's `declares_inline` onto
  the synthesized member `WordDef` (`parser.rs:4107-4112` reads it; both the concrete arm
  at `:4186` and the generic arm below it set it).
- `collect_combinators` keys `poly.combinators` on `word.name`
  (`check/combinators.rs:119-121`), and a member word's `name` is
  `synth_member_word_name(...)` (`parser.rs:1024-1038`, called at `:4128`) —
  `map;Functor;0;Opt`, subsequently module-mangled (`resolve.rs:814`).
- The source call spells `map`.

An `inline` member word is therefore already sitting in `poly.combinators`, under a key no
source call ever spells. `poly.combinators.get("map")` at `terms.rs:787` misses, the term
falls through to `resolve_splice_member_call` / `resolve_mono_member_call`, and both of
those resolve a **symbol** and never call `inline_combinator`. **The `inline` keyword on a
member is currently inert for check-time splicing.** That is the whole of S3's member-axis
gap, and it explains F9 exactly: under m4 the semantics were already right (the correct
`sooth_mono_map_…` monomorph ran), only the splice was missing.

**S3-1.c — the splice-caller hop.** At the point `resolve_splice_member_call` resolves the
symbol (`check/poly.rs:1587-1620`), it has the winning impl and, through
`imp.resolved`, the member word's index `idx`.

- `TraitResolveCtx` (`check/poly.rs:122-131`) gains `words: &'a [WordDef]` — the same
  slice `check.rs:666`'s `overload_symbols` is computed from, and the same one the
  transitive walker already carries as `self.words` (`check/poly.rs:7095`). The member's
  `WordDef` is `words[idx]`.
  **Do not key on the symbol.** `overload_symbols` (`ast.rs:2894-2914`) suffixes `name$$i`
  when two words share a name, so `word_symbols[idx] == words[idx].name` is a coincidence
  of the current corpus, not an invariant. Going through `words[idx]` makes both the
  `declares_inline` test and the `poly.combinators` key derive from one object.
- If `words[idx].declares_inline`, look up `poly.combinators.get(&words[idx].name)` and
  select the entry whose `word` is that same `WordDef` (the vec may hold overloads;
  compare by `span`+`module`, which `synth_member_word_name` already makes unique per
  impl). A `declares_inline` word absent from the map is an internal inconsistency, so it
  is an `expect` on a `collect_combinators` invariant, not a silent fall-through.
- Call `inline_combinator` with the caller's live `stack` and this call's `span`, instead
  of writing `splice_trait_calls[(uid, span)]`. Everything path A guarantees is inherited:
  fresh uid, θ from the live operands, alpha-rename, the full `(uid, span)` record family.
- **Signature cost, named because it is the bulk of the diff.** `resolve_splice_member_call`
  takes `(name, span, stack, ctx, arrays, refs, poly, prov)`; `inline_combinator`
  additionally needs `env`, `cells`, `slices`, `scope`, `granted`, `tail`. Ruling: widen
  `resolve_splice_member_call` to `resolve_mono_member_call`'s existing parameter list
  (`check/poly.rs:1697`, already passed in full at `check/terms.rs:884`), and compute
  `granted`/`tail` at the `terms.rs:842` call site the same way the `poly.combinators` hit
  at `:787-800` computes them. No new information has to be plumbed: it is all live in
  that arm.
- **θ agreement.** `check_poly_combinator_args` derives θ from operands alone; S3-8's
  `find_bound_impl` independently returns the impl's own `Subst`. These must agree on the
  target's variables. Ruling: `inline_combinator` gains `seed: Option<&Subst>`, threaded
  into `check_poly_combinator_args` as the starting substitution (`None` at every existing
  call site, so path A is byte-identical), and a seeded binding that operand unification
  contradicts is a **located error**, not an overwrite. Phase 3 unit-tests that the two
  agree on the goldens' shapes before relying on the seed.

**S3-1.d — the mono-caller hop is a *lowering* change, not the same hop.** Per S3-5, an
`inline` member splices regardless of the caller's inline-ness, so the non-inline poly
caller needs this too. But it is not analogous, and specifying it as "call
`inline_combinator` there too" would be wrong:

- there is no live operand stack at `resolve_user_bound` (`check/poly.rs:7771`). It runs in
  the *post-hoc* transitive fixpoint, over recorded obligations, with grounded slot **types**
  (`site_slots`), not `Slot`s in a caller's walk;
- it already resolves per-θ correctly. `impl_monos.push((word_sym, theta))`
  (`check/poly.rs:8070`, `:8076`) feeds `impl_mono_seed` (`:7086`), which walks the member
  body **per θ** and builds that instantiation's own `enum_words` (`:7158-7167`) inside a
  `CallInst`. The semantics are already right; the monomorph is real.

What is wrong is only that the monomorph is *emitted as a function*. The dedup/emission
stretch at `ir/driver.rs:319-339` (the chain over
`instantiations`/`transitive_instantiations`/`splice_records` at `:327-337`, the
`sort_by` at `:338`, the emitting `for` at `:339`) emits every distinct instantiation
without consulting `combinator_indices` — that filter is applied only to the per-word
pass (`:142`, `:206`). So an `inline` generic-target member mints `sooth_mono_map_…`,
which is precisely the symbol F9 measured under m4.

**Ruling: divert via a pre-pass, not just exclude.** A bare emission-loop exclusion is not sufficient,
because the symbol keys do not meet: `resolve_user_bound` records
`trait_calls[ob.span] = instantiation_symbol(word_sym, θ)` (`check/poly.rs:8082`), while
`lower_resolved_word_call`'s combinator branch looks up `self.combinators.get(sym_name)`
(`ir/func_builder/calls.rs:229`), and `combinator_index` is keyed on `word.name`
(`check/combinators.rs:73`, the name-keyed entry at `:93`; built at `ir/driver.rs:61`). An instantiation symbol is
never a `word.name`, so the lookup misses, and the fall-through ordinary-user-call path
panics (`checked resolved call exists`, its own doc block `calls.rs:174-179`) the moment
the monomorph mints no `IrFunc`. So the reroute is two coordinated changes:

- the classification cannot live in the emission loop itself, because that loop runs
  too late twice over: ordinary concrete words' `FuncBuilder`s are constructed and
  lowered earlier, in the per-word pass at `ir/driver.rs:203`, and the `distinct` list
  is alphabetically sorted (`sort_by`, `:338`), so even a poly caller can be emitted
  before the member instantiation it needs to consult. So a **pre-pass**, placed above
  the lowering env build at `:135` (before both consumers), walks the same
  `instantiations`/`transitive_instantiations`/`splice_records` chain the dedup loop
  walks (`:327-337`), with the name-keyed `poly_words` map (`:273-278`) hoisted above
  it; for every instantiation whose recovered `WordDef` `is_combinator`, it records the
  `(symbol, CallInst)` pair into a new lowering-side map,
  `combinator_instantiations: HashMap<String, CallInst>`, threaded into every
  `FuncBuilder`. The later passes then only read an already-complete map: the `Arity`
  env-insert loop (`:284-314`) and the dedup/emission loop skip every symbol in it, so
  a diverted instantiation mints no `IrFunc` and no env entry;
- `lower_resolved_word_call`, on a `combinators` miss, consults
  `combinator_instantiations.get(sym_name)`. A hit recovers the member through
  `combinators.get(&inst.callee)` — the same name key the emission loop's own
  `poly_words[inst.callee]` indexing already relies on — and splices through the existing
  P7.S8 R1 uid bracket, with that instantiation's own `enum_words`/`trait_calls` installed
  for the splice duration (`ir/func_builder/mod.rs:984` is the install precedent, here
  scoped to the bracket and restored on exit). `member_uid_seeds` (`ir/driver.rs:67`,
  name-keyed) needs no parallel fix on this path: the bracket consults it by the recovered
  `inst.callee`, never by the instantiation symbol.

Why lowering-side recovery rather than the check-side alternative (have
`resolve_user_bound` write the *bare* member symbol into `trait_calls` when the member
`is_combinator`, mirroring the splice-caller path's `word_symbols[idx]` write at
`check/poly.rs:1619`): the bare name would sever the θ link. The splice must install that
instantiation's own `enum_words`/`trait_calls`, and the instantiation symbol is the only
per-θ key lowering has — `trait_calls` is span-keyed but the instantiations it points at
are symbol-keyed, never span-keyed. Lowering-side recovery also keeps `resolve_user_bound`
uniform across combinator and non-combinator members (the check side keeps resolving
symbols; the splice-or-call decision stays at the one place that already owns it,
`calls.rs:229`). The `impl_monos` pushes (`:8070`, `:8076`) are untouched:
`impl_mono_seed`'s per-θ walk (`:7086`, its `enum_words` at `:7158-7167`) is what builds
the very `CallInst` the diversion carries. Both `inst.callee` name lookups — the
pre-pass's classification and the splice point's recovery — share the pre-existing
symbol==name coincidence
S3-1.c documents for members; this ruling does not widen it, and S3-1.c's
two-same-named-words unit test covers the check-side key. The prior draft's open question
(whether the `CallInst` is reachable at the splice point) is answered structurally: the
diversion carries it. What Phase 3 still measures is whether the bracket-scoped
install/restore of the instantiation's tables composes with the uid bracket under nested
member splices.

**S3-1.e — `splice_enum_words`: the one new table.** A poly *call* inside a spliced body
carries its `enum_words` inside its own per-splice `CallInst`, so it is already per-splice.
A **spliced body's own** enum elimination/construction sites have nowhere per-splice to
live. Locating that gap precisely: path A's body walk is `check_terms_relaxed`
(`check/combinators.rs:543`) over concrete `Slot`s — it never enters the abstract walk's
`poly_eliminator_call`/`poly_construct_generic`, so the R1.5 raise sites
(`check/poly.rs:4151`/`:5611`, which read `TraitCtx` and have no `Provenance` in scope)
fire only in the pre-pass's abstract walk (S3-1.f) and **cannot** be the write site. In
the concrete walk the per-splice *resolution* is already right — each splice's θ-concrete
operands pick (or mint) the right instantiation — the gap is what gets **recorded for
lowering**:

- **construction and destructure**: the generated-word candidate arm in `check_term`
  resolves the mangled per-instantiation word and records
  `builtin_overloads[span] = symbol` — `check/terms.rs:926` (multi-candidate) and `:914`
  (single-candidate, which records only when the name is also a combinator's). Span-keyed:
  two splices at two θ collide last-write-wins, or worse — the first splice (one candidate,
  no record) is served by the bare-key `enums.words` lookup, which is last-write-wins
  across a family's monomorphs (`ir/func_builder/calls.rs:780-800`);
- **elimination**: the eliminator arm (`check/terms.rs:625-655`) reads the operative
  `EnumId` off the live scrutinee (`scrutinee_enum_id_of_family`, then
  `check_eliminator_call`, `check.rs:2294`) and records **nothing** for lowering — a
  concrete body never needed a record, but under a splice the bare-key fall-through hands
  every splice the same family-wide id.

That, and nothing more exotic, is R1.5's representation limit. So:

- Add `Module.splice_enum_words: HashMap<(u32, Span), EnumId>` (`ast.rs`, beside
  `splice_records` `:89` / `splice_trait_calls` `:99`, same key shape, same relay — S3-12).
- It is written **during the path-A walk, in the concrete walk's own arms**, gated on
  `prov.splice_uid == Some(uid)` **and on the chosen candidate being a generated enum
  word**: the candidate arm writes `(uid, span) -> EnumId` (the chosen candidate's enum
  identity read off its generated sig, the way the nullary-variant detection at
  `check/terms.rs:1054` already reads a ctor's output) **instead of** the span-keyed
  `builtin_overloads` insert — a redirect, mirroring `check_poly_call`'s
  `splice_records` redirect at `check/poly.rs:6746`; leaving the span-keyed write in
  place would let `lower_call`'s earlier `builtin_overloads` arm serve a stale symbol
  before the enum-word arm is ever reached — and `check_eliminator_call` writes the
  operative id under the same gate. The enum-word condition is **membership, not sig
  shape**: an `Overload` carries only `sig` + `symbol` (`check/builtins.rs:37`) and a
  user word's sig can also output an enum, so the discriminator is whether
  `(name, chosen.symbol)` appears in `enum_generated_sigs` (`check/declarations.rs:1852`,
  constructors) or `variant_generated_sigs` (`:1885`, destructures) over the extended
  type slices — the same source `mint_fallback_candidates` already reads
  (`check/terms.rs:1362`). The two non-enum cases the `:914`/`:926` inserts serve keep
  them **unchanged even inside a splice**: the Slice-10c single-candidate record at
  `:914` (forcing a real call when the name is also a combinator's) and the D7/R5
  generic-struct ctor/accessor symbols at `:926` (consumed at
  `ir/func_builder/calls.rs:370-378`); stripping either inside a splice would regress a
  mechanism that is not enum-specific. No standalone pass
  populates it, and the abstract-walk raise sites are untouched (they keep firing for the
  pre-pass case S3-1.f leaves them).
- `lower_call`'s enum-word arm (`ir/func_builder/calls.rs:787`) consults
  `splice_enum_words[(splice_uid_stack.last(), span)]` **before** the span-keyed
  `self.enum_words`, falling through to it and then to the bare-key monomorphic path on a
  miss — the same three-step shape the arm already documents.

A parallel map is chosen over widening `splice_trait_calls`' value to a struct: that value
type is threaded through the ~30 sites S3-12 censuses, none of which care.

**S3-1.f — R1.5's gates are re-sited, not deleted.** `is_combinator_splice` is set in
**exactly one place**: `check.rs:865`, in the poly pre-pass's one-time walk (verified by
grep: the only other mentions are the field `check/poly.rs:56`, its `false` default `:86`,
and the two reads `:4150`/`:5610`). It is a *definition-time pre-emptive rejection*, not a
per-splice guard — the pre-pass's Span-keyed `enum_sites` cannot express "this resolves
differently per splice", so the pre-pass refuses the shape outright.

Once a member routes through path A, its body is re-walked per splice with a real θ, and a
definition-time refusal is the wrong gate for it. Ruling, in order of preference and to be
settled by measurement in Phase 3's first step:

1. **Preferred:** the pre-pass keeps walking member combinator bodies (S3-10 puts them
   there) but sets `is_combinator_splice: false` for a `word.is_trait_member` combinator,
   so the walk stays a soundness check and stops raising R1.5 on a body that will be
   re-walked per splice. The gate is unchanged for every non-member combinator, where the
   pre-pass walk *is* the only walk.
2. **Fallback**, if (1) leaves the pre-pass walk unable to complete on an ungrounded member
   body: skip the pre-pass walk for `is_trait_member` combinators the way S3-9/S3-10 already
   narrow other checks by that marker, and record the skip as a ledger item.

Either way the gates keep firing for their surviving case: a non-member combinator body, and
the standalone check, where θ is the stand-in and the enum's own arguments stay unground.
Their builders and the `:10637` doc block stay, with the text re-pointed.

Pin census (verified by grep, not prose): exactly one test pins either text —
`tests/phase7_slice12.rs:235` (`err.contains("this combinator's own splice determines")`,
the construction side). It is **migrated, not retired**: its fixture
(`: wrap_it inline ['T: Foo] ( 'T -- Pair['T] ) bar One ;`) uses a non-member combinator,
so under ruling (1) it still rejects unchanged and the test **stays as-is**; only under
ruling (2) would it need revisiting. Phase 3 records which. The elimination side has no pin
at all today, which is itself worth recording.

### S3-2 (Q2) — gate G5's discriminator: a structural marker, not a name convention

**Ruled: `WordDef::is_trait_member: bool`, set where the member word is synthesized.**

m3's `word.name.contains(';')` is the right *mechanism* with the wrong *key*. The `;`
convention is already load-bearing in two places (`resolve.rs:120`, `check/poly.rs:1675`)
and re-deriving it a third time by string search would make a synthesized-name detail
into a checker predicate. The information is known at desugar time: `parser.rs:4128`
calls `synth_member_word_name` and builds the word.

- `WordDef` (`ast.rs:1785`) gains `pub is_trait_member: bool`, set `true` only at that
  synthesis site and `false` everywhere else.
- G5's skip becomes
  `if is_combinator(word) && !word.is_trait_member && !sig.bounds.iter().any(User)`.
  Byte-identical behaviour for every non-member combinator, so the measured-red widening
  the tree rejects (`sig.bounds.is_empty()`, 40+ tests) is not approached.
- The same marker narrows G4 (S3-3), so one discriminator serves both member-axis gates.

A bool is chosen over a richer `member_of: Option<(TraitId, …)>`: nothing in this slice
reads more than the predicate, and the trait/member identity is already recoverable at
every site that needs it.

### S3-3 (Q5) — the non-HKT case is in scope, and is the phase-2 verification vehicle

**Ruled: in scope, not by generosity but because excluding it would cost extra code.**

The brief's p3 shows an ordinary Star-kind trait with a generic impl target
(`impl: Sized for Box['T]`) dies at the *identical* G4 error. Every gate above keys on
"the impl target is generic", never on the header's kind; fencing HKT-only would mean
adding an Arrow-kind condition to gates that do not have one — a fake fence around a
path that already works.

More usefully, p3's body (`: size drop 1 ;`) eliminates **no** enum, so it exercises the
caller-side gates (G1–G3) and the member-axis gates (G4/G5) *without* touching R1.5.
That makes it the only shape that can witness a real splice before S3-1 lands, so
Phase 2 uses it as its exit witness
rather than only as a golden.

### S3-4 (Q3) — S3 does not wait for P7b.S4

**Ruled: proceed on fixture twins.** p6 confirms S4's module-identity gap is live (a real
`core::option` ctor impl gets "no `impl:` in this program dispatches on these operands"),
and S2 set the twin precedent for W3/W4 for exactly this reason. Everything S3 must prove
— splice, no frame, no runtime dispatch — is fully observable on twins via `nm`/`objdump`.
The phase doc's real-`Option`/`Result`/`List` dogfood stays where it sits, dependent on
S4.

### S3-5 (Q5) — fences kept

- **The struct route to `Functor` stays closed.** p5/p5b: `Box>` on a generic type is
  rejected identically with and without `inline`. Pre-existing, not S3's; a golden pins
  that it is unchanged, so a future reader does not mistake it for S3 breakage.
- **S4's module identity is not fixed here** (S3-4).
- **Caller inline-ness does not gate splicing, and S3 does not make it one.** Splicing is
  decided by the *member's* own `inline` keyword: matrix rows 1 and 2 already show a
  concrete-target `inline` member splicing from an inline caller *and* from a non-inline
  one. What S3 fixes is the *target*-genericity axis, for every caller flavour. A
  **non-inline** member still monomorphizes, from every caller (rows 5–8) — that is the
  fence, and control C#1 pins it.

## Requirements

### The caller axis (Phase 2)

**S3-6 (G1) — an Arrow-kinded stand-in for the standalone check.** The standalone check
seeds `subst.ty.push((v, Type::I64))` for every variable (`check/poly.rs:556`). For a
variable of Arrow kind that is nonsense: `i64` is not a constructor, and the App arm of
`apply_subst` rightly refuses it. Ruling: **seed an Arrow-kinded variable with a
`Type::CtorImage(g)`** (the S1-12 representation, already the thing `unify_poly_input`'s
App arm binds), where `g` is the constructor target of the **first** `impl:` of the
variable's user bound in declaration order. The stand-in then grounds `'F['T 'E]` to a
real monomorph (`Opt[i64 i64]`) and the body is checked against a representative
instance — stronger than the `i64` stand-in, not weaker, and deterministic.

**Rescue, narrowed.** The stand-in is a *representative*, not a proof obligation: a word
whose body is fine at some other impl's constructor must not be rejected because the first
impl in declaration order does not fit it. But degrading **any** standalone failure to a
skip would disable the body check for exactly the words this slice introduces — the
skip-guard failure mode this spec rejects elsewhere. Ruling: the rescue fires only for the
two failure classes the stand-in's arbitrariness can cause:

- a **grounding** failure (`apply_subst`/`try_ground_member_type` returns `None`, or an
  unground-variable refusal) against the stand-in's constructor;
- an **operand type mismatch** whose reported types mention the stand-in's constructor.

Every other standalone failure — arity/underflow, linearity, an unknown word, a borrow or
move violation, a bound not discharged — is impl-independent and stays a **hard error**.
Phase 2 implements this by tagging the raise sites rather than by matching on message text.
(A rescued word is not silently accepted: it is checked at every splice site, the same
mechanism E#1 pins, so a genuinely broken body fails at its first splice.)

Fallback when the bound has no impl in the program (or the variable is Arrow-kinded with
no user bound): the word's standalone check is **skipped**. This is a real hole and is
named as one — such a word is checked only at its splice sites, and if it is never
spliced it is never checked. It is the same trade the pre-existing G5 skip makes, and it
is unreachable for any word that is actually used. Of the fallback's two triggers, only
the no-user-bound one is witnessable (a word whose bound has no impl can never be spliced
at all — no operand can discharge the bound — so nothing can observe its skip); error
golden E#1 pins that half: a broken body behind an unbounded Arrow variable is still
rejected at its first splice site.

**S3-7 (G2) — delete `splice_member_hkt_error`.** With S3-6 and S3-8 in place, an
App-headed member signature *does* have a grounding on the splice path: the caller's θ
binds the head variable to a `CtorImage`, and S2-6's leading-slot grounding is the rule
that grounds it (`ast.rs:2242`, App arm `:2296`). The splice path stops grounding member
slots via `try_ground_member_type` at a single `Type` (`check/poly.rs:1527-1544`) and
grounds them through the caller's θ instead, the same way `resolve_user_bound` does at
its re-grounding step. The builder at `:1633` and the guard at `:1514` are removed, and **both**
candidate-disambiguation skips that exist only to route around them go with them:

- the App-headed skip at `:1411-1417` (`member_ty_mentions_app` → `continue`);
- the `try_ground_member_type`-returns-`None` skip at `:1426-1431`, which fires for
  exactly the bare-`Var`/`CtorImage` slot S3-6 and S2-6's leading-slot grounding now
  ground.

Both are the same permissive-fixture failure mode: a `continue` that silently drops the
now-fittable candidate would leave a two-candidate program dispatching to the wrong impl
(or reporting "no candidate fits") on precisely the shapes this slice adds. Each is
replaced by grounding the candidate through the caller's θ and letting the operand match
decide, so the disambiguation loop and the single-candidate path use one grounding rule.

**When θ-grounding itself fails.** Removing the two `continue`s removes the tree's
panic-avoidance route, so the replacement must say what happens when the caller's θ still
does not ground a candidate's slot. It is a **located error, never a panic and never a
silent skip**:

- in the multi-candidate loop, an ungroundable candidate is recorded with its reason and
  excluded from `fitting`. If `fitting` ends empty, the error names every candidate and why
  each failed to ground — strictly more information than today's "no candidate fits";
- on the single-candidate path, the failure raises immediately at the call span, naming the
  member, the trait and the variable θ left unbound. `splice_member_ctor_image_error`
  (`:1647`) is the existing builder for the `CtorImage`-shaped case and is reused; the
  bare-unbound case gets a sibling message.

`ground_member_type`'s `unreachable!` arms must therefore stay unreachable by construction,
not by a `continue` upstream: Phase 2 asserts this with a unit test per arm.

`splice_member_ctor_image_error` (`:1647`) is **narrowed, not deleted**. Its `ground_slots`
closure (`:1524-1544`) stops being the grounding path for member slots (S2-6's θ-grounding
replaces it), but the error stays reachable for the case it actually describes: a splice
whose bound variable carries a bare `CtorImage` that θ does *not* complete — the
instantiate-at-the-constructor-alone mistake its own text names. If Phase 2 finds that
case unreachable once θ-grounding lands, it joins the deletion list and its message goes
with it; that ruling is a Phase 2 exit item, not an open question carried past the slice.

Its unit test (`check/poly.rs:11552`) is **migrated**: its own doc block says it pins only
"the body-check half", i.e. that a missing-operand member call is a located error rather
than a panic. That claim survives S3 unchanged and the test keeps it; the paragraph
explaining why the guard is source-unreachable is replaced by a pointer here.

**S3-8 (G3) — route the splice path's impl lookup through `find_bound_impl`.**

`resolve_splice_member_call`'s impl lookup (`check/poly.rs:1588-1591`) is a bare
`impls.iter().find(...)` calling `match_impl_target(&i.target.pattern, ty, &[], &[], &[],
None)`: empty arrays/cells/refs registries and `generics: None`. A `Generic` target pattern
cannot match a monomorph operand through it, because the operand is a
`Type::Struct`/`Type::Enum` id and recovering that id's header identity is exactly what the
live `GenericTypes` registry is for. That is matrix row 7's rejection, verbatim:
`` `Box[i64]` does not satisfy `Sized` `` from a lookup structurally unable to see that it
does.

**Not** the S2-8 selector. Sharing `resolve_user_bound`'s candidate-selection block
(`:7795-7900`) is inert here: that selector is gated on CtorImage identity and never reads
its `generics` parameter on the path row 7 takes, so extracting it would move code without
moving behaviour. Ruling: **the splice path calls `find_bound_impl` (`check/poly.rs:7654`,
candidate collection `:7676-7699`) instead of its own `find`**, passing `trait_id`, the
ground operand `ty`, `source_impl_idx: None`, the call `span`, `ctx`, `poly.trait_resolve`,
the live `arrays`/`cells`/`refs` slices and a fresh `visited` vector.

Why that flips row 7, by reasoning (Phase 2 measures it, this spec does not claim it
measured): `find_bound_impl` builds its candidate list with
`match_impl_target(&i.target.pattern, ty, arrays, cells, refs, generics.as_deref())` where
`generics = ctx.generics()` — the same live registry `resolve_user_bound` and
`resolve_mono_member_call` already pass, and the same call that succeeds for the *non*-inline
caller on row 8. Row 7 differs from row 8 only in which resolver runs; feeding the working
resolver the same operand is what makes the two rows agree. `find_bound_impl` additionally
brings what the bare `find` lacks and the splice path silently wanted: R6 bound discharge
of the candidate's own `where` bounds, R7 cycle detection, and R3 most-specific selection,
so a program with two fitting impls dispatches identically whether or not the caller is
`inline`. A first-match `find` on the splice path is the divergence hazard; this removes
it.

The return shape changes from `&ImplDecl` to `Option<(usize, Subst)>`: the index recovers
the `ImplDecl` for the existing `resolved`-symbol lookup, and the `Subst` is the impl's own
substitution, which S3-1.c needs anyway as `inline_combinator`'s `seed`. `None` keeps the existing
`unsatisfied_user_bound_error`; an `Err` (cycle) propagates.

### The member axis (Phase 3)

**S3-9 (G4) — narrow R3's refusal to non-member words.** `check/poly.rs:566-571` refuses
any top-level `PolyType::Generic { .. } | GenericVariant { .. }` input. A synthesized
member word's inputs are *always* that shape once the target is generic (S2's bare-ctor
desugar). Ruling: the refusal is skipped when `word.is_trait_member` (S3-2), and is
unchanged for every other word. m1 measured that the stand-in grounding is sufficient for
these body shapes once the refusal is dropped; S3-6 makes the stand-in better still.

The three integration pins and one unit pin on R3's text
(`tests/phase7_slice11.rs:131`, `tests/phase7_slice12.rs:671`, `check/poly.rs:19368`, plus
the `:248` doc reference) all use non-member combinators and are **unaffected**; a phase-3
step re-runs them to confirm, rather than assuming.

**S3-10 (G5) — record member combinators in the poly pre-pass**, per S3-2's marker, so
`PolyCtx::recorded` carries their `WordObligations` and `word_sig_of`
(`check/poly.rs:181`) hits. m2 localized the miss to the `tr.word_sig_of(word_sym)` call
at `:7981`; that is the site that stops erroring.

**S3-11 (G6) — R1.5 re-sited, not replaced by new machinery**, per S3-1 in full: members
route onto `inline_combinator` (S3-1.c/.d), the body's own enum sites gain
`splice_enum_words` (S3-1.e), and `is_combinator_splice`'s single set site is narrowed
(S3-1.f).

### Cross-cutting

**S3-12 — forced-arm and consumer inventory.** `Module` gains **one** map
(`splice_enum_words`, S3-1.e), `WordDef` gains a field (S3-2), and `TraitResolveCtx` gains
a `words` slice (S3-1.c). None is a `match` variant, so **rustc will not find the
readers**: the relay is grep-driven. The census below is the live
`splice_records`/`splice_trait_calls` site list (one grep over `src/`), and every hop needs
a `splice_enum_words` companion unless the row says otherwise. A map that is built but never
relayed is exactly the failure mode F9 describes: green tests, no splice.

The prior draft's second map (`member_splice_uids`) is gone with the derive pass: under
S3-1 a member's splice records are written under a uid `inline_combinator` already minted,
which lowering already finds through `splice_uid_stack`.

| Hop | Sites |
| --- | --- |
| Declaration | `ast.rs:89` / `:99` |
| `Module` literal construction | `ast.rs:4010`, `:4164`, `:4242`; `driver.rs:806`; `resolve.rs:1827`; `parser.rs:1555`; `check/declarations.rs:2423`, `:2494` |
| Empty statics | `ir.rs:66` (`empty_splice_records`), `:75` (`empty_splice_trait_calls`) — add an `empty_splice_enum_words` companion |
| Destructor pass | `ir/destructors.rs:387-388` passes the empty statics; add the companion |
| Checker locals → `Module` | `check.rs:767`, `:772` (declare), `:985-986` (`PolyCtx` wiring), `:949-950` (scratch twins), `:1093-1094` (store); `PolyCtx` fields `:169`, `:175`; destructuring `:707-708`; `check/engine.rs:1227-1229` |
| Explicit `&mut` params through the fixpoint | `check/poly.rs:6850`, `:6917`, `:6926`, `:6945`, `:7002`, `:7028`, plus the caller at **`check.rs:1077`** (`discover_transitive_instantiations` takes `&mut splice_records` as a parameter, not through `PolyCtx`). **No `splice_enum_words` companion here.** The fixpoint threads `splice_records` because it *walks* them, grounding each `CallInst`'s callee into further monomorphs; `splice_enum_words` is terminal data (`(uid, span) -> EnumId`) that mints nothing and is walked by nothing. Phase 3 confirms this by grep before deciding the omission is safe, and records the confirmation. |
| Checker writes | `check/poly.rs:1619` (`splice_trait_calls`), `:6758` (`splice_records`); `splice_enum_words` (S3-1.e): the generated-word candidate arm's redirected record (`check/terms.rs:914`/`:926`) and `check_eliminator_call`'s operative-id recovery (`check.rs:2294`) |
| `Module` → `ir` | `ir/driver.rs:248-249`, `:392-393` (pass-down), `:331` (`splice_records.values()` feeds the distinct-instantiation chain — `splice_enum_words` has no analogue; record the *absence* deliberately) |
| Instantiation emission (S3-1.d) | `ir/driver.rs:319-339` — the dedup/emission stretch applies no `combinator_indices` filter (`:142`, `:206` apply it to the per-word pass only); S3-1.d's pre-pass (above `:135`) diverts an `is_combinator` instantiation into `combinator_instantiations` up front, and the `Arity` env-insert loop (`:284-314`) and emission loop skip its symbol |
| `FuncBuilder` | `ir/func_builder/mod.rs:379`, `:385` (fields), `:488-489` (empty defaults), `:956-957`, `:989-990`, `:1134-1135`, `:1161-1162`, `:1211-1212` (threading); S3-1.d threads `combinator_instantiations` alongside |
| Reads | `ir/func_builder/calls.rs:366` (`splice_trait_calls`), `:435` (`splice_records`), `:787` (`enum_words`, S3-1.e's insertion point), `:229` (`lower_resolved_word_call`, the S3-1.d splice bracket) |
| Test-facing invariant | `ir/driver.rs:734-761` — the P7.S8 R1/R2 unit test measures a member's seed off `module.splice_trait_calls`' uids with a `uid / STRIDE == idx` filter. S3-1 allocates **no** uids outside `prov.inline_uid`'s existing sequence, so the filter's premise is untouched; re-run it as a guard, not as a change. |

**S3-13 (Q4) — the golden assertion contract.**

Goldens are numbered in one scheme throughout this spec: **P#n** positive, **C#n**
control, **E#n** error. Every S3 **positive** golden asserts all of:

1. stdout and exit code (necessary, never sufficient);
2. `nm`: the binary contains **no** `<member>.3b.<Trait>` mangled member symbol **and**
   no `sooth_mono_<member>_<Trait>` monomorph symbol. Both patterns are required: the
   matrix above shows a generic-target member mints the *monomorph* form
   (`sooth_mono_size_Sized_0_Box__T0___m0__t0_i64`), so an nm assertion written only
   against the `.3b.` form is vacuous for exactly the shapes this slice is about;
3. `objdump`: zero call edges to either symbol pattern, reachable from `sooth_main` in
   the call graph;
4. a **non-inline twin control in the same test**, byte-identical but for the `inline`
   keyword, asserting the symbol **is** present and the call edges **are** there;
5. when the **caller word itself declares `inline`**, the caller's own frame must be
   absent too. Applicability, enumerated golden by golden: **applies to P#1, P#3 and P#4**
   (the splice goldens whose caller carries the keyword). **Exempt: P#2** (a
   monomorphization golden — it asserts the monomorph symbol's *presence*, so a
   caller-frame-absence clause would contradict its point), **P#5** (deliberately a
   non-inline caller; its frame being present is the golden's point) and **P#6** (a
   rejection golden — no binary, no caller-frame semantics). A golden that
   only checks the member's absence passes just as well when the member was spliced into a
   caller that was itself emitted as a called function — a frame the exit criterion's "no
   call frame" clause does not allow, and which no member-side assertion can see.

   Concretely, for a caller word `W` in a `single_file_hosted` fixture (module 0), the
   symbol is `mangle("W", 0)` = **`W__m0`** (`resolve.rs:36-41`; only `main` and `drop` are
   exempt). The two assertions:

   ```text
   nm <binary>                       # no line whose symbol field == "W__m0"
   objdump -d <binary>               # via call_graph(): no vec in the graph contains "W__m0"
   ```

   Use `nm`'s whitespace-split symbol field, not `contains("W__m0")`: a substring test is
   satisfied by a typo and by any longer symbol that embeds the name. The `objdump` half
   goes through the relocated `call_graph` helper, which keys edges by caller symbol and
   reads targets out of objdump's `<...>` annotations, so it needs no calling-convention
   knowledge. A positive control in the same test (clause 4's non-inline twin) must show
   `W__m0` **present** in both, or the assertion is a placebo.

Clause 4 is not belt-and-braces. Without it, (2) and (3) are assertions that a string is
absent from a binary, which a typo in the mangling pattern satisfies just as well as a
successful splice, and which the standing MEMORY note about placebo `nm` assertions
records as a live failure mode in this repo. m4 is the concrete adversary: it passes (1)
and would pass a mis-written (2).

Tooling: `nm` per `tests/phase4_slice11_inline.rs:84`; the `call_graph` helper at
`tests/phase7_slice3s_oracle.rs:67` gains a second consumer, so it **moves to
`tests/common/mod.rs`** (elevate rule) and the oracle test imports it from there.

## Considered and rejected

- **Delete the three brief gates and ship** (the m4 reading). Rejected by F9: correct
  output, identical symbols, identical call counts, criterion unmet, and R1.5's hazard
  untested rather than disproven.
- **A second, standalone derivation pass** — a `member_splices` worklist keyed by
  `instantiation_symbol(word, θ)`, drained to fixpoint before
  `discover_transitive_instantiations`, re-walking each member body via `check_poly_body`
  at a uid allocated above `words.len() * INLINE_UID_STRIDE`. This was this spec's own
  earlier ruling and it is **rejected on inspection**: it rests on the premise that a
  combinator body is walked once per *word*, which is false (`inline_combinator` walks it
  once per *call site*, `check/combinators.rs:503-543`), and it is unimplementable as
  written — `check_poly_body` (`check/poly.rs:745-762`) takes no `Provenance`, no
  `PolyCtx` and no ground `Subst`, so it structurally cannot write
  `splice_records`/`splice_trait_calls`/`splice_enum_words` at all. S3-1 routes to the
  existing walk instead; the memo, the worklist, the uid window and the `generics_cell`
  open question all go with it.
- **Widen `splice_trait_calls`' key or value shape** to carry the per-splice enum
  resolution. Rejected in favour of a parallel `(u32, Span)`-keyed map (S3-1.e): the
  existing value type is threaded through ~20 signatures that have no interest in it, and
  the tree's own precedent is one map per concern at a shared key.
- **Passing the live registries to the splice path's existing bare `find`** (G3's minimal
  fix). Rejected: the bare `find` is a first-match with no bound discharge, no cycle check
  and no most-specific rule, so a program with two fitting impls would dispatch differently
  depending on whether the caller was `inline` — a silent divergence. S3-8 routes through
  `find_bound_impl` instead.
- **Extracting and sharing S2-8's candidate selector** (this spec's own earlier ruling for
  G3). Rejected on inspection: that selector is gated on CtorImage identity and never reads
  its `generics` parameter on the path row 7 takes, so the extraction is inert for G3 — a
  refactor wearing a fix's name.
- **`word.name.contains(';')` as G5's discriminator** (m3's expedient). Rejected: a
  synthesized-name detail promoted to a checker predicate. S3-2 uses a field.
- **Widening G5's skip to `sig.bounds.is_empty()`.** Rejected by the tree's own
  measurement (40+ tests red, `lib/core.sth`'s Copy-bounded loop combinators).
- **Fencing S3 to HKT traits only.** Rejected (S3-3): every gate keys on target
  genericity, so an HKT-only fence is extra code around a working path.
- **Keeping the `i64` stand-in and skipping App-headed slots at the standalone check.**
  Rejected: a skip-guard that disables the body check for exactly the shapes this slice
  introduces. S3-6 seeds a real constructor instead, and confines the skip to the
  no-impl-exists case where nothing is callable anyway.

## Deliberate limitations (ledger)

1. **A non-inline member still monomorphizes, from every caller flavour.** Splicing is
   member-inline-gated, never caller-inline-gated: an `inline`-declared member splices
   whether its caller is mono, non-inline poly, or inline (rows 1/2 measure this today for
   a concrete target, and S3 extends it to generic targets), and a member without the
   keyword monomorphizes in all three cases. Control C#1 asserts the monomorph symbol is
   still minted for the non-inline member, which is also what makes the positive goldens'
   absence assertions meaningful.
2. **An HKT-bounded inline word whose trait has no impl in the program is not checked
   standalone** (S3-6's fallback, which also covers an Arrow-kinded variable with no user
   bound). Checked at its splice sites; never spliced means never checked. The no-impl
   half is **unwitnessable by construction** (no impl means no call site can discharge
   the bound, so the word is never spliced); error golden E#1 pins the witnessable
   no-user-bound half at its first splice site.
3. **Fixture twins, not `core::option`/`core::result`** (S3-4) — P7b.S4.
4. **The struct route to `Functor` stays closed** (S3-5) — `Box>` on a generic type,
   pre-existing and identical with or without `inline`.
5. **The S2-8 tie rule's pin metric still counts top-level pins only.** S3-8 no longer
   touches that selector at all (it routes the splice path through `find_bound_impl`
   instead), so the limitation is untouched: neither worsened nor fixed.
6. **Bound dispatch inside a materialized quotation stays rejected** (P7.S3o R5,
   `check/poly.rs:1488`): a materialized quotation lowers to its own `IrFunc` with an
   empty `splice_uid_stack`, so no `(uid, span)` key resolves. S3 widens the splice path,
   so this fence gets a regression golden (E#2) rather than an assumption.

## Phased delivery plan

Each phase is independently verifiable: its goldens pass and its new stage code carries
unit coverage before it is done (CLAUDE.md). Green =
`cargo fmt --check && cargo clippy -- -D warnings && cargo test`. Baseline at HEAD is
3044 passing.

Phases are ordered so that **each one's exit is observable on its own**. That constraint
is what puts the caller axis before the member axis: the p3 shape (S3-3) splices for real
at the end of Phase 2, with no enum elimination anywhere, so Phase 2 can be verified by
the F9 assertion contract instead of by an intermediate error message.

### Phase 1 — The marker, and a measured baseline

S3-2 (`WordDef::is_trait_member`, set at `parser.rs:4128`, read nowhere yet) and S3-13's
tooling move (`call_graph` → `tests/common/mod.rs`, oracle test re-pointed).

- **Unit tests:** the synthesized member word carries the marker and an ordinary word does
  not; a member word declared `inline` carries both the marker and `declares_inline`.
- **Verifiable:** full suite green and byte-identical behaviour (the marker has no
  readers yet); the oracle test still passes against the relocated helper.
- **Measurement, recorded in the phase's commit message:** the 2×2×2 matrix above re-run
  as a scratch harness, so the later phases' claims of flipping a cell have a baseline
  that is not this document.

### Phase 2 — The splice-caller path (G1, G2, G3) + G4, G5

S3-6 (Arrow-kinded stand-in), S3-7 (delete `splice_member_hkt_error`, ground member slots
through θ), S3-8 (splice-path impl lookup routed through `find_bound_impl`), S3-9 (R3
narrowed by the marker), S3-10
(pre-pass records member combinators).

G4/G5 land here rather than in a member-axis phase of their own because the p3 witness
needs all five: without G4/G5 the member word never gets a signature to dispatch to, and
an intermediate phase boundary between them can only be verified by an error message.

- **Unit tests:** the standalone stand-in binds an Arrow-kinded variable to a `CtorImage`
  and to the *first* impl's constructor (declaration order, pinned); the no-impl fallback
  and the failed-stand-in rescue both skip the standalone check without erroring; the
  splice path and `resolve_user_bound` pick the *same* impl for a two-fitting-impl program
  (S3-8's `find_bound_impl` routing — a first-match `find` would pick the other one for one
  of the two callers); a candidate whose `where` bound fails to discharge is not selected on
  the splice path; `word_sig_of` hits for a recorded member combinator; a non-rescuable
  standalone failure against the stand-in (an arity error) stays a hard error and is not
  rescued (S3-6's rescue fires only for the two stand-in-caused failure classes).
- **Golden P#1 (matrix row 3), the full S3-13 contract and the phase's exit witness:**
  `inline_member_splices_into_an_inline_bound_caller` — p3's shape (`trait: Sized`,
  `impl: Sized for Box['T]`, `size inline`, `usesize inline` caller). Non-HKT witness
  *and* the phase's exit proof; clauses 1–5 all apply (no caller frame either). The
  matrix-row-4 twin (the **non-inline** caller) is *not* a Phase 2 golden: its
  no-monomorph clause depends on S3-1.d's diversion, which is Phase 3 work, so it lands
  there as P#5. Why P#1 is exit-provable *before* S3-1.c/S3-1.d land: p3's member body
  is degenerate — `drop 1`, no enum sites, no nested generic call, nothing θ-directed —
  so the pre-existing machinery already carries it end-to-end: the splice-caller hop
  still writes `splice_trait_calls[(uid, span)] = symbol` (`check/poly.rs:1619`),
  lowering's `(uid, span)` read (`calls.rs:366`) hands that symbol to the symbol-keyed
  member splice bracket (`calls.rs:229`; the member is already in lowering's
  `combinators` because `is_combinator` is just `declares_inline`), and this path
  records **no instantiation**, so no monomorph is minted and clause 2's absence holds
  without S3-1.d. The hard case S3-1.c/S3-1.d exist for — a body with enum sites or
  per-θ tables — is exactly what Phase 3's mutation check 2 shows failing on P#3/P#4.
  This reasoning is from reading the source, not from a run: Phase 2's opening golden
  run confirms it, and if any clause fails at phase entry that is recorded as the
  phase's opening measurement, not treated as a surprise.
- **Golden P#2 (matrix row 7):** `non_inline_member_on_a_generic_target_from_an_inline_caller`
  — the row S3-8 exists for and the one no golden currently covers: `size` **without**
  `inline`, `impl: Sized for Box['T]`, `usesize inline`. Today it is rejected
  (`` `Box[i64]` does not satisfy `Sized` ``); after S3-8 it must *accept, run, and
  monomorphize* — so its assertions are clause 1 plus the **presence** of
  `sooth_mono_size_Sized_0_Box…`, matching row 8's non-inline-caller behaviour exactly. It
  is the direct test that S3-8 removed a resolver divergence rather than adding a splice.
- **Verifiable:** R3's three integration pins and one unit pin re-run and unchanged
  (S3-9); the S2 suite (`tests/phase7b_slice2.rs` 17/17) and `tests/phase7b_slice1.rs`
  16/16 green; the HKT exit shape now fails at R1.5 rather than at G1 (a pinned
  intermediate located error, never a panic).

### Phase 3 — Routing members onto the splice path (G6)

S3-1 in full: the `TraitResolveCtx::words` slice and the splice-caller hop (S3-1.c), the
instantiation-emission filter and its lowering-side splice (S3-1.d), `splice_enum_words`
and its relay (S3-1.e, S3-12), and R1.5's re-siting (S3-1.f).

Phase 3 opens with **two measurements**, both of which can change the shape below:

1. **S3-1.d's install bracket:** the diversion carries the instantiation's `CallInst` to
   the splice point by construction, so the old reachability question is answered; what
   remains measured is whether the bracket-scoped install/restore of that instantiation's
   `enum_words`/`trait_calls` composes with the P7.S8 uid bracket under *nested* member
   splices. If it does not, the bracket plumbing is the phase's largest single item and it
   lands before anything else.
2. **S3-1.f's ruling:** does the pre-pass walk of a member combinator body complete with
   `is_combinator_splice: false` (preferred), or must the walk be skipped for
   `is_trait_member` combinators (fallback)? Measured, then recorded in the spec's closing
   section.

- **Unit tests:** a member call at a splice site is intercepted by `inline_combinator`
  rather than resolving a symbol (`splice_trait_calls` gains **no** entry for it); the
  combinator entry is found by the member's synth name and not by `word_symbols[idx]`
  (a fixture with two same-named words, so `overload_symbols`' `$$` suffixing makes the
  two keys differ — the test fails if the lookup keys on the symbol); the impl `Subst`
  seed and the operand-derived θ agree, and a contradiction is a located error; a spliced
  body's generic-enum construction writes `(uid, span) -> EnumId` into
  `splice_enum_words` and leaves the span-keyed `builtin_overloads` without an entry for
  that site (the redirect); a spliced elimination records the operative id under the same
  key; two splices of one body at two θ record two *different* `EnumId`s for the same span
  (the assertion moved out of P#4's golden, which cannot see in-process `Module` state);
  lowering prefers the per-splice entry and falls through to `enum_words` and then to the
  bare key (all three arms); an `inline` member's instantiation is absent from the
  emission loop's `distinct` list and present in `combinator_instantiations`.
- **Golden P#3:** `hkt_member_splices_through_an_inline_bound_caller` — the exit criterion,
  p4's `Opt`/`Res` program with `twice inline`. Full S3-13 contract, clauses 1–5.
- **Golden P#4:** `two_splices_of_one_member_at_two_thetas_resolve_independently` — one
  impl, one inline caller, two θ (`Opt[i64 i64]` and `Opt[Bool i64]`), each site
  eliminating the enum. Two `inline_combinator` calls, two uids, two `splice_enum_words`
  entries. Without per-splice resolution this is a **miscompile, not a rejection**, so the
  golden must run and print both values. The two-*different*-`EnumId`s assertion is **not**
  writable in this golden's harness (`single_file_hosted` runs a compiled binary; no
  in-process `Module` state), so it lives in this phase's unit-test list instead; the
  golden carries the stdout/`nm`/`objdump` clauses only.
- **Golden P#5 (matrix row 4):**
  `inline_member_on_generic_target_splices_from_a_non_inline_poly_caller` — S3-1.d's
  witness and the half no other golden covers: the same `inline` member reached through a
  **non-inline** poly caller (p3's shape with the non-inline `usesize ['S: Sized]`
  caller). Contract clauses 1–4 apply; clause 5 is exempt (the caller keeps its frame —
  that is the golden's point). Clause 2's monomorph half is the load-bearing assertion:
  `sooth_mono_<member>_…` is now **absent** (it is present today, which is F9's finding)
  while the program still runs. Per R1 the member splices here **because it is declared
  `inline`**, not because of anything about the caller — under S3-5 this is the fence's
  positive side: caller inline-ness does not decide splicing.
- **Golden P#6 (conditional, migrated):**
  `combinator_constructing_a_generic_enum_resolves_per_splice` —
  `tests/phase7_slice12.rs`'s R1.5 construction fixture (`wrap_it`). Under S3-1.f's
  preferred ruling the fixture is a **non-member** combinator and still rejects, so the
  existing test stays untouched and P#6 is **not written**; it exists only if measurement
  2 lands on the fallback. Record which, and do not write a golden that pins a behaviour
  the ruling did not change.
- **Mutation check (required, not optional):** with the phase green, stub S3-1.e's lowering
  read of `splice_enum_words` and confirm **P#4** fails (its two splices then share one
  bare-key family resolution: a wrong-layout miscompile that surfaces in the stdout
  clause) and the per-splice unit tests fail with it. Scoped to P#4 deliberately: P#3's
  per-family single-θ sites still land on a correct bare-key resolution, so P#3 is *not*
  expected to fail on this stub and asserting that it does would be a placebo. If P#3 does
  fail, that is information and gets recorded, not assumed.
- **Second mutation check:** revert S3-1.c's hop alone — restore the
  `splice_trait_calls[(uid, span)] = symbol` write (`check/poly.rs:1619`) in place of
  the `inline_combinator` call — and **measure**. The m4 signature ("stdout passes,
  clause 2/3 fail") is **no longer the prediction**: with S3-1.d's pre-pass still
  active, a combinator instantiation is never emitted, so "right answers with a
  monomorph symbol present" cannot recur. What the reverted state actually does:
  lowering's `(uid, span)` read (`calls.rs:366`) hands the member symbol to the
  symbol-keyed bracket at `calls.rs:229`, which splices the member body **without**
  S3-1.c's per-splice re-walk — no `splice_enum_words` entries exist, so the body's
  enum sites fall through to bare-key family resolution. Expected signature, to be
  confirmed rather than asserted: P#4 fails its **stdout** clause (two θ sharing one
  bare-key layout is a miscompile), and P#3 fails in stdout-or-ICE territory (an
  ungrounded body spliced without its per-θ tables); record the actual failing clause
  per golden. If the reverted state instead passes every clause, the mutation as cut is
  too weak — widen it (also disable the `calls.rs:229` combinator hit for member
  symbols) and record that. Either way this stays the S3-1-specific adversary: a golden
  set that survives the revert unwidened is not testing the hop.
- **Third mutation check:** revert S3-1.d's diversion alone — emit the combinator
  instantiation's monomorph again and drop its `combinator_instantiations` entry — and
  confirm **P#5** fails on the monomorph-symbol-absence assertion. Without it the
  mono-caller half is unwitnessed, since P#3/P#4 both run through inline callers.

### Phase 4 — Goldens, controls and non-regression

The full suite in `tests/phase7b_slice3.rs`, harness style from `tests/phase7b_slice2.rs`
(`single_file_hosted`) plus the relocated `call_graph`.

- **Control C#1:** `non_inline_member_still_mints_its_monomorph_symbol` — the p0/m4
  control. Asserts presence, and is the twin that makes the splice goldens' (P#1,
  P#3–P#5) absence assertions non-vacuous (S3-13 clause 4 is per-test, but this one
  stands alone as the documented adversary).
- **Non-regression C#2:** `s2_shared_bound_golden_unchanged` (p0); **C#3**
  `concrete_target_inline_member_unchanged` (p1/p7); **C#4** `core_cmp_lt_still_splices`
  (p8 — the interception path S3 must not disturb).
- **Mutation check (required):** the m4 adversary, run against the *final* goldens.
  Re-apply m4's shape — lift the checker gates but leave lowering monomorphizing the member
  (concretely: force `lower_resolved_word_call` to take the non-splice arm for member calls)
  — and confirm **P#1, P#3 and P#5 all fail on their `nm`/`objdump` clauses while their
  stdout clause still passes** (clause 5 is exercised by P#1/P#3, the inline-caller pair).
  No other mutation in this spec exercises clauses 2/3/5; F9
  is the whole reason those clauses exist, and an unexercised assertion about a *missing*
  string is the placebo this repo has been bitten by before. Record which clause each
  golden fails on, not just that it failed.
- **Error E#1:** `unbounded_arrow_variable_inline_word_is_rejected_at_its_splice_site`
  — S3-6's fallback hole (and its rescue path), pinned closed at the point of use.
  Deliberately targeted at the fallback's *witnessable* trigger, an Arrow-kinded variable
  with **no user bound**: such a word is still callable, so its first splice site can
  observe the broken body's rejection. The no-impl trigger is unwitnessable by
  construction — with no impl anywhere in the program, no call site can discharge the
  bound, so the word is never spliced and nothing can observe it (recorded in ledger
  item 2).
- **Error E#2:** `bound_member_in_a_materialized_quotation_still_rejects` — ledger item 6.
- **Error E#3:** `struct_route_to_functor_still_rejects_identically` — F6, with and without
  `inline`, pinned as pre-existing.
- **Error E#4:** `generic_enum_in_a_non_member_combinator_body_still_rejects` — S3-1.f's
  surviving R1.5 gates. Under the preferred ruling the *only* narrowing is
  `is_trait_member`, so this pins that a non-member combinator body is untouched by S3;
  under the fallback ruling it also pins the standalone check's unground-θ case. Which
  clauses it carries is fixed by Phase 3's measurement 2.
- **Phase-exit growth-signal re-run (CLAUDE.md):** `check/poly.rs` is already the
  recorded `poly.rs` split candidate (3/5 signals at S2). S3-1's one new table (singular,
  `splice_enum_words` — consistent with S3-1.e) changes the balance; re-run the five
  signals against the file as it then
  stands and record the verdict in the spec's closing section, as S2 did.
- **MEMORY correction:** the "inline trait member panics at lowering" note is retired by
  F1; the note replacing it is this slice's actual shape (six gates, caller axis first).

## Anchor status

Re-verified against HEAD `403618f` while writing this spec.

| Anchor | At |
| --- | --- |
| `check_poly_combinator_standalone` / stand-in seed / R3 refusal | `check/poly.rs:537` / `:552-556` / `:566-571` |
| `resolve_splice_member_call` / materialized-quot fence / HKT guard / impl `find` | `:1343` / `:1488` / `:1514` / `:1588-1591` |
| `splice_member_hkt_error` / `splice_member_ctor_image_error` (narrowed, S3-7) | `:1633` / `:1647` |
| candidate-disambiguation skips: App-headed / `try_ground_member_type` `None` | `:1411-1417` / `:1426-1431` |
| member-slot `ground_slots` closure (S3-7 replaces it with θ-grounding) | `:1524-1544` |
| `find_bound_impl` (S3-8's target) / its live-registry candidate collection | `:7654` / `:7676-7699` |
| `resolve_mono_member_call` / `poly_trait_member_call` / `word_sig_of` | `:1697` / `:2041` / `:181` |
| R1.5 elimination / construction / message | `:4151` / `:5611` / `:10637` |
| per-splice `CallInst` redirect (the S3-1 template) | `:6742-6770` (write `:6758`) |
| `TraitResolveCtx` (S3-1.c adds `words`) / `is_combinator_splice` field | `:122-131` / `:56` (default `:86`) |
| bare member-call symbol write (S3-1.c replaces it with a splice) | `:1587-1620` (write `:1619`) |
| `resolve_splice_member_call` call site (S3-1.c widens its arg list) | `check/terms.rs:842`; the combinator hit that models `granted`/`tail` `:787-800` |
| `resolve_user_bound` / tie rule / `ctor_pin_count` / `word_sig_of` miss | `:7771` / `:7795` / `:8093` / `:7981` |
| `match_impl_target` (6th param `generics`) | `:8252` |
| `poly_rendered_type_mismatch_error` (G1's raise) | `:9611` |
| S2-15.f guard-a unit test (documents G1/G2) | `:11552` |
| poly pre-pass skip + rationale block | `check.rs:837` / `:822-838` |
| `splice_records` / `splice_trait_calls` declarations | `ast.rs:89` / `:99` |
| `WordDef` / `declares_inline` | `ast.rs:1785` / `:1810` |
| `synth_member_word_name` / its call site | `parser.rs:1024` / `:4128` |
| `inline_combinator` / uid mint / θ from live operands / body walk | `check/combinators.rs:313` / `:503-504` / `:591` / `:543` |
| `collect_combinators` key (`word.name`) / `is_combinator` | `check/combinators.rs:119-121` / `:139-141` |
| `synth_member_word_name` body / member `declares_inline` propagation | `parser.rs:1024-1038` / `:4107-4112`, `:4186` |
| `overload_symbols` (`$$` suffixing — why symbol ≠ name) / per-module `mangle` | `ast.rs:2894-2914` / `resolve.rs:36-41`, applied `:814` |
| `impl_mono_seed` (per-θ member walk, its own `enum_words`) | `check/poly.rs:7086` / `:7158-7167` |
| instantiation dedup/sort/emission (S3-1.d's pre-pass runs above `:135`) / per-word combinator filter | `ir/driver.rs:319-339` (`sort_by` `:338`) / `:142`, `:206` |
| per-instantiation `enum_words` install | `ir/func_builder/mod.rs:984` |
| member splice bracket / uid rule doc / its panic fall-through doc | `ir/func_builder/calls.rs:229` / `:170-228` / `:174-179` |
| `trait_calls` write in `resolve_user_bound` (S3-1.d's check-side key) | `check/poly.rs:8082` |
| name-keyed `poly_words` / `Arity` env-insert loop (S3-1.d skips both for combinators) | `ir/driver.rs:273-278` / `:284-314` |
| `enum_generated_sigs` / `variant_generated_sigs` / `Overload` (sig+symbol only) / `mint_fallback_candidates` (S3-1.e's membership source) | `check/declarations.rs:1852` / `:1885` / `check/builtins.rs:37` / `check/terms.rs:1362` |
| concrete-walk enum-site handling (S3-1.e's write sites) | eliminator arm `check/terms.rs:625-655`; generated-word records `:914`/`:926`; ctor-output read precedent `:1054`; `check_eliminator_call` `check.rs:2294` |
| `combinator_index` (name-keyed, S3-1.d's lowering key) | `check/combinators.rs:73` (entry `:93`), built at `ir/driver.rs:61` |
| `splice_trait_calls` read / `splice_records` read / `enum_words` read | `ir/func_builder/calls.rs:366` / `:434` / `:787` |
| `member_uid_seeds` construction / P7.S8 R1 seed unit test | `ir/driver.rs:67` / `:734-761` |
| distinct-instantiation chain over `splice_records` | `ir/driver.rs:331` |
| `empty_splice_records` / `empty_splice_trait_calls` / destructor pass | `ir.rs:66` / `:75` / `ir/destructors.rs:387-388` |
| `impl_monos` worklist / its push sites / `discover_transitive_instantiations` | `check.rs:763` / `check/poly.rs:8070`, `:8076` / `:6847` |
| poly pre-pass body walk / `is_combinator_splice`'s only set site | `check.rs:813` (loop), `:847` (`check_poly_body`) / `:865` |
| `discover_transitive_instantiations` caller threading `&mut splice_records` | `check.rs:1077` |
| `nm` assertion precedent / `call_graph` helper | `tests/phase4_slice11_inline.rs:84` / `tests/phase7_slice3s_oracle.rs:67` |
| R1.5 construction-side test pin | `tests/phase7_slice12.rs:235` |
| R3 text pins (unaffected, re-run in Phase 2) | `tests/phase7_slice11.rs:131`, `tests/phase7_slice12.rs:671`, `check/poly.rs:19368` |

## Phases (JSON)

```json
[
  { "phase": 1, "focus": "Marker and baseline: WordDef::is_trait_member set at the member-word synthesis site (parser.rs:4128) with no readers yet; move the call_graph objdump helper from tests/phase7_slice3s_oracle.rs to tests/common/mod.rs and re-point the oracle test; unit-test the marker; re-run the member-inline x generic-target x caller-inline matrix as a recorded baseline. Behaviour byte-identical, full suite green.", "effort": "S", "difficulty": "L" },
  { "phase": 2, "focus": "The splice-caller path plus the two member-axis gates: S3-6 seed an Arrow-kinded standalone variable with Type::CtorImage of its bound's first impl constructor, skipping the standalone check when the bound has no impl (or the Arrow variable has no user bound), and rescuing a stand-in failure ONLY for the two classes the stand-in's arbitrariness can cause (a grounding failure against the stand-in's constructor, or an operand mismatch mentioning it) -- every other failure class (arity/underflow, linearity, unknown word, borrow/move, undischarged bound) stays a hard error, unit-tested via an arity error that is not rescued; S3-7 delete splice_member_hkt_error and BOTH candidate-disambiguation skips (the App-headed one at check/poly.rs:1411-1417 and the try_ground_member_type-None one at :1426-1431), grounding member slots through the caller's theta via S2-6 leading-slot grounding, and narrow splice_member_ctor_image_error to the theta-does-not-complete case (or delete it if Phase 2 finds that unreachable); S3-8 route resolve_splice_member_call's impl lookup through find_bound_impl (check/poly.rs:7654) with the live registries instead of the bare find at :1588-1591, taking (impl idx, Subst) back; S3-9 narrow R3's top-level-Generic refusal by the marker; S3-10 record member combinators in the poly pre-pass. Exit witness: golden P#1 alone -- the non-HKT p3 shape (impl: Sized for Box['T], size inline) splices for real under the full S3-13 assertion contract from an inline caller (row 3, the splice-caller path, which is complete at this phase); plus P#2, the row-7 golden where a NON-inline member on a generic target now accepts and monomorphizes from an inline caller. The row-4 (non-inline caller) golden is Phase 3's P#5: its no-monomorph clause needs S3-1.d. P#1 is exit-provable before Phase 3 because p3's body (drop 1) has no enum sites and nothing theta-directed, so the pre-existing splice_trait_calls write (check/poly.rs:1619) plus the symbol-keyed bracket (calls.rs:229) carries it and no instantiation record mints a monomorph; confirm at the phase's opening golden run and record any clause failure as the opening measurement.", "effort": "L", "difficulty": "H" },
  { "phase": 3, "focus": "Route inline trait members onto the EXISTING combinator splice path (inline_combinator), replacing R1.5. Open with two measurements: (a) does the bracket-scoped install/restore of a diverted instantiation's enum_words/trait_calls compose with the P7.S8 uid bracket under nested member splices (the CallInst itself is carried by S3-1.d's diversion, so reachability is settled structurally); (b) does the poly pre-pass walk of a member combinator body complete with is_combinator_splice: false, or must the walk be skipped for is_trait_member combinators. Then: S3-1.c give TraitResolveCtx (check/poly.rs:122-131) a words: &[WordDef] slice, and at check/poly.rs:1587-1620 branch on words[idx].declares_inline -- look the member up in poly.combinators by words[idx].name (its synth name, NOT word_symbols[idx], which overload_symbols may have $$-suffixed) and call inline_combinator with the caller's live stack instead of writing splice_trait_calls[(uid,span)]; widen resolve_splice_member_call to resolve_mono_member_call's parameter list and compute granted/tail at check/terms.rs:842 the way the combinator hit at :787-800 does; give inline_combinator a seed: Option<&Subst> for S3-8's impl substitution, with a contradiction between seed and operand-derived theta a located error. S3-1.d the mono-caller half is a LOWERING change and a PRE-PASS DIVERSION, not a bare exclusion (resolve_user_bound records instantiation_symbol(word_sym, theta) into trait_calls at check/poly.rs:8082, which can never hit the word.name-keyed combinator_index, so exclusion alone panics at the ordinary-call fall-through, calls.rs:174-179): a pre-pass above the lowering env build (ir/driver.rs:135 -- before BOTH the per-word FuncBuilder pass at :203 and the alphabetically-sorted dedup/emission stretch at :319-339, sort_by :338, either of which can otherwise lower a consumer before the map entry exists) walks the instantiations/transitive_instantiations/splice_records chain (:327-337) with the name-keyed poly_words (:273-278) hoisted above it, and diverts every is_combinator instantiation's (symbol, CallInst) into a new combinator_instantiations: HashMap<String, CallInst> threaded into FuncBuilder; the Arity env-insert loop (:284-314) and the emission loop then skip those symbols (no IrFunc, no env entry); lower_resolved_word_call, on a combinators miss, consults that map, recovers the member via combinators.get(&inst.callee), and splices through the existing uid bracket with the instantiation's own enum_words/trait_calls installed for the bracket's duration (ir/func_builder/mod.rs:984 precedent). Lowering-side recovery, NOT a bare-name re-key in resolve_user_bound: the bare name would sever the per-theta link to the CallInst the splice must install; member_uid_seeds stays name-keyed and is consulted by the recovered inst.callee. S3-1.e add Module.splice_enum_words: HashMap<(u32,Span),EnumId>, written in the CONCRETE path-A walk (check_terms_relaxed's arms -- the abstract-walk R1.5 sites at check/poly.rs:4151/:5611 have no Provenance in scope and are never entered by the concrete walk) when prov.splice_uid is Some AND the chosen candidate is a generated ENUM word -- (name, chosen.symbol) membership in enum_generated_sigs/variant_generated_sigs over the extended type slices, the same source mint_fallback_candidates reads (check/terms.rs:1362), since an Overload carries only sig+symbol and sig shape cannot discriminate: the candidate arm then writes (uid,span)->EnumId INSTEAD of the span-keyed builtin_overloads insert (a redirect, mirroring check_poly_call's splice_records redirect at check/poly.rs:6746), while the :914 combinator-name-collision record and :926's D7/R5 struct ctor/accessor records stay untouched for non-enum candidates even inside a splice (both serve non-enum mechanisms consumed at ir/func_builder/calls.rs:370-378), and check_eliminator_call (check.rs:2294) records the operative id it reads off the live scrutinee; read at ir/func_builder/calls.rs:787 before the span-keyed enum_words; relay it through S3-12's census including ir.rs empty_* companions and ir/destructors.rs:387-388, but NOT through discover_transitive_instantiations (check.rs:1077) -- it is terminal data that mints nothing; confirm by grep and record. S3-1.f re-site R1.5: is_combinator_splice is set only at check.rs:865 in the pre-pass, so set it false for is_trait_member combinators (preferred) or skip the pre-pass walk for them (fallback). No new uid scheme, no worklist, no memo, no derive pass. Goldens: P#3 the HKT exit shape through an inline caller; P#4 two splices at two thetas (stdout/nm/objdump only -- the two-DIFFERENT-EnumIds assertion lives in this phase's in-process unit tests, since the single_file_hosted golden harness cannot see Module state); P#5 the merged row-4 golden, an inline member on a generic target splicing from a NON-inline poly caller (the monomorph symbol now absent; clause 5 exempt). Three required mutation checks: stub the splice_enum_words lowering read (P#4 must fail; P#3 is not expected to); revert the inline_combinator hop alone and MEASURE -- with the pre-pass active the m4 signature cannot recur, so the expected signature is P#4 failing stdout (bare-key two-theta miscompile) and P#3 failing stdout-or-ICE, recorded per golden, with the mutation widened (also disabling calls.rs:229's combinator hit for member symbols) if the revert survives as cut; revert the S3-1.d pre-pass diversion alone (P#5 fails its monomorph-absence assertion).", "effort": "M", "difficulty": "H" },
  { "phase": 4, "focus": "Goldens, controls and non-regression in tests/phase7b_slice3.rs: control C#1, the non-inline monomorph presence control that makes the absence assertions non-vacuous; non-regression C#2-C#4 on S2's shared-bound golden, the concrete-target inline member, and core::cmp's lt interception; error goldens E#1-E#4 for the standalone-skip hole, the materialized-quotation fence, the unchanged struct-route rejection, and R1.5's surviving unground-theta gates. Required mutation check: re-apply the m4 adversary (checker gates lifted, lowering forced onto the non-splice arm for member calls) against the FINAL goldens and confirm P#1/P#3/P#5 fail on their nm/objdump clauses while their stdout clause passes -- no other mutation in this slice exercises S3-13 clauses 2, 3 and 5. E#1 targets the fallback's witnessable trigger (an Arrow-kinded variable with NO user bound, still callable); the no-impl trigger is unwitnessable by construction and is recorded as such in ledger item 2. CLAUDE.md phase-exit growth-signal re-run on check/poly.rs; retire the stale MEMORY note about inline members panicking at lowering.", "effort": "M", "difficulty": "M" }
]
```
