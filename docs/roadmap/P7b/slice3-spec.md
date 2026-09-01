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
| G3 | The splice path's impl lookup is a bare `find` with **empty registries and `generics: None`**, so a `Generic` target pattern can never match a monomorph operand | `check/poly.rs:1588-1591` (cf. `:7817`, `:7898`, which pass the live registries) | caller | measured (matrix row 7) |
| G4 | R3's grounding refusal: a declared top-level `PolyType::Generic` input is refused at the combinator standalone check | `check/poly.rs:566-571` | member | brief p2/p3, m1 |
| G5 | The poly pre-pass skip: a member combinator carries no `Bound::User`, is never recorded, and `word_sig_of` misses | `check.rs:837` (rationale block `:822-838`) | member | brief m2/m3 |
| G6 | R1.5, both sides: a generic enum eliminated/constructed inside a combinator splice is refused because "each splice would need its own resolution, and none is recorded" | `check/poly.rs:4151` / `:5611` (text `:10637`) | member | brief m3 |

G5's rationale block in the tree ends "**Revisit the skip if that restriction is
lifted**", naming G4's lift as the trigger. This slice is that revisit.

## Rulings on the brief's open questions

Each of Q1–Q5 is ruled here, not carried forward.

### S3-1 (Q1) — what replaces R1.5: per-splice resolution records

**Ruled: keyed per splice, mirroring `splice_records`/`splice_trait_calls`. R1.5's two
gates are deleted only once the record they name exists.**

R1.5's message states the requirement exactly: *each splice would need its own
resolution, and none is recorded*. The tree already solves this problem twice, at the
same key shape:

- `splice_records: HashMap<(u32, Span), CallInst>` (`ast.rs:89`), written at
  `check/poly.rs:6742-6770` when `prov.splice_uid` is `Some`, read at
  `ir/func_builder/calls.rs:434`;
- `splice_trait_calls: HashMap<(u32, Span), String>` (`ast.rs:99`), read at
  `ir/func_builder/calls.rs:366`.

The enum resolution is the missing third. A poly *call* inside a splice carries its
`enum_words` inside its own per-splice `CallInst`; a **spliced body's own** enum sites
have nowhere per-splice to live, because a spliced combinator mints no `CallInst` for
itself. That, and nothing more exotic, is R1.5's representation limit.

**S3-1.a.** Add `Module.splice_enum_words: HashMap<(u32, Span), EnumId>` (`ast.rs`,
beside `splice_records`/`splice_trait_calls`, same key shape and the same
`empty_*`/relay plumbing at `ir.rs`, `check.rs:767-790`, `check.rs:1093`,
`ir/driver.rs`, `ir/func_builder/mod.rs`). During a real splice
(`prov.splice_uid == Some(uid)`), a generic enum elimination or construction in the
spliced body grounds through the splice's θ (`poly.combinator_subst`, already threaded)
and writes `(uid, span) -> EnumId` instead of raising.

**S3-1.b.** `lower_call`'s enum-word arm (`ir/func_builder/calls.rs:787`) consults
`splice_enum_words[(splice_uid_stack.last(), span)]` **before** the span-keyed
`self.enum_words`, falling through to it and then to the bare-key monomorphic path on a
miss — the same three-step shape the arm already documents.

**S3-1.c — the member-splice uid must be per site, not per member word.** Today
`lower_resolved_word_call` derives the spliced member's uid from
`member_uid_seeds[name]` (`ir/driver.rs:67`, one seed per *word*). Two splices of the
**same** member word at two θ (`twice` at `Opt[i64 i64]` and at `Opt[Bool i64]`) would
then collide on every `(uid, span)` key inside the member body — R1.5's hazard, moved
one level out. The checker therefore mints a fresh uid per member splice site
(`prov.inline_uid += 1`, exactly as `inline_combinator` does at
`check/combinators.rs:503`) and records it in a map parallel to `splice_trait_calls`:
`member_splice_uids: HashMap<(u32, Span), u32>`. `lower_resolved_word_call` prefers that
entry over `member_uid_seeds`, keeping the existing seed lookup as the fallback for the
concrete-target member case (which has no per-site record and needs none).

A parallel map is chosen over widening `splice_trait_calls`' value to a struct: the value
type is threaded through ~20 signatures across `check.rs`, `ir/driver.rs`,
`ir/func_builder/mod.rs` and `ir/destructors.rs`, and none of them care about the uid.

