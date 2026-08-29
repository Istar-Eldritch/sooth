# P7.S6b -- Explicit length arguments at word call sites

**Status:** Planned (4 phases).
**Discovery:** `docs/roadmap/P7/slice6b-brief.md`
**Sequence:** After S6a (length parameters in `type:`/`trait:` headers, merged
`108630e`). Touches `src/ast.rs`, `src/parser.rs`, `src/check/poly.rs`, and
`src/check/terms.rs` (two pre-existing allow-guards there must extend to cover
the new length-argument list), plus one integration golden and its fixture.
Two review rounds against earlier drafts corrected: a false "only 3 files"
scope claim; several stale line numbers; a missing guard-widening requirement;
and, in round 2, a fatal phase-4 design fork (below) that changed the
exit-criterion fixture.

A word signature can already *declare* a length variable
(`sum['T 'N: Len] ( array['T 'N] -- 'T )`, parseable since S6a) and a caller
can already bind it *by inference* (`sum[i64]` seeds `'T`, and `'N` is read off
a concrete array operand's count in `unify_poly_input`). What is missing is
syntax to bind that length **explicitly** at the call site: `sum[i64 4]` does
not parse. This slice is the length-variable parallel of the existing explicit
type-argument path (`sum[i64]`), narrowly scoped to word call sites.

The header-application path (`Buffer[u8 256]`, S6a) already parses a mixed
type/length bracket, but through a *different* parser (`parse_type_arguments`,
which knows arity statically from `self.generics`). A word call goes through
`parse_explicit_type_args` (`src/parser.rs:6315`), which has no `PolySig`
available at parse time and so cannot split type-vs-length by position. The
split therefore stays untyped at parse time and is validated in the checker,
exactly as the existing bare `type_args.len() != sig.ty_var_names.len()` arity
check in `check_poly_call` already does for types.

**Length-argument carrier is `Vec<Len>`, not `Vec<u32>`.** `Len` already has a
`Var` arm (`src/ast.rs:2061`); narrowing the call-site carrier to a bare `u32`
would permanently foreclose ever writing a length *variable* (not just a
literal) as an explicit argument, a one-way syntax commitment nobody has
signed off on. `Vec<Len>` costs nothing extra to parse (a length position is
still just "an integer literal", producing `Len::Concrete`); the `Var` arm
exists in the type but has no literal syntax that produces it at a call site
in this slice (see R2b).

**`sum[i64 'N]` (forwarding a variable, not a literal) is not reachable in this
slice, and the spec does not pretend otherwise.** Confirmed by reading
`src/check/poly.rs:965-971`: *any* call inside a polymorphic word's own body
with a non-empty explicit instantiation list is already rejected outright by a
pre-existing guard, `type_arguments_in_poly_body_error`, dating to P7.S3t
("a call inside a polymorphic word's own body is checked symbolically -- there
is no `Subst` here to seed, and reaching one would be the multi-hop forwarding
case R7 leaves out of the slice"). A concrete (non-generic) body, meanwhile,
has no length variable in scope to name at all. `parse_explicit_type_args`'s
own pre-scan (`instantiation_list_ty_var`, `src/parser.rs:6328-6330`) rejects
a type *variable* anywhere in an instantiation list before this ever reaches
the checker, so `sum[i64 'N]` is a parse error today, not merely a
checker-level rejection -- an even stronger guarantee than the brief assumed.
`Vec<Len>` keeps the AST honest about what a length argument *is* without
implying this slice makes variable-forwarding reachable; R2b makes the
checker-level rejection explicit and testable rather than an accidental
silent no-op, for the day a future slice's grammar might reach it.

## Rulings

- **R1, parser disambiguation is lexical (decision, not open question).** A
  length argument is a bare decimal integer token; a type expression is never a
  bare integer (types are word-shaped: `i64`, `Box[...]`).
  `parse_explicit_type_args` greedily parses type expressions until it meets an
  integer token, then switches to parsing integers as `Len::Concrete` values
  until `]`. No arity split is needed at parse time and no `PolySig` is
  consulted. The encounter order is fixed by the token stream (`sum[i64 4]`
  records `i64` then `4`); a type token appearing *after* an integer token is a
  parse error. This is **a grammar choice, not a fact derived from declaration
  order**: `ty_var_names`/`len_var_names` are independent id spaces
  (`intern_ty_var`/`intern_len_var`, `src/parser.rs:1486-1497`, index into
  separate vectors) and a callee may legally declare `['N: Len 'T]` today
  (`attach_bracket_bounds`, `src/parser.rs:2498-2523`, validates membership
  only, not order). The rule is sound anyway because position `i` in the call
  bracket's length sublist indexes `len_var_names[i]` and position `i` in the
  type sublist indexes `ty_var_names[i]` regardless of how the callee ordered
  its own declaration bracket -- the call-site grammar fixes "types first,
  then lengths" as its own convention, independent of and not constrained by
  the declaration side.
  - **R1a, the empty-list guard widens.** `parse_explicit_type_args`'s existing
    `if args.is_empty() { return Err(empty_instantiation_error(...)) }`
    (`src/parser.rs:6347-6349`) fires on an empty *type* list today; it must
    widen to "both the type list and the length list are empty" so
    `sum[4]` (no explicit type, one explicit length) parses. `sum[]` (both
    empty) keeps erroring exactly as today -- add a unit test pinning that
    regression.
  - **R1b, the range-check message is call-site-shaped, not reused verbatim.**
    `parse_array_count`'s existing range check (`src/parser.rs:4762-4788`)
    hardcodes the phrase `` array type `array[{element} {n}]` `` -- calling it
    directly for a length argument at a word call site would misdescribe the
    construct (`sum[i64 0]` should not say "array type"). Mirror the
    `1..=u32::MAX` range check with a new message parameterized on the actual
    call-site shape (`sum[i64 0]`, not `array[sum 0]`).

- **R2, `TermKind::Call` widens to carry length arguments.**
  `Call(String, Vec<Type>)` (`src/ast.rs:2948`) becomes
  `Call(String, Vec<Type>, Vec<Len>)` (a third positional field; a struct only
  if a third tuple field reads poorly at the consuming sites, an
  implementation call). `cargo test` (not `cargo build` -- several
  `TermKind::Call` match sites are `#[cfg(test)]`-only, e.g.
  `src/parser.rs:6805` onward, and would not be caught by a build-only gate)
  is the completeness gate for every pattern/construction site outside
  `check/poly.rs` and `check/terms.rs`; each forwards `Vec::new()` or clones
  the field unchanged, confirmed by reading every site: `rename_call`
  (`src/ast.rs:3056`), the member-call rewrite (`src/parser.rs:656`),
  `src/resolve.rs:842,1735`, `src/ir/driver.rs:664`, `src/ir/func_builder/calls.rs:45`,
  `src/ir/func_builder/mod.rs:71`, `src/check/drop_graph.rs:193,288,701`,
  `src/check/globals.rs:101,337`, `src/check/engine.rs:610,687,739,776`,
  `src/check/captures.rs:22`, `src/check/poly.rs:6596,11273,11418,13733,14437,14441,14459`,
  `src/parser.rs:10033`, plus the test-only sites above.

  - **R2a, two allow-guards need real logic, not a forward.** Two sites read
    the type-argument list to decide whether to *reject* it, and must extend
    to the length list identically, or a `sum[i64 4]` written in the wrong
    context silently drops the length instead of being rejected (the same
    "miscompile, not a diagnostic" hazard both sites' own comments already
    name for the type-argument case):
    - `src/check/terms.rs:183-197`: the non-poly dispatch route's
      `!type_args.is_empty()` guard, `no_type_arguments_error`. Widen the
      condition to `!type_args.is_empty() || !len_args.is_empty()`. Note this
      guard also already excludes combinators (`inline` words) from ever
      taking an explicit instantiation list at all (`poly_call_takes_type_args`,
      `src/check/terms.rs:1156-1157`, `&& !poly.combinators.contains_key(name)`,
      mutation-tested load-bearing) -- widening the condition does not change
      that exclusion, it stays exactly as restrictive for length arguments as
      it already is for type arguments. This is why phase 4's fixture cannot
      be `inline` (see Phase 4 below).
    - `src/check/poly.rs:965-971`: the poly-body guard,
      `type_arguments_in_poly_body_error`. Widen the same way -- this is also
      where `Len::Var` forwarding is rejected today (see above); no separate
      mechanism is needed since the guard already fires on any non-empty list,
      literal or variable.
  - **R2b, a `Len::Var` explicit argument reaching the checker is always an
    internal-consistency error, not a user-facing diagnostic.** `check_poly_call`
    (its one caller is `src/check/terms.rs:802`) has no enclosing generic
    signature to resolve a variable name against, and R1's grammar can only
    ever mint `Len::Concrete` (an integer token), so `Len::Var` cannot reach
    `check_poly_call`'s seeding loop through any path this slice's parser
    builds. Guard with `unreachable!()`, not `debug_assert!()` -- a
    `debug_assert!` degrades to a silent no-op in a release build, which is
    exactly the "miscompile, not a diagnostic" hazard R2a's guards exist to
    prevent; `unreachable!()` keeps failing loudly in every build profile.

- **R3, `check_poly_call` seeds `subst.len` from explicit length arguments.**
  `check_poly_call` (`src/check/poly.rs:5600`), called from its one site
  `src/check/terms.rs:802`, extracts the length-argument list from the widened
  `TermKind::Call`. After the existing type-arg arity check and seeding loop
  (`:5653-5662`), it adds a parallel step:
  - a length-arity check: a non-empty length list whose length is not
    `sig.len_var_names.len()` is an error. `instantiation_arity_error`'s
    existing note (`src/check/poly.rs:9418-9421`, *"a length (`'N`) or row
    (`..s`) variable is not named by an explicit instantiation; only type
    variables are"*) becomes false once this slice ships and is pinned by a
    live test, `an_instantiation_of_a_length_variable_is_rejected`
    (`src/check/poly.rs:11922-11933`) -- narrow the note to rows only, and
    **rename and re-scope** that test (its name itself becomes a lie once
    `alen[4]`-shaped calls are accepted, not just its assertion body). Add a
    length-specific sibling (`length_instantiation_arity_error`) reporting the
    expected/actual length counts, mirroring S6a's `generic_arity_error`
    two-count shape.
  - a seeding loop: each `Len::Concrete(count)` in `len_args` (R2b rules out
    `Len::Var` reaching here) pushes `(i as u32, count)` into `subst.len` and
    records `i as u32` in a new `seeded_len: Vec<u32>` set, the length twin of
    `seeded`.

  An explicit length list requires the type list be present too only insofar
  as arity demands it; the two arity checks are independent (a caller may
  bind lengths without an explicit type if the callee declares no type
  variable, and vice versa). Both checks fire against the callee's `PolySig`,
  which is available at check time.

- **R4, both `Len::Var` conflict arms route through the seeded-length set.**
  This is the brief's correction to the roadmap: today both `Len::Var` arms in
  `unify_poly_input` (`src/check/poly.rs:8022` under `PolyType::Array`, `:8230`
  under `PolyType::Generic`) *always* raise the generic `poly_len_conflict_error`
  on a mismatch, whereas the `PolyType::Var` arm (`:7986`) routes through
  `seeded.contains(v)`: seeded -> `explicit_instantiation_conflict_error`,
  unseeded -> `poly_var_conflict_error`. With R3 able to seed `subst.len`, this
  asymmetry becomes live: an explicit `sum[i64 4]` over a length-8 operand would
  otherwise report the generic "conflicting bindings" message instead of the
  caller-context "instantiated at `'N` = `4` but its operand is `8`" the exit
  criterion asks for. Both `Len::Var` arms gain the same `seeded_len.contains(ln)`
  routing the `Var` arm already uses. The unseeded (inferred) path is unchanged.

  `unify_poly_input` (`src/check/poly.rs:7964`, already
  `#[allow(clippy::too_many_arguments)]`) gains a `seeded_len: &[u32]`
  parameter. This threads through its 7 production call sites
  (`src/check/poly.rs:5396,5477,5695,5720,5780`, and
  `src/check/combinators.rs:623,649`) and 6 internal recursions
  (`src/check/poly.rs:8013,8076,8081,8109,8137,8215`). Every production site
  outside `check_poly_call` passes `&[]` (no explicit length context there).
  Existing unit tests calling `unify_poly_input` directly
  (`:10781,10858,11957,13398,13415,13447,14007,14024,14048`) gain the new
  argument, `&[]` unless a test specifically exercises seeded-length routing.
  These counts are mechanically re-checkable with one grep
  (`grep -rn "unify_poly_input(" src`) if drift is suspected by the time an
  implementer reaches this phase.

- **R5, a length-typed explicit-instantiation-conflict diagnostic.**
  `explicit_instantiation_conflict_error` (`src/check/poly.rs:9373`) is
  `Type`-typed (`instantiated: Type, operand: Type`). A length conflict compares
  two `u32`s. Add a thin sibling
  (`explicit_len_instantiation_conflict_error(ctx, span, callee, var, instantiated: u32, operand: u32)`)
  reusing the same message template ("was instantiated at `'N` = `4` but its
  operand is `8`"); `u32`'s `Display` renders identically to the `Type` path, so
  a sibling is simpler than generalizing the existing function over a trait.

## Phases

Kept tight and proportionate; each stage function gets unit tests beside it, and
the exit criterion is one integration golden.

1. **AST + parser (R1, R1a, R1b, R2).** Widen `TermKind::Call` to
   `Call(String, Vec<Type>, Vec<Len>)`; take the `cargo test`-forced ripple
   through every non-poly, non-`terms.rs` consumer (each forwards `Vec::new()`
   or clones). Extend `parse_explicit_type_args` with the integer-token mode
   switch, the widened empty-list guard (R1a), and the call-site-shaped range
   message (R1b). Unit tests in `src/parser.rs`: `sum[i64 4]` parses to one
   type + one length; `sum[i64]` parses to one type + no length (regression);
   `sum[4]` parses to no type + one length; `sum[]` still errors
   (`empty_instantiation_error`, regression); a type token after an integer
   (`sum[4 i64]`) is a parse error; a length below `1` or above `u32::MAX` is
   the call-site range error, not the array-type message. This phase carries
   the widest ripple (25+ enumerated `TermKind::Call` sites across 12 files),
   a new parser mode, a new range-error message, and six unit tests --
   labeled medium effort, not small.

2. **Guard widening + checker seeding (R2a, R2b, R3).** Widen the two
   allow-guards (`check/terms.rs:183`, `check/poly.rs:965`) to also gate on a
   non-empty length list. Thread the length list from `check/terms.rs:802`
   into `check_poly_call`; add the length-arity check (narrowing
   `instantiation_arity_error`'s stale note, and renaming/re-scoping its
   pinned test) and the `subst.len`/`seeded_len` seeding. Unit tests in
   `src/check/poly.rs`: an explicit `sum[i64 4]` against a length-4 operand
   checks clean, where `sum` is a **non-inline** word whose body reads `len`
   back rather than indexing (see Phase 4 for why); a wrong length count is
   the arity error; `sum[i64 4]` written inside a polymorphic word's own body
   is rejected by the widened poly-body guard (not silently dropped); a
   non-poly-eligible callee given an explicit length is rejected by the
   widened `check/terms.rs` guard.

3. **Conflict routing (R4, R5).** Add `seeded_len: &[u32]` to
   `unify_poly_input`, threading it through all production/recursive/test call
   sites listed in R4 (`&[]` where no seeding context exists). Route both
   `Len::Var` arms through it; add `explicit_len_instantiation_conflict_error`.
   Unit tests: a seeded length mismatch (`sum[i64 4]` over a length-8 operand)
   yields the explicit-instantiation message; an *inferred* length mismatch
   (two operands of differing length, no explicit arg) still yields
   `poly_len_conflict_error` (guards the negative half of the routing so a
   placebo cannot pass).

4. **Integration golden + fixture (exit criterion).** **The fixture does not
   index the array, and is not `inline`.** Round 2 review (run twice,
   independently, one against the live binary) found the original draft's
   `inline`-and-indexing fixture unbuildable: `inline` *is* the definition of
   a combinator (`src/check/combinators.rs:139-141`,
   `is_combinator(word) == word.declares_inline`), and combinators are
   categorically excluded from ever taking an explicit instantiation list
   (`poly_call_takes_type_args`, `src/check/terms.rs:1156-1157`,
   mutation-tested load-bearing) -- an `inline` word never reaches
   `check_poly_call` at all, so nothing phases 2-3 build would have a
   reachable witness. Following the user's decision, indexing stays out of
   this slice entirely and is deferred to P7.S6c, whose own future exit
   criterion will cover it once that slice lifts the non-inline
   generic-length-indexing restriction. This slice's fixture instead mirrors
   `tests/phase7_slice6a.rs:86-89`'s proven `capacity['T 'N: Len]`/`len`
   pattern -- a plain (non-`inline`) word whose body reads the length back
   out rather than indexing:

   ```forth
   : sum['T 'N: Len] ( array['T 'N] -- usize ) len ;
   ```

   (`len` on an owned generic-length array in a poly body already works,
   per S6a's brief: `src/check/poly.rs:1263`.) `tests/phase7_slice6b.rs` with:
   an accept dogfood (`sum[i64 4]` over a length-4 array, exit-value
   witnessed, e.g. printing `4`); a length-conflict rejection asserting the
   *explicit-instantiation* message, not the generic one; and a
   length-arity rejection. `cargo fmt --check && cargo clippy -- -D warnings
   && cargo test` green.

## Test notes (regression-fragile shapes)

- **The routing negative is load-bearing.** R4's win is only witnessed by the
  *seeded* mismatch reporting the explicit message; the inferred mismatch must
  keep reporting `poly_len_conflict_error`. A golden that tests only the seeded
  side is a placebo (it would pass under a length-blind arm that always routes
  to the explicit message). Test both sides; this is a phase-3 unit test, not
  a phase-4 golden -- phase 4's golden set is deliberately accept/
  length-conflict/length-arity only.
- **`sum[i64]` must stay a pure type-arg call.** The empty-length path is
  byte-identical downstream (R2); a parser regression here is easy to miss, so
  the phase-1 unit set pins it explicitly.
- **The two widened guards need their own negative tests.** R2a's point is
  that an explicit length list is *rejected*, not silently dropped, in the
  wrong context; a test that only checks the *accepting* path (phase 4's
  golden) cannot distinguish "rejected" from "silently ignored". Phase 2's
  unit tests cover both guards directly.
- **The `sum[]` regression is easy to lose in R1a's widening.** Widening
  "type list empty" to "both lists empty" must not also widen away the
  genuinely-empty case; pin `sum[]` still errors as part of phase 1.
- **The phase-4 fixture must not be `inline`.** This is the round-2 finding:
  an `inline` fixture would silently make phases 2-3's machinery untested (the
  golden would pass or fail for reasons unrelated to `check_poly_call`/
  `unify_poly_input` entirely, since the call never reaches them). Confirm the
  fixture word has no `inline` keyword as part of writing it, not just as an
  afterthought.

## Exit criteria

A caller can write `sum[i64 4]` to explicitly bind both `'T = i64` and `'N = 4`
against a word declared with an explicit length variable in its bracket
(`sum['T 'N: Len] ( array['T 'N] -- usize )`, reading the length back rather
than indexing), and a conflicting operand -- type or length -- produces the
routed `explicit_instantiation_conflict_error` /
`explicit_len_instantiation_conflict_error`, not the generic
`poly_var_conflict_error`/`poly_len_conflict_error`. `cargo fmt --check &&
cargo clippy -- -D warnings && cargo test` is green.

## Out of scope

- Type headers' length arguments (`Buffer[u8 256]`) -- done, S6a.
- **Generic-length array indexing in a non-inline body**
  (`poly_generic_length_index_error`, `src/check/poly.rs:9312`, call site
  `:4941`) -- unchanged checker limitation. Probed and confirmed a real
  cross-layer change (checker + lowering + QBE backend, not a guard removal);
  tracked as P7.S6c, unscoped, needs its own discovery pass before it becomes
  a real slice. Decided: S6c's own future exit criterion will include an
  indexing demo once that slice lands; this slice's golden does not attempt
  indexing at all (see Phase 4).
- **An `inline` word taking an explicit type or length argument** --
  categorically rejected today (`poly_call_takes_type_args` excludes every
  combinator, mutation-tested load-bearing); not this slice's problem, since
  this slice's own fixture is deliberately non-`inline`.
- **`sum[i64 'N]` (forwarding a length variable as an explicit argument)** --
  not reachable today: the parser's own instantiation-list scan rejects a
  type variable at any position before the checker-level
  `type_arguments_in_poly_body_error` guard would even apply, and a concrete
  body has no length variable in scope to name. `Len::Var` stays
  representable in the AST (`Vec<Len>`, not `Vec<u32>`) so this is a one-way
  door left open, not a syntax commitment against it, but no new resolution
  mechanism is built in this slice.
- Migrating any lib word to bracket-declared `['T 'N: Len]` syntax -- none
  exist; this slice ships a test fixture, not a retrofit.
- Length *inference* at construction of a length-carrying header (S6a's known
  gap in `poly_bind_construction_arg`) -- untouched.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "AST + parser: widen TermKind::Call to carry Vec<Len>, extend parse_explicit_type_args with the lexical integer-token mode switch, widen the empty-instantiation guard, and give the range check a call-site-shaped message (R1, R1a, R1b, R2)",
      "effort": "medium",
      "difficulty": "medium"
    },
    {
      "phase": 2,
      "focus": "Widen the two allow-guards in check/terms.rs and check/poly.rs to reject a non-empty explicit length list in the wrong context, thread the length list into check_poly_call, add the length-arity check (narrowing and renaming the stale instantiation_arity_error test) and subst.len/seeded_len seeding (R2a, R2b, R3)",
      "effort": "medium",
      "difficulty": "medium"
    },
    {
      "phase": 3,
      "focus": "Thread seeded_len through unify_poly_input's 7 production, 6 recursive, and 9 test call sites, route both Len::Var arms through it, add explicit_len_instantiation_conflict_error (R4, R5)",
      "effort": "medium",
      "difficulty": "medium"
    },
    {
      "phase": 4,
      "focus": "Integration golden + fixture: a non-inline word with an explicit length variable that reads len back (not indexing), tests/phase7_slice6b.rs covering accept/length-conflict/length-arity, full green gate",
      "effort": "small",
      "difficulty": "low"
    }
  ]
}
```
