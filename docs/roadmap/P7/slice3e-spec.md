# Spec: P7.S3e user-declarable trait bounds

**Status:** Draft (v2, post-review-loop redesign)
**Created:** 2025-07-22
**Revised:** 2026-08-22 -- a 3-reviewer soundness/implementability loop found Phase 4's
lowering design non-viable and the cross-module trait-identity story unspecified; this
revision resolves both by construction rather than patching around them. See "Design
decisions resolved this revision" below for the full record.
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
   (`src/ast.rs:375-433`), not a per-module table -- `TraitId` is the same shape: an
   index into a flat `traits: Vec<TraitDecl>`, `TraitDecl.module: u32` recording the
   declaring module. This directly fixes a reviewer-found blocker: a `trait_id: usize`
   "unique per module" cannot name a cross-module trait; a whole-program index can, at
   zero new mechanism cost (it is a missed application of an existing pattern, not a new
   one).

4. **Export/import is symmetric with `type:`/`extern:`, with no exception.** The v1
   spec's claim that traits are "implicitly exported... similar to `type:`" is false:
   `type:` requires explicit `export:`, gated by `not_exported_error`
   (`src/resolve.rs:270`). A `trait:` declaration follows the identical rule: `export:
   Order ;` in its module, `import: mylib::order o | Order | ;` to bring it into scope.
   Naming a trait in a bound (`'T: Order`) resolves through the same import-gated path a
   type name in an effect already uses -- one resolution story, not two. The orphan rule
   (an `impl:` must live in the trait's or the type's declaring module) is then a direct
   `module: u32` field comparison, no new plumbing.

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
   rule needs inventing.** A bare `PolyType::Var`/`PolyType::Ref(PolyType::Var, _)` at a
   concrete-typed operand position is *already* a hard error today
   (`poly_var_to_concrete_error`, fired inside the `env.get(name)` dispatch at
   `src/check/poly.rs:1224` when a chosen concrete overload's input doesn't match) -- so
   a concrete overload was never reachable from an abstract operand to begin with. The
   new bound-directed branch (Requirement R6) is gated on exactly that abstract-operand
   shape, so it can only ever intercept calls the env lookup could never have served.
   Both directions get a pinned test (R10) since "a bounded variable appears to shadow
   an in-scope concrete word" is the kind of thing that looks like a bug to a future
   reader even though it provably cannot conflict.

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

## Requirements

- **R1.** The parser must accept a top-level `trait: TraitName 'T member ( &'T ... -- ...
  ) ... ;` declaration with one or more required member signatures over exactly one
  type variable (single-type-variable traits only, dogfood-confirmed sufficient -- see
  brief OQ3). A trait declaration is module-scoped and requires explicit `export:` to
  cross a module boundary, exactly like `type:`/`extern:` (decision 4) -- there is no
  implicit-export special case.

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

