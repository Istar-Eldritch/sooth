# Spec: P7.S3e user-declarable trait bounds

**Status:** Draft (v2.1, post-round-1-review corrections)
**Created:** 2025-07-22
**Revised:** 2026-08-22 -- a 3-reviewer soundness/implementability loop found Phase 4's
lowering design non-viable and the cross-module trait-identity story unspecified; this
revision resolves both by construction rather than patching around them. See "Design
decisions resolved this revision" below for the full record.
**Revised again:** 2026-08-23 -- a round-1 review loop against v2 (correctness,
implementability, consistency/citations, 3 reviewers) found no product/scope defects but
five real completeness gaps: an obligation-recording order hazard (decisions 10/R17), an
incomplete barrier-partition justification (decision 7/R10), understated Phase 4
lowering plumbing (R9), an incomplete export-gating citation plus a newly-ruled-on
qualified-trait-reference scope question (decision 4/11/R1/R18), and several drifted
line citations. All five are folded into this revision as corrections, not a redesign --
no decision from the 2026-08-22 revision was reversed.
**Discovery:** /root/code/ordfruma/sooth/docs/roadmap/P7/slice3e-brief.md

## Problem Statement

Polymorphic words today can only state bounds `Copy` (via `poly_is_copy`) or `Ord` (via
`is_ord`, which is a hardcoded numeric predicate, `src/check/poly.rs:11`). These
primitives cannot express user-defined relationships between types for collection keys
or sorting behavior. The `Bound` variant set is closed (`src/ast.rs:1283`), and `Ord`
itself is `is_numeric` and nothing else, so a user struct or enum key is unwritable with
the bounds that exist.

This slice opens `Bound` to a user-declarable third variant, `User(TraitId)`, satisfied
nominally via an `impl:` block, and delivers a trait member call inside a bounded poly
body as a two-time-split resolution: the *obligation* (which trait, which member) is
known abstractly at body-check time; the *symbol* (which concrete word) is known only
once a call site's substitution `θ` is concrete. The design commits to resolving that
gap at check time (never at lowering), by recording the obligation on the abstract body
walk and resolving it against the concrete `θ` at the call site, threading the result
through `CallInst` the same way P7.S3f's `quot_inputs` field already does for a
different phase-split problem.

The forcing consumer is the array form of `sort` (Program 2 in
`docs/roadmap/P7/slice3-dogfood.md`): `'T: Copy Order`, `cmp` as `Order`'s required
member. `Map['K 'V]` is explicitly out of scope (blocked separately on P7.S3n).

## Design decisions resolved this revision

These were open questions or reviewer-found defects in the v1 spec, resolved in a
structured design conversation (parent session, 2026-08-22) before this rewrite. Each is
stated as a ruling, not a menu.

1. **`impl:` is a binding, not a body.** An `impl:` block does not contain code; it maps
   trait member names to *existing, already-declared* concrete words:

   ```
   : point-cmp ( &Point &Point -- Ordering ) ... ;
   impl: Order for Point  cmp point-cmp ;
   ```

   Bare-pair syntax (no `| ... |`, which already means locals and selective-import name
   lists elsewhere), mirroring `type:`'s bare `field Type` pairs and `extern:`'s bare
   `name ( sig ) "symbol"` triple. Multi-member traits repeat the pattern one line per
   member (`impl: Eq for Point  eq point-eq  hash point-hash ;`). This was OQ2's original
   "nominal via `impl: Trait for Type ; <body> ; ;`" ruling; it is now nominal via a
   binding instead of a body. Reason: this removes two of the three Phase 4 blockers a
   reviewer found (no impl-body lowering path exists; a poly word masquerading as a
   member has nowhere to hide) rather than working around them, and it is genuinely
   simpler to implement (impl checking becomes signature comparison, not body checking).
   The implementing word may share the member's name (`impl: Order for Point  cmp cmp
   ;`) -- this is legal and just ordinary static overload resolution (Phase 4 Slice 8)
   picking the right `cmp` by operand type.

2. **A member's implementing word must be concrete, checked at the `impl:` site.** Since
   an impl member is always concrete by construction (its target type is concrete), R14
   from v1 ("a bound-satisfying trait member that is itself a poly word must be
   rejected") **dissolves rather than needing a fix**: it cannot arise. The v1 spec's
   `poly_calls_poly_word_error`-reuse plan is deleted. What replaces it: `impl:`
   validation rejects a binding to a polymorphic word with its own located error at
   declaration time, not at a bounded body's call site. (An impl member's *body* calling
   a poly word is unaffected and stays legal -- that is a concrete word calling a poly
   word, the ordinary monomorphization path.)

3. **Trait identity is a flat, whole-program index, mirroring `StructId`/`EnumId`.**
   `StructDecl`/`EnumDecl` already carry `pub module: u32` and live in a flat registry
   (`src/ast.rs:375-436`), not a per-module table -- `TraitId` is the same shape: an
   index into a flat `traits: Vec<TraitDecl>`, `TraitDecl.module: u32` recording the
   declaring module. This directly fixes a reviewer-found blocker: a `trait_id: usize`
   "unique per module" cannot name a cross-module trait; a whole-program index can, at
   zero new mechanism cost (it is a missed application of an existing pattern, not a new
   one).

4. **Export/import is symmetric with `type:`/`extern:`, with no exception.** The v1
   spec's claim that traits are "implicitly exported... similar to `type:`" is false:
   `type:` requires explicit `export:`. **Correction (round-1 review):** the actual gate
   is not the `not_exported_error` check alone (`src/resolve.rs:270`, downstream) but
   `exportable_names` (`src/resolve.rs:496`), the hand-rolled per-kind name list an
   `export:` line is validated against -- as specified without this correction, `export:
   MyTrait` would fail with a no-origin error before ever reaching `:270`. A `trait:`
   declaration follows the identical rule once `exportable_names` grows a `traits` loop
   (see R1, Codebase Map): `export: Order ;` in its module, `import: mylib::order o |
   Order | ;` to bring it into scope. The orphan rule (an `impl:` must live in the
   trait's or the type's declaring module) is then a direct `module: u32` field
   comparison, no new plumbing.

   **Ruling: a trait named in a bound (`'T: Order`) or in a qualified disambiguating call
   (decision 6/R12) IS in scope this slice.** **Correction (round-2 review):** the
   round-1 correction wrongly modeled this as a `Resolver::rewrite` branch. `rewrite`
   (`src/resolve.rs:723`) rewrites call *names in term bodies*, post-parse
   (`resolve_modules` runs on an already-parsed `&mut Module`, `src/driver.rs:533`). A
   bound is baked into a `Bound::User(TraitId)` at **parse time**, inside
   `parse_capabilities` (`src/parser.rs:2299-2320`) -- by the time `rewrite` runs, the
   trait name is no longer a token to rewrite. A bound-side trait name resolves through
   the parser's own machinery instead, the same `self.imports`/`type_is_exported` gate
   `resolve_type_or_apply` already uses for a qualified generic type header
   (`src/parser.rs:3455-3460`, `:3080`) -- not a new `NameTables`/`rewrite` branch. See
   R18 (corrected).

   R12's qualified member call (`qualifier::member`) needs no new `rewrite` branch
   either: an unrecognized qualified word already falls through `rewrite`'s existing
   word branch to `Ok(None)` (`:288`), left raw for the checker -- which is exactly what
   a bound-directed obligation lookup needs. The one real gap: if the qualifier's target
   module happens to export an unrelated concrete word sharing the member's name,
   `rewrite` mangles the call before the checker ever sees it (`:289-292`), silently
   discarding trait-member intent. R18 states this ordering explicitly rather than
   leaving it implicit.

5. **Multi-bound member-name collision is a call-site rejection, not a declaration-site
   one.** v1's R8 rejected `'T: A B` outright if both traits require a same-named
   member, even in a body that never calls that member -- this makes composing two
   traits you don't control impossible for no reason tied to the actual conflict.
   Revised: `'T: A B` stays legal; the ambiguous *call* is the error:

   ```
   error: `t1` is required by both `A` and `B` on 'T (line N, col C)
     note: a member required by two of a variable's bounds cannot be called unqualified
   ```

   This is also cheaper to implement: the bound-directed dispatch branch is already
   walking the bound set looking for one matching member; finding two is the natural
   error site, no extra pass.

6. **Module-qualified calls disambiguate a collision when the two traits live in
   different modules; same-module collisions have no escape hatch this slice.** Reuses
   the existing `qualifier::name` mechanism verbatim (`src/parser.rs:3449`,
   `self.imports.get(qualifier)` -- a *module* alias, confirmed by reading the code, not
   a trait-name namespace). `o::t1` (where `o` is the import alias for the module
   declaring `A`) resolves unambiguously if `A` and `B` are declared in different
   modules. If both traits are declared in the *same* module, they share a qualifier and
   the collision cannot be broken this way -- ruled to stay a hard rejection for this
   slice (no trait-name-based qualifier syntax; no forcing consumer for one yet).