**S3-1.d.** Only then are `poly_combinator_generic_enum_elimination_error` (`:4151`) and
`poly_combinator_generic_enum_construction_error` (`:5611`) deleted, with their builders
and the `:10637` doc block. Pin census (verified by grep, not by prose): exactly one test
pins either text — `tests/phase7_slice12.rs:235`
(`err.contains("this combinator's own splice determines")`, the construction side). That
test is **migrated, not retired**: its fixture (`: wrap_it inline ['T: Foo] ( 'T --
Pair['T] ) bar One ;`) becomes a positive golden asserting the construction now resolves
per splice, and the assertion becomes a `splice_enum_words` lookup plus a run. The
elimination side has no pin at all today, which is itself worth recording.

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
- **The mono and non-inline-poly caller paths keep monomorphizing the member.** The exit
  criterion names an inline caller; those two paths already work (matrix rows 5–8) and
  changing them is P7b.S4/S5 territory. This is a *stated* limitation with a golden
  (control #5), not a silent one — see the ledger.

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

Fallback when the bound has no impl in the program (or the variable is Arrow-kinded with
no user bound): the word's standalone check is **skipped**. This is a real hole and is
named as one — such a word is checked only at its splice sites, and if it is never
spliced it is never checked. It is the same trade the pre-existing G5 skip makes, it is
unreachable for any word that is actually used, and error golden E1 pins that a broken
body of this shape is still rejected at its first splice site.

**S3-7 (G2) — delete `splice_member_hkt_error`.** With S3-6 and S3-8 in place, an
App-headed member signature *does* have a grounding on the splice path: the caller's θ
binds the head variable to a `CtorImage`, and S2-6's leading-slot grounding is the rule
that grounds it (`ast.rs:2242`, App arm `:2296`). The splice path stops grounding member
slots via `try_ground_member_type` at a single `Type` (`check/poly.rs:1527-1544`) and
grounds them through the caller's θ instead, the same way `resolve_user_bound` does at
its re-grounding step. The builder at `:1633` and the guard at `:1514` are removed; the
candidate-disambiguation skip at `:1404-1416` that exists only to route around them is
removed with them (its `continue` would otherwise silently drop the now-fittable
candidate — a permissive fixture line disabling the very gate S3 adds).

Its unit test (`check/poly.rs:11552`) is **migrated**: its own doc block says it pins only
"the body-check half", i.e. that a missing-operand member call is a located error rather
than a panic. That claim survives S3 unchanged and the test keeps it; the paragraph
explaining why the guard is source-unreachable is replaced by a pointer here.

**S3-8 (G3) — one candidate-selection path, shared.** `resolve_splice_member_call`'s impl
lookup (`check/poly.rs:1588-1591`) passes `&[], &[], &[], None` to `match_impl_target`, so
a `Generic` pattern cannot be matched against a monomorph operand — the operand is a
`Type::Struct`/`Type::Enum` id and recovering its header identity needs the live
`GenericTypes`. Ruling: **extract S2-8's candidate selection** (CtorImage identity match
on `(idx, module)` + the compatibility-conditioned tie rule + `ctor_pin_count`, currently
inline in `resolve_user_bound` at `check/poly.rs:7795-7900`) **into one function, and call
it from both** `resolve_user_bound` and `resolve_splice_member_call`, with the live
registries and `Some(generics)` from `ctx`.

This is the CLAUDE.md elevate rule applied literally: two callers need it, so it moves to
the nearest shared parent and no higher. It is also the only way the two paths can agree
about which impl wins — a second, laxer selector on the splice path would be an
S2-tie-rule bypass, and the tie rule exists because without its compatibility condition a
pinned target wins at every ctor-abstract site.

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

**S3-11 (G6) — the R1.5 replacement**, per S3-1 in full.

### Cross-cutting

**S3-12 — forced-arm and consumer inventory.** `Module` gains two maps and `WordDef` gains
a field; both are structural changes with a construction-site census
(`ast.rs:4010`/`:4164`/`:4242`, `driver.rs:807`, `resolve.rs:1828` for `Module`; every
`WordDef` literal for the field). Neither is a `match` variant, so **rustc will not find
the readers** — the relay chain (`check.rs` → `Module` → `ir/driver.rs` →
`FuncBuilder`) is grep-driven and every hop is listed in the phase plan. A map that is
built but never relayed is exactly the failure mode F9 describes: green tests, no splice.