- **R8.** `check_poly_call`'s existing bound loop (`src/check/poly.rs:3533-3547`, `for
  (v, bound) in &sig.bounds`) gains a `Bound::User(trait_id)` arm: given the concrete
  `ty = subst.ty_of(v)`, look up `(trait_id, ty)` in the whole-program impl registry. If
  absent, emit a located error naming the trait, the missing member's full signature,
  and the concrete type (e.g., `` `i64` does not satisfy `Show`: no `( &i64 -- )`
  found ``). If present, for each obligation recorded in R7 whose `trait_id`/`var_id`
  match, resolve the concrete implementing word's mangled symbol and record `(call_span,
  symbol)` on this instantiation's `CallInst` (a new field, `trait_calls: Vec<(Span,
  String)>`, mirroring `quot_inputs`'s existing shape, `src/ast.rs:1431-1438`). This is
  where resolution actually happens -- once, at check time, with the concrete `Subst`
  already in hand -- so "lowering never re-runs resolution" is preserved for real rather
  than by wording.

- **R9.** Lowering makes no resolution decision: it reads `CallInst.trait_calls` and
  emits an ordinary call to the recorded symbol at the recorded span, the same way it
  already reads `builtin_overloads`/`quot_inputs` today. No new lowering pass, no new
  `IrFunc` emission path -- an impl member is an ordinary concrete word (R4/R11) and
  lowers through the existing word-lowering path exactly once, regardless of how many
  poly-word instantiations call it.

- **R10.** Bound-directed dispatch (R7) and the ordinary `env.get(name)` lookup
  partition cleanly and provably (decision 7): a bare/ref-to-bare type variable at a
  concrete-typed operand position was already an unconditional error before this slice
  (`poly_var_to_concrete_error`), so no concrete overload was ever reachable from that
  shape. Both directions (a bounded call dispatching correctly; an unrelated concrete
  overload of the same name remaining reachable from a genuinely concrete receiver) get
  a pinned unit test.

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
  per-instantiation `CallInst.trait_calls` symbols (R8/R9), or an impl member whose name
  happens to collide with a builtin operator name (e.g. a trait requiring `add`) would
  be silently pruned before it is ever called. This needs its own golden: an impl member
  named after a builtin operator, called only via a bound, must run.

- **R16.** Multi-type-variable traits and the compiler-known third trait kind
  (`bool`-shaped/`Fallible`) are explicitly out of scope -- single-variable user traits
  and the two existing predicate-kind traits (`Copy`/`Ord`) only, per the brief's OQ3
  and its "test before designing it as a trait" ruling on the third kind.

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
  recording.
- The `Bound::User` arm in `check_poly_call`'s bound loop (R8), including concrete
  resolution and `CallInst.trait_calls` population.
- `CallInst.trait_calls: Vec<(Span, String)>`, new field.
- The `uncalled_operator_overloads` extension (R15).
- Ambiguous-member-call rejection and qualified-call disambiguation (R12).
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
existing shape (`src/ast.rs:375-433`): a flat, whole-program registry with a `module:
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
slice).** `check_poly_body` (`src/check/poly.rs:271-`) walks a poly word's body exactly
once, abstractly, with no concrete `Subst` in sight -- this is where R7's new branch
lives, and it can only ever record an *obligation* (which trait, which member), never a
symbol. `check_poly_call` (`src/check/poly.rs:1430-`) runs per call site with a concrete
`Subst` already unified -- this is where R8's extended bound loop lives, and it is the
first and only point where `θ(T)` is known, so it is the first and only point capable of
resolving a concrete symbol. The obligation list threads from the body walk to the call
site (both already operate on the same `PolySig`/call context), and `CallInst.trait_calls`
carries the resolved `(span, symbol)` pairs onward to lowering. This is structurally
identical to P7.S3f's `quot_inputs` field (`src/ast.rs:1431-1438`): a decision made
abstractly during body checking (there, "this position materializes"; here, "this call is
a trait-member obligation"), resolved concretely once instantiation-specific information
exists, and threaded through `CallInst` so lowering only ever reads.

**Lowering layer.** Two independent, and independently simple, consequences of the
design above:

- *Impl members lower as ordinary words.* Since R4 requires an `impl:` binding's target
  to be an already-declared, already-concrete word, there is nothing impl-specific to
  lower -- `int-show` (say) is mangled and lowered through the existing per-module word
  path (`resolve::mangle`, `lower()`'s `module.words` walk) exactly once, the same as
  any other concrete word, regardless of how many poly-word instantiations dispatch to
  it via a bound. No new `IrFunc` emission path, satisfying R9 directly.
- *Bound-directed calls lower as ordinary calls.* `lower_instantiation` reads
  `CallInst.trait_calls` (a plain `Vec<(Span, String)>`) and emits `Instr::Call` to the
  recorded symbol at the recorded span -- mechanically identical to how it already reads
  `builtin_overloads` today, just a different field.
- *Dead-code pruning (R15).* `uncalled_operator_overloads`
  (`src/ir/driver.rs:102-116`) must also treat a symbol appearing in some
  `CallInst.trait_calls` as "called," the same way it already treats
  `builtin_overloads.values()` -- otherwise a bound-only-reachable impl member sharing a
  builtin operator's name is silently pruned.

**Ambiguity and qualification (R12).** The obligation-recording branch in `poly_call_term`
(R7) walks `v`'s bound set looking for a trait declaring the called name; finding two is
the natural, no-extra-pass detection point for the ambiguous-call rejection. A
module-qualified call (`qualifier::name`) is parsed and resolved exactly the way any
other qualified name is (`src/parser.rs:3449`, `self.imports.get(qualifier)`) -- the
obligation-recording branch, on seeing a qualifier, restricts its search to the trait
declared in that qualifier's target module rather than searching the whole bound set.

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
| `src/ast.rs:375-433` | `StructDecl`/`EnumDecl` (existing shape to mirror) | `TraitDecl` copies this shape exactly: flat whole-program `Vec`, `module: u32` field per entry |
| `src/ast.rs:18` | `Module` struct | Add the whole-program trait registry and impl registry fields (exact placement/shape TBD against current field layout at implementation time -- confirm whether structs/enums live directly on `Module` or on a shared registry it holds, and match that, not a `HashMap` guess) |
| `src/ast.rs:1419-1438` | `CallInst`, incl. `quot_inputs` (S3f precedent) | Add `trait_calls: Vec<(Span, String)>`, same shape and same "lowering only reads" role as `quot_inputs` |
| `src/parser.rs:2299` (`:2305`/`:2309` are the two hardcoded string compares) | `parse_capabilities` | Replace with a trait-table lookup; `Copy`/`Ord` become pre-seeded predicate-kind entries |
| `src/parser.rs:974-977`, `:2107-2141`, `:2136` | `intern_ty_var`/`parse_poly_ty_var`/`bound_on_use_error` | Confirms R5's "first occurrence" bound rule is already how binding-vs-use is tracked; no change needed here, cited for the implementer's confidence |
| `src/parser.rs:3449` | Qualified-name resolution (`qualifier::base`, `self.imports.get(qualifier)`) | Reused as-is for R12's qualified-call disambiguation; this is a *module*-alias lookup, not a trait-name namespace -- do not add a second qualifier kind |
| `src/check/poly.rs:11` | `is_ord(ty: Type)` | Unchanged; `Bound::Ord` keeps using it |
| `src/check/poly.rs:24` | `poly_is_copy(...)` | Unchanged; `Bound::Copy` keeps using it |
| `src/check/poly.rs:1197` | `poly_call_term`'s `env.get(name)` lookup | Insert R7's new branch immediately before this |
| `src/check/poly.rs:1224` | `PolyType::Var(v) => poly_var_to_concrete_error(...)` | The existing barrier decision 7 relies on -- confirms bound-directed dispatch and this arm never compete for the same call |
| `src/check/poly.rs:1365` | `poly_calls_poly_word_error` call site | *Not* used by this slice (R14 dissolved, decision 2) -- listed to prevent an implementer from re-adding the v1 plan to reuse it |
| `src/check/poly.rs:3533-3547` | `check_poly_call`'s bound loop (`for (v, bound) in &sig.bounds`) | Add the `Bound::User(trait_id)` arm: registry lookup, diagnostic on miss, `CallInst.trait_calls` population on hit |
| `src/ir/driver.rs:102-116` | `uncalled_operator_overloads` | Extend to also spare a symbol appearing in some instantiation's `CallInst.trait_calls` (R15) |
| `src/ir/driver.rs` (`lower_instantiation`, reads `CallInst` fields including `quot_inputs` today) | `lower_instantiation` | Add a read of `CallInst.trait_calls`, emitting `Instr::Call` per recorded `(span, symbol)` pair -- no new pass |
| `src/resolve.rs:270` | `is_exported`/`not_exported_error` | The existing `type:` export-gate this slice's trait export must mirror exactly (decision 4) |
| `tests/phase7_slice3e.rs` (new, one file) | Golden + unit tests | Matches `tests/phase7_slice3{a,b,c,d,f,g,i}.rs`'s existing single-file convention |

**Load-bearing constraints:**

- Do not add a new lowering pass or `IrFunc` emission path -- R9 is explicit that impl
  members lower through the existing word path unchanged.
- Do not reuse `poly_calls_poly_word_error` for anything in this slice -- the shape it
  would have guarded (R14) cannot arise under impl-as-binding (decision 2).
- Do not build a trait-name-based qualifier -- R12's disambiguation reuses the existing
  module-alias `::` mechanism exactly as it exists today.

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
  - `src/resolve.rs`: trait export/import gating mirroring `type:`'s existing
    `is_exported`/`not_exported_error` path (decision 4) -- this is new plumbing (v1's
    spec wrongly assumed it already existed; it does not, confirmed by grep).
- Files to create: none yet (tests land in the single `tests/phase7_slice3e.rs`, created
  in this phase and extended by later phases).
- Out of scope: any bound syntax consumption (`'T: Show`), body-side dispatch, call-site
  satisfaction checking -- pure declaration/export/import surface only.

**Entry Conditions:** none beyond the existing `type:`/`extern:` parsing being in place
(it is).

**Exit Criteria:**

- A trait declares, duplicate-rejects, and requires `export:` to cross a module
  boundary, with the identical error shape `type:` already produces.
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

**Requirements Covered:** R5, R6, R7, R10, R12

**Scope:**

- Files to modify:
  - `src/check/poly.rs`: the new branch before `poly_call_term`'s `env.get(name)`
    lookup (`:1197`), obligation recording, the ambiguous-member-call rejection and its
    qualified-call escape (R12).
- Files to extend: `tests/phase7_slice3e.rs`.
- Out of scope: concrete resolution against `θ` (Phase 3), `CallInst.trait_calls`
  population (Phase 3), lowering (Phase 4).

**Entry Conditions:** Phase 1's `TraitDecl`/`ImplDecl`/registries exist and are
queryable.

**Exit Criteria:**

- A poly body calling a bounded member body-checks successfully, pushing the trait's
  abstract outputs, with the obligation recorded (verifiable via a unit test inspecting
  the recorded obligation list, not just the checker's `Ok` result).
- `'T: A B` with a colliding member name parses; the unqualified call is rejected with
  the exact wording from decision 5; the qualified call resolves when `A`/`B` are in
  different modules and still rejects when they share a module.
- A pinned test proves a bounded call and an unrelated concrete overload of the same
  name never compete (R10).

**Parallelism:** SEQUENTIAL (precedes Phase 3).
**Relative Effort:** M.
**Difficulty:** `hard` -- the insertion point is sensitive (ahead of an existing,
heavily-tested error path) and the ambiguity/qualification logic is genuinely new
decision logic, not a lookup extension.

---

### Phase 3: Call-site resolution and `CallInst.trait_calls`

**Goal:** `check_poly_call`'s bound loop resolves each recorded obligation against the
concrete `θ`, emits the bound-satisfaction diagnostic on a missing impl, and records
`(span, symbol)` on `CallInst` for lowering to read.

**Requirements Covered:** R8

**Scope:**

- Files to modify: `src/check/poly.rs`'s bound loop (`:3533-3547`), `src/ast.rs`'s
  `CallInst` (new `trait_calls` field).
- Files to extend: `tests/phase7_slice3e.rs`.
- Out of scope: lowering (Phase 4).

**Entry Conditions:** Phase 2's obligation list exists and is threaded into the call-site
checking context.

**Exit Criteria:**

- A satisfied bound resolves to the correct concrete symbol, recorded on `CallInst`,
  verified via a unit test reading `CallInst.trait_calls` directly (not just an
  end-to-end golden -- this is the load-bearing new mechanism and needs a check-level
  test that doesn't depend on lowering succeeding too).
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

**Goal:** Lowering reads `CallInst.trait_calls` and emits ordinary calls; impl members
lower as ordinary concrete words; the dead-code-pruning gap (R15) is closed; the array
`sort` consumer (the slice's forcing consumer) compiles, lowers, and runs correctly at
two concrete instantiations.

**Requirements Covered:** R9, R13, R15, R16 (scope confirmation)

**Scope:**

- Files to modify:
  - `src/ir/driver.rs`: `lower_instantiation` reads `CallInst.trait_calls` (no new
    pass); `uncalled_operator_overloads` (`:102-116`) extended per R15.
- Files to extend: `tests/phase7_slice3e.rs` with the `sort` golden (`type: Ordering |
  Less | Equal | Greater ;`, `trait: Order 'T cmp ( &'T &'T -- Ordering ) ;`, two
  `impl:` bindings on `i64` and a second concrete type, the insertion-sort body from
  `slice3-dogfood.md`'s Program 2), plus the R15 golden (an impl member named after a
  builtin operator, reachable only via bound dispatch).

**Entry Conditions:** Phases 1-3 complete; `CallInst.trait_calls` is populated correctly
at check time (verified independently in Phase 3, not assumed here).

**Exit Criteria:**

- The `sort` golden builds, lowers to QBE, and runs correctly, sorting arrays of two
  distinct concrete types via their own `impl:`'s `cmp`.
- The R15 golden passes (no silent pruning).
- Full suite green: `cargo fmt --check && cargo clippy --all-targets -- -D warnings &&
  cargo test`.
- No `Map`, generic-struct, multi-variable-trait, or third-trait-kind code appears
  anywhere in the diff (R16 confirmation, checked by inspection at phase close, not by a
  test).

**Parallelism:** SEQUENTIAL (final phase).
**Relative Effort:** S -- deliberately smaller than v1's "M, 1.5-2 weeks" estimate,
because decision 8 removed the actual hard problem (per-instantiation lowering design)
from this phase entirely; what remains is two small reads (`CallInst.trait_calls`,
`uncalled_operator_overloads`) plus the consumer golden.
**Difficulty:** `standard` -- downgraded from v1's `hard`, since the load-bearing design
risk moved to Phase 3 (where resolution actually happens) and this phase is now pure
plumbing plus a test.

---

### Effort Summary

- Phase 1: M
- Phase 2: M
- Phase 3: M
- Phase 4: S
- Total: roughly M+M+M+S, smaller in aggregate than v1's estimate despite adding real
  export/import plumbing v1 missed, because Phase 4's previously-unbounded lowering risk
  is designed away rather than absorbed.

---

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "Trait declaration, impl: binding, and export/import parity", "effort": "M", "difficulty": "standard" },
    { "phase": 2, "focus": "Body-side obligation recording and bound composition/collision", "effort": "M", "difficulty": "hard" },
    { "phase": 3, "focus": "Call-site resolution and CallInst.trait_calls", "effort": "M", "difficulty": "hard" },
    { "phase": 4, "focus": "Lowering, dead-code-pruning fix, and the sort consumer golden", "effort": "S", "difficulty": "standard" }
  ]
}
```
