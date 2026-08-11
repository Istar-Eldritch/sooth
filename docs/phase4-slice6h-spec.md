# Phase 4 Slice 6h spec: a raw array constructor `[ Type ; Count ]`

A body-level, statically-sized array constructor `[ Type ; Count ]`, reachable from both
the concrete and the polymorphic checking path, lowering to a single O(1) `Alloc` with
unspecified contents. It gives a polymorphic body its own fixed-size scratch array (which
it could not construct before), and its O(1) lowering is the shape that closes a
long-standing measured compile-cost defect.

## Correction to the brief (read this first)

The brief's **D4 -- "demote `fill` to an ordinary Sooth-defined library word" -- is not
implementable and is dropped.** This is a defect in the brief, not a scoping choice, and
the spec states it plainly rather than working around it silently.

Verified directly against the current binary:

- `fill` requires a **literal** count and always has. Feeding it a runtime `usize` local
  yields `error: fill requires a literal count, found a computed usize (no const-expr
  eval)` (the `fill_count_not_literal_error` arm, `check.rs:9452`, reached from
  `check_array_word`'s `"fill"` arm). So `fill`'s output array length is the *value* of its
  count argument: `check_array_word`'s `"fill"` arm computes the result type ad hoc per call
  site via `intern_array_type(arrays, element.ty, count_val as u32)` (`check.rs:10265`),
  reading the literal's actual value, **not** through ordinary signature unification.
- That makes `fill`'s type dependent on an argument's value, which no declared word
  signature can express. D4's own proposed signature `( 'T: Copy usize -- ['T 'N] )` has
  an output-only `'N`. Probed on the current binary:
  `: mk ( 'T: Copy usize -- ['T 'N] ) | v n | v drop n drop ;` fails with
  `stack effect mismatch in 'mk' -- body leaves ``, but the declared outputs are ['T 'N]`.
  This is the identical shape open-question-2 already cited. OQ2 answered "does `Count`
  need `'N` on the *input* side" and concluded no; it missed that `fill`'s *output* needs
  exactly that same output-only length-variable binding its own `mk` probe had already
  falsified. `fill`'s rewrite does need the machinery OQ2 scoped out, so it cannot be an
  ordinary word this slice (or without dependent types / const generics at all).
- The two poly-check paths are disjoint (`check.rs:2298`): a quotation-taking poly word
  (a *combinator*, `is_combinator`, `check.rs:7039`) is monomorphized (`'T`->`i64`,
  `'N`->`STANDALONE_LEN=4`) and checked by the ordinary concrete `check_word`
  (`check_poly_combinator_standalone`, `check.rs:4951`), which *does* reach
  `check_array_word`; a non-combinator poly word like `fill` goes through
  `check_poly_body`/`poly_walk` (`check.rs:5062`), which has no array dispatch at all.
  `fill` has no quotation input, so it is the `poly_walk` path -- where both the missing
  dispatch **and** the unsatisfiable output length would bite.

**What replaces D4:** `fill` stays a compiler builtin, byte-for-byte unchanged in its
type-checking (its `check_array_word` `"fill"` arm, its literal-count restriction, its
Copy/no-reference/range gates are all untouched). Only its **lowering** changes: from N
unrolled `alloc + store` instructions to one `Alloc` plus a small runtime counted loop
storing the seed `N` times. That closes the compile-cost exit criterion on its own merits,
independent of the new constructor.

The genuine benefit D4 was reaching for -- "a polymorphic body can build its own array" --
is delivered directly by the new constructor with a literal count (`[ 'T ; 4 ]` produces
`['T 4]`, a concrete-length, variable-element array; confirmed satisfiable via the probe
`: id4 ( ['T: Copy 4] -- ['T 4] ) ;`, which type-checks clean). `fill`'s dependent,
literal-count form was never expressible in a poly body regardless of dispatch wiring.

## Grounding facts (each verified against current source)