**S3-13 (Q4) — the golden assertion contract.**

Every S3 **positive** golden asserts all four of:

1. stdout and exit code (necessary, never sufficient);
2. `nm`: the binary contains **no** `<member>.3b.<Trait>` mangled member symbol **and**
   no `sooth_mono_<member>_<Trait>` monomorph symbol. Both patterns are required: the
   matrix above shows a generic-target member mints the *monomorph* form
   (`sooth_mono_size_Sized_0_Box__T0___m0__t0_i64`), so an nm assertion written only
   against the `.3b.` form is vacuous for exactly the shapes this slice is about;
3. `objdump`: zero call edges to either symbol pattern, reachable from `sooth_main` in
   the call graph;
4. a **non-inline twin control in the same test**, byte-identical but for the `inline`
   keyword, asserting the symbol **is** present and the call edges **are** there.

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
- **Widen `splice_trait_calls`' key or value shape** to carry the per-splice enum
  resolution. Rejected in favour of a parallel `(u32, Span)`-keyed map (S3-1.a/c): the
  existing value type is threaded through ~20 signatures that have no interest in it, and
  the tree's own precedent is one map per concern at a shared key.
- **A second, laxer candidate selector on the splice path** (the minimal fix for G3 is to
  pass the live registries to the existing bare `find`). Rejected: the bare `find` is a
  first-match with no tie rule, so a program with a pinned and an abstract impl would
  dispatch differently depending on whether the caller was `inline` — a silent
  divergence. S3-8 shares one selector instead.
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

1. **The mono caller and the non-inline poly caller still monomorphize the member.** Both
   work today and are unchanged; only the inline-caller path splices. Control golden #5
   asserts the monomorph symbol is still minted there, which is also what makes the
   positive goldens' absence assertions meaningful.
2. **An HKT-bounded inline word whose trait has no impl in the program is not checked
   standalone** (S3-6's fallback). Checked at its splice sites; never spliced means never
   checked. Error golden E1 pins the splice-site rejection.
3. **Fixture twins, not `core::option`/`core::result`** (S3-4) — P7b.S4.
4. **The struct route to `Functor` stays closed** (S3-5) — `Box>` on a generic type,
   pre-existing and identical with or without `inline`.
5. **The S2-8 tie rule's pin metric still counts top-level pins only.** S3-8 shares the
   selector, so it inherits the limitation verbatim; it neither worsens nor fixes it.
6. **Bound dispatch inside a materialized quotation stays rejected** (P7.S3o R5,
   `check/poly.rs:1488`): a materialized quotation lowers to its own `IrFunc` with an
   empty `splice_uid_stack`, so no `(uid, span)` key resolves. S3 widens the splice path,
   so this fence gets a regression golden (E2) rather than an assumption.

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
through θ), S3-8 (shared candidate selection), S3-9 (R3 narrowed by the marker), S3-10
(pre-pass records member combinators).

G4/G5 land here rather than in a member-axis phase of their own because the p3 witness
needs all five: without G4/G5 the member word never gets a signature to dispatch to, and
an intermediate phase boundary between them can only be verified by an error message.

- **Unit tests:** the standalone stand-in binds an Arrow-kinded variable to a `CtorImage`
  and to the *first* impl's constructor (declaration order, pinned); the no-impl fallback
  skips the standalone check without erroring; the shared candidate selector returns the
  same winner from both call sites for a pinned/abstract pair (the tie rule is not
  bypassed on the splice path); `word_sig_of` hits for a recorded member combinator.
