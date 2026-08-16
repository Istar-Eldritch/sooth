# Phase 4 Slice 6h: raw array constructor `[ Type ; Count ]` (concrete path)

A body-level, statically-sized array constructor `[ Type ; Count ]` for the **concrete**
checking path, lowering to one O(1) `Alloc` plus a fixed-size zero-init loop. `fill`'s
lowering is fixed the same way (one `Alloc` plus a runtime store loop, replacing its N
unrolled stores). Together these close a measured compile-cost defect (QBE-quadratic on
one large straight-line block).

**Not delivered:** a polymorphic body constructing its own array. That was the original
motivating goal; round-1 review found it unbuildable and scope was narrowed. What ships is
a concrete-path constructor needing no seed value, plus the `fill` compile-cost fix.
Neither reaches a polymorphic body's own type variables.

## Deferred to a future slice (do not resurrect)

- **No interning route for a body-internal array shape in a poly body.** `subst_polytype`
  and `array_id_of` only *look up* an already-interned shape and panic otherwise. A poly
  body constructing a shape absent from its own signature has nothing to intern against.
- `poly_term`/`poly_walk`/`check_poly_body` hold `arrays: &[ArrayDecl]` immutably;
  threading `&mut Vec<ArrayDecl>` through is a signature change reaching `repl.rs`.
- The combinator path has no binding for a type-variable element at a splice site
  (`subst_polytype`'s `Quotation` arm is `unreachable!`). A **concrete** element inside a
  combinator body is in scope (D5).
- A literal-only `Count` does not serve `lib/arrays.sth`'s `sort`, which needs a scratch
  array of the caller's length `'N` (value-to-length-variable inference, not built here).

## Decisions

**D1 — the constructor is a body-level `TermKind` carrying a parse-time-interned
`Type::Array(id)`.** The parser resolves the element name via `Parser::resolve_type`,
validates `Count`, and interns the whole array shape through its own `&mut Vec<ArrayDecl>`
exactly as `parse_array_type_expr` does. Lowering needs no `array_id_of` search and no
`.expect`; checking needs no name resolution or interning.
- **Consequence:** interning takes a `u32`, so `Count` is validated *before* interning —
  a grammar-level literal in `1..=u32::MAX`, and an out-of-range count is a located
  **parse** error. A compound element type (`[ [i64 3] ; 4 ]`) is a located parse error via
  `expect_word_any_spanned` ("expected a word, found LBracket").
- **Disambiguation:** at `parse_term`'s `Token::LBracket` arm, one lookahead mirroring
  `quotation_type_ahead`'s depth scan — a top-depth `;` before the matching `]` means
  array constructor, else quotation literal. Once `;` is seen the parse **commits** to the
  constructor (what makes the element/count parse errors located). A stray `;` in a
  malformed quotation now reports a constructor-shaped error; an unterminated quotation
  with **no** `;` keeps today's "unterminated quotation" (scan returns `false` at EOF).

**D2 — the shared gate is type-directed only; the constructor adds a zero-validity check.**
One helper, called from both `check_array_word`'s `"fill"` arm and the new constructor's
`check_term` arm, owns exactly the three checks that read a `Type`: `contains_reference`,
`is_copy`, and (constructor only) D3's zero-validity predicate. It does **not** own the
quotation or literal-count checks (they read `Slot` fields) nor the count range check (D1
moved it to parse time); those stay in `fill`'s arm.
- Signature: `(ctx, span, site: &str, element: Type, structs, enums, arrays)`, with
  `#[allow(clippy::too_many_arguments)]` if it reaches 8.
- `fill_of_linear_element_error` gains a `site` parameter rendered as a bare code span;
  `fill` passes `"fill"` so its text stays byte-identical. `constructed_reference_error`
  already takes a noun phrase and is unchanged.

**D3 — contents are zero-initialized; the element type must be provably zero-safe.**
Zeroing is unsound for pointer-shaped `Copy` types (`str`, `cstr`, quotations are `Copy`,
invisible to `contains_reference`, and null-dereference on first read).
- **The gate (checker):** reject an element type transitively containing `Type::Str`,
  `Type::Cstr`, or `Type::Quotation`, recursing through struct fields, **all** enum variant
  fields, and array elements, with a located diagnostic naming the offending inner type and
  the path. All-variant recursion (not just variant 0) is deliberately conservative.