- `check_array_word`'s `"fill"` arm: `src/check.rs:10226` (fn `check_array_word` at
  `:10217`). Enforces a `Copy` seed (`fill_of_linear_element_error`, `:9485`), no bare
  reference (`constructed_reference_error`, `:9815`; the arm's own comment: "a construction
  site the declaration-site rule cannot reach"), and a literal count in `1..=u32::MAX`
  (`fill_count_not_literal_error` `:9452`, `fill_count_out_of_range_error` `:9466`).
  **Verified.**
- `is_copy(Type::Array(id, _))` derives an array's Copy-ness structurally from its element
  (`src/check.rs:503`); it is a derivation, not a construction-time gate. **Verified.**
- `check_array_word` is called only from the concrete `check_term` (`src/check.rs:8561`).
  `poly_walk`/`poly_call_term` (`:5115`/`:5295`) have no array dispatch: a poly body's
  `fill` falls through to `unknown_word_error`. **Verified** (grep across the poly range:
  zero hits for `"fill"`/`check_array_word`).
- `fill`'s lowering, `lower_array_word`'s `"fill"` arm (`src/ir.rs:4102`, "alloc + N
  unrolled stores"): one straight-line block of `2N`-`3N` sequential instructions.
  **Verified.**
- The compile-cost defect is QBE-quadratic on one large straight-line block, not about
  arrays. Reproduced this slice on a zero-array flat chain of N `1 +` calls:
  N=5000 -> 0.13s, 10000 -> 0.33s, 20000 -> 0.98s -- superlinear (~2.5x-3x per doubling),
  the exact shape the brief records. **Verified.**
- `parse_field_type_expr` (`src/parser.rs:1680`) already parses `[ elem count ]`
  (space-separated) in type position. **Verified.**
- `parse_term`'s `Token::LBracket` arm (`src/parser.rs:2026`) is unconditional and always
  starts a quotation literal. There is no `;` arm anywhere in `parse_term`'s match; a bare
  `;` inside `[ ... ]` falls to the `other => Err(...)` arm. Probed:
  `[ 1 ; 2 ]` in a body yields `parse error: unexpected token Semicolon`. A `;`-lookahead
  before the matching `]` is therefore unambiguous against every existing `[`. **Verified.**
- `alloc_array` (`src/ir.rs:4028`) emits one hoisted `Instr::Alloc`. The `times` loop
  machinery (`begin_loop`/`Instr::Cmp`/`Terminator::Jnz`/`Jmp`/`finalize_loop`, with
  `Alloc` hoisted into the invariant preheader) is at `src/ir.rs:3441`. **Verified**;
  `fill`'s new loop reuses this shape.

## Decisions

**D1 -- the constructor is a body-level term `[ Type ; Count ]`, a new `TermKind`.**
`Type` is a single type-name token: a concrete type name (resolved via the module registry,
`resolve_type_name`/`resolve_type_name_in_module`, read directly out of the term, no
unification) or, inside a polymorphic body, a bound type-variable name matching the
enclosing signature (`'T`). A compound/nested element type (`[ [i64 3] ; 4 ]`) is out of
scope: one type-name token only. `Count` is a **literal integer** in `1..=u32::MAX`,
**everywhere** -- both the concrete and the polymorphic path. A runtime-valued `Count` was
never in scope for either path; a bound length-variable `Count` (`'N`) is out of scope
(see open questions). Disambiguation: at `parse_term`'s existing `Token::LBracket` arm,
add one lookahead (mirroring `quotation_type_ahead`'s style, `parser.rs:1564`) that scans
for a top-depth `;` before the matching `]`. Present -> array constructor; absent ->
quotation literal, exactly as today.

**D2 -- the constructor owns the construction-time gates in both paths.** The `Copy`-only
and no-bare-reference restrictions live on the constructor's own check, enforced in *both*
`check_term` (concrete) and `poly_walk`/`poly_term` (polymorphic). A linear-element or
bare-reference-element array stays unconstructible; reaching that case, and building the
disposal/move machinery it would need (which does not exist in `ir.rs`), is not this
slice's job. This is the whole point of the slice: the gate must genuinely work in the poly
path from day one.

**D3 -- contents are unspecified; lowering is one `Alloc`, no loop, no store.** The
constructor lowers to exactly one `Instr::Alloc` (via `alloc_array`), O(1) regardless of
`Count`. Whatever populates the array afterward does so with an ordinary runtime loop.

**D4 (replaced) -- `fill` stays a builtin; only its lowering changes.** See the correction
section above. `lower_array_word`'s `"fill"` arm (`ir.rs:4102`) changes from N unrolled
`alloc + store` to one `Alloc` plus a small runtime counted loop storing the seed `N`
times, reusing the loop shape at `ir.rs:3441`. `check_array_word`'s `"fill"` arm is **not
touched** -- same literal-count restriction, same Copy/no-reference/range gates, same
surviving-set forwarding (`ir.rs`'s `store_elem`/closure-carrying element semantics must be
preserved).

## Open questions -- resolved

- **Where `fill`'s rewritten definition lives (OQ1): dissolved.** D4 is dropped; `fill`
  stays a builtin, so no new library file is created and `lib/arrays.sth`/
  `lib/combinators.sth` are untouched. The slice's poly-body dogfood is a golden under
  `examples/`, not a library word.
- **Does `Count` accept a bound length-variable `'N` (OQ2): no.** Only a literal integer,
  in both paths. A runtime-valued `Count` was never in scope for either path. Supporting
  `'N`-as-`Count` needs value-to-length-variable inference this slice does not build, and
  no in-scope consumer needs it: the achievable poly-body case is a literal count
  (`[ 'T ; 4 ]` -> `['T 4]`, satisfiable per the `id4` probe). Grounding: `STANDALONE_LEN`
  (`check.rs:4971`) pins every length variable to a sentinel during a combinator's
  standalone check, and there is no value-to-length-variable binding path in the checker;
  the `mk` probe confirms an output-only length variable is unsatisfiable.
- **Is `check_destructure_drop_guard` the right template for the shared gate (OQ3):
  partly.** Its *spirit* -- one free function reachable from both `check_term` and the poly
  path -- is right, but `check_destructure_drop_guard` (`check.rs:10653`) is a **name**-
  dispatched guard `(name, span, ctx)`, whereas the constructor's gate is **type**-directed
  and spans two type representations (`Type` on the concrete `Vec<Slot>` stack vs
  `PolyType` on the `Vec<PolyType>` stack). Conclusion: a **dedicated shared helper**
  parameterized on a resolved element descriptor (concrete `Type`, or a poly type-variable
  id plus the enclosing `sig`) rather than a name. It performs (a) the count-range check in
  `1..=u32::MAX` and its diagnostic, and (b) the element gate: for a concrete `Type`,
  `contains_reference` then `is_copy` (reusing the exact primitives `check_array_word`
  uses, so the diagnostics are identical -- `constructed_reference_error` /
  `fill_of_linear_element_error`); for a type variable, `poly_copy_gate` (`check.rs:5500`,
  which reports `poly_copy_body_error` for an unbounded variable). Each path resolves its
  element to that descriptor, then calls the one helper.
- **Re-verify the QBE finding empirically (OQ4): done pre-implementation** (see grounding
  facts; the zero-array `1 +` chain reproduces the superlinear shape). The
  *post*-implementation re-measure of the 6a `fill` timings is an exit criterion below.

## Out of scope (do not resurrect)

- Linear-element or bare-reference-element arrays (D2 keeps them unconstructible; the
  disposal/move machinery is not this slice's problem).
- `lib/arrays.sth`'s own fate (`sort`/`bin_search`) -- a separate decision.
- Runtime/heap allocation. The constructor is a stack-frame `Alloc` exactly as `fill` is;
  nothing here is `Vec`/Phase 6's `alloc`/`free`.
- A runtime-valued or `'N`-valued `Count`; a compound/nested element type.
- Any change to `check_array_word`'s `"fill"` arm type-checking (only `fill`'s IR lowering
  changes).

## Exit criteria

- `[ i64 ; 10 ]` (concrete) compiles, runs, and produces a well-typed `Copy` `[i64 10]`
  array (e.g. `len` folds to `10`).
- `[ 'T ; 4 ]` inside a polymorphic body (`'T` bound by the enclosing word) compiles and
  produces a well-typed `Copy` `['T 4]` array; a polymorphic word constructs its own
  fixed-size array internally where it previously produced "unknown word" (dogfooded by at
  least one such word using a **literal** count -- not a runtime `n`).
- A linear element type and a bare-reference element type are each a **located** error at
  the constructor, in **both** the concrete and the polymorphic path (asserting the specific
  diagnostic, not merely that it fails). A count of `0` or `> u32::MAX` is a located range
  error. A non-literal count is a located parse (or check) error.
- `docs/phase4-slice6a-spec.md`'s superlinear `fill`-compile-cost numbers (10k / 100k, and
  1M if it completes within a reasonable bound) are re-measured and shown linear/flat, now
  attributed to `fill`'s fixed **lowering** (one `Alloc` + runtime loop), not to demotion.
- Every existing `fill`-using golden in the corpus is unchanged in observable behavior
  (identical program output), `fill`'s type-checking untouched.

## Test coverage (per CLAUDE.md conventions)

- **Parser (unit, beside `parser.rs`):** `[ i64 ; 10 ]` parses to the new `TermKind`;
  `[ 'T ; 4 ]` parses (element token carried verbatim); a quotation containing a bare `;`
  still errors unchanged (regression on the disambiguation); `[ i64 ; ]` (missing count),
  `[ i64 ; 5 6 ]` (extra token), and `[ i64 ; x ]` (non-literal count) each produce a
  located parse error.
- **Concrete check (unit, beside `check.rs`):** `[ i64 ; 10 ]` yields a `[i64 10]` slot;
  `[ &i64 ; 4 ]` -> `constructed_reference_error` (located); a linear element ->
  `fill_of_linear_element_error` (located); `[ i64 ; 0 ]` and a `> u32::MAX` count ->
  range error (located).
- **Poly check (unit, beside `check.rs`):** in a poly body, `[ 'T ; 4 ]` with `'T: Copy`
  pushes `PolyType::Array(Var, Len::Concrete(4))`; `[ 'T ; 4 ]` with an **unbounded** `'T`
  -> `poly_copy_body_error` (located); a concrete bare-reference element in a poly body ->
  `constructed_reference_error` (located). These discriminate the shared gate on the poly
  path directly (do not rely on an e2e build alone).
- **Lowering (unit, beside `ir.rs`):** the constructor lowers to exactly one `Instr::Alloc`
  and no `store`/`Blit` (assert the instruction shape, not just that it compiles) --
  the mutation guard for D3. `fill`'s new lowering emits one `Alloc` and a single counted
  loop body (a fixed instruction count independent of `N`), not `N` unrolled stores.
- **Goldens:** (1) a concrete `[ i64 ; 10 ]` program (source -> output). (2) the poly-body
  dogfood (a minimal generic constructor, e.g.
  `: mkbuf ( 'T: Copy 'T -- ['T 4] ) drop [ 'T ; 4 ] ;` instantiated at `i64`, `len .`
  prints `4`) -- proving construction from a poly body where `fill` previously produced
  "unknown word". This golden must actually **build and run**, not merely type-check:
  `poly_walk` is rarely exercised by the corpus, so the dogfood is also the check that the
  poly path lowers. (3) a regression golden or corpus-wide `cargo test` proving every
  existing `fill`-using example's output is byte-identical after `fill`'s re-lowering.
- **Located-error goldens:** the linear-element and bare-reference-element rejections, in
  both paths, as source-in -> diagnostic-out (diagnostics are behavior).
- **Mutation discipline:** for each new guard/test, prove it can fail by deleting the
  guard it protects (the constructor's Copy gate, the no-reference gate, the range gate,
  the `;`-lookahead, the "one `Alloc`, no store" lowering assertion).

## Risks

- `poly_walk` (`check.rs:5115`) is the least-exercised checker path; the poly dogfood may
  surface an unrelated gap (borrow/`times`/output handling). Keep the dogfood minimal
  (`drop [ 'T ; 4 ]`, no loop, no borrows) so the golden isolates the constructor. If a
  richer dogfood (a fill-shaped `times` loop in a poly body) fails, that is a poly_walk
  finding to record in this section, not a constructor bug -- do not widen scope to fix
  poly_walk here.
- `fill`'s re-lowering must preserve exact observable behavior including the seed's `Copy`
  replication and the closure-carrying-element surviving-set forwarding (`ir.rs`
  `store_elem` and the checker's `surviving` forward at `check.rs:10270`). The regression
  golden across the whole `fill`-using corpus is the guard.

## Phases

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Parser: add the `[ Type ; Count ]` TermKind and the `;`-lookahead at parse_term's Token::LBracket arm (parser.rs:2026), mirroring quotation_type_ahead (parser.rs:1564). Element is one type-name token carried verbatim; Count is a single integer literal. Unit tests: constructor parses (concrete and 'T element); a quotation with a bare `;` still errors (disambiguation regression); missing/extra/non-literal count each a located parse error.",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "Concrete path: a new check_term arm resolves the element type via the module registry and gates it through a dedicated shared helper (count range 1..=u32::MAX, contains_reference then is_copy, reusing check_array_word's exact diagnostics), interns the array type and pushes the Slot; a new lower_term arm (ir.rs) emits exactly one Instr::Alloc via alloc_array, no store. Golden: `[ i64 ; 10 ]` builds and runs (len folds to 10). Located-error tests: bare-reference element, linear element, count 0 / > u32::MAX.",
      "difficulty": "standard"
    },
    {
      "phase": 3,
      "focus": "Polymorphic path: a new TermKind arm in poly_term (check.rs:5145) resolves the element against the enclosing signature's ty_var_names (bound 'T -> PolyType::Var) or the module registry (concrete), gates it via the same shared helper (poly_copy_gate for a variable, contains_reference/is_copy for a concrete element), and pushes PolyType::Array(elem, Len::Concrete(count)). Dogfood golden: a minimal generic constructor (e.g. `: mkbuf ( 'T: Copy 'T -- ['T 4] ) drop [ 'T ; 4 ] ;`) builds and RUNS at a concrete instantiation. Located-error tests on the poly path: unbounded-'T element (poly_copy_body_error), concrete bare-reference element (constructed_reference_error).",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "Re-lower fill only: change lower_array_word's `fill` arm (ir.rs:4102) from N unrolled alloc+store to one Alloc plus a single runtime counted loop storing the seed N times, reusing the loop shape at ir.rs:3441; leave check_array_word's `fill` arm and all its type-checking untouched, and preserve the Copy-seed replication and closure-carrying surviving-set semantics. Regression: every existing fill-using golden in the corpus is byte-identical in output (full cargo test). Re-measure docs/phase4-slice6a-spec.md's 10k/100k(/1M) fill timings and show them linear/flat, attributed to the fixed lowering.",
      "difficulty": "hard"
    }
  ]
}
```