7. **Bound-directed dispatch and ordinary env lookup partition cleanly; no precedence
   rule needs inventing -- but the partition is enforced by three barriers, not one.**
   **Correction (round-1 review):** decision 7 originally credited
   `poly_var_to_concrete_error` alone; that arm (`src/check/poly.rs:1224-1225`) fires
   only when a *non-builtin-named* call matches a concrete overload with a mismatched
   operand. A bare/ref-to-bare type variable at a concrete-typed operand position
   actually partitions through three separate paths depending on shape: (a) a
   non-builtin-named call reaches `poly_var_to_concrete_error` (`:1225`) as before; (b) a
   builtin-*named* call (an operator-spelled member, e.g. `+`) never reaches that arm at
   all -- `exact` is always false for a `Var` operand (`:1215`), so control falls to
   `poly_delegate_op` (`:1305`, def `:3111`), whose concrete-suffix extraction
   (`:3119-3126`) stops before the `Var`, yielding `Ok(None)` and ultimately
   `poly_calls_poly_word_error`/`unknown_word_error` (`:1360-1366`); (c) a
   `PolyType::Ref(Var, _)` operand falls to the `other =>` arm at `:1290` ->
   `poly_op_on_variable_error`. All three are hard errors before this slice, so no
   concrete overload was ever reachable from any of these shapes -- the conclusion holds,
   but R10's pinned tests must cover a builtin-named trait member (an operator-spelled
   member name) as well as the ordinary case, or the partition claim ships tested for
   only one of its three shapes.

8. **Lowering: impl members desugar to ordinary concrete words (Part 1); a
   check-time-resolved obligation threads through `CallInst` (Part 2).** This replaces
   v1's entire Phase 4 lowering design, which a reviewer correctly found architecturally
   non-viable (three concrete gaps: no call-site marker for lowering to find a
   bounded-method call; no impl-body lowering path exists in `lower()` at all, which
   only walks `module.words` and `module.instantiations`; and "mint fresh records during
   lowering" is itself the re-resolution R7 forbids, since no earlier phase determines
   the concrete symbol). See Solution Approach's "Lowering and instantiation layer" for
   the full mechanism; it needs no new lowering pass, no new `IrFunc` emission, and no
   invariant renegotiation, because it makes both of those problems not exist rather
   than solving them under the wrong design.

   **Correction (round-1 review, then re-corrected in round-2 review):** round-1 found
   that `lower_instantiation` never receives a `CallInst`, and (over-)corrected by
   rejecting the `CallInst` field entirely and redirecting the map through
   `lower_instantiation`. Round-2 found this redirection targets the wrong function:
   `lower_instantiation` (`src/ir/driver.rs:788`) has exactly one call site in the whole
   tree, `src/repl.rs:1554` -- it is **REPL-only**. The compiled build never calls it;
   `lower` inlines monomorphized instantiations directly (`src/ir/driver.rs:249-286`,
   `for (symbol, inst) in distinct`), and at that loop `inst: &CallInst` **is already in
   hand** (`:270`) before calling `lower_word_parts` directly (`:271`) with
   `&module.builtin_overloads` (`:282`). So the original `CallInst` field design (Part 2
   above) was right for the path that matters; round-1's blanket claim ("`CallInst` does
   not reach the lowering site that needs this data") was true only of the REPL path and
   wrongly generalized. **Restored ruling:** the per-instantiation map IS a new field on
   `CallInst` (or `Module`, whichever the existing `subst`/output-type fields already
   live on for the compiled instantiation record -- confirm at implementation time), read
   at `src/ir/driver.rs:271-286`'s existing loop and threaded into `lower_word_parts`
   exactly the way `builtin_overloads` already is there. The REPL path
   (`lower_instantiation`/`repl.rs:1554`) is explicitly out of scope for this field --
   consistent with the already-documented REPL-bypass pattern for `uncalled_operator_overloads`
   (R15) -- and must not be assumed to gain trait-dispatch coverage by this slice.

9. **A trait member is callable via a bound without importing the implementing word's
   name; a direct call to that name still needs an ordinary import.** Dispatch through a
   bound resolves via the trait table (for the signature) and the whole-program impl
   registry (for the symbol, keyed by `(TraitId, Type)`) -- neither is scoped by the
   caller's import list, mirroring how `drop` overload dispatch is already whole-program
   and type-directed (`find_drop_overloads`, walks all words, keyed by struct id, never
   imported by name). The orphan rule is what keeps this coherent: importing both the
   trait and the concrete type guarantees the declaring module (which must be one of
   theirs) is already in the build. Calling the same word *by name*, outside a bound,
   is ordinary name resolution and needs the ordinary import. One concrete hazard this
   creates: an impl member reachable only via bound dispatch, whose name collides with a
   builtin operator name, would be dead-code-pruned by the existing
   `uncalled_operator_overloads` filter (`src/ir/driver.rs:102-116`) unless that filter
   also consults the new per-instantiation obligation-resolution record the way it
   already consults `builtin_overloads.values()`. This is now R15 below; needs a
   dedicated test, not just a mention.

10. **Obligation recording must not silently depend on source order.** **Found in
   round-1 review:** `check_poly_body` and `check_poly_call` are not two clean passes --
   both run interleaved, in one source-order loop (`src/check.rs:758`). A monomorphic
   word declared *before* the polymorphic word it calls reaches `check_poly_call`'s
   bound loop while that callee's own obligation list (recorded by its own, *later*,
   `check_poly_body` pass) is still empty. As drafted, R8's "match the recorded
   obligation" step would then silently find nothing and record no `trait_calls`,
   producing an unresolved call at lowering with no diagnostic pointing at the cause.
   Resolved by a pre-pass (R17): collect obligations for every poly word in the module
   before the main check loop runs any call-site check, and make an unmatched obligation
   at resolution time a located internal-consistency error rather than a silent no-op
   (a cheap backstop if the pre-pass ever misses a shape). The poly-combinator-standalone
   path (`src/check.rs:772-782`) already uses scratch maps distinct from the module-level
   ones; the obligation pre-pass's map must be scratch there too, or a combinator's
   obligations would leak into real instantiations.

11. **A trait name in a bound resolves at parse time, not via `Resolver::rewrite`; a
   qualified member call needs an explicit fallthrough-ordering ruling, not a new
   branch.** **Found in round-1 review, corrected again in round-2 review:** round-1's
   R18 modeled bound-side trait resolution as a new `NameTables::build`/`rewrite` branch.
   Round-2 found this backwards: `rewrite` operates post-parse on term bodies
   (`src/resolve.rs:723`), but a bound is already baked into `Bound::User(TraitId)` at
   parse time by `parse_capabilities` (`src/parser.rs:2299-2320`) -- by the time
   `rewrite` would run, there is no trait-name token left to rewrite. The correct
   mechanism is the parser's own qualified-name resolution, the same
   `self.imports`/`type_is_exported` gate `resolve_type_or_apply` already uses for a
   qualified generic type header (`src/parser.rs:3455-3460`, `:3080`), invoked while
   `parse_capabilities` builds the bound. Separately, R12's qualified member call
   (`qualifier::member`) needs no new branch at all: an unrecognized qualified word
   already falls through `rewrite`'s existing branch to `Ok(None)` (`:288`), which is the
   behavior a bound-directed obligation lookup wants. The one real gap `rewrite`
   introduces: if the qualifier's target module happens to export an unrelated concrete
   word sharing the member's name, `rewrite` mangles the call before the checker ever
   sees it (`:289-292`), silently discarding the trait-member intent -- R18 states this
   ordering explicitly (the obligation-recording branch must run, or be checked for,
   before this mangling can apply) rather than leaving it as an implicit assumption.

12. **Three additional gaps in the export/import and duplicate-declaration story,
   found in round-2 review.** (a) `local_decl_names`
   (`src/check/declarations.rs:562-586`) is a *third* hand-rolled per-kind name list,
   feeding `check_selective_imports`' local-vs-selective-import collision check
   (`:514`) -- it also needs a `traits` branch, or a locally declared `trait: Show`
   alongside `import: dep | Show |` produces two live, uncaught `Show` bindings. (b) The
   parse-time qualified-type export gate (`resolve_type_or_apply`'s
   `type_is_exported`/`not_exported_error`, `src/parser.rs:3455-3460`/`:3080`) has no
   trait-shaped twin; a qualified trait reference in a bound (`'T: q::Order`) needs the
   identical gate, not `Resolver::rewrite`'s post-parse mechanism (see decision 11). (c)
   No shared cross-kind duplicate-name check exists today -- `check_duplicate_type_names`
   (`src/check/declarations.rs:873`), `check_duplicate_word_names` (`:932`), and
   `duplicate_static_error` (`:376`) are each independently scoped to their own kind, so
   R2's "fails as an ordinary duplicate-declaration error" needs its own new
   `check_duplicate_trait_names`, and nothing today would reject `trait: Point`
   coexisting with `type: Point` in one module. All three are folded into R1/R18 below.

