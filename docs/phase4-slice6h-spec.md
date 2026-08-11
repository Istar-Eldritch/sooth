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
polymorphic body's own type variables.

## Correction to the brief (read this first)

The brief's **D4 -- "demote `fill` to an ordinary Sooth-defined library word" -- is not
implementable and is dropped.** This is a defect in the brief, not a scoping choice.

Verified directly against the current binary:

- `fill` requires a **literal** count and always has. Feeding it a computed count yields,
  in word context: `error: type mismatch in main (line N)`, then
  `` `fill` requires a literal count, found a computed `usize` (no const-expr eval)`` (the
  echoed type follows the operand, so a computed `i64` operand renders `i64`), then
  `note: declared ( -- )` -- the `fill_count_not_literal_error` arm, `check.rs:9452`,
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
  discriminate this, since an empty body mismatches *any* declared output) is a pair:
  `` : mkn ( ['T: Copy 4] -- ['T 'N] ) ; `` fails with
  `` body leaves `['T 4]`, but the declared outputs are `['T 'N]` ``, while
  `` : mkn2 ( ['T: Copy 'N] -- ['T 'N] ) ; `` type-checks clean. A concrete length on the
  stack does not bind an output-only length variable; `'N` on both sides is fine. `fill`'s
  rewrite needs exactly the first shape (an output length bound from nothing but a runtime
  count), which is unsatisfiable.
- The two poly-check paths are disjoint (`check.rs:2298`): a quotation-taking poly word
  (a *combinator*, `is_combinator`, `check.rs:7039`) is monomorphized (`'T`->`i64` at
  `check.rs:4967`, `'N`->`STANDALONE_LEN = 4` declared at `check.rs:4963` and applied at
  `check.rs:4971`) and checked by the ordinary concrete `check_word`
  (`check_poly_combinator_standalone`, `check.rs:4951`), which *does* reach
  `check_array_word`; a non-combinator poly word like `fill` goes through
  `check_poly_body`/`poly_walk` (`check.rs:5062`/`5115`), which has **no `check_array_word`
  dispatch** (a `fill` call there falls through to `unknown_word_error`; the poly path is
  not blind to arrays generally -- `poly_call_term`'s `"len"` arm matches
  `PolyType::Array(..)` at `check.rs:5364`, and `poly_copy_gate` recurses through
  `PolyType::Array` at `check.rs:5526-5527`). `fill` has no quotation input, so it is the
  `poly_walk` path -- where the missing `check_array_word` dispatch **and** the
  unsatisfiable output length both bite.

**What replaces D4:** `fill` stays a compiler builtin, unchanged in its type-checking
control flow. Only its **lowering** changes: from N unrolled `alloc + store` instructions
to one `Alloc` plus a small runtime counted loop storing the seed `N` times. See D4 below.

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
  unification) would silently pass without exercising this gap at all.
- **`poly_term`/`poly_walk`/`check_poly_body` hold `arrays: &[ArrayDecl]` immutably**
  (`check.rs:5068`/`5122`/`5152`), and the representation invariant set by
  `raw_to_poly_type` (`parser.rs:1356-1360`) folds a fully-concrete array shape straight to
  `PolyType::Concrete`, never `PolyType::Array`. Threading `&mut Vec<ArrayDecl>` through
  that call chain is a signature change across an API also reached from `repl.rs:2390`.
- **The combinator path has no binding for a type-variable element at a splice site.** A
  combinator is never monomorphized into an `IrFunc`; it is term-spliced into the caller
  (`is_combinator`, `check.rs:7039`), so at the splice site an element token like `'T` has
  no concrete binding to resolve against (`subst_polytype`'s `Quotation` arm is
  `unreachable!` for exactly this reason, `ir.rs:2266-2271`). Note this is about a *type
  variable* element only; a **concrete** element inside a combinator body is in scope and
  already works, see D5.
- **A literal-only `Count` does not serve the one real consumer in this repo.**
  `lib/arrays.sth`'s own `sort` needs a scratch array whose length is the *caller's* array
  length (`'N`), not a fixed literal. A future slice reopening this needs `'N`-as-`Count`,
  which needs value-to-length-variable inference this slice does not build.

## Grounding facts (each verified against current source)

- `check_array_word`'s `"fill"` arm: `check.rs:10226` (fn `check_array_word` at `:10217`).
  Enforces a `Copy` seed (`fill_of_linear_element_error`, `:9485`), no bare reference
  (`constructed_reference_error`, `:9815`; the arm's own comment at `:10250-10252`: "a
  construction site the declaration-site rule cannot reach"), no quotation element
  (`reject_quotation_stored`, called at `:10239`), and a literal count in `1..=u32::MAX`
  (`fill_count_not_literal_error` `:9452`, `fill_count_out_of_range_error` `:9466`).
  Element and count are read off the operand stack at `check.rs:10231-10232`; the interned
  result type is pushed with the element's forwarded `surviving` set
  (`check.rs:10265-10274`).
- `is_copy(Type::Array(id, _))` derives an array's Copy-ness structurally from its element
  (`check.rs:503`); it is a derivation, not a construction-time gate.
- `check_array_word` is called only from the concrete `check_term` (`check.rs:8561`).
- `fill`'s lowering, `lower_array_word`'s `"fill"` arm (`ir.rs:4102`): one straight-line
  block of `2N`-`3N` sequential instructions (alloc + N unrolled `elem_addr`/store pairs).
  `store_elem` (`ir.rs:4082`) is its only element writer and already handles a `Blit` for
  aggregate elements; its own doc notes `fill`'s unrolled stores are its only caller.
- The compile-cost defect is QBE-quadratic on one large straight-line block, not about
  arrays specifically. Reproduced this slice, three times independently, on a zero-array
  flat chain of N `1 +` calls: N=5000 -> 0.11-0.13s, 10000 -> 0.30-0.33s,
  20000 -> 0.95-0.98s -- superlinear (~2.5x-3x per doubling), matching the shape
  `docs/phase4-slice6a-spec.md:350-354` records for `fill` itself (10k~0.36s, 100k~25s,
  1M>300s at `:353`, with the 8 MB-of-stack retirement of the 1M *runtime* case at
  `:351-352` -- this slice's re-measurement stays compile-time-only for the same reason).
- `parse_field_type_expr` (`parser.rs:1680`) already parses `[ elem count ]` in type
  position, and `Parser::resolve_type` (`parser.rs:1729`) is the existing parse-time
  name-to-`Type` resolver (five call sites, `parser.rs:1439/1461/1500/1534/1695`, the last
  being `parse_field_type_expr`'s own tail): it calls `resolve_type_name_in_module`
  (`:1731`) and the qualified-name export check via `type_is_exported` (`:1751`).
  `assemble_module` runs the type pre-pass over every module before any body parses
  (`driver.rs:178-182`), so a type resolved during body parsing already carries its final
  merged id and a type declared later in the file resolves fine; the REPL path passes
  accumulated registries (`repl.rs:1145`).
- `parse_term`'s `Token::LBracket` arm (`parser.rs:2026`) is unconditional and always
  starts a quotation literal; there is no `;` arm anywhere in `parse_term`'s match, a bare
  `;` falls to the `other => Err(...)` arm at `parser.rs:2041`. Probed: `[ 1 ; 2 ]` in a
  body yields `parse error: unexpected token Semicolon at line 1, col 19`. `parse_word`'s
  failure arm is a located `parse error: expected a word, found LBracket`
  (`parser.rs:1721-1723`), which is what rejects a compound element type.
- A struct/enum construction word already lowers from a checked term carrying its own
  `StructId`/`EnumId`, with no operand to read a type off: registered at `ir.rs:714`
  (`StructWord::Construct(id)`) and `ir.rs:730` (`EnumWord::Construct(id, vi)`), consumed
  by lowering arms at `ir.rs:4862` and `ir.rs:4936`. This is the precedent the new
  constructor's lowering follows, and it lets the term carry its interned `Type::Array(id)`
  directly rather than going through `array_id_of`'s structural search and its `.expect`
  panic (`ir.rs:4047-4055`).
- `begin_loop`/`finalize_loop` (`ir.rs:3102`/`3167`) are the general-purpose loop-building
  primitives. `finalize_loop` clears only `carried_slots`/`back_edges`, never
  `header`/`entry_block`/`alloca_home` (`ir.rs:3005-3016`), so every existing
  loop-opening site pairs `save_loop_state` (`:3013`) with `restore_loop_state` (`:3027`)
  -- see the `times` arm's own pair at `ir.rs:3449` and `:3529`. `ElemAddr`
  (`ir.rs:1067`, helper `elem_addr` at `:4073`) is the **runtime-indexed** addressing op
  (`base + index*stride`); `PtrOffset` (`ir.rs:1060-1061`) takes a **compile-time**
  immediate byte offset and therefore cannot advance by a loop variable.
  (`ir.rs:3441`, the `times` arm, is not directly reusable: it pops a quotation value and
  branches on `quot_bodies` provenance (`ir.rs:3456`) to splice or `lower_indirect_call`
  (`ir.rs:3494`), which neither this constructor nor `fill` has.)
- `ArrayLayout` sets `size = stride * count` with `align = elem_align`
  (`ir.rs:998-1008`), so an array is **not** padded to a word: `[i8 10]` is size 10,
  align 1, and a scalar enum element is size 1 / align 1 (`ir.rs:898-909`). A
  word-granular zero-fill would write past the end of such an allocation.
- **Which element types are safe to zero (the D3 gate's basis).** `is_copy` has a
  `_ => true` catch-all (`check.rs:509`), so `Type::Str`, `Type::Cstr` and
  `Type::Quotation` are all `Copy`, and none is a `Type::Ref`, so neither D2's Copy gate
  nor its `contains_reference` gate excludes them. All three are 8-byte **pointer-shaped**
  at the IR level (`IrType::Ptr | OwnedCell | Str | Cstr | Code => 8`, `ir.rs:555`):
  a `Str` is the address of a static `{ptr,len}` descriptor whose consumers dereference it
  (`StrLen` emits `add src, STR_LEN_OFFSET` then `loadl`, `backend/qbe.rs:1186-1190`;
  `StrPtr` `loadl`s through it at `:1193`), and a quotation carries a code pointer. An
  all-zero slot for any of them is a **null pointer**, so reading it is a null dereference,
  not a valid value. `Type::Str`'s own doc states the invariant this would break: `Copy`,
  invisible to `contains_reference`, "and constructible only by a literal (R11), which is
  what makes both of those sound" (`ast.rs:855-861`). Both holes are reachable today,
  verified by probe: `"hi" 3 fill` builds (a `[str 3]`), and with
  `type: Boxed f [ i64 -- i64 ] ;`, `[ 1 + ] Boxed 3 fill` builds (a struct-with-quotation
  element). By contrast `OwnedCell` is already non-`Copy` (`check.rs:508`) and a mutable
  `Ref` is too (`:510`), while a shared `Ref` is caught by `contains_reference`.
- Enum tags are assigned by declaration order from 0 (`variants.iter().enumerate()`,
  `ir.rs:729`) and an empty enum is rejected at `parser.rs:1814`, so tag `0` always names
  a real declared variant -- an all-zero enum is a well-formed variant-0 value *provided
  its payload is itself zero-safe*, which the D3 gate below enforces recursively.

## Decisions

**D1 -- the constructor is a body-level term `[ Type ; Count ]`, a new `TermKind`, concrete
path only.** `Type` is a single type-name token, resolved to a `Type` **at parse time** via
the existing `Parser::resolve_type` (`parser.rs:1729`) -- so the new `TermKind` carries a
resolved `Type`, not a raw string, and needs no name-resolution step later in `check.rs`.
A compound/nested element type (`[ [i64 3] ; 4 ]`) is out of scope and needs no new
rejection logic: the element read expects one word token, so it lands on `parse_word`'s
located "expected a word, found LBracket" error (`parser.rs:1721-1723`). `Count` is a
**literal integer** in `1..=u32::MAX`. A bound type-variable element (`'T`) and a bound
length-variable count (`'N`) are both out of scope (see "Deferred to a future slice").
Disambiguation: at `parse_term`'s existing `Token::LBracket` arm, add one lookahead
(mirroring `quotation_type_ahead`'s style, `parser.rs:1564`) scanning for a top-depth `;`
before the matching `]`. **The scan must stop at a top-depth `;`** rather than continuing
to hunt for a `]`: a `;` also terminates a word definition, so in
`: f ( -- ) [ 1 2 ; : g ( -- ) 3 ] drop ;` an unbounded scan would find a later `]` and
misfire into a constructor-shaped error where today's message is the clearer
"unexpected token Semicolon". Present (and before the matching `]`) -> array constructor;
absent -> quotation literal, exactly as today, including the existing "unterminated
quotation" fallback when no matching `]` exists at all.

**D2 -- the constructor's construction-time gates are shared with `fill`'s, parameterized
on the construction site, and extended with a zero-validity gate.** One helper, called
from both `check_array_word`'s `"fill"` arm and the new constructor's `check_term` arm,
performs: the count-range check; `contains_reference`; `is_copy`; the existing quotation
rejection; **and, for the new constructor only, the zero-validity check below (D3).**
Because `is_copy`/`contains_reference` need the registries (`check.rs:491`, `:527`), the
helper's signature is
`(ctx, span, position: &str, element: Type, count: i64, structs, enums, arrays)`, not the
narrower shape an earlier draft named. Since `fill_of_linear_element_error` and
`fill_count_out_of_range_error` hardcode the word `` `fill` `` in their rendered text, give
both a `position: &str` parameter (mirroring `constructed_reference_error`'s existing
pattern, `check.rs:9815`) so a rejection at the new constructor names the constructor, not
`fill`; `fill`'s own call sites pass `"fill"` so its diagnostics stay byte-identical. This
is a diagnostic-constructor signature change, not a type-checking behavior change.

**D3 -- contents are zero-initialized, and the element type must be provably zero-safe.**
"Unspecified contents" was unsound (an uninitialized `[ Bool ; 4 ]` would let the checker
treat an arbitrary byte as a valid `Bool`), but zeroing alone is *also* unsound for
pointer-shaped `Copy` types: `str`, `cstr` and quotations are `Copy`, invisible to
`contains_reference`, and represented as addresses, so a zeroed slot is a null pointer
whose first read is a null dereference (grounding fact above; both cases probed reachable
today). Therefore:

- **The gate (checker):** reject an element type that transitively contains `Type::Str`,
  `Type::Cstr` or `Type::Quotation`, recursing through struct fields, enum variant fields,
  and array elements, with a located diagnostic naming the offending inner type and the
  path to it. Recursion is deliberately **conservative over all enum variants**, not just
  variant 0: only variant 0's payload is readable in an all-zero value, but rejecting any
  variant's pointer-shaped payload avoids a subtle tag-gating argument for a case
  (a `str`-carrying enum in a zero-init scratch array) with no known use. This is
  element-type-directed logic; an earlier draft's claim that D3 "needs no
  element-type-directed logic at all" was wrong, and only the *fill loop* is type-agnostic.
- **The lowering:** one `Alloc` (`alloc_array`, `ir.rs:4028`, with the term carrying its
  own interned `Type::Array(id)` per the `StructWord::Construct` precedent), then a
  `begin_loop`/`finalize_loop`-bounded loop zeroing exactly `ArrayLayout::size` bytes,
  **byte-granular**: `ElemAddr` with `stride = 1` (**not** `PtrOffset`, whose byte offset
  is a compile-time immediate and cannot advance by the loop variable) and a `FieldStore`
  of a `Const` typed as an 8-bit int, whose store width follows the value's own `IrType`
  (`backend/qbe.rs:1178-1181`). Byte granularity is chosen over a word-granular loop
  because an array is not padded to a word (`[i8 10]` is size 10, align 1), so a
  word-granular loop would write past the allocation and clobber the neighbouring frame
  slot; a word/tail split is a later optimization, not this slice's business. The runtime
  cost (one iteration per byte) is a deliberate trade for a single obviously-correct code
  path, and it is a *runtime* cost only: code size stays O(1) in `Count`, which is the
  defect this slice exists to close.
- **Loop-state hygiene:** the loop must be wrapped in `save_loop_state`/`restore_loop_state`
  (`ir.rs:3013`/`3027`), as every existing loop-opening site does, because
  `finalize_loop` does not reset `header`/`entry_block`/`alloca_home`. Without it a later
  `Alloc` hoists into a now-dead preheader (`push_alloc`, `ir.rs:2989-2998`), a later
  self-tail call jumps to this loop's header with the wrong argument row
  (`ir.rs:3795-3800`), and the `tail && self.header.is_some()` check at `ir.rs:3385`
  misfires.
- **Aggregate-staging hazard:** the destination `Alloc` must be emitted before
  `begin_loop` and referenced by dominance, never passed as a `begin_loop` carried
  parameter -- `begin_loop(&params, stage_aggregates: true)` (`ir.rs:3468`) routes carried
  aggregates through a stable-slot + back-edge staging blit (`ir.rs:3121-3147`,
  `finalize_loop` pass 2 at `:3193`), which would blit a stale snapshot over each
  iteration's stores.

**D4 (replaces the brief's D4) -- `fill` stays a builtin; only its lowering changes.**
`lower_array_word`'s `"fill"` arm (`ir.rs:4102`) changes from N unrolled `alloc + store` to
one `Alloc` plus a `begin_loop`/`finalize_loop`-bounded runtime loop storing the seed value
`N` times via `elem_addr` (`ir.rs:4073`) + `store_elem` (`ir.rs:4082`, which already
handles the aggregate `Blit`). `check_array_word`'s `"fill"` arm's type-checking is **not
touched** beyond D2's `position` parameter: same literal-count restriction, same
Copy/no-reference/quotation/range gates, same `surviving`-set forwarding, same rendered
diagnostics. `fill` keeps accepting `str`/`cstr`/quotation-carrying elements, because it
replicates a real seed value and never mints one from zeroed memory -- D3's extra gate is
the constructor's alone. **Both** hazards from D3 apply here identically and for the first
time (`fill` has never opened a loop before): `save_loop_state`/`restore_loop_state` must
wrap the loop, and the destination array must not be a carried loop parameter.
The runtime profile changes (N unrolled stores become a counted loop); that is an accepted
trade for the compile-time fix, and the exit criteria check identical program output rather
than runtime.

**D5 -- a concrete element inside a combinator body is in scope and works for free.** A
combinator (a quotation-taking poly word) is monomorphized and checked by the *ordinary
concrete* `check_word` (`check_poly_combinator_standalone`, `check.rs:4951`), then
term-spliced at concrete call sites, so a concrete `[ i64 ; 4 ]` (like a concrete `fill`,
which already works there today -- probed) is accepted and lowered with no extra work.
This slice therefore does **not** claim "unreachable from any polymorphic word": what is
out of scope is a *type-variable* element or count, and reachability through
`poly_walk`/`check_poly_body`. Pin the working case with a test so it is a decided outcome
rather than an accident.

## Open questions -- resolved

- **Where `fill`'s rewritten definition would live: dissolved.** `fill` is not being
  rewritten as a word (D4); no new library file, `lib/arrays.sth` untouched.
- **Does `Count` accept a bound length-variable `'N`: no, and moot this slice.** No
  polymorphic type-variable path is in scope; see "Deferred to a future slice" for why a
  literal-only `Count` cannot serve `sort`'s scratch-array need.
- **Is a name-dispatched guard (`check_destructure_drop_guard`-style) the right template
  for the shared gate: no.** Only the concrete path needs it, so it is not a
  type-representation-spanning helper; D2 gives the concrete signature. `poly_copy_gate`
  (`check.rs:5500`) is not touched or needed this slice.

## Out of scope (do not resurrect)

- A bound type-variable element (`'T`) or bound length-variable count (`'N`) anywhere, and
  reachability through `poly_walk`/`check_poly_body`. (A *concrete* element inside a
  combinator body is in scope, D5.)
- Linear-element, bare-reference-element, or quotation-element arrays (D2 keeps them
  unconstructible; the disposal/move machinery is not this slice's problem).
- An element type that is or contains `str`/`cstr`/a quotation, **for the new constructor
  only** (D3's zero-validity gate). `fill` continues to accept them.
- `lib/arrays.sth`'s own fate (`sort`/`bin_search`) -- a separate decision.
- Runtime/heap allocation. The constructor is a stack-frame `Alloc` exactly as `fill` is.
- A compound/nested element type (`[ [i64 3] ; 4 ]`).
- A word/tail-split or `memset`-style optimization of the zero-fill loop.
- Any change to `check_array_word`'s `"fill"` arm's **type-checking control flow**; D2's
  `position` parameter is the one sanctioned exception, and `fill`'s own rendered messages
  must stay byte-identical.

## Exit criteria

- `[ i64 ; 10 ]` compiles, runs, and produces a well-typed `Copy` `[i64 10]` array with
  every slot zero: assert `len` folds to `10` **and** index `9` (the last slot) reads `0`.
- A **sub-word** element array (`[ i8 ; 10 ]`, size 10 / align 1) runs correctly with a
  neighbouring frame value intact afterwards, proving the byte-granular loop does not
  overrun the allocation.
- `[ Bool ; 4 ]` reads a valid `Bool` at every slot (the scalar-enum zero case).
- Each of `[ str ; 4 ]`, `[ cstr ; 4 ]`, a struct-with-`str`-field element, and a
  struct-with-quotation-field element is a **located** rejection naming the offending
  inner type. A linear element and a bare-reference element are likewise located
  rejections, naming the constructor's site rather than `fill`. A count of `0` or
  `> u32::MAX` is a located range error naming the constructor. A non-literal count and a
  compound element type are each located **parse** errors.
- The constructor's lowering is exactly one `Instr::Alloc` with size/align matching the
  layout, plus a zero-init loop whose **instruction count is independent of `Count`**:
  lowering at two counts (4 and 64) must produce equal instruction counts.
- `fill`'s re-lowering is one `Alloc` plus a loop whose instruction count is likewise
  independent of `N` (same two-count test). A `fill` at a count `> 1` has its **last** slot
  equal to the seed (the aggregate-staging guard, which instruction counting cannot catch).
  Every existing `fill`-using example produces **identical program output** before and
  after.
- A word containing a `fill` (or the constructor) and then a `times` loop compiles and runs
  correctly, and a **tail-recursive** word containing one does too -- the loop-state
  hygiene guard.
- `docs/phase4-slice6a-spec.md`'s superlinear `fill` compile-cost numbers (10k / 100k, and
  1M compile-time-only per that spec's retirement of the 1M runtime case) are re-measured
  and shown linear/flat.

## Test coverage (per CLAUDE.md conventions)

- **Parser (unit, beside `parser.rs`):** `array_constructor_with_concrete_type_parses`;
  `array_constructor_bare_semicolon_in_quotation_still_errors`;
  `array_constructor_lookahead_stops_at_a_definition_terminating_semicolon` (the
  `: f ( -- ) [ 1 2 ; : g ...` case from D1);
  `unterminated_quotation_with_a_semicolon_inside_still_reports_unterminated`;
  `array_constructor_missing_count_is_parse_error`;
  `array_constructor_extra_token_after_count_is_parse_error`;
  `array_constructor_non_literal_count_is_parse_error`;
  `array_constructor_compound_element_type_is_parse_error`;
  `array_constructor_type_declared_later_in_file_resolves` (pins the pre-pass ordering
  claim in the grounding facts).
- **Concrete check (unit, beside `check.rs`):** `array_constructor_i64_ten_yields_slot`;
  `array_constructor_str_element_is_rejected`;
  `array_constructor_cstr_element_is_rejected`;
  `array_constructor_struct_containing_str_element_is_rejected`;
  `array_constructor_struct_containing_quotation_element_is_rejected`;
  `array_constructor_enum_with_str_payload_element_is_rejected` (pins D3's conservative
  all-variant recursion); `array_constructor_bare_reference_element_is_rejected`;
  `array_constructor_linear_element_is_rejected`;
  `array_constructor_zero_count_is_range_error`;
  `array_constructor_over_u32_max_count_is_range_error`;
  `fill_still_accepts_a_str_element` (pins D4's "`fill` keeps its looser gate");
  `fill_diagnostics_unchanged_after_position_parameterization`.
- **Lowering (unit, beside `ir.rs`):** `array_constructor_lowers_to_one_alloc_of_correct_size`
  (assert the `Alloc`'s size/align operands);
  `array_constructor_zero_init_loop_instruction_count_is_independent_of_count` (lower at
  4 and 64, assert equality -- the D3 mutation guard);
  `array_constructor_zero_init_uses_elem_addr_not_ptr_offset` (pins the B3 fix);
  `fill_lowering_instruction_count_is_independent_of_n`;
  `fill_lowering_preserves_copy_seed_and_surviving_set`;
  `array_constructor_restores_loop_state` and `fill_restores_loop_state` (assert
  `header`/`alloca_home` are back to their pre-term values after lowering the term).
- **Goldens:** `[ i64 ; 10 ]` (len 10, index 9 is 0); `[ i8 ; 10 ]` with a live
  neighbouring value (overrun guard); `[ Bool ; 4 ]` all slots valid; a `fill` at count 8
  asserting the last slot; a word with a constructor followed by a `times`; a
  tail-recursive word containing a `fill`; a concrete `[ i64 ; 4 ]` **inside a combinator
  body** (D5); and a corpus-wide regression proving every existing `fill`-using example's
  output is unchanged.
- **Located-error goldens:** the `str`, `cstr`, struct-with-`str`, struct-with-quotation,
  linear-element, bare-reference-element, zero-count, over-max-count, non-literal-count,
  and compound-element rejections, each source-in -> diagnostic-out.
- **Mutation discipline:** prove each guard can fail by deleting it: the `;`-lookahead and
  its definition-terminating stop, the range check, the Copy gate, the reference gate,
  **the zero-validity predicate (delete it and watch `[ str ; 4 ]` build)**, the
  `Alloc`-size assertion, both instruction-count-independence assertions, and the
  loop-state restore.

## Risks

- **Loop-state hygiene is new exposure for both lowerings.** `fill` has never opened a loop
  before, so nothing in the corpus currently exercises "a `fill` inside/around another
  loop or a tail call". Missing `save_loop_state`/`restore_loop_state` fails *silently* in
  the surrounding code, not at the constructor. The tail-recursive and
  constructor-then-`times` goldens are the guards; the corpus regression will not catch it
  unless an existing example happens to nest a `fill` in a loop.
- **The aggregate-staging hazard** (D3, D4): the destination array must be an invariant
  `Alloc` referenced by dominance, never a carried loop parameter. Instruction counting
  cannot detect a staging blit, which is why the last-slot goldens (index 9 for the
  constructor, count-8 for `fill`) exist for both.
- **The zero-validity predicate is easy to under-implement.** Rejecting bare `str`/`cstr`
  but forgetting the transitive struct/enum/array cases leaves the same null-pointer hole
  one level down; both empirically reachable today. The four rejection tests plus the
  mutation guard cover it.
- **Byte-granular zeroing costs runtime.** One iteration per byte is 8x a word-granular
  loop for an `i64` array. Accepted deliberately (one correct path, no tail handling); if
  it ever matters, a word/tail split is a self-contained later change.
- **`fill`'s re-lowering must preserve exact observable behavior**, including the seed's
  `Copy` replication and closure-carrying `surviving`-set forwarding
  (`check.rs:10265-10269`, `store_elem`'s `Blit` path, `ir.rs:4082`).

## Phases

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Parser: add the [ Type ; Count ] TermKind and the semicolon-lookahead at parse_term's Token::LBracket arm (parser.rs:2026), mirroring quotation_type_ahead (parser.rs:1564). The scan must stop at a top-depth semicolon (which also terminates a word definition) rather than hunting further for a closing bracket, so a quotation containing a stray semicolon keeps today's clearer error. Type resolves at parse time via the existing Parser::resolve_type (parser.rs:1729), so the TermKind carries a resolved Type, not a raw token; Count is a single integer literal. Unit tests: constructor parses; bare-semicolon-in-quotation still errors; the definition-terminating-semicolon case does not misfire; unterminated quotation containing a semicolon still reports unterminated; missing/extra/non-literal count and a compound element type each a located parse error; a type declared later in the file still resolves.",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "Shared gate (including the new zero-validity predicate) plus the concrete check. Extract a helper from check_array_word's fill arm (check.rs:10217) with signature (ctx, span, position: &str, element: Type, count: i64, structs, enums, arrays) covering the count-range check, contains_reference, is_copy and the quotation rejection, parameterized on position so fill_of_linear_element_error and fill_count_out_of_range_error name the actual construction site (fill's call sites pass \"fill\", diagnostics byte-identical). Add a recursive zero-validity predicate used by the CONSTRUCTOR ONLY: reject an element transitively containing Type::Str, Type::Cstr or Type::Quotation (through struct fields, all enum variant fields, and array elements), with a located diagnostic naming the offending inner type; fill keeps accepting them. New check_term arm calls the helper, interns the array type, pushes the Slot. Tests: str/cstr/struct-with-str/struct-with-quotation/enum-with-str-payload rejections; linear and bare-reference rejections naming the constructor; count 0 and > u32::MAX; fill still accepts a str element; fill's diagnostics unchanged; mutation guard (delete the predicate, [ str ; 4 ] must start building).",
      "difficulty": "standard"
    },
    {
      "phase": 3,
      "focus": "Constructor lowering. One Instr::Alloc via alloc_array (ir.rs:4028) with the term carrying its own interned Type::Array(id) (the StructWord::Construct precedent, ir.rs:714/4862), then a begin_loop/finalize_loop-bounded byte-granular zero-fill of exactly ArrayLayout::size bytes using ElemAddr with stride 1 (NOT PtrOffset, whose offset is a compile-time immediate and cannot advance by the loop variable) and a FieldStore of an 8-bit-typed Const zero. Wrap the loop in save_loop_state/restore_loop_state (ir.rs:3013/3027) as every existing loop-opening site does, and emit the Alloc before begin_loop, referenced by dominance, never as a carried loop parameter (aggregate-staging blit, ir.rs:3468/3121-3147). Goldens: [ i64 ; 10 ] (len 10, index 9 is 0); [ i8 ; 10 ] with a live neighbouring value (byte-granular overrun guard); [ Bool ; 4 ] valid at every slot; a concrete [ i64 ; 4 ] inside a combinator body (D5); a constructor followed by a times loop. Lowering tests: Alloc size/align; instruction count equal at Count=4 and Count=64; uses ElemAddr not PtrOffset; loop state restored.",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "Re-lower fill only: change lower_array_word's fill arm (ir.rs:4102) from N unrolled alloc+store to one Alloc plus a begin_loop/finalize_loop-bounded runtime loop storing the seed N times via elem_addr (ir.rs:4073) + store_elem (ir.rs:4082). Same two hazards as phase 3, both new for fill since it has never opened a loop: wrap in save_loop_state/restore_loop_state, and keep the destination Alloc out of the carried loop parameters. Leave check_array_word's fill arm's type-checking control flow untouched beyond phase 2's position parameter, and preserve the Copy-seed replication and closure-carrying surviving-set semantics. Tests: instruction count equal at N=4 and N=64; a fill at count 8 has its last slot equal to the seed; a tail-recursive word containing a fill; loop state restored; corpus-wide regression proving every existing fill-using example produces identical program output. Re-measure docs/phase4-slice6a-spec.md's 10k/100k(/1M compile-time-only) timings and show them linear/flat.",
      "difficulty": "hard"
    }
  ]
}
```
