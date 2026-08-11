# Phase 4 Slice 6h spec: a raw array constructor `[ Type ; Count ]` (concrete path)

A body-level, statically-sized array constructor `[ Type ; Count ]` for the **concrete**
checking path, lowering to one O(1) `Alloc` plus a fixed-size zero-init loop. `fill`'s own
lowering is fixed the same way (one `Alloc` plus a runtime store loop, replacing its N
unrolled stores). Together these close a long-standing measured compile-cost defect.

**This slice does not deliver a polymorphic body's ability to construct its own array.**
That was this slice's original motivating goal; round-1 review found it unbuildable as
briefed (see "Deferred to a future slice" below) and the scope was narrowed in response.
What ships here is: a new concrete-path constructor that needs no seed value (useful when
a buffer will be immediately overwritten index-by-index), and the fix to `fill`'s
compile-cost defect. Both are real, both are independently useful, neither reaches a
polymorphic body.

## Correction to the brief (read this first)

The brief's **D4 -- "demote `fill` to an ordinary Sooth-defined library word" -- is not
implementable and is dropped.** This is a defect in the brief, not a scoping choice.

Verified directly against the current binary:

- `fill` requires a **literal** count and always has. Feeding it a runtime `usize` local
  yields, in word context: `error: type mismatch in main (line 3)`, then
  `` `fill` requires a literal count, found a computed `usize` (no const-expr eval)`` ,
  then `note: declared ( -- )` -- the `fill_count_not_literal_error` arm, `check.rs:9452`,
  reached from `check_array_word`'s `"fill"` arm at `check.rs:10244-10246`. (The doubled
  `error: error:` prefix on native diagnostics is a pre-existing, repo-wide artifact,
  documented and deliberately unfixed at `docs/phase4-slice6a-spec.md:358`, not a defect
  in this citation.) So `fill`'s output array length is the *value* of its count argument:
  `check_array_word`'s `"fill"` arm computes the result type ad hoc per call site via
  `intern_array_type(arrays, element.ty, count_val as u32)` (`check.rs:10264`), reading the
  literal's actual value, **not** through ordinary signature unification.