## Requirements

- **R1.** The parser must accept a top-level `trait: TraitName 'T member ( &'T ... -- ...
  ) ... ;` declaration with one or more required member signatures over exactly one
  type variable (single-type-variable traits only, dogfood-confirmed sufficient -- see
  brief OQ3). A trait declaration is module-scoped and requires explicit `export:` to
  cross a module boundary, exactly like `type:`/`extern:` (decision 4). Concretely, this
  requires a `traits` branch in three places, all found by round-1/round-2 review, not
  one: (a) `exportable_names` (`src/resolve.rs:496`) gains a `traits` loop mirroring its
  existing `generic_structs` loop -- without it, `export: TraitName ;` fails with a
  no-origin error before reaching the existing `not_exported_error` check (`:270`); (b)
  `local_decl_names` (`src/check/declarations.rs:562-586`) gains a `traits` loop, or a
  locally declared trait colliding with a selectively imported one of the same name goes
  uncaught by `check_selective_imports` (`:514`); (c) a new `check_duplicate_trait_names`
  function, mirroring `check_duplicate_type_names` (`:873`)/`check_duplicate_word_names`
  (`:932`), since no shared cross-kind duplicate check exists today -- without it,
  `trait: Point` and `type: Point` in the same module both silently declare.

- **R2.** The parser must replace the two hardcoded string compares for `Copy`/`Ord` in
  `parse_capabilities` (`src/parser.rs:2299`, string matches at `:2305`/`:2309`) with a
  single lookup against a pre-seeded trait table where `Copy`/`Ord` are **predicate-kind**
  entries (satisfaction still runs `is_copy`/`is_ord`, unchanged). A user `trait: Copy`
  or `trait: Ord` fails as an ordinary duplicate-declaration error, in the same message
  style as the existing word/type/static duplicate errors -- not a bespoke reserved-word
  check.

- **R3.** `TraitId` is an index into a flat, whole-program `traits: Vec<TraitDecl>`,
  mirroring `StructId`/`EnumId`'s existing shape (decision 3). `TraitDecl` carries
  `module: u32` (declaring module, for the orphan rule and export gating) alongside
  `name` and `members: Vec<TraitMember>` (`TraitMember { name: String, sig: Sig }`).

- **R4.** An `impl: Trait for Type  member1 word1 [member2 word2 ...] ;` block is a pure
  binding (decision 1), not a body: bare `member word` pairs, one per required member,
  each `word` naming an existing, already-declared word whose signature matches the
  trait's declared member signature with `'T` substituted for the concrete `Type`. The
  orphan rule: the `impl:` block's own module must equal either the trait's `module` or
  the target type's declaring module, checked as a direct field comparison (decision 4).
  Every required member must be bound (partial impls are rejected, same rule as v1's
  R6/OQ resolution: reject on any missing member). No two `impl:` blocks may exist for
  the same `(TraitId, Type)` pair (duplicate-declaration style error). Each bound word
  must be concrete (not a polymorphic word) -- a located rejection at the `impl:` site
  names the trait, member, and the offending word (decision 2; this is what replaces
  v1's R14, which is deleted as dissolved rather than fixed).

- **R5.** Bounds on type variables follow the existing grammar unchanged: a bound rides
  on the variable's *first occurrence* in the signature (`( &'T: Show &'T -- )`, not
  `( 'T: Show &'T &'T -- )`, which would declare a spurious bare `'T` input) --
  confirmed accurate against `intern_ty_var`'s binding-occurrence tracking and
  `bound_on_use_error`'s rejection of a bound on a non-binding occurrence
  (`src/parser.rs:974-977`, `:2107-2141`, `:2136`). The capability list composes
  (`'T: Copy Order`), same shape as today's `'T: Copy Ord`, greedy over recognized trait
  names looked up in the trait table.

- **R6.** A bound constraint desugars into a `Vec<(u32, Bound)>` entry on
  `PolySig.bounds`, the `u32` indexing `PolySig.ty_var_names`. `Bound` gains a third
  variant, `User(TraitId)`, alongside the existing `Copy`/`Ord` (`src/ast.rs:1283`).

- **R7.** During body checking, `poly_call_term`'s `env.get(name)` lookup
  (`src/check/poly.rs:1197`) and its `PolyType::Var(v)` rejection arm
  (`poly_var_to_concrete_error`, `src/check/poly.rs:1224`) gain a preceding branch: if
  the top-of-stack slot is `PolyType::Ref(PolyType::Var(v), _)` or bare
  `PolyType::Var(v)`, and `v`'s bound set has a `Bound::User(trait_id)` whose trait
  declares a member matching the called name, this is a **trait-member obligation**, not
  an ordinary call. Push the trait's declared abstract output types (the signature is
  known; the symbol is not -- `'T` is still abstract here). **Also record the obligation**
  -- `(call_span, var_id, trait_id, member_name)` -- in a new per-word obligation list
  threaded through the checker's call-site machinery to `check_poly_call` (decision 8,
  Part 2). This is the corrected half of v1's R6, which said "no symbol, no site table"
  and thereby created the Phase 2->4 gap a reviewer found: the site table is exactly
  what closes that gap, recorded here abstractly and resolved concretely in R9. If two
  bounds on `v` both declare a matching member, this is the ambiguous-call case
  (decision 5) -- reject with the located error naming both traits, unless the call is
  module-qualified and the qualifier disambiguates (decision 6).

