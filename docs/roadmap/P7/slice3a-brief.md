# Phase 7 Slice 3a: generic instantiation over a poly word's own type variable (brief)

Split out from P7.S3b (`docs/roadmap/P7/slice3b-brief.md`) as its own prerequisite:
discovered as a compiled blocker during that slice's paper dogfood
(`docs/roadmap/P7/slice3-dogfood.md`, finding #5), not planned work. A polymorphic
word cannot today name a generic type applied to its own type variable —
`Box['T]`, `Option['T]`, `Map['K 'V]` inside a poly signature all fail — while a
concrete generic application (`Result[i64 str]` in a monomorphic word) and an
array carrying a poly variable (`['T: Copy 4]`) both already work.

## Recon (verified against the built compiler by the dogfood worker; parser side

only — unification/monomorphization/lowering not yet traced)

1. **The failure is confirmed by compiling, not inferred.**
   `: unbox ( Box['T] -- 'T )` and `: or-default ( 'T Option['T] -- 'T )` both fail
   `` error: unknown type 'T ``. Control: `: setat ( ['T: Copy 4] 'T -- ['T 4] )`
   (`examples/poly_borrow_setat.sth`) builds green — an *array* type carrying a
   poly variable in a signature works; a *named generic* carrying one does not.
   This isolates the gap to generic-type-argument resolution specifically, not to
   poly signatures containing type variables in general.

2. **Root cause traced to one function: generics monomorphize eagerly, at parse
   time, and only accept concrete arguments.** `resolve_type_or_apply`
   (`src/parser.rs:3129-3172`) is the single path a generic name goes through. For
   a struct or enum generic it calls `parse_type_arguments` to collect the
   argument list, then immediately calls `instantiate_struct`/`instantiate_enum`
   (`parser.rs:3153`, `3167`) to produce a concrete `Type`. Each argument is
   itself resolved through the ordinary `resolve_type` path
   (`parser.rs:2735-`), which only knows registered concrete type names — a bare
   `'T` is not one, hence "unknown type `'T`."

3. **Every existing use of a generic type is a concrete argument to a
   monomorphic word.** `Result[i64 str]`, `Option[i64]`, etc. all resolve fine
   because nothing upstream of `resolve_type_or_apply` is itself abstract. This
   slice's gap only appears the first time a generic is nested inside *another*
   generic word's own signature — a case Phase 5 never needed and never built.

4. ~~The likely shape of a fix, by analogy...~~ **Superseded by a compiled probe
   (worker-verified, full report and reverted diff described below) — the analogy
   to quotations is where the guess goes wrong.** See "Resolved recon" below.

## Resolved recon (worker-verified: a working end-to-end probe was built, run

`nm`-inspected, and fully reverted — `git status`/`cargo build` confirmed clean
afterward)

Probe used `Result['T 'E]` (an existing two-variable generic) applied to a poly
word's own variables (`: reorder ( 'T Result['T 'E] -- Result['T 'E] 'T )`),
instantiated asymmetrically at `t0=i64,t1=str` and its swap `t0=str,t1=i64` —
not the same type twice, per the project's own symmetric-instantiation-placebo
precedent. Both instantiations compiled, ran, and printed independently-correct,
position-dependent output; `nm` showed two distinct monomorph symbols
(`sooth_mono_reorder__m0__t0_i64_t1_str`, `..._t0_str_t1_i64`), so nothing was
silently shared.

- **A new `PolyType::Generic` variant is required (OQ1 answered: yes, new, not
  reusable) — and it is not cheap.** `PolyType` is matched exhaustively in
  roughly 13 places across 6 files, and every one needs a deliberate arm, not a
  mechanical stub: `src/check/audits.rs` (3 walks: Copy-reference containment,
  poly-quotation rejection), `src/check/declarations.rs` (export-privacy),
  `src/check/poly.rs` (6: Copy-ness, unification, `apply_subst`, diagnostic
  rendering), `src/ir/driver.rs` (`subst_polytype`), `src/repl.rs`
  (`remap_poly_type`). Several require a real decision (e.g. is `Result['T 'E]`
  `Copy` when its args are? the probe took the conservative "always linear",
  which a real spec should revisit).
- **The real cost center is a registry-lifetime asymmetry the brief's analogy
  missed (OQ2 answered, with a sharp caveat).** Arrays and refs mint monomorphs
  *on demand, downstream*: `apply_subst`/`subst_polytype` are handed a `&mut
  Vec<ArrayDecl>`/`&mut Vec<RefDecl>` and call `intern_array_type`/
  `intern_ref_type` to mint-or-find a shape at the point of use. **Named generic
  structs/enums have no such downstream registry** — `GenericTypes` (the value
  that owns the dedup keys and mints monomorphs) is consumed and dropped at
  `src/driver.rs:308-309` (`structs.extend(generics.inst_structs); ...`) before
  check or lowering ever runs. After that point nothing in the pipeline can mint
  a `Result[i64 str]` that wasn't already materialized at parse time. **This is
  the load-bearing plumbing change** — keeping an instantiator alive and mutable
  through check and lowering — not "add a variant + a unify arm" as the original
  recon guessed. Monomorphization identity itself did come out correct in the
  probe (a grounded `reorder` output type-checked cleanly against an independent
  concrete eliminator of the same generic), but *only* because the probe
  deliberately routed grounding through the same parse-time dedup table — that's
  a design decision this slice must make explicitly, not a free property.
- **OQ3 answered: no placebo.** The two-type asymmetric run (above) specialized
  positionally and correctly; `unify_poly_input`'s new arm had to recurse
  positionally into the generic's args and respect a variable shared between a
  bare slot and a generic argument, and it did.
- **New, load-bearing finding not in the original recon: S3a's hard case is
  currently unreachable through legal source, because it's entangled with a
  separate, already-known gap.** The probe's "pre-existing monomorph only" wall
  (the `apply_subst` arm that has nowhere to mint a monomorph) never actually
  fired in any legal test program, because **generic construction inside a poly
  body is itself unsupported today** (`: wrap ( 'T -- Result['T i64] ) Ok ;`
  fails with `` unknown word `Ok` ``, the pre-existing
  `generic-enum-elimination-blocked` gap, P7.S3b/construction territory). A poly
  word can never *produce* a generic monomorph that isn't already present as one
  of its inputs, so S3a's minting wall and S3b's construction gap must be
  scoped/sequenced together, or S3a needs "instantiation must already exist
  elsewhere in the program" stated as an explicit hard precondition with its own
  rejection test.

Full probe report, evidence, and file:line list of every layer touched (parser,
AST, `poly.rs` unify + `apply_subst`, `src/ir/driver.rs` lowering, `src/driver.rs`
plumbing) is preserved in the resolving session; all changes were reverted, only
`.sth` scratch files under `/tmp` were created, and `git status`/`cargo build`
were confirmed clean/green afterward.

## Open questions

1. ~~Does this need a new `PolyType` variant, or can an existing one be
   repurposed?~~ **Resolved above: yes, a new variant, and it costs ~13 exhaustive-
   match arms across 6 files**, several requiring real semantic decisions (Copy-ness
   chief among them) rather than mechanical stubs.

2. ~~Interaction with monomorphization identity.~~ **Resolved above, with a
   caveat: no collision in the probe, but only because grounding was deliberately
   routed through the parse-time dedup table.** The spec must make this routing
   an explicit design decision, since the alternative (independent downstream
   interning, the way arrays/refs already work) would *not* preserve identity for
   free — and downstream, on-demand interning is exactly what's missing for
   generics today (see the registry-lifetime finding above), so the two
   questions (identity, and where minting happens) are the same question.

3. ~~Does this interact with the asymmetric-instantiation hazard already on
   record?~~ **Resolved: no placebo found**, confirmed via a genuine two-variable,
   asymmetric, `nm`-verified probe (`Result['T 'E]` at `[i64 str]` vs.
   `[str i64]`).

4. **Scope: does this need to support a generic applied to a *poly variable of
   a poly variable*, or nesting depth > 1?** (`Box[Box['T]]`). Not exercised by
   the probe (representable in the new variant since `args: Vec<PolyType>` is
   recursive, but grounding was never tested at depth 2, where the "must
   pre-exist" wall would bite on both the inner and outer monomorph). No
   consumer forces this yet; recommend scoping to depth 1, as before.

5. **New (probe): must this slice be sequenced with, or scoped around, the
   generic-construction gap?** The probe found the two gaps are entangled: S3a's
   own hardest case (minting a monomorph that isn't already present elsewhere)
   is unreachable through any program that compiles today, because generic
   construction inside a poly body (`Ok`/`Err` etc. called on a bare `'T`) is
   itself blocked. Options: (a) spec S3a with "instantiation must already exist
   concretely elsewhere in the program" as a stated, tested precondition, and
   defer true on-demand minting to whenever construction is fixed; or (b) treat
   the registry-lifetime fix and the construction fix as one combined unit of
   work, since neither is independently exercisable by real source today. Needs
   a decision before spec-writing, not discovery mid-implementation.

6. **Relationship to P7.S3b.** Independent in mechanism from the *bounds*
   half of S3b (checker-whitelist-and-lowering), but entangled with the
   *construction* gap S3b's own dogfood also hit (see OQ5) — these may need to
   be the same slice, not two independently-sequenced ones. S3b's array-`sort`
   consumer remains independent of all of this and can proceed regardless.

## Out of scope

- Trait bounds (P7.S3b's concern entirely).
- Nesting depth beyond 1 (OQ4), unless a real consumer forces it.
- Any change to how a *concrete* generic argument resolves — that path is
  unaffected and stays exactly as it is.

## Ready to spec?

**Closer, but not yet — one open decision (OQ5) blocks writing the spec's own
scope line.** The probe (worker-verified, full evidence above) resolved the
mechanical unknowns: a new `PolyType::Generic` variant is needed and its cost is
now itemized by file:line; monomorphization identity is preservable, but only by
a routing decision the spec must state explicitly; the asymmetric-instantiation
hazard did not materialize.

What's left is not "probe more," it's **decide**: OQ5 found that this slice's own
hard case (on-demand minting when a monomorph doesn't already exist elsewhere in
the program) is inseparable from the pre-existing generic-construction gap
(`unknown word 'Ok'` on a bare `'T`), because no legal program can exercise one
without the other. Before writing the spec, pick one of OQ5's two options — scope
S3a to "instantiation must already exist concretely elsewhere" with a tested
rejection case, or fold the registry-lifetime fix and the construction fix into
one combined slice — since the spec's exit criteria read differently depending
on which is chosen, and choosing after the spec is written is exactly the kind of
mid-implementation discovery this project's pre-spec discipline exists to avoid.
