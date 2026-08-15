# Phase 4 Slice 13: `PolyType::Ref` — borrows inside a generic word (brief)

A generic (`'T`-bounded) word cannot borrow a local at its own top level, nor declare a
signature slot that borrows a still-generic type. `&x`/`&!x` and the accessor family
(`&>`, `&^`, `&Struct>field`) exist only on the monomorphic side of the checker; the poly
side has no representation for "a reference to a value whose type isn't known yet." A
combinator body (`times`/`while`/`map`/...) dodges the gap today because a combinator is
spliced and monomorphized at every call site before its body is checked at all — every
borrow that currently works in a generic word (`lib/arrays.sth`'s `bin_search`, `sort`)
lives inside a combinator's quotation argument, never at the enclosing word's own top
level. This slice adds `PolyType::Ref` and threads it through poly dispatch,
unification, substitution, copy-checking, and lowering, so a plain (non-combinator)
generic word can borrow.

The motivation is direct: a generic word that needs to peek at or mutate a `'T`-typed
argument without going through a combinator — a polymorphic element-swap, a generic
`first`/`last` accessor, a search helper that isn't shaped like `bin_search` — cannot be
written today. It is not a hypothetical; the concrete-array analogue of the witness below
(`[i64 3]` instead of `['T 'N]`) compiles and runs fine, so genericity alone is what
blocks it.

## Recon (measured against the built compiler, 2026-08-15, `main` at `f740f78`)

`cargo test` is green at this HEAD. Two independent witnesses below, both confirmed by
running the compiler, not by reading.

1. **A `&`-sigil name never reaches poly dispatch's reference handling because there is
   none.** `poly_call_term` (`src/check/poly.rs:368`) starts with a local-name lookup
   (`scope.locals.get(name)`, `:381`) against the *literal* name (sigil included, since
   `&x` lexes as one token, `Token::Word("&x")` — no delimiter splits it). That misses for
   any local `x`, and the function has no branch anywhere that recognises a leading `&`
   and dispatches to something like the monomorphic `check_reference_word`
   (`src/check/word_families.rs:12`, which does the sigil-stripping/lookup/borrow-check
   work for exactly this family: `&x`, `&!x`, `&>`, `&^`, `&Struct>field`). Dispatch falls
   through the shuffle words, comparisons, and the `env`/`BUILTIN_TABLE` lookup, and
   bottoms out as an ordinary unresolved-name error. Witness:

   ```text
   : first ( ['T 'N] -- usize ) | a | &a len ;
   : main ( -- ) [ 1 2 3 ] first drop ;
   => error: unknown word `&a` in `first` (line 1)
   ```

   The concrete twin of this word (`a` typed `[i64 3]` instead of `['T 'N]`) compiles and
   runs; only the type variable turns the borrow into an unresolved name.

2. **A signature cannot declare a borrow of a still-generic type either**, a second,
   parser-level half of the same root cause. `parse_poly_slot` (`src/parser.rs:1388`) has
   arms for `~[`/`[` (quotations, arrays) and a `'`-prefixed type variable, then falls
   through to `parse_type_expr()` (`:1742`) for anything else — including a leading `&`.
   `parse_type_expr` only resolves *concrete* type names, so `&'T` or `&['T 'N]` in a
   signature slot reports the type variable itself as unknown, before the poly checker
   ever runs:

   ```text
   : peek ( ['T 'N] -- &['T 'N] ) | a | &a ;
   => error: unknown type `'T` at line 1, col 23
   ```

3. **`PolyType` (`src/ast.rs:623`) has exactly four variants — `Concrete`, `Var`, `Array`,
   `Quotation` — no `Ref`.** The monomorphic `Type::Ref` (`src/ast.rs:857`,
   `RefId, bool /*mutable*/, &'static str`) names a concrete referent; there is no way to
   say "a reference whose referent is still `'T`." Adding `PolyType::Ref(Box<PolyType>,
   bool)` (referent + mutability, mirroring `Type::Ref`'s shape) is the natural fix, but
   `PolyType` is matched exhaustively across the checker and IR, not just constructed:
   `src/check/poly.rs` alone carries ~10 distinct exhaustive matches over it — unification
   (`unify_poly_input`, `~:1051`), substitution (`apply_subst`, `~:1186`), the `Copy` gate
   (`poly_is_copy`, `:15`), the bare-variable-id extractor (`poly_var_id`, `:561`), the
   diagnostic renderer (`poly_type_str`, `:1463`) and the "what is this slot" describer
   (`~:1287`) all need a new arm, each requiring an actual semantic answer (what does
   unifying a `&'T` slot against a concrete borrow mean; is a reference always `Copy`
   regardless of its referent, matching `Type::Ref`'s own unconditional-`Copy` status).
   Outside `poly.rs`: `src/check/audits.rs` (quotation-position rejection, 2 sites),
   `src/check/declarations.rs` (`collect_poly_concrete`, 1-2 sites), `src/ir/driver.rs`
   (`subst_polytype`, the ground-to-`Type` step lowering depends on), and `src/repl.rs`
   (`remap_poly_type`, shifting ids across REPL generations) each need one exhaustive-match
   arm apiece. None of these is boilerplate-only; each is a real "what does a poly
   reference mean at this stage" decision, comparable in shape to how slice 10a threaded
   `PolyType::Quotation`'s row variables through the same set of files.

4. **Borrow-checking itself (aliasing/liveness, `check_reference_word`'s tail,
   `src/check/word_families.rs:12` from ~`:150` on) is entirely monomorphic-checker
   machinery** — `Provenance`, `Liveness`, `scope.moves` — and is not exercised by
   `poly_call_term` at all today. Part of this slice's scope is deciding whether a poly
   body's borrow gets the *same* aliasing/exclusivity checks as a monomorphic one (almost
   certainly yes, since the poly checker still owns a `Scope`/locals map), or whether it's
   deferred to the monomorphized instantiation. This needs its own design note before a
   spec locks it (open question, not yet answered here).

## Scope (draft — not yet a locked spec)

Two parts, dependency-ordered:

- **A**: add `PolyType::Ref`, thread it through every exhaustive match enumerated in
  recon 3, and make `parse_poly_slot` (recon 2) parse `&'T`/`&['T 'N]`/`&!...` in a
  signature slot, folding to `PolyType::Ref` via `raw_to_poly_type`
  (`src/parser.rs:1622`).
- **B**: teach `poly_call_term` (recon 1) to recognise a leading `&` and dispatch to a
  poly-side borrow check — either a genuinely separate function mirroring
  `check_reference_word`'s shape over `PolyType`/`PolyScope`, or (pending recon 4's open
  question) a thin adapter that defers to the existing monomorphic aliasing machinery.

This is compiler type-system work in the caliber of the slice 10 series, not a bug-fix.
Recommend routing it through `.pi-spec-pipeline` (brief → spec → phased implement →
review) per CLAUDE.md, not hand-rolling in chat — open question OQ1 (recon 4) needs an
answered design note before the spec locks, and part A's blast radius (5 files, ~15
exhaustive-match sites) warrants the pipeline's phased-implementation discipline rather
than a single sitting.