- **R8.** `check_poly_call`'s existing bound loop (`src/check/poly.rs:3535-3548`, `for
  (v, bound) in &sig.bounds`) gains a `Bound::User(trait_id)` arm: given the concrete
  `ty = subst.ty_of(v)`, look up `(trait_id, ty)` in the whole-program impl registry. If
  absent, emit a located error naming the trait, the missing member's full signature,
  and the concrete type (e.g., `` `i64` does not satisfy `Show`: no `( &i64 -- )`
  found ``). If present, for each obligation recorded in R7 whose `trait_id`/`var_id`
  match, resolve the concrete implementing word's mangled symbol and record `(call_span,
  symbol)` (this instantiation's `θ`-specialized value, not a module-global one) in a
  new per-instantiation span->symbol map (see R9 for exactly how this threads to
  lowering). If R17's pre-pass recorded an obligation for this call site but no matching
  entry is found here, this is a located internal-consistency error (R17), distinct
  from the bound-not-satisfied diagnostic above. This is where resolution actually
  happens -- once, at check time, with the concrete `Subst` already in hand -- so
  "lowering never re-runs resolution" is preserved for real rather than by wording.

- **R9.** Lowering makes no resolution decision. **Correction (round-1 review), then
  re-corrected (round-2 review):** round-1 found the original wording ("reads
  `CallInst.trait_calls` the same way it already reads `builtin_overloads`/`quot_inputs`")
  overstated how free this is, since `quot_inputs` is consumed at a structurally
  different, caller-side site -- true, but round-1 then over-corrected by redirecting the
  map through `lower_instantiation`, which round-2 found is **REPL-only**
  (`src/ir/driver.rs:788`, one call site in the whole tree: `src/repl.rs:1554`). The
  compiled build never calls it; `lower` inlines monomorphized instantiations directly
  (`src/ir/driver.rs:249-286`), and at that loop `inst: &CallInst` **is already in hand**
  (`:270`) before `lower_word_parts` is called directly (`:271`) with
  `&module.builtin_overloads` (`:282`). **Restored ruling:** the per-instantiation map IS
  a new field alongside `CallInst`'s existing per-instantiation data (confirm the exact
  host struct/field layout at implementation time against the compiled-path record that
  already carries `subst`/output types for this loop -- not the REPL's `PolyWordEntry`,
  which is REPL-only), populated by R8's resolution and read at
  `src/ir/driver.rs:271-286`'s existing loop, threaded into `lower_word_parts`
  (`src/ir/func_builder/mod.rs:718`, assignment at `:748`, `FuncBuilder` field at
  `:185` -- correcting the round-1 citation, which pointed at `driver.rs:283,428`,
  `:428` being `lower_line`'s REPL-line assignment, not `lower_word_parts`'s) exactly the
  way `builtin_overloads` already is. `lower_word_parts`'s other callers
  (`driver.rs:200`, `:759` `lower_word`, `func_builder/mod.rs:936`,
  `ir/destructors.rs:384`, and existing test call sites) pass an empty default via the
  existing `empty_builtin_overloads()` convention (`src/ir.rs:69`) for the new parameter,
  the same way they already do for `builtin_overloads` on paths that don't need it. The
  REPL path (`lower_instantiation`/`repl.rs:1554`) is explicitly out of scope for this
  field -- consistent with the already-documented REPL-bypass pattern for
  `uncalled_operator_overloads` (R15) -- and must not be assumed to gain trait-dispatch
  coverage by this slice. No new lowering pass, no new `IrFunc` emission path -- an impl
  member is an ordinary concrete word (R4/R11) and lowers through the existing
  word-lowering path exactly once, regardless of how many poly-word instantiations call
  it.

- **R10.** Bound-directed dispatch (R7) and the ordinary `env.get(name)` lookup
  partition cleanly and provably (decision 7): a bare/ref-to-bare type variable at a
  concrete-typed operand position was already an unconditional error before this slice,
  via one of three barriers depending on call shape --
  `poly_var_to_concrete_error` (`src/check/poly.rs:1225`) for a non-builtin-named call,
  `poly_delegate_op`'s concrete-suffix truncation (`:3111`, `:3119-3126`) for a
  builtin-named call, and `poly_op_on_variable_error` (`:1290`) for a `Ref(Var, _)`
  operand -- so no concrete overload was ever reachable from any of these shapes.
  Pinned unit tests must cover all three: a bounded call with a plain (non-operator)
  member name dispatching correctly, a bounded call whose member name is
  operator-spelled (e.g. a trait requiring `+`) dispatching correctly, and an unrelated
  concrete overload of the same name remaining reachable from a genuinely concrete
  receiver in each case.

- **R11.** Two `impl:` blocks for the same `(TraitId, Type)` are a duplicate-declaration
  error (folded into R4, restated for clarity as its own testable point).

- **R12.** A member name required by two traits in one variable's bound set is legal to
  *declare* (`'T: A B` parses even if both declare `t1`) and illegal to *call*
  unqualified: a located rejection naming both traits, the member, and the bound
  variable (decision 5). A module-qualified call (`qualifier::name`, reusing
  `src/parser.rs:3449`'s existing import-alias resolution) disambiguates when `A` and
  `B` are declared in different modules; same-module collisions have no escape hatch
  this slice (decision 6).

- **R13.** All stage functions (`lex`, `parse`, `check`, `lower`) get unit tests beside
  them: the happy path for each new grammar/check path, plus at least one error/edge
  case per branch (duplicate trait/impl, orphan violation, missing member, polymorphic
  impl-member rejection, ambiguous unqualified call, qualified-call disambiguation,
  bound-directed-vs-concrete-env partition). Every phase exit criterion is a golden test
  in `tests/phase7_slice3e.rs` (one file, matching every landed P7.S3 slice's
  convention -- `phase7_slice3{a,b,c,d,f,g,i}.rs` are each a single file; no
  `.golden.sth` sidecar convention exists anywhere in `tests/`, goldens are inline via
  `Scratch::write`/`check_err`). Golden tests assert the *exact* wording of diagnostics,
  not just presence.

- **R14.** *(deleted -- see decision 2: the shape R14 guarded against cannot arise under
  impl-as-binding, since a member's implementing word is checked concrete at the `impl:`
  site.)*

- **R15.** An impl member reachable only through bound-directed dispatch (never called
  by name) must not be dead-code-pruned. `uncalled_operator_overloads`
  (`src/ir/driver.rs:102-116`) currently spares a word if its symbol appears in
  `module.builtin_overloads.values()`; it must be extended to also check the new
  per-instantiation trait-call symbols recorded per R9 (accessible via
  `module.instantiations`, already in scope at this point in `lower`, confirmed
  reachable with no restructuring), or an impl member whose name happens to collide with
  a builtin operator name (e.g. a trait requiring `add`) would be silently pruned before
  it is ever called. Two conditions confirmed by round-1 review: (a) the stored symbol
  must be the exact resolved lowering symbol, matching `overload_symbols`'s spelling
  (`src/ir/driver.rs:99`) byte-for-byte, or the spare silently misses; (b) this filter
  does not run for the REPL's word-lowering path (`driver.rs:98-100`; session defs lower
  via `lower_word`), so R15 gives no REPL coverage of bound-directed dispatch -- this is
  the same known REPL-bypass pattern already true of drop/operator overloads and is not
  a regression this slice introduces, but it must not be assumed fixed by this slice's
  golden either. This needs its own golden: an impl member named after a builtin
  operator, called only via a bound, must run in a compiled (non-REPL) build.

- **R16.** Multi-type-variable traits and the compiler-known third trait kind
  (`bool`-shaped/`Fallible`) are explicitly out of scope -- single-variable user traits
  and the two existing predicate-kind traits (`Copy`/`Ord`) only, per the brief's OQ3
  and its "test before designing it as a trait" ruling on the third kind.

- **R17.** *(added, round-1 review; decision 10; identity ruled in round-2 review)*
  Obligation recording (R7) is order-independent: a pre-pass collects obligations for
  every poly word in the module before the main check loop (`src/check.rs:758`) runs
  `check_poly_call` for any call site, so a monomorphic caller declared before its
  polymorphic callee's obligations are recorded still sees them at resolution time.
  **Ruling (round-2 review found this unruled in round-1):** the pre-pass **is
  `check_poly_body` hoisted**, not a second, parallel obligation-only walk -- a separate
  walk would have to duplicate `check_poly_body`'s traversal and trait-member resolution
  exactly, which is strictly worse (two places that must agree forever) than running the
  real thing once, earlier. This has a real consequence the risk table must carry: since
  `check_poly_body`'s errors propagate with `?` (`src/check.rs:810-822`), hoisting it
  means **every poly-body diagnostic in a module now fires before any monomorphic word's
  diagnostic**, changing first-error ordering for any file mixing both kinds of error.
  This is a new regression-risk row (see Risks table), not just an implementation
  footnote. If R8's resolution step finds no matching pre-pass obligation for a call
  span it expected one for, this is a located internal-consistency error, not a silent
  no-op. The pre-pass's obligation map is scratch (not module-level state) inside the
  poly-combinator-standalone check path (`src/check.rs:772-782`), matching that path's
  existing scratch-map handling for everything else it tracks. Round-2 review also
  confirmed there is no second, cross-module ordering hazard: `assemble_module`
  (`src/driver.rs:290-299`) flattens the whole import closure into one `Module` before
  checking runs, so "every poly word in the module" already means every poly word in the
  program, and a poly body calling another poly word is independently a hard error
  (`poly_calls_poly_word_error`, `src/check/poly.rs:1364`) -- so the pre-pass's
  dependency graph is depth-1 by construction, never chained.