- **Golden (positive #1), the full S3-13 contract:**
  `inline_member_on_generic_target_splices_from_a_mono_caller` — p3's shape
  (`trait: Sized`, `impl: Sized for Box['T]`, `size inline`, non-inline `usesize`
  caller). Non-HKT witness *and* the phase's exit proof.
- **Golden (positive #2):** `inline_bound_caller_splices_a_generic_target_member` — the
  same trait with `usesize inline`, flipping matrix row 7.
- **Verifiable:** R3's three integration pins and one unit pin re-run and unchanged
  (S3-9); the S2 suite (`tests/phase7b_slice2.rs` 17/17) and `tests/phase7b_slice1.rs`
  16/16 green; the HKT exit shape now fails at R1.5 rather than at G1 (a pinned
  intermediate located error, never a panic).

### Phase 3 — Per-splice resolution (G6)

S3-1 in full: `splice_enum_words`, `member_splice_uids`, the lowering reads, the two R1.5
deletions, S3-12's relay census.

- **Unit tests:** a spliced body's enum site writes `(uid, span) -> EnumId` and the
  span-keyed table is untouched; lowering prefers the per-splice entry and falls through
  to `enum_words` then to the bare key on a miss (all three arms); two splices of one
  member word at two θ mint two distinct uids and two distinct entries.
- **Golden (positive #3):** `hkt_member_splices_through_an_inline_bound_caller` — the exit
  criterion, p4's `Opt`/`Res` program with `twice inline`. Full S3-13 contract.
- **Golden (positive #4):**
  `two_splices_of_one_member_at_two_thetas_resolve_independently` — one impl, one inline
  caller, two θ (`Opt[i64 i64]` and `Opt[Bool i64]`). This is the golden S3-1.c exists
  for; without the per-site uid it is a miscompile, not a rejection, so it must run and
  print both values.
- **Golden (positive #5, migrated):**
  `combinator_constructing_a_generic_enum_resolves_per_splice` — `tests/phase7_slice12.rs`'s
  R1.5 construction fixture (`wrap_it`), flipped from rejection to a run.
- **Mutation check (required, not optional):** with the phase green, re-stub S3-1.b's
  lowering read and confirm goldens #3/#4 fail. A per-splice table that lowering never
  consults is the F9 failure mode in a new costume.

### Phase 4 — Goldens, controls and non-regression

The full suite in `tests/phase7b_slice3.rs`, harness style from `tests/phase7b_slice2.rs`
(`single_file_hosted`) plus the relocated `call_graph`.

- **Control #6:** `non_inline_member_still_mints_its_monomorph_symbol` — the p0/m4
  control. Asserts presence, and is the twin that makes #1–#5's absence assertions
  non-vacuous (S3-13 clause 4 is per-test, but this one stands alone as the documented
  adversary).
- **Non-regression #7:** `s2_shared_bound_golden_unchanged` (p0), `#8`
  `concrete_target_inline_member_unchanged` (p1/p7), `#9` `core_cmp_lt_still_splices`
  (p8 — the interception path S3 must not disturb).
- **Error #1 (E1):** `hkt_bounded_inline_word_with_no_impl_is_rejected_at_its_splice_site`
  — S3-6's fallback hole, pinned closed at the point of use.
- **Error #2 (E2):** `bound_member_in_a_materialized_quotation_still_rejects` — ledger
  item 6.
- **Error #3 (E3):** `struct_route_to_functor_still_rejects_identically` — F6, with and
  without `inline`, pinned as pre-existing.
- **Phase-exit growth-signal re-run (CLAUDE.md):** `check/poly.rs` is already the
  recorded `poly.rs` split candidate (3/5 signals at S2). S3-8's extraction and S3-1's
  new table change the balance; re-run the five signals against the file as it then
  stands and record the verdict in the spec's closing section, as S2 did.
- **MEMORY correction:** the "inline trait member panics at lowering" note is retired by
  F1; the note replacing it is this slice's actual shape (six gates, caller axis first).

## Anchor status

Re-verified against HEAD `403618f` while writing this spec.

| Anchor | At |
| --- | --- |
| `check_poly_combinator_standalone` / stand-in seed / R3 refusal | `check/poly.rs:537` / `:552-556` / `:566-571` |
| `resolve_splice_member_call` / materialized-quot fence / HKT guard / impl `find` | `:1343` / `:1488` / `:1514` / `:1588` |
| `splice_member_hkt_error` / `splice_member_ctor_image_error` | `:1633` / `:1647` |
| candidate-disambiguation App skip | `:1404-1416` |
| `resolve_mono_member_call` / `poly_trait_member_call` / `word_sig_of` | `:1697` / `:2041` / `:181` |
| R1.5 elimination / construction / message | `:4151` / `:5611` / `:10637` |
| per-splice `CallInst` redirect (the S3-1 template) | `:6742-6770` (write `:6758`) |
| `resolve_user_bound` / tie rule / `ctor_pin_count` / `word_sig_of` miss | `:7771` / `:7795` / `:8093` / `:7981` |
| `match_impl_target` (6th param `generics`) | `:8252` |
| `poly_rendered_type_mismatch_error` (G1's raise) | `:9611` |
| S2-15.f guard-a unit test (documents G1/G2) | `:11552` |
| poly pre-pass skip + rationale block | `check.rs:837` / `:822-838` |
| `splice_records` / `splice_trait_calls` declarations | `ast.rs:89` / `:99` |
| `WordDef` / `declares_inline` | `ast.rs:1785` / `:1810` |
| `synth_member_word_name` / its call site | `parser.rs:1024` / `:4128` |
| `inline_combinator` / uid mint | `check/combinators.rs:313` / `:503` |
| member splice bracket / uid rule doc | `ir/func_builder/calls.rs:229` / `:170-228` |
| `splice_trait_calls` read / `splice_records` read / `enum_words` read | `ir/func_builder/calls.rs:366` / `:434` / `:787` |
| `member_uid_seeds` construction | `ir/driver.rs:67` |
| `nm` assertion precedent / `call_graph` helper | `tests/phase4_slice11_inline.rs:84` / `tests/phase7_slice3s_oracle.rs:67` |
| R1.5 construction-side test pin | `tests/phase7_slice12.rs:235` |
| R3 text pins (unaffected, re-run in Phase 2) | `tests/phase7_slice11.rs:131`, `tests/phase7_slice12.rs:671`, `check/poly.rs:19368` |

## Phases (JSON)

```json
[
  { "phase": 1, "focus": "Marker and baseline: WordDef::is_trait_member set at the member-word synthesis site (parser.rs:4128) with no readers yet; move the call_graph objdump helper from tests/phase7_slice3s_oracle.rs to tests/common/mod.rs and re-point the oracle test; unit-test the marker; re-run the member-inline x generic-target x caller-inline matrix as a recorded baseline. Behaviour byte-identical, full suite green.", "effort": "S", "difficulty": "L" },
  { "phase": 2, "focus": "The splice-caller path plus the two member-axis gates: S3-6 seed an Arrow-kinded standalone variable with Type::CtorImage of its bound's first impl constructor (skip the standalone check when the bound has no impl); S3-7 delete splice_member_hkt_error and its candidate-skip, grounding member slots through the caller's theta via S2-6 leading-slot grounding; S3-8 extract S2-8's candidate selection (CtorImage identity + compatibility-conditioned tie rule + ctor_pin_count) from resolve_user_bound and share it with resolve_splice_member_call, passing the live registries and Some(generics); S3-9 narrow R3's top-level-Generic refusal by the marker; S3-10 record member combinators in the poly pre-pass. Exit witness: the non-HKT p3 shape (impl: Sized for Box['T], size inline) splices for real under the full S3-13 assertion contract, from both a mono and an inline caller.", "effort": "L", "difficulty": "H" },
  { "phase": 3, "focus": "Per-splice resolution, replacing R1.5: add Module.splice_enum_words keyed (uid, Span) written during a real splice from the splice's theta, and member_splice_uids parallel to splice_trait_calls so the spliced member body gets a per-site uid instead of member_uid_seeds' per-word seed; lowering reads splice_enum_words before the span-keyed enum_words and prefers the per-site uid in lower_resolved_word_call; relay both maps through check.rs, Module, ir/driver.rs and FuncBuilder (grep-driven census, no compiler forcing); delete both R1.5 gates and migrate the one test pinning the construction-side text. Goldens: the HKT exit shape through an inline bound caller, and two splices of one member word at two thetas. Required mutation check: re-stub the lowering read and confirm the goldens fail.", "effort": "L", "difficulty": "H" },
  { "phase": 4, "focus": "Goldens, controls and non-regression in tests/phase7b_slice3.rs: the non-inline monomorph control that makes the absence assertions non-vacuous; non-regression on S2's shared-bound golden, the concrete-target inline member, and core::cmp's lt interception; error goldens for the no-impl standalone-skip hole, the materialized-quotation fence, and the unchanged struct-route rejection; CLAUDE.md phase-exit growth-signal re-run on check/poly.rs; retire the stale MEMORY note about inline members panicking at lowering.", "effort": "M", "difficulty": "M" }
]
```