- **The lowering:** one `Alloc` (`alloc_array`, using the term's own interned id), then a
  `begin_loop`/`finalize_loop`-bounded loop zeroing exactly `ArrayLayout::size` bytes,
  **byte-granular**: `ElemAddr` with `stride = 1` (not `PtrOffset`) and a `FieldStore` of an
  8-bit `Const` zero. Byte granularity because an array is not word-padded (`[i8 10]` is
  size 10 / align 1); a word/tail split is a later optimization. Code size stays O(1) in
  `Count` (the defect being closed); runtime cost is one iteration per byte.
- **Carried parameters:** pass **only** the induction index, `stage_aggregates: false`. Do
  not copy the `times` arm's `mem::take(&mut self.stack)`; the destination `Alloc` reaches
  the body by dominance, and carrying it as an aggregate would blit a stale snapshot over
  each iteration's stores.
- **Loop-state hygiene:** wrap in `save_loop_state`/`restore_loop_state` and reset
  `self.terminated` after `start_block(exit)` as the `times` arm does. Without the former, a
  later `Alloc` hoists into a dead preheader and self-tail sealing uses the wrong phi row;
  without the latter, every term after the loop is silently dropped.

**D4 (replaces the brief's D4) — `fill` stays a builtin; only its lowering changes.** The
brief's D4 (demote `fill` to a library word) is **not implementable and is dropped**:
`fill` requires a literal count and mints its result type ad hoc per call site
(`intern_array_type`), which no declared signature's output-only `'N` can express, and the
two poly-check paths are disjoint (`check_poly_body`/`poly_walk` has no `check_array_word`
dispatch). So:
- `lower_array_word`'s `"fill"` arm changes from N unrolled `field_ptr`+`store_elem` pairs
  to one `Alloc` plus a `begin_loop`/`finalize_loop` loop storing the seed N times via
  `elem_addr` + `store_elem` (whose `Blit` arm accepts a runtime destination). This is a
  switch from compile-time `PtrOffset` to runtime `ElemAddr`.
- `fill`'s type-checking is untouched beyond D2's `site` parameter: same literal-count
  restriction, same gates, same `surviving`-set forwarding, byte-identical diagnostics.
  `fill` keeps accepting `str`/`cstr`/quotation elements (it replicates a real seed, never
  mints one from zeroed memory), so D3's extra gate is the constructor's alone.
- **All four D3 loop hazards apply here for the first time** — `fill` has never opened a
  loop.
- Two committed artifacts handled deliberately: the unit test
  `lower_fill_allocs_and_unrolls_n_stores` (encodes the retired behaviour) is **replaced**,
  and `tests/qbe_baseline/` (byte-identical QBE IL snapshot corpus, 14 files use `fill`) is
  **regenerated as a reviewed step**, not to make a red test pass. Runtime profile changes
  (unrolled → counted loop); exit criteria check identical program output, not runtime.

**D5 — a concrete element inside a combinator body works for free.** A combinator is
monomorphized and checked by ordinary concrete `check_word`, then term-spliced at concrete
call sites, so a concrete `[ i64 ; 4 ]` is accepted and lowered with no extra work. Out of
scope is a *type-variable* element/count and reachability through
`poly_walk`/`check_poly_body`. Pinned with a golden.

## Out of scope (do not resurrect)

- A bound type-variable element (`'T`) or bound length-variable count (`'N`); reachability
  through `poly_walk`/`check_poly_body`.
- Linear-element, bare-reference-element, or quotation-element arrays.
- An element type that is or contains `str`/`cstr`/a quotation, **for the new constructor
  only** (D3's gate); `fill` continues to accept them.
- `lib/arrays.sth`'s fate; runtime/heap allocation; a compound/nested element type; a
  word/tail-split or `memset`-style optimization of the zero-fill loop.
- Any change to `fill`'s type-checking control flow beyond D2's `site` parameter (rendered
  messages stay byte-identical).

## Exit criteria

- `[ i64 ; 10 ]` compiles, runs, yields a `Copy` `[i64 10]`: `len` folds to `10` and,
  **after a dirty-frame preamble**, index `9` reads `0`. The preamble is load-bearing:
  `Alloc` never zeroes, so stack residue supplies zeros for free and makes the test a
  placebo without it.
- `[ Bool ; 4 ]`, after the preamble, prints exactly its variant-0 value at all four slots
  (assert concrete output, not "a valid `Bool`").
- `[ i8 ; 10 ]` (size 10 / align 1) runs correctly after the preamble with index `9`
  reading `0` and a live neighbour intact. The deterministic overrun guard is the IR
  assertion below.
- Located rejections naming the offending inner type: `[ str ; 4 ]`; a struct with a `str`
  field; a **depth-2** struct (struct → struct → `str`); a struct with an
  **array-of-`str`** field; an enum carrying a `str` on a **non-zero** variant; a struct
  with a quotation field. A linear element and a bare-reference element are located
  rejections naming the constructor's site rather than `fill`.
- A non-literal count, an out-of-range count (`0`, `> u32::MAX`), and a compound element
  type are each located **parse** errors.
- The constructor's lowering emits **exactly one** `Instr::Alloc` (size/align matching the
  layout); its zero-init loop's `ElemAddr` has `stride == 1` and its loop-bound `Const`
  equals `ArrayLayout::size`; instruction count equal at `Count` 4 and 64, above a small
  floor.
- `fill`'s re-lowering emits one `Alloc` plus a loop, instruction count equal at `N` 4 and
  64 (same floor). `fill` at count 8 with a scalar seed has its last slot equal to the seed;
  with an aggregate (struct) seed, its last slot's fields equal the seed (the `Blit` path).
- Every existing `fill`-using example produces **identical program output** to a baseline
  captured from the **pre-change** binary and committed first. `tests/qbe_baseline/` is
  regenerated deliberately.
- A word containing the constructor (or a `fill`) followed by a `times` compiles and runs
  correctly; a **tail-recursive** word containing one does too; terms after the loop are not
  dropped (loop-state and `terminated` guards).
- A concrete `[ i64 ; 4 ]` inside a **combinator body** compiles and runs (D5).
- The compile-cost claim is a golden: `fill` at `N = 10000` emits an instruction count
  within a constant of `N = 4`. Wall-clock re-measurement recorded in the commit message,
  compile-time only.

## Test coverage

- **Parser (unit):** constructor carries an interned `Type::Array`; shape interned once (a
  second identical constructor reuses the id); type declared later in file resolves;
  missing count / extra token after count / non-literal count / zero count / over-`u32::MAX`
  count / compound element type are each parse errors; quotation without a `;` still parses
  as a quotation; unterminated quotation without a `;` still reports "unterminated" (EOF
  path — a `;`-containing unterminated quotation never reported it, do not assert to it);
  REPL input with a constructor is not complete until the definition ends.
- **Concrete check (unit):** `i64;10` yields a slot; `str` element rejected; struct
  containing `str` rejected; depth-2 struct containing `str` rejected (proves recursion);
  struct with array-of-`str` field rejected (the array arm); enum with `str` on a nonzero
  variant rejected (conservative all-variant recursion); struct with quotation field
  rejected; bare-reference element rejected; linear element rejected; `fill` still accepts a
  `str` element; `fill`'s diagnostics byte-for-byte unchanged after `site` parameterization.
- **Lowering (unit):** exactly one `Alloc` of correct size/align; `ElemAddr` stride `== 1`
  and loop bound `== ArrayLayout::size` (catches a stride of 8 and a bound of `count`, which
  a kind-only assertion misses); instruction count independent of `Count` (equal at 4 and
  64, above a floor); same for `fill`; `fill` uses `elem_addr` after re-lowering (a real
  transition — it uses `field_ptr`/`PtrOffset` today); `fill` preserves the surviving set.
  The retired `lower_fill_allocs_and_unrolls_n_stores` is **replaced** by these. Loop-state
  hygiene is asserted by observable consequence (goldens), since `header`/`alloca_home` are
  not reachable from `lower_src`'s `IrModule`.
- **Goldens:** each zero-init golden carries the dirty-frame preamble; `i64;10`; `i8;10`
  plus a live neighbour; `Bool;4` printing variant-0 four times; `fill` count 8 scalar seed
  (last slot); `fill` count 8 struct seed (last slot's fields); a constructor followed by a
  `times` asserting output *after* the loop; a tail-recursive word containing a `fill`; a
  concrete `[ i64 ; 4 ]` in a combinator body; the pre-change-baselined corpus regression
  over all 14 `fill`-using examples.
- **Located-error goldens:** every rejection in the exit criteria, source-in → diagnostic.
- **Mutation discipline:** prove each guard fails when its target is deleted — the
  `;`-lookahead, parse-time count validation, Copy gate, reference gate, the zero-validity
  predicate at **each recursion depth** (delete struct-field recursion → depth-2 test fails;
  delete array-element recursion → array-of-`str` test fails), the `Alloc` count/size
  assertions, stride and bound assertions, both instruction-count-independence assertions,
  and the `save_loop_state`/`terminated` resets (name which golden fails).

## Risks

- **Zero goldens are placebos without the dirty-frame preamble** — `Alloc` never zeroes, so
  fresh residue often reads as 0 and tests pass flakily with the whole loop deleted.
- **Loop-state hygiene is new exposure for both lowerings** and fails silently in
  *surrounding* code. `examples/combinator_in_times.sth` gives partial cover; the dedicated
  tail-recursive and after-the-loop goldens are the real guards.
- **The zero-validity predicate is easy to under-implement:** rejecting bare `str`/`cstr`
  while missing depth-2, array-through-struct, or non-zero-variant paths leaves the same
  null-pointer hole one level down.
- **Baseline ordering is a trap:** writing expected corpus output from the post-change
  binary makes the regression tautological. Capture and commit pre-change first.
- **Two stale doc comments updated in the same phases:** `store_elem`'s "`fill`'s unrolled
  stores are its only caller" and `elem_addr`'s "every caller is a reference projection".
- **Phases 3 and 4 build two structurally identical loops**, kept separate for phase
  independence; if the second drifts, extract a shared `counted_store_loop` helper.

## Implementation status

Delivered across four phases:

- **Phase 1 — parser + plumbing** (`5e200b18`): the `[ Type ; Count ]` `TermKind` with a
  parse-time-interned `Type::Array(id)`, the `;`-lookahead disambiguation, the four
  exhaustive-match arms (`check_term`/`lower_term`/`resolve::rewrite_terms` stubs,
  `poly_term` a real located "not yet supported in a polymorphic body" diagnostic), and the
  `repl.rs input_is_complete` fix (only clear `open_def` at bracket depth 0). Touched
  `ast.rs`, `check.rs`, `ir.rs`, `parser.rs`, `repl.rs`, `resolve.rs`.
- **Phase 2 — shared type-directed gate + concrete check** (`1b05bc93`): the D2 helper, the
  `site` parameter on `fill_of_linear_element_error`, D3's zero-validity predicate, and the
  real `check_term` arm. Touched `check.rs`, `parser.rs`.
- **Phase 3 — constructor lowering** (`bd8cc659`, review fix `01dd5bc2`): one `Alloc` plus
  the byte-granular `ElemAddr`/`FieldStore` zero-fill loop with index-only carried params,
  `stage_aggregates: false`, save/restore loop state, and `terminated` reset. Added
  `examples/array_ctor.sth`, `tests/phase4_slice6h.rs`; updated `elem_addr`'s doc comment.
- **Phase 4 — re-lower `fill`** (baseline `4735cfbe`, then `e15989a2`): pre-change stdout of
  `fill`-using examples committed first (`tests/fill_corpus/*.stdout`), then `fill`'s arm
  switched to one `Alloc` plus a counted `elem_addr`/`store_elem` loop; the retired unit
  test replaced; `tests/qbe_baseline/` regenerated. Added `examples/fill_relower.sth`,
  `tests/phase4_slice6h_fill.rs`; updated `store_elem`'s doc comment.