- **R18.** *(added, round-1 review, decision 11; mechanism corrected in round-2 review)*
  A trait name in a bound (`'T: Order`) resolves at **parse time**, through the same
  `self.imports`/`type_is_exported` gate `resolve_type_or_apply` already uses for a
  qualified generic type header (`src/parser.rs:3455-3460`, `:3080`), invoked from
  inside `parse_capabilities` (`src/parser.rs:2299-2320`) as it builds the bound.
  **Correction (round-2 review):** round-1's R18 wrongly modeled this as a
  `NameTables::build`/`Resolver::rewrite` branch; `rewrite` operates post-parse on term
  bodies (`src/resolve.rs:723`), but a bound is already baked into a concrete
  `Bound::User(TraitId)` at parse time, before `rewrite` ever runs -- there is no
  trait-name token left for `rewrite` to see. Separately, R12's qualified member call
  (`qualifier::member`) needs **no new branch**: an unrecognized qualified word already
  falls through `rewrite`'s existing word branch to `Ok(None)` (`:288`), left raw for the
  checker, which is exactly the behavior a bound-directed obligation lookup wants. The
  one real ordering gap this leaves: if the qualifier's target module happens to export
  an unrelated concrete word sharing the member's name, `rewrite` mangles the call before
  the checker ever sees it (`:289-292`), silently discarding the trait-member intent --
  the obligation-recording branch (R7) must be checked, and take precedence, before this
  mangling is allowed to apply. This is a located requirement (not an assumption): a
  golden must exercise exactly this collision shape (a qualified member call where the
  qualifier's module also exports an unrelated same-named concrete word) and assert the
  trait-member call wins.

## Success Criteria

- [ ] `trait: Show 'T show ( &'T -- ) ;` parses and is rejected as a duplicate on a
      second declaration in the same module (same message shape as `type:`/`static:`).
- [ ] `: foo ( 'T: Show -- ) show ;` body-checks successfully against a user `Show`
      trait, pushing the trait's declared abstract outputs.
- [ ] `impl: Show for i64  show int-show ;` (with `int-show` declared separately)
      validates against the trait and registers in the whole-program impl registry, with
      a located orphan-rule error when the impl lives outside the trait's or the type's
      module.
- [ ] `impl: Show for i64  show poly-thing ;` where `poly-thing` is a polymorphic word is
      a located rejection at the `impl:` site (decision 2/R4), not at any later call
      site.
- [ ] A poly word bounded by `'T: Show`, instantiated at a concrete type with no
      satisfying impl, fails with `` `String` does not satisfy `Show`: no `( &String --
      )` found (line N, col C) ``.
- [ ] `'T: A B` where both `A` and `B` require `t1` parses without error; calling `t1`
      unqualified inside that body is the located rejection (decision 5/R12); calling
      `qualifier::t1` resolves correctly when `A`/`B` are declared in different modules.
- [ ] The array `sort` consumer (`'T: Copy Order`, `cmp` via `Order`) compiles, lowers,
      and runs correctly at two distinct concrete instantiations (e.g. `i64` and `f64`),
      each dispatching to its own `impl:`'s `cmp` symbol.
- [ ] An impl member named after a builtin operator, reachable only via bound dispatch,
      is not dead-code-pruned and runs correctly (R15).
- [ ] A bounded call and an unrelated concrete overload of the same name coexist without
      either shadowing the other (R10).
- [ ] Unit tests exist for every new stage function/branch listed in R13, both happy and
      error paths.
- [ ] Golden tests in `tests/phase7_slice3e.rs` (one file) pass; `cargo fmt --check &&
      cargo clippy --all-targets -- -D warnings && cargo test` green.
- [ ] No `Map`, no generic-instantiation-over-own-variable, no multi-type-variable
      trait, no third trait kind appears anywhere in the slice's scope.

## Scope & Boundaries

**In scope:**

- Parser: `trait:` declaration grammar, `impl:` binding-block grammar (bare pairs, no
  body).
- AST: `TraitDecl { name, module: u32, span, members: Vec<TraitMember> }`,
  `TraitMember { name, sig: Sig }`, `ImplDecl { trait_id: TraitId, target_ty: Type,
  module: u32, span, bindings: Vec<(String, String)> }` (member name -> implementing
  word name), a flat whole-program `traits: Vec<TraitDecl>` on `Module` (or an
  equivalent whole-program registry matching how `structs`/`enums` are organized today
  -- confirm exact placement against `Module`'s actual field layout before implementing,
  do not assume a `HashMap` where the existing pattern is a `Vec`), and a whole-program
  impl registry keyed by `(TraitId, Type)`.
- `Bound::User(TraitId)` variant; `Copy`/`Ord` pre-seeded as predicate-kind trait-table
  entries.
- The bound-directed dispatch branch in `poly_call_term` (R7) and its obligation
  recording, including the order-independence pre-pass (R17).
- The `Bound::User` arm in `check_poly_call`'s bound loop (R8), including concrete
  resolution and population of a new per-instantiation span->symbol map (R9).
- The new per-instantiation span->symbol map itself, as a new `CallInst` field, read at
  the compiled-path instantiation loop (`src/ir/driver.rs:249-286`) and threaded
  through `lower_word_parts`/`FuncBuilder` alongside the existing module-global
  `builtin_overloads` map (R9) -- NOT through `lower_instantiation`, which is REPL-only.
- The `uncalled_operator_overloads` extension (R15).
- Ambiguous-member-call rejection and qualified-call disambiguation (R12) -- no new
  `Resolver::rewrite` branch needed (an unrecognized qualified word already falls
  through); the ordering ruling that a trait obligation must pre-empt `rewrite`'s
  ordinary word mangling (R18).
- A bound's trait name resolving at parse time via `parse_capabilities`, reusing
  `resolve_type_or_apply`'s existing `self.imports`/`type_is_exported` gate (R18).
- The `exportable_names`, `local_decl_names`, and new `check_duplicate_trait_names`
  extensions for `trait:` (R1).
- Unit and golden tests per R13.

**Out of scope:**

- Trait objects / runtime dispatch (`dyn Show`, `^Any`) -- compile-time only, per
  DESIGN.md.
- Associated types, default method bodies, blanket impls, supertraits, generic
  constants.
- Multi-type-variable traits.
- The compiler-known third trait kind (`Fallible`/`bool`-shaped).
- `Map['K 'V]` and any consumer needing generic-struct-array-of-own-type-variable
  fields (blocked separately on P7.S3n) or generic-instantiation-over-own-variable
  beyond what P7.S3a already landed.
- A trait-name-based qualifier syntax for same-module member collisions (decision 6) --
  the existing module-alias `::` mechanism is reused as-is, nothing new is added to it.
- Any new lowering pass or `IrFunc` emission path -- R9 is explicit that none is needed.

## Solution Approach

Four layers, each with a direct precedent in already-landed code, built in dependency
order.

**AST layer.** `TraitDecl`/`TraitMember`/`ImplDecl` mirror `StructDecl`/`EnumDecl`'s
existing shape (`src/ast.rs:375-436`): a flat, whole-program registry with a `module:
u32` field per entry, not a per-module table. `TraitId`/`ImplId` (if needed) are plain
indices into these `Vec`s, exactly like `StructId`/`EnumId` are today. `ImplDecl` holds
`bindings: Vec<(String, String)>` (member name, implementing word name) rather than a
body -- there is no member body to check, only a signature comparison.

**Parser layer.** `parse_trait_decl` follows `type:`'s dispatch shape (declaration
keyword, name, then a body list) but the "body" is member signatures under one type
variable. `parse_impl_decl` follows `extern:`'s bare-pair shape (`extern: name ( sig )
"symbol"` becomes `impl: Trait for Type  member word ...`) -- no braces, no `| ... |`,
no term list. `parse_capabilities` (`src/parser.rs:2299`) is rewritten from two string
compares into one trait-table lookup; `Copy`/`Ord` are pre-seeded predicate-kind
entries so they hit the same lookup path a user trait does, and a colliding user
`trait: Copy` becomes an ordinary duplicate-declaration error rather than a reserved-word
check.

**Check layer -- the obligation/resolution split (decision 8, the core of this
slice).** `check_poly_body` (`src/check/poly.rs:393`) walks a poly word's body exactly
once, abstractly, with no concrete `Subst` in sight -- this is where R7's new branch
lives, and it can only ever record an *obligation* (which trait, which member), never a
symbol. `check_poly_call` (`src/check/poly.rs:3460`) runs per call site with a concrete
`Subst` already unified -- this is where R8's extended bound loop lives, and it is the
first and only point where `θ(T)` is known, so it is the first and only point capable of
resolving a concrete symbol.

**Correction (round-1 review, decision 10/R17; identity ruled in round-2 review):**
`check_poly_body` and `check_poly_call` are not two clean sequential passes -- both run
interleaved inside one source-order loop (`src/check.rs:758`). Without a pre-pass, a
monomorphic word declared before its polymorphic callee would reach `check_poly_call`'s
bound loop before that callee's own `check_poly_body` pass has recorded any obligations.
R17 closes this with a pre-pass, ruled to be `check_poly_body` **hoisted** (run once,
early, for every poly word) rather than a second parallel obligation-only walk -- a
separate walk would have to duplicate `check_poly_body`'s traversal exactly, which is a
permanent two-places-must-agree liability the hoist avoids. This changes first-error
ordering (every poly-body diagnostic now fires before any monomorphic word's, since
`check_poly_body`'s errors propagate with `?` at `src/check.rs:810-822`) -- a named risk,
not an implementation footnote (see Risks table). An unmatched obligation at resolution
time (R8) is a located internal-consistency error rather than a silent no-op.

The obligation list, once collected by the pre-pass, threads from the body walk to the
call site (both already operate on the same `PolySig`/call context), and a new
per-instantiation span->symbol map (see R9's correction) carries the resolved `(span,
symbol)` pairs onward to lowering.

**Lowering layer.** **Correction (round-1 review), re-corrected (round-2 review):**
round-1 found the original version assumed `lower_instantiation` already holds a
`CallInst`; it does not. Round-1 then redirected the map through `lower_instantiation`,
which round-2 found is **REPL-only** (`src/ir/driver.rs:788`, sole call site
`src/repl.rs:1554`). The compiled build inlines monomorphized instantiations directly
(`src/ir/driver.rs:249-286`), where `inst: &CallInst` is already in hand (`:270`) before
`lower_word_parts` is called (`:271`). Corrected mechanism, three parts:

- *Impl members lower as ordinary words.* Since R4 requires an `impl:` binding's target
  to be an already-declared, already-concrete word, there is nothing impl-specific to
  lower -- `int-show` (say) is mangled and lowered through the existing per-module word
  path (`resolve::mangle`, `lower()`'s `module.words` walk) exactly once, the same as
  any other concrete word, regardless of how many poly-word instantiations dispatch to
  it via a bound. No new `IrFunc` emission path, satisfying R9 directly.
- *Bound-directed calls lower as ordinary calls, via a new per-instantiation map on the
  compiled-path instantiation record, precedented by `builtin_overloads` (not
  `quot_inputs`, and not routed through `lower_instantiation`).* R8's resolution produces
  a `HashMap<Span, String>` per instantiation, stored alongside `CallInst`'s existing
  per-instantiation data and read at the existing `src/ir/driver.rs:249-286` loop, then
  threaded into `lower_word_parts` (`src/ir/func_builder/mod.rs:718`, `FuncBuilder`
  field `:185`, assignment `:748` -- the real precedent pair; not `driver.rs:283,428`,
  where `:428` is `lower_line`'s REPL-line assignment) alongside `builtin_overloads`.
  `lower_word_parts`'s other callers pass an empty default via the existing
  `empty_builtin_overloads()` convention (`src/ir.rs:69`). This is new, real plumbing --
  not a pre-existing read path -- and is explicitly compiled-path-only; the REPL's
  `lower_instantiation`/`PolyWordEntry` path does not gain this data this slice.
- *Dead-code pruning (R15).* `uncalled_operator_overloads`
  (`src/ir/driver.rs:102-116`) must also treat a symbol appearing in some
  instantiation's new per-instantiation map as "called," the same way it already treats
  `builtin_overloads.values()` -- `module.instantiations` is already in scope at this
  point in `lower` (read again later at `:244`), so this requires no restructuring,
  just an additional `.any(...)` clause. Two conditions apply (R15): the stored symbol
  must exactly match `overload_symbols`'s spelling (`:99`), and this filter does not run
  for the REPL's `lower_word` path.

**Ambiguity and qualification (R12), and bound-side trait resolution (R18).** The
obligation-recording branch in `poly_call_term` (R7) walks `v`'s bound set looking for a
trait declaring the called name; finding two is the natural, no-extra-pass detection
point for the ambiguous-call rejection. A module-qualified call (`qualifier::name`) is
parsed and resolved exactly the way any other qualified name is (`src/parser.rs:3449`,
`self.imports.get(qualifier)`) -- no new `Resolver::rewrite` branch is needed, since an
unrecognized qualified word already falls through to `Ok(None)` there (`:288`); the
obligation-recording branch, seeing a qualifier, restricts its search to the trait
declared in that qualifier's target module. **Correction (round-2 review):** a trait
name *in a bound* is a separate, earlier concern -- it resolves at **parse time**, inside
`parse_capabilities`, through the same `self.imports`/`type_is_exported` gate
`resolve_type_or_apply` already uses for a qualified generic type header
(`src/parser.rs:3455-3460`, `:3080`), not through `Resolver::rewrite` (which runs
post-parse on term bodies and never sees a bound's trait name, already baked into
`Bound::User(TraitId)` by then).

**Project convention compliance:** unit tests beside every new/modified function
(`#[cfg(test)] mod tests`, happy path + at least one error case); one golden test file,
`tests/phase7_slice3e.rs`, matching every landed P7.S3 slice's actual convention (not the
four-file, `.golden.sth`-sidecar convention v1 of this spec proposed, which matches no
existing test file in the repo). Start in `src/check/poly.rs` for the check-side changes
(no new file preemptively) per CLAUDE.md's growth-structure rule -- re-run the split
signals at phase exit, not before.