- That makes `fill`'s type dependent on an argument's value, which no declared word
  signature can express. D4's own proposed signature `( 'T: Copy usize -- ['T 'N] )` has
  an output-only `'N`. The discriminating probe (the brief's original `mk` probe did not
  discriminate this -- see the citations-review correction below) is a pair:
  `` : mkn ( ['T: Copy 4] -- ['T 'N] ) ; `` fails with
  `` body leaves `['T 4]`, but the declared outputs are `['T 'N]` ``, while
  `` : mkn2 ( ['T: Copy 'N] -- ['T 'N] ) ; `` type-checks clean. A concrete length on the
  stack does not bind an output-only length variable; `'N` on both sides is fine. `fill`'s
  rewrite needs exactly the first shape (an output length bound from nothing but a runtime
  count), which is unsatisfiable.
- The two poly-check paths are disjoint (`check.rs:2298`): a quotation-taking poly word
  (a *combinator*, `is_combinator`, `check.rs:7039`) is monomorphized (`'T`->`i64`,
  `'N`->`STANDALONE_LEN=4`, `check.rs:4964/4971`) and checked by the ordinary concrete
  `check_word` (`check_poly_combinator_standalone`, `check.rs:4951`), which *does* reach
  `check_array_word`; a non-combinator poly word like `fill` goes through
  `check_poly_body`/`poly_walk` (`check.rs:5062`/`5115`), which has **no `check_array_word`
  dispatch** (a `fill` call there falls through to `unknown_word_error` -- the poly path is
  not blind to arrays generally: `poly_call_term`'s `"len"` arm does match
  `PolyType::Array(..)` at `check.rs:5364`, and `poly_copy_gate` already recurses through
  `PolyType::Array` at `check.rs:5526-5527`, which OQ3 below reuses). `fill` has no
  quotation input, so it is the `poly_walk` path -- where the missing `check_array_word`
  dispatch **and** the unsatisfiable output length both bite.

**What replaces D4:** `fill` stays a compiler builtin, byte-for-byte unchanged in its
type-checking control flow. Only its **lowering** changes: from N unrolled `alloc + store`
instructions to one `Alloc` plus a small runtime counted loop storing the seed `N` times.
See D4 below.

## Deferred to a future slice (do not resurrect here)

Round-1 review found the polymorphic-body half of the original brief unbuildable as
scoped, on independent grounds from two reviewers plus a third finding that undercuts its
motivation. Recorded here so a future slice's design does not have to re-derive it:

- **No interning route for a body-internal array shape in a poly body.** `ir.rs`'s
  `subst_polytype` (`ir.rs:2245`) and `array_id_of` (`ir.rs:4048`) both *look up* an
  already-interned array shape and panic (`.expect(...)`) otherwise. The only interning
  sites are `parser.rs:1552` (concrete type positions), `check.rs:10264` (`fill`'s arm),
  and `check.rs:6103` (`apply_subst`, which walks a **signature**, never a body). A poly
  body that constructs an array whose shape does not already appear in its own declared
  signature has nothing to intern against. A naive dogfood that returns the constructed
  array (so its shape is the declared output, interned via the existing call-site
  unification) would silently pass without exercising this gap at all -- exactly why this
  is a real design problem and not just missing wiring.
- **`poly_term`/`poly_walk`/`check_poly_body` hold `arrays: &[ArrayDecl]` immutably**
  (`check.rs:5068`/`5122`/`5152`), and the representation invariant set by
  `raw_to_poly_type` (`parser.rs:1356-1360`) folds a fully-concrete array shape straight to
  `PolyType::Concrete`, never `PolyType::Array`. Threading `&mut Vec<ArrayDecl>` through
  that call chain to let a poly body intern a genuinely new shape is a signature change
  across a `pub(crate)` API also reached from `repl.rs:2390`.
- **The combinator path (`sort`/`each`-shaped words) has no binding for `'T` at all.** A
  combinator is never monomorphized into an `IrFunc`; it is term-spliced into the caller
  (`is_combinator`, `check.rs:7039`), so at the splice site a type-variable element token
  like `'T` has no concrete binding to resolve against (`subst_polytype`'s `Quotation` arm
  is `unreachable!` for exactly this reason, `ir.rs:2266-2271`). A future slice needs an
  explicit ruling here, not silent fallthrough to a confusing "unknown type `'T`" error.
- **A literal-only `Count` does not serve the one real consumer in this repo.**
  `lib/arrays.sth`'s own `sort` needs a scratch array whose length is the *caller's* array
  length (`'N`), not a fixed literal such as 4 -- exactly the case a literal-only `Count`
  excludes. A future slice reopening this needs `'N`-as-`Count` (bound-length-variable
  support), which needs value-to-length-variable inference this slice still does not
  build; a literal-only poly-body constructor would ship a capability nobody in this repo
  can use.

None of this affects the concrete path this slice actually delivers: `fill` and the new
constructor remain exactly as reachable from a polymorphic body as they are today (not at
all), unchanged from the status quo.

## Grounding facts (each verified against current source)

- `check_array_word`'s `"fill"` arm: `check.rs:10226` (fn `check_array_word` at `:10217`).
  Enforces a `Copy` seed (`fill_of_linear_element_error`, `:9485`), no bare reference
  (`constructed_reference_error`, `:9815`; the arm's own comment at `:10250-10252`: "a
  construction site the declaration-site rule cannot reach"), and a literal count in
  `1..=u32::MAX` (`fill_count_not_literal_error` `:9452`, `fill_count_out_of_range_error`
  `:9466`). Element and count are read off the operand stack at `check.rs:10231-10232`;
  the interned result type is pushed with the element's forwarded `surviving` set
  (`check.rs:10265-10274`, comment "Review fix: forward the element's surviving set").
- `is_copy(Type::Array(id, _))` derives an array's Copy-ness structurally from its element
  (`check.rs:503`); it is a derivation, not a construction-time gate.
- `check_array_word` is called only from the concrete `check_term` (`check.rs:8561`).
- `fill`'s lowering, `lower_array_word`'s `"fill"` arm (`ir.rs:4102`): one straight-line
  block of `2N`-`3N` sequential instructions (alloc + N unrolled `elem_addr`/store pairs).
- The compile-cost defect is QBE-quadratic on one large straight-line block, not about
  arrays specifically. Reproduced this slice, twice independently (this document and
  round-1 review), on a zero-array flat chain of N `1 +` calls: N=5000 -> 0.11-0.13s,
  10000 -> 0.30-0.33s, 20000 -> 0.95-0.98s -- superlinear (~2.5x-3x per doubling), matching
  the shape `docs/phase4-slice6a-spec.md:350-354` records for `fill` itself
  (10k~0.36s, 100k~25s, 1M>300s, and explicitly retired the 1M case there because 1M `i64`
  is 8 MB of stack before any loop frame exists -- this slice's own re-measurement stays
  compile-time-only for the same reason, not a revival of criterion 14).
- `parse_field_type_expr` (`parser.rs:1680`) already parses `[ elem count ]` (space
  separated) in type position, and `Parser::resolve_type` (`parser.rs:1729`) is the
  existing, already-used-elsewhere (five call sites, including `parse_field_type_expr`)
  parse-time name-to-`Type` resolver: it calls `resolve_type_name_in_module` and the
  qualified-name export check, exactly what a concrete `Type` slot needs, and needs no new
  machinery in `check.rs`/`ir.rs`.
- `parse_term`'s `Token::LBracket` arm (`parser.rs:2026`) is unconditional and always
  starts a quotation literal; there is no `;` arm anywhere in `parse_term`'s match, a bare
  `;` falls to the `other => Err(...)` arm at `parser.rs:2041`. Probed: `[ 1 ; 2 ]` in a
  body yields `parse error: unexpected token Semicolon at line 1, col 19`. A `;`-lookahead
  before the matching `]` is therefore unambiguous against every existing `[`.
- A struct/enum construction word already lowers from a checked term that carries its own
  `StructId`/`EnumId` directly, with no operand to read a type off (`StructWord::Construct(id)`
  / `EnumWord::Construct(id, vi)`, `ir.rs:761-772`) -- the precedent this constructor's
  lowering follows, since it likewise has no source operand for its element type.
- `begin_loop`/`finalize_loop` (`ir.rs:3102`/`3167`) are the general-purpose loop-building
  primitives; `elem_addr` (`ir.rs:4073`) computes a runtime index-to-pointer address;
  `PtrOffset` (`ir.rs:1060`) and `Store`/`FieldStore` (`ir.rs:1071`/`1084`/`1088`) are
  existing scalar-write ops. (`ir.rs:3441`, the `times` word's own arm, is *not* directly
  reusable: it is driven by a quotation value on the stack via `quot_bodies`/
  `lower_indirect_call`, `ir.rs:3479-3496`, which neither this constructor nor `fill` has.)
- Zero-value soundness for D3: every enum's variant tags are assigned by declaration
  order starting at 0 (`variants.iter().enumerate()`, `ir.rs:729`), so tag `0` always names
  a real, declared variant for any non-empty enum. Recursively, an all-zero-bytes region
  therefore decodes as a valid value for any type built only from scalars and enums (any
  Copy, non-reference type this constructor's own gate already restricts elements to) --
  including `Bool`, a scalar enum since slice 9.

## Decisions

**D1 -- the constructor is a body-level term `[ Type ; Count ]`, a new `TermKind`, concrete
path only.** `Type` is a single type-name token, resolved to a `Type` **at parse time** via
the existing `Parser::resolve_type` (`parser.rs:1729`) -- the same call already used for
other type-position syntax, so the new `TermKind` carries a resolved `Type`, not a raw
string, and needs no name-resolution step later in `check.rs`. A compound/nested element
type (`[ [i64 3] ; 4 ]`) is out of scope: `Parser::resolve_type` expects a single name
token, so this case is already a located parse error with no new rejection logic to write
(the `;`-lookahead still fires on it correctly, since its scan sees `;` at depth 1; the
element read then hits an unexpected `[` where a name token is expected -- this needs a
test, not new code). `Count` is a **literal integer** in `1..=u32::MAX`. A bound
type-variable element (`'T`) and a bound length-variable count (`'N`) are both out of
scope this slice (see "Deferred to a future slice"). Disambiguation: at `parse_term`'s
existing `Token::LBracket` arm, add one lookahead (mirroring `quotation_type_ahead`'s
style, `parser.rs:1564`) that scans for a top-depth `;` before the matching `]`. Present ->
array constructor; absent -> quotation literal, exactly as today, including the existing
"unterminated quotation" fallback when no matching `]` exists at all.

**D2 -- the constructor's construction-time gates are shared with `fill`'s, parameterized
on the construction site.** The `Copy`-only and no-bare-reference restrictions
(`contains_reference`/`is_copy` against `fill_of_linear_element_error`/
`constructed_reference_error`, `check.rs:10253-10262`) and the count-range check
(`fill_count_out_of_range_error`, `:9466`) are extracted into one small helper called from
both `check_array_word`'s `"fill"` arm and the new constructor's `check_term` arm. Since
`fill_of_linear_element_error` and `fill_count_out_of_range_error` currently hardcode the
word `` `fill` `` in their rendered text, give both a `position: &str` parameter (mirroring
`constructed_reference_error`'s existing pattern, `check.rs:9815`) so a rejection at the
new constructor names the constructor, not `fill`; `fill`'s own call sites pass `"fill"`
so its diagnostics are byte-for-byte unchanged. This is a diagnostic-constructor signature
change, not a type-checking behavior change, and does not conflict with "`fill`'s
type-checking is untouched" below. A linear-element or bare-reference-element array stays
unconstructible either way; the disposal/move machinery that would let one exist does not
exist in `ir.rs` and building it is not this slice's job.

**D3 -- contents are zero-initialized via a fixed-size runtime loop, not "unspecified".**
Round-1 review correctly flagged that "unspecified contents" is unsound: elements are
gated `Copy`, which includes enums, and `Bool` is a scalar enum since slice 9, so an
uninitialized `[ Bool ; 4 ]` would let the checker treat an arbitrary byte as a valid
`Bool` -- exactly the class of silent failure this project exists to eliminate. The fix
must stay O(1) in **code size** (a per-element unrolled zero-store would reintroduce the
same QBE-quadratic defect this slice exists to close for `fill`), so it is a runtime loop,
not an unrolled fill: one `Alloc` (`alloc_array`, `ir.rs:4028`, following the same
"the term already carries its own type, no operand needed" precedent `StructWord::Construct`
uses), then a `begin_loop`/`finalize_loop`-bounded loop writing a `Const(_, 0)` across the
allocation's byte range via `PtrOffset`+`Store`/`FieldStore` -- a raw byte-range zero-fill,
not a per-element `store_elem` call, so it needs no element-type-directed logic at all (an
all-zero-bytes region is a valid value for every element type this constructor can
construct, per the grounding fact above). The loop body's instruction count is fixed,
independent of `Count`; only the loop's trip count varies.

**D4 (replaces the brief's D4) -- `fill` stays a builtin; only its lowering changes.**
`lower_array_word`'s `"fill"` arm (`ir.rs:4102`) changes from N unrolled `alloc + store` to
one `Alloc` plus a small `begin_loop`/`finalize_loop`-bounded runtime loop storing the seed
value `N` times via `elem_addr` (`ir.rs:4073`) + `store_elem` (`ir.rs:4082`, which already
handles a `Blit` for aggregate elements). `check_array_word`'s `"fill"` arm's
type-checking is **not touched** beyond D2's diagnostic-constructor parameterization: same
literal-count restriction, same Copy/no-reference/range gates, same `surviving`-set
forwarding, and `fill`'s own rendered diagnostics are unchanged (D2 passes `"fill"` as the
position). The loop-carried destination array must **not** be a `begin_loop` carried
parameter: `begin_loop(&params, stage_aggregates: true)` (`ir.rs:3467`) routes carried
aggregates through the back-edge staging blit built for loop-invariant-mutation safety
(the slice-3 aggregate-aliasing fix); staging the array here would blit a stale copy over
each iteration's store. The `Alloc` must be emitted before `begin_loop` (it already hoists
to the function's `alloca_home` via `push_alloc`) and referenced directly by dominance from
inside the loop body, exactly the same hazard D3's zero-init loop must also avoid.

## Open questions -- resolved

- **Where `fill`'s rewritten definition would live: dissolved.** `fill` is not being
  rewritten as a word (D4); no new library file, `lib/arrays.sth` untouched.
- **Does `Count` accept a bound length-variable `'N`: no, and moot this slice.** There is
  no polymorphic path in scope to make the question live; see "Deferred to a future
  slice" for why a literal-only `Count` cannot serve `sort`'s scratch-array need, and why
  this is recorded as a real gap rather than resolved away.
- **Is a name-dispatched guard (`check_destructure_drop_guard`-style) the right template
  for the shared gate: no, but the fix is simpler than the brief expected.** Only the
  concrete path needs this gate now (no poly path in scope), so it is not a
  type-representation-spanning helper -- a plain function taking `(ctx, span, position:
  &str, element: Type, count: i64, arrays)` that runs the range check then
  `contains_reference`/`is_copy`, called from both `check_array_word`'s `"fill"` arm and
  the new constructor's `check_term` arm, is enough. `poly_copy_gate` (`check.rs:5500`) is
  not touched or needed this slice (no poly path reached).

## Out of scope (do not resurrect)

- The entire polymorphic checking path: neither the new constructor nor `fill` becomes
  reachable from a poly body this slice (see "Deferred to a future slice"). This includes
  the combinator/splice case.
- A bound type-variable element (`'T`) or bound length-variable count (`'N`) anywhere.
- Linear-element or bare-reference-element arrays (D2 keeps them unconstructible; the
  disposal/move machinery is not this slice's problem).
- `lib/arrays.sth`'s own fate (`sort`/`bin_search`) -- a separate decision, though its
  header comment is load-bearing evidence for why a literal-only `Count` does not close
  the poly-body motivation (see "Deferred to a future slice").
- Runtime/heap allocation. The constructor is a stack-frame `Alloc` exactly as `fill` is;
  nothing here is `Vec`/Phase 6's `alloc`/`free`.
- A compound/nested element type (`[ [i64 3] ; 4 ]`).
- Any change to `check_array_word`'s `"fill"` arm's **type-checking control flow**; the
  diagnostic-constructor parameterization in D2 is the one sanctioned exception, and
  `fill`'s own rendered messages must stay byte-identical.

## Exit criteria

- `[ i64 ; 10 ]` compiles, runs, and produces a well-typed `Copy` `[i64 10]` array with
  every slot zero: assert both `len` folds to `10` **and** the value at index `9` (the
  last slot, not just index `0`) reads as `0`, so the test cannot pass on a wrong-sized or
  partially-zeroed allocation.
- A linear element type and a bare-reference element type are each a **located** error at
  the constructor (asserting the specific diagnostic text, naming the constructor's
  construction site, not `fill`). A count of `0` or `> u32::MAX` is a located range error,
  also naming the constructor. A non-literal count is a located **parse** error (Phase 1
  makes `Count` a literal at the grammar level, so this is a parse-stage error, not a
  check-stage one -- name the exact diagnostic). A compound element type
  (`[ [i64 3] ; 4 ]`) is a located parse error.
- The constructor's lowering is exactly one `Instr::Alloc` (correct size/align for the
  element/count, not just "some `Alloc`") plus a zero-init loop whose **instruction count
  is independent of `Count`**: lowering the same constructor at two different literal
  counts (e.g. 4 and 64) must produce loop bodies of equal instruction count. This is the
  automated proxy for the compile-cost claim; it must fail if the implementation reverts
  to per-element unrolling.
- `fill`'s re-lowering is exactly one `Alloc` plus a loop whose instruction count is
  likewise independent of `N` (same two-count equal-instruction-count test as above,
  applied to `fill`). Every existing `fill`-using example in the corpus produces
  **identical program output** (not "byte-identical `.ssa`", which will differ) before and
  after the re-lowering.
- `docs/phase4-slice6a-spec.md`'s superlinear `fill`-compile-cost numbers (10k / 100k, and
  1M only as a compile-time-only measurement, per that spec's own retirement of a 1M
  *runtime* claim) are re-measured and shown linear/flat, attributed to `fill`'s fixed
  lowering.

## Test coverage (per CLAUDE.md conventions)

- **Parser (unit, beside `parser.rs`):** `array_constructor_with_concrete_type_parses`;
  `array_constructor_bare_semicolon_in_quotation_still_errors` (disambiguation
  regression); `array_constructor_missing_count_is_parse_error`;
  `array_constructor_extra_token_after_count_is_parse_error`;
  `array_constructor_non_literal_count_is_parse_error`;
  `array_constructor_compound_element_type_is_parse_error` (the `[ [i64 3] ; 4 ]` case --
  confirms the existing single-name-token resolution already rejects it, per D1);
  `unterminated_quotation_with_a_semicolon_inside_still_reports_unterminated` (a
  regression pinning the existing "unterminated quotation" fallback when no matching `]`
  exists, so a naive "any `;` before EOF" implementation of the lookahead is caught).
- **Concrete check (unit, beside `check.rs`):** `array_constructor_i64_ten_yields_slot`;
  `array_constructor_bare_reference_element_is_constructed_reference_error` (assert the
  rendered text names the constructor's own site, not `fill`);
  `array_constructor_linear_element_is_rejected` (assert the position-parameterized
  message); `array_constructor_zero_count_is_range_error`;
  `array_constructor_over_u32_max_count_is_range_error`;
  `fill_diagnostics_unchanged_after_position_parameterization` (mutation guard: assert
  `fill`'s own rendered error text is byte-identical to before D2's refactor).
- **Lowering (unit, beside `ir.rs`):** `array_constructor_lowers_to_one_alloc_of_correct_size`
  (assert the `Instr::Alloc`'s size/align operands, not just its presence);
  `array_constructor_zero_init_loop_instruction_count_is_independent_of_count` (lower at
  `Count=4` and `Count=64`, assert equal instruction counts -- the D3 mutation guard);
  `fill_lowering_instruction_count_is_independent_of_n` (same shape, for D4);
  `fill_lowering_preserves_copy_seed_and_surviving_set` (an aggregate/closure-carrying
  seed element still forwards its `surviving` set through the new loop-based lowering).
- **Goldens:** (1) `[ i64 ; 10 ]` builds and runs: `len` folds to `10`, index `9` reads
  `0`. (2) A `[ Bool ; 4 ]` golden reading every slot: each reads a valid `Bool` (proves
  D3's zero-init soundness argument empirically, not just by proof). (3) A regression
  golden (or corpus-wide `cargo test`) proving every existing `fill`-using example's
  output is unchanged after the re-lowering.
- **Located-error goldens:** the linear-element, bare-reference-element, zero-count,
  over-max-count, non-literal-count, and compound-element rejections, each as
  source-in -> diagnostic-out (diagnostics are behavior, per CLAUDE.md).
- **Mutation discipline:** for each new guard, prove it can fail by deleting the guard it
  protects: the `;`-lookahead, the range check, the Copy gate, the reference gate, the
  "one `Alloc` sized correctly" assertion, and both instruction-count-independent-of-N
  assertions (D3's zero-init and D4's `fill`).

## Risks

- **The zero-init loop and `fill`'s loop must each avoid the aggregate-staging hazard
  independently** (D3, D4): the destination array must be an invariant `Alloc` referenced
  by dominance, never a `begin_loop` carried parameter, or the back-edge staging blit
  built for loop-invariant mutation (slice 3) will blit a stale copy over live stores each
  iteration. This is the most likely way either lowering produces silently wrong output
  that a small-`Count`/small-`N` golden would still pass; the index-9/last-slot golden and
  the two-count instruction-equality tests are the guards, not a golden at `Count=1`.
- **`fill`'s re-lowering must preserve exact observable behavior**, including the seed's
  `Copy` replication and the closure-carrying-element `surviving`-set forwarding
  (`check.rs:10265-10269`; `store_elem`'s `Blit` path for aggregate elements,
  `ir.rs:4082`). The corpus-wide regression golden is the guard.
- **D2's diagnostic-constructor parameterization touches call sites inside
  `check_array_word`'s `"fill"` arm.** Low risk (the change is purely a new parameter with
  `fill`'s call sites passing the literal `"fill"`), but it is the one place this slice
  legitimately edits code inside an arm the Out-of-scope section otherwise protects --
  keep the diff to the parameter threading only, and the mutation guard above (byte-
  identical `fill` diagnostics) catches any accidental behavior drift.

## Phases

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Parser: add the [ Type ; Count ] TermKind and the ;-lookahead at parse_term's Token::LBracket arm (parser.rs:2026), mirroring quotation_type_ahead (parser.rs:1564). Type resolves at parse time via the existing Parser::resolve_type (parser.rs:1729), so the TermKind carries a resolved Type, not a raw token. Count is a single integer literal. Unit tests: constructor parses to a Type-carrying term; a quotation with a bare semicolon still errors (disambiguation regression); an unterminated quotation containing a semicolon still reports unterminated (regression); missing/extra/non-literal count and a compound element type ([ [i64 3] ; 4 ]) each a located parse error.",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "Shared gate + concrete check + lowering. Extract a shared helper from check_array_word's fill arm (check.rs:10217) covering the count-range check and the Copy/no-reference element gate, parameterized on a position: &str (mirroring constructed_reference_error's existing pattern) so fill_of_linear_element_error and fill_count_out_of_range_error name the actual construction site; fill's own call sites pass \"fill\" so its diagnostics are byte-identical. New check_term arm calls the shared helper, interns the array type (intern_array_type), pushes the Slot. New lower_term arm: one Instr::Alloc via alloc_array (ir.rs:4028, following the StructWord::Construct precedent of a term carrying its own type with no operand), then a begin_loop/finalize_loop-bounded loop zero-filling the allocation's byte range via PtrOffset+Store/FieldStore -- the Alloc must not be a begin_loop carried parameter (aggregate-staging hazard, see spec Risks). Goldens: [i64 ; 10] builds and runs (len folds to 10, index 9 reads 0); [Bool ; 4] reads a valid Bool at every slot. Located-error tests: bare-reference element, linear element, count 0 / > u32::MAX (each naming the constructor, not fill), fill's own diagnostics unchanged. Lowering tests: Alloc size/align correct; zero-init loop instruction count equal at Count=4 and Count=64.",
      "difficulty": "standard"
    },
    {
      "phase": 3,
      "focus": "Re-lower fill only: change lower_array_word's fill arm (ir.rs:4102) from N unrolled alloc+store to one Alloc plus a begin_loop/finalize_loop-bounded runtime loop storing the seed N times via elem_addr (ir.rs:4073) + store_elem (ir.rs:4082); the Alloc must not be a begin_loop carried parameter (same aggregate-staging hazard as phase 2). Leave check_array_word's fill arm's type-checking control flow untouched beyond phase 2's position parameter. Preserve the Copy-seed replication and closure-carrying surviving-set semantics (test: an aggregate/closure-carrying seed still forwards its surviving set). Lowering test: instruction count equal at N=4 and N=64. Regression: every existing fill-using golden in the corpus produces identical program output (full cargo test). Re-measure docs/phase4-slice6a-spec.md's 10k/100k(/1M compile-time-only) fill timings and show them linear/flat, attributed to the fixed lowering.",
      "difficulty": "hard"
    }
  ]
}
```