## Codebase Map

| Location | Symbol | Role in this work |
|----------|--------|-------------------|
| `src/ast.rs:1283` | `pub enum Bound { Copy, Ord }` | Add `User(TraitId)` |
| `src/ast.rs:375-436` | `StructDecl`/`EnumDecl` (existing shape to mirror) | `TraitDecl` copies this shape exactly: flat whole-program `Vec`, `module: u32` field per entry |
| `src/ast.rs:18` | `Module` struct | Add the whole-program trait registry and impl registry fields (exact placement/shape TBD against current field layout at implementation time -- confirm whether structs/enums live directly on `Module` or on a shared registry it holds, and match that, not a `HashMap` guess) |
| `src/ast.rs:1419-1438` | `CallInst`, incl. `quot_inputs` | Add the new per-instantiation span->symbol map here (restored round-2 ruling -- round-1 wrongly rejected `CallInst` as unreachable from lowering; it is reachable at the compiled-path loop, `driver.rs:249-286`, just not from `lower_instantiation`, which is REPL-only). `quot_inputs` remains a field-shape precedent only, not a threading precedent (it is consumed at a different, caller-side site). |
| `src/parser.rs:2299` (`:2305`/`:2309` are the two hardcoded string compares) | `parse_capabilities` | Replace with a trait-table lookup; `Copy`/`Ord` become pre-seeded predicate-kind entries |
| `src/parser.rs:974-977`, `:2107-2141`, `:2136` | `intern_ty_var`/`parse_poly_ty_var`/`bound_on_use_error` | Confirms R5's "first occurrence" bound rule is already how binding-vs-use is tracked; no change needed here, cited for the implementer's confidence |
| `src/parser.rs:3449` | Qualified-name resolution (`qualifier::base`, `self.imports.get(qualifier)`) | Reused as-is for R12's qualified-call disambiguation; this is a *module*-alias lookup, not a trait-name namespace -- do not add a second qualifier kind |
| `src/check.rs:758` | Main per-word check loop (`for word in words.iter()`) | R17's pre-pass must run before this loop dispatches any `check_poly_call` |
| `src/check.rs:772-782` | Poly-combinator-standalone scratch maps | R17's obligation pre-pass map must be scratch here too, matching this path's existing handling |
| `src/check/poly.rs:11` | `is_ord(ty: Type)` | Unchanged; `Bound::Ord` keeps using it |
| `src/check/poly.rs:24` | `poly_is_copy(...)` | Unchanged; `Bound::Copy` keeps using it |
| `src/check/poly.rs:1197` | `poly_call_term`'s `env.get(name)` lookup | Insert R7's new branch immediately before this |
| `src/check/poly.rs:1225` | `PolyType::Var(v) => poly_var_to_concrete_error(...)` | One of three barriers decision 7 relies on (non-builtin-named call); see also `:1290` and `poly_delegate_op` below |
| `src/check/poly.rs:1290` | `other => poly_op_on_variable_error(...)` | Second barrier: covers a `Ref(Var, _)` operand |
| `src/check/poly.rs:3111` (`poly_delegate_op`, concrete-suffix truncation `:3119-3126`) | `poly_delegate_op` | Third barrier: covers a builtin-named (operator-spelled) call on a `Var` operand |
| `src/check/poly.rs:1365` | `poly_calls_poly_word_error` call site | *Not* used by this slice (R14 dissolved, decision 2) -- listed to prevent an implementer from re-adding the v1 plan to reuse it |
| `src/check/poly.rs:3535-3548` | `check_poly_call`'s bound loop (`for (v, bound) in &sig.bounds`) | Add the `Bound::User(trait_id)` arm: registry lookup, diagnostic on miss, per-instantiation span->symbol map population on hit (R8/R9) |
| `src/ir/driver.rs:99` | `overload_symbols` | The spelling R15's stored symbols must match byte-for-byte |
| `src/ir/driver.rs:102-116` | `uncalled_operator_overloads` | Extend to also spare a symbol appearing in some instantiation's new trait-call map (R15) |
| `src/ir/func_builder/mod.rs:718` (assignment `:748`, field `:185`) | `lower_word_parts`/`FuncBuilder` `builtin_overloads` threading | The real precedent for R9's new per-instantiation map (not `quot_inputs`); corrects round-1's citation of `driver.rs:283,428` (`:428` is `lower_line`'s REPL-line assignment, a different function) |
| `src/ir/driver.rs:249-286` | Compiled-path instantiation loop (`for (symbol, inst) in distinct`) | **The actual read site (round-2 correction).** `inst: &CallInst` is already in hand at `:270`, before `lower_word_parts` is called at `:271` -- this is where the new per-instantiation map is read and threaded, not `lower_instantiation`, which is REPL-only (`:788-823`, sole caller `src/repl.rs:1554`) and out of scope for this field |
| `src/ir.rs:69` | `empty_builtin_overloads()` | The existing default-value convention `lower_word_parts`'s other callers use for a parameter they don't need -- reuse for the new parameter's default |
| `src/resolve.rs:496` | `exportable_names` | The actual per-kind gate `trait:` export must extend (R1) -- `:270`'s `not_exported_error` is downstream of this, not a substitute for it |
| `src/resolve.rs:270` | `not_exported_error` | Downstream check; correct but incomplete on its own (decision 4 correction) |
| `src/check/declarations.rs:562-586` | `local_decl_names` | Needs a `traits` loop alongside structs/enums/words/externs, or a local trait colliding with a selectively imported one goes uncaught (R1) |
| `src/check/declarations.rs:873`, `:932`, `:376` | `check_duplicate_type_names`, `check_duplicate_word_names`, `duplicate_static_error` | No shared cross-kind check exists; a new `check_duplicate_trait_names` is needed, mirroring these (R1/R2) |
| `src/parser.rs:3455-3460` (`type_is_exported`/`not_exported_error` def `:3080`), inside `resolve_type_or_apply` | Parse-time qualified-type export gate | The actual mechanism a trait name in a bound must reuse (R18, round-2 correction) -- NOT `NameTables::build`/`Resolver::rewrite`, which run post-parse and never see a bound's trait name (already baked into `Bound::User(TraitId)` by parse time) |
| `src/resolve.rs:723`, `:288` | `Resolver::rewrite`, its word-branch fallthrough | R12's qualified member call needs no new branch here -- an unrecognized qualified word already falls to `Ok(None)`; the gap is ordering (`:289-292`'s mangling must not pre-empt a matching trait obligation), not a missing branch |
| `tests/phase7_slice3e.rs` (new, one file) | Golden + unit tests | Matches `tests/phase7_slice3{a,b,c,d,f,g,i}.rs`'s existing single-file convention |

**Load-bearing constraints:**

- Do not add a new lowering pass or `IrFunc` emission path -- R9 is explicit that impl
  members lower through the existing word path unchanged (new parameters/fields on
  existing functions are in scope; a new pass is not).
- Do not reuse `poly_calls_poly_word_error` for anything in this slice -- the shape it
  would have guarded (R14) cannot arise under impl-as-binding (decision 2).
- Do not build a trait-name-based qualifier -- R12's disambiguation reuses the existing
  module-alias `::` mechanism exactly as it exists today.
- Do not skip R17's pre-pass on the assumption that source order will happen to work --
  `check.rs`'s check loop is a single interleaved pass, not two, and this is exactly the
  shape that produces an order-dependent silent bug if skipped.
- Do not model R18 as a new `NameTables::build`/`Resolver::rewrite` branch -- round-2
  review found this backwards; a bound's trait name resolves at parse time and never
  reaches `rewrite`.
- Do not route the per-instantiation span->symbol map through `lower_instantiation` --
  round-2 review found it is REPL-only; the compiled path never calls it.

## Open Questions

All open questions from the brief and the v1 spec are resolved; see "Design decisions
resolved this revision" above for the ruling and reasoning on each. None remain open for
this slice.

## Risks & Mitigations

| Risk | Likelihood | Mitigation |
|------|------------|------------|
| The new `poly_call_term` branch (R7) regresses an existing quotation/self-recursion/bool-as-enum path | Low | The branch is gated on `Bound::User`, a variant that does not exist in any landed code today; every existing `match bound`/dispatch site is non-exhaustive on the new variant, so the compiler forces every touch point to be handled explicitly. Run the full existing `phase7_slice3{a,b,c,d,f,g,i}.rs` suite unmodified as a regression gate. |
| `parse_capabilities`'s rewrite (R2) silently changes `'T: Copy Ord`'s existing parse/error behavior | Medium | This is the highest-blast-radius change for landed slices. Add a regression test asserting `'T: Copy Ord` still parses to `[Bound::Copy, Bound::Ord]` byte-for-byte, and re-run `tests/phase4_generics.rs`, `tests/phase7_slice3b_follow.rs` (both exercise `'T: Copy Ord` today) plus `src/parser.rs`'s own `unknown_capability_error`/`parse_x3_unknown_capability_is_error` tests unmodified. |
| R15's dead-code-pruning fix is forgotten, and a bound-only-reachable impl member sharing a builtin operator's name is silently pruned | Medium | Dedicated golden test (see R15); do not fold this into a general "impl works" test where the failure mode (silent absence) would not be noticed. |
| The `Vec<(u32, Bound)>`/`PolySig.bounds` change to add `User(TraitId)` breaks an existing exhaustive match somewhere outside `poly.rs` | Low | `Bound` is crate-internal; grep every `match bound`/`Bound::Copy \| Bound::Ord` site before starting Phase 1 and confirm the compiler's non-exhaustiveness errors surface all of them (do not rely on grep alone). |
| Module-qualified disambiguation (R12) is under-tested for the same-module-collision non-escape case | Medium | A golden proving the same-module case still rejects even with a qualifier attempted, alongside the cross-module success case, so both halves of decision 6 are pinned. |
| R17's pre-pass is skipped or scoped only to the current module's top-level words, silently reproducing the source-order hazard it exists to close | Medium | A pinned golden with a monomorphic word declared *before* its polymorphic, bounded callee in source order (`round-1 review` found this ships broken without R17); assert it resolves correctly, not just that some ordering happens to work. |
| R17's hoisted `check_poly_body` pre-pass changes first-error ordering for any file mixing a poly-body error with a monomorphic-word error (found in round-2 review) | Medium | A regression test asserting which diagnostic fires first for a file with both kinds of error before and after this slice lands; if any existing golden relies on monomorphic-error-first ordering, it must be identified and updated deliberately, not silently broken. |
| R10's pinned tests cover only the non-builtin-named barrier (`poly_var_to_concrete_error`) and miss the other two (`poly_delegate_op`, `poly_op_on_variable_error`), leaving a builtin-named trait member untested | Medium | A trait requiring an operator-spelled member (e.g. `+`) is one of R10's mandated goldens, not optional -- ship it in Phase 2, not deferred to Phase 4's `sort` consumer (which uses `cmp`, a non-operator name, and would not catch this). |
| R15's dead-code-pruning fix gives false confidence about REPL coverage of bound-directed dispatch | Low | State explicitly in the golden's comment that this is a compiled-build-only guarantee (round-1 finding); do not write a REPL-session test claiming R15 coverage there. |
| R1's three-place export/duplicate-name fix (`exportable_names`, `local_decl_names`, a new `check_duplicate_trait_names`) is implemented as only the first, most-obviously-named place (found in round-2 review) | Medium | Each of the three needs its own golden: cross-module export, local-vs-selective-import collision, and same-module cross-kind collision (`trait: Point` alongside `type: Point`) -- do not accept one test standing in for all three. |
| R18's qualified-call-vs-mangling ordering (a qualified member call must win over `rewrite`'s ordinary word mangling when both are possible) ships unspecified in code review, relying on incidental branch order (found in round-2 review) | Medium | The dedicated golden R18 requires (a qualified member call where the qualifier's module also exports an unrelated same-named concrete word) must assert the trait-member call wins, not just that some call succeeds. |

## Delivery Plan

### Phase 1: Trait declaration, `impl:` binding, and export/import parity

**Goal:** `trait: Show 'T show ( &'T -- ) ;` and `impl: Show for i64  show int-show ;`
parse into the new AST, land in a flat whole-program registry mirroring
`StructDecl`/`EnumDecl`'s shape, and require explicit `export:`/`import:` exactly like
`type:`/`extern:` (no implicit-export exception).

**Requirements Covered:** R1, R2, R3, R4 (parse-time half), R11

**Scope:**

- Files to modify:
  - `src/ast.rs`: `TraitDecl`, `TraitMember`, `ImplDecl` (bindings-shaped, per decision
    1/R4), whole-program registries mirroring the *actual current* `structs`/`enums`
    field layout (read it first, don't assume a shape).
  - `src/parser.rs`: `parse_trait_decl`, `parse_impl_decl` (bare-pair grammar, no `| |`,
    no body), `parse_capabilities`'s trait-table rewrite (R2).
  - `src/resolve.rs`: `exportable_names` (`:496`) gains a `traits` loop; this is the
    actual export gate, not `:270`'s `not_exported_error` alone (decision 4 correction).
  - `src/check/declarations.rs`: `local_decl_names` (`:562-586`) gains a `traits` loop
    (R1, round-2 finding); a new `check_duplicate_trait_names`, mirroring
    `check_duplicate_type_names` (`:873`)/`check_duplicate_word_names` (`:932`) (R1/R2,
    round-2 finding -- no shared cross-kind check exists today).
- Files to create: none yet (tests land in the single `tests/phase7_slice3e.rs`, created
  in this phase and extended by later phases).
- Out of scope: any bound syntax consumption (`'T: Show`, including its parse-time
  resolution -- R18 lands in Phase 2, not here, since it needs a real bound to exercise
  against), body-side dispatch, call-site satisfaction checking -- pure
  declaration/export/import/duplicate-check surface only.

**Entry Conditions:** none beyond the existing `type:`/`extern:` parsing being in place
(it is).

**Exit Criteria:**

- A trait declares, duplicate-rejects (same-kind AND cross-kind, e.g. `trait: Point`
  alongside `type: Point` in one module -- round-2 finding), and requires `export:` to
  cross a module boundary, with the identical error shape `type:` already produces.
- A locally declared trait colliding with a selectively imported one of the same name is
  rejected (round-2 finding, `local_decl_names`).
- An `impl:` binding validates member-signature match, the orphan rule, and rejects a
  polymorphic implementing word (decision 2) at the `impl:` site.
- Full existing suite green (no regressions from the `parse_capabilities` rewrite --
  explicitly re-run `tests/phase4_generics.rs` and `tests/phase7_slice3b_follow.rs`).

**Parallelism:** SEQUENTIAL (precedes Phase 2).
**Relative Effort:** M -- larger than v1's "S" estimate for this phase, because the
export/import plumbing is genuinely new (v1 wrongly assumed it was free) and
`impl:`'s orphan-rule + polymorphic-member rejection both need real checking, not just
parsing.
**Difficulty:** `standard`.

---

### Phase 2: Body-side obligation recording and bound composition/collision

**Goal:** A poly body calling a bounded trait member records an obligation (R7) rather
than resolving a symbol; two colliding bounds on one variable are legal to declare and
rejected only at an ambiguous unqualified call (R12), with qualified-call
disambiguation working across module boundaries.

**Requirements Covered:** R5, R6, R7, R10, R12, R17, R18

**Scope:**

- Files to modify:
  - `src/check.rs`: the obligation pre-pass (R17) -- **is `check_poly_body` hoisted**
    (round-2 ruling, not a new parallel walk), inserted before the main check loop
    (`:758`); its map must be scratch inside the poly-combinator-standalone path
    (`:772-782`), not module-level state.
  - `src/check/poly.rs`: the new branch before `poly_call_term`'s `env.get(name)`
    lookup (`:1197`), obligation recording, the ambiguous-member-call rejection and its
    qualified-call escape (R12).
  - `src/parser.rs`: `parse_capabilities` (`:2299-2320`) resolves a bound's trait name at
    parse time via the same `self.imports`/`type_is_exported` gate
    `resolve_type_or_apply` uses (`:3455-3460`, `:3080`) -- R18's corrected mechanism
    (round-2 finding: NOT a `NameTables`/`rewrite` branch, which cannot see a bound's
    trait name post-parse).
- Out of scope: concrete resolution against `θ` (Phase 3), the per-instantiation
  span->symbol map population (Phase 3), lowering (Phase 4).

**Entry Conditions:** Phase 1's `TraitDecl`/`ImplDecl`/registries exist and are
queryable.

**Exit Criteria:**

- A poly body calling a bounded member body-checks successfully, pushing the trait's
  abstract outputs, with the obligation recorded (verifiable via a unit test inspecting
  the recorded obligation list, not just the checker's `Ok` result).
- The pre-pass (R17) is proven order-independent: a golden with a monomorphic word
  declared *before* its polymorphic, bounded callee in source order resolves correctly
  (this is the specific shape round-1 review found broken without R17 -- do not accept a
  test that only exercises the already-easy declared-after ordering).
- `'T: A B` with a colliding member name parses; the unqualified call is rejected with
  the exact wording from decision 5; the qualified call resolves when `A`/`B` are in
  different modules (exercising R18's new resolution branch) and still rejects when
  they share a module.
- Pinned tests prove a bounded call and an unrelated concrete overload of the same name
  never compete, covering all three barrier shapes from R10's correction: a plain
  member name, an operator-spelled (builtin-named) member name, and a `Ref`-to-variable
  operand.

**Parallelism:** SEQUENTIAL (precedes Phase 3).
**Relative Effort:** M.
**Difficulty:** `hard` -- the insertion point is sensitive (ahead of an existing,
heavily-tested error path), the ambiguity/qualification logic is genuinely new decision
logic, and R17's pre-pass is new check-ordering machinery, not a lookup extension.

---

### Phase 3: Call-site resolution and the per-instantiation span->symbol map

**Goal:** `check_poly_call`'s bound loop resolves each recorded obligation against the
concrete `θ`, emits the bound-satisfaction diagnostic on a missing impl, and records
`(span, symbol)` as a new field on `CallInst` (round-2 ruling: `CallInst` IS the right
home -- round-1 wrongly rejected it based on a REPL-only fact, see R9/Codebase Map).

**Requirements Covered:** R8

**Scope:**

- Files to modify: `src/check/poly.rs`'s bound loop (`:3535-3548`); `src/ast.rs`'s
  `CallInst` gains the new per-instantiation span->symbol map field.
- Files to extend: `tests/phase7_slice3e.rs`.
- Out of scope: lowering (Phase 4).

**Entry Conditions:** Phase 2's obligation list exists and is threaded into the call-site
checking context.

**Exit Criteria:**

- A satisfied bound resolves to the correct concrete symbol, recorded in the new
  per-instantiation map, verified via a unit test reading that map directly (not just an
  end-to-end golden -- this is the load-bearing new mechanism and needs a check-level
  test that doesn't depend on lowering succeeding too).
- An unmatched obligation (R17) is a located internal-consistency error, distinct from
  the bound-not-satisfied diagnostic -- both get their own test.
- An unsatisfied bound emits the exact diagnostic wording from R8.
- Two distinct instantiations of the same poly word (e.g. `'T=i64`, `'T=f64`) resolve to
  two distinct concrete symbols.

**Parallelism:** SEQUENTIAL (precedes Phase 4).
**Relative Effort:** M.
**Difficulty:** `hard` -- this is where check-time and lowering-time symbol identity
must be proven to match, the same proof P7.S3f's own R9 needed (byte-identical
monomorph symbols at both sites).

---

### Phase 4: Lowering, dead-code-pruning fix, and the `sort` consumer golden

**Goal:** The compiled-path instantiation loop (`src/ir/driver.rs:249-286`, round-2
correction -- NOT `lower_instantiation`, which is REPL-only) reads the new `CallInst`
field and threads it through `lower_word_parts` onto `FuncBuilder` (real, bounded new
plumbing, precedented by `builtin_overloads`); impl members lower as ordinary concrete
words; the dead-code-pruning gap (R15) is closed; the array `sort` consumer (the
slice's forcing consumer) compiles, lowers, and runs correctly at two concrete
instantiations.

**Requirements Covered:** R9, R13, R15, R16 (scope confirmation)

**Scope:**

- Files to modify:
  - `src/ir/driver.rs`: the compiled-path loop (`:249-286`) reads the new `CallInst`
    field (already in hand as `inst` at `:270`) and passes it into `lower_word_parts`
    (`:271`); `uncalled_operator_overloads` (`:102-116`) extended per R15, matching
    `overload_symbols`'s spelling (`:99`) exactly.
  - `src/ir/func_builder/mod.rs`: `lower_word_parts` (`:718`) gains the new parameter,
    `FuncBuilder` (`:185`) gains the new field, mirroring the existing
    `builtin_overloads` thread (assignment `:748`) -- no new pass. Other callers
    (`driver.rs:200`, `:759` `lower_word`, `func_builder/mod.rs:936`,
    `ir/destructors.rs:384`, existing test call sites) pass the existing
    `empty_builtin_overloads()`-style default (`src/ir.rs:69`).
- Files to extend: `tests/phase7_slice3e.rs` with the `sort` golden (`type: Ordering |
  Less | Equal | Greater ;`, `trait: Order 'T cmp ( &'T &'T -- Ordering ) ;`, two
  `impl:` bindings on `i64` and a second concrete type, the insertion-sort body from
  `slice3-dogfood.md`'s Program 2), plus the R15 golden (an impl member named after a
  builtin operator, reachable only via bound dispatch, in a compiled build -- explicitly
  not a REPL-session test, since this filter does not run on the REPL's `lower_word`
  path).

**Entry Conditions:** Phases 1-3 complete; the per-instantiation span->symbol map is
populated correctly at check time (verified independently in Phase 3, not assumed
here).

**Exit Criteria:**

- The `sort` golden builds, lowers to QBE, and runs correctly, sorting arrays of two
  distinct concrete types via their own `impl:`'s `cmp`.
- The R15 golden passes (no silent pruning) in a compiled build.
- Full suite green: `cargo fmt --check && cargo clippy --all-targets -- -D warnings &&
  cargo test`.
- No `Map`, generic-struct, multi-variable-trait, or third-trait-kind code appears
  anywhere in the diff (R16 confirmation, checked by inspection at phase close, not by a
  test).

**Parallelism:** SEQUENTIAL (final phase).
**Relative Effort:** S -- smaller than v1's "M, 1.5-2 weeks" estimate, because decision
8 removed the actual hard problem (per-instantiation lowering *design*) from this phase
entirely; what remains is real but bounded plumbing (one new parameter threaded through
two functions plus a builder field, precedented by `builtin_overloads`) and the
`uncalled_operator_overloads` extension, plus the consumer golden. Round-1 review
confirmed this is small-plumbing, not zero-plumbing -- kept at S rather than downgraded
further, but not raised to M.
**Difficulty:** `standard` -- downgraded from v1's `hard`, since the load-bearing design
risk moved to Phase 3 (where resolution actually happens) and this phase is now bounded
plumbing plus a test.

---

### Effort Summary

- Phase 1: M
- Phase 2: M (now also covers R17's pre-pass and R18's resolution branch, both found in
  round-1 review; still M, not raised, since both are bounded additions to work already
  planned for this phase)
- Phase 3: M
- Phase 4: S
- Total: roughly M+M+M+S, smaller in aggregate than v1's estimate despite adding real
  export/import plumbing v1 missed and the round-1-review-found ordering/resolution/
  lowering-plumbing corrections, because Phase 4's previously-unbounded lowering *design*
  risk is still designed away rather than absorbed -- round-1 review corrected the
  amount of bounded plumbing involved, not the underlying architecture.

---

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "Trait declaration, impl: binding, and export/import parity", "effort": "M", "difficulty": "standard" },
    { "phase": 2, "focus": "Body-side obligation recording, order-independence pre-pass, and bound composition/collision", "effort": "M", "difficulty": "hard" },
    { "phase": 3, "focus": "Call-site resolution and the per-instantiation span-to-symbol map", "effort": "M", "difficulty": "hard" },
    { "phase": 4, "focus": "Lowering plumbing, dead-code-pruning fix, and the sort consumer golden", "effort": "S", "difficulty": "standard" }
  ]
}
```
