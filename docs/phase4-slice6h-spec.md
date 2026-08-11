# Phase 4 Slice 6h spec: a raw array constructor `[ Type ; Count ]` (concrete path)

A body-level, statically-sized array constructor `[ Type ; Count ]` for the **concrete**
checking path, lowering to one O(1) `Alloc` plus a fixed-size zero-init loop. `fill`'s own
lowering is fixed the same way (one `Alloc` plus a runtime store loop, replacing its N
unrolled stores). Together these close a long-standing measured compile-cost defect.

**This slice does not deliver a polymorphic body's ability to construct its own array.**
That was this slice's original motivating goal; round-1 review found it unbuildable as
briefed (see "Deferred to a future slice") and the scope was narrowed in response. What
ships here is: a new concrete-path constructor that needs no seed value, and the fix to
`fill`'s compile-cost defect. Neither reaches a polymorphic body's own type variables.

## Correction to the brief (read this first)

The brief's **D4 -- "demote `fill` to an ordinary Sooth-defined library word" -- is not
implementable and is dropped.** This is a defect in the brief, not a scoping choice.

Verified directly against the current binary:

- `fill` requires a **literal** count and always has. A computed count yields, in word
  context: `error: type mismatch in main (line N)`, then
  `` `fill` requires a literal count, found a computed `usize` (no const-expr eval)`` (the
  echoed type follows the operand), then `note: declared ( -- )` -- the
  `fill_count_not_literal_error` arm, `check.rs:9452`, reached from `check_array_word`'s
  `"fill"` arm at `check.rs:10244-10246`. (The doubled `error: error:` prefix is a
  pre-existing repo-wide artifact, documented at `docs/phase4-slice6a-spec.md:358`.) So
  `fill`'s output length is the *value* of its count argument: the arm mints the result
  type ad hoc per call site via `intern_array_type(arrays, element.ty, count_val as u32)`
  (`check.rs:10264`), **not** through signature unification.
- No declared word signature can express that. D4's proposed
  `( 'T: Copy usize -- ['T 'N] )` has an output-only `'N`. The discriminating probe (the
  brief's original `mk` probe did not discriminate, since an empty body mismatches *any*
  declared output) is a pair: `` : mkn ( ['T: Copy 4] -- ['T 'N] ) ; `` fails with
  `` body leaves `['T 4]`, but the declared outputs are `['T 'N]` ``, while
  `` : mkn2 ( ['T: Copy 'N] -- ['T 'N] ) ; `` type-checks clean. A concrete length on the
  stack does not bind an output-only length variable.
- The two poly-check paths are disjoint (`check.rs:2298`): a combinator (`is_combinator`,
  `check.rs:7039`) is monomorphized (`'T`->`i64` at `check.rs:4968`,
  `'N`->`STANDALONE_LEN = 4` declared at `check.rs:4963`, applied at `check.rs:4971`) and
  checked by the ordinary concrete `check_word` (`check_poly_combinator_standalone`,
  `check.rs:4951`), which *does* reach `check_array_word`; a non-combinator poly word goes
  through `check_poly_body`/`poly_walk` (`check.rs:5062`/`5115`), which has **no
  `check_array_word` dispatch** (a `fill` there falls through to `unknown_word_error`; the
  poly path is not blind to arrays generally -- `poly_call_term`'s `"len"` arm matches
  `PolyType::Array(..)` at `check.rs:5364`, `poly_copy_gate` recurses through it at
  `check.rs:5526-5527`).

**What replaces D4:** `fill` stays a builtin, unchanged in type-checking control flow. Only
its **lowering** changes. See D4.

## Deferred to a future slice (do not resurrect here)

- **No interning route for a body-internal array shape in a poly body.** `subst_polytype`
  (`ir.rs:2245`) and `array_id_of` (`ir.rs:4048`) both *look up* an already-interned shape
  and panic otherwise. Interning sites are `parser.rs:1552` (type positions),
  `check.rs:10264` (`fill`), and `check.rs:6103` (`apply_subst`, signatures only). A poly
  body constructing a shape absent from its own signature has nothing to intern against,
  and a dogfood that *returns* the array hides this (the shape is then the declared output).
- **`poly_term`/`poly_walk`/`check_poly_body` hold `arrays: &[ArrayDecl]` immutably**
  (`check.rs:5068`/`5122`/`5152`), and `raw_to_poly_type` folds a fully-concrete array to
  `PolyType::Concrete` (`parser.rs:1356-1360`). Threading `&mut Vec<ArrayDecl>` through is a
  signature change also reaching `repl.rs:2390`.
- **The combinator path has no binding for a type-variable element at a splice site**
  (`subst_polytype`'s `Quotation` arm is `unreachable!` for this reason,
  `ir.rs:2266-2271`). This is about *type variables* only; a **concrete** element inside a
  combinator body is in scope, see D5.
- **A literal-only `Count` does not serve the one real consumer**: `lib/arrays.sth`'s
  `sort` needs a scratch array of the caller's length `'N`, which needs
  value-to-length-variable inference this slice does not build.

## Grounding facts (each verified against current source)

- `check_array_word`'s `"fill"` arm: `check.rs:10226` (fn at `:10217`). Enforces a `Copy`
  seed (`fill_of_linear_element_error`, `:9485`), no bare reference
  (`constructed_reference_error`, `:9815`, passed the noun phrase
  `"the element`fill`would store"` at `:10257`), no quotation element or count
  (`reject_quotation_stored` at `:10239`, `reject_quotation_operand` at `:10242`), and a
  literal count in `1..=u32::MAX` (`fill_count_not_literal_error` `:9452`,
  `fill_count_out_of_range_error` `:9466`). **The quotation and literal-count checks read
  `Slot` fields** (`element.quot`, `count.quot`, `count.int_val`, `check.rs:10238-10244`),
  not `Type` -- which is why D2's shared helper cannot own them.
- `is_copy` (`check.rs:491-511`): `Array` arm derives structurally at `:503`, `OwnedCell`
  is non-Copy at `:505`, mutable `Ref` non-Copy at `:508`, catch-all `_ => true` at `:509`.
  `contains_reference` is `check.rs:537-559` and recurses through struct fields, enum
  variant fields and array elements at `:544-556`.
- `fill`'s current lowering (`lower_array_word`'s `"fill"` arm, `ir.rs:4100-4118`): reads
  the count from `const_vals`, calls `array_id_of(elem, n)` (`ir.rs:4048`, a structural
  search with an `.expect`), then `alloc_array`, then a `for i in 0..n` loop emitting
  `field_ptr(dst, i * stride)` + `store_elem` -- i.e. **`PtrOffset` with a compile-time
  offset** (`field_ptr`, `ir.rs:4106-4109`), *not* `elem_addr`. `elem_addr` (`ir.rs:4073`)
  is currently called only by the `&>` reference projection (`ir.rs:3866`). So phase 4 is
  precisely a switch from compile-time `PtrOffset` to runtime `ElemAddr`.
- `store_elem` (`ir.rs:4082-4098`) dispatches on the element `IrType` and its `Blit` path
  takes its destination as a plain `Value`, so a runtime-computed `elem_addr` destination
  is fine for both the scalar and aggregate cases.
- The compile-cost defect is QBE-quadratic on one large straight-line block, not
  array-specific. Reproduced three times independently on a zero-array flat chain of N
  `1 +` calls: N=5000 -> 0.11-0.13s, 10000 -> 0.30-0.33s, 20000 -> 0.95-0.98s. Matches
  `docs/phase4-slice6a-spec.md:350-354` for `fill` (10k~0.36s, 100k~25s, 1M>300s at
  `:353`; the 1M *runtime* case retired at `:351-352` for 8 MB of stack, so this slice's
  re-measurement is compile-time-only).
- **The parser can intern the array shape itself.** `Parser` holds
  `arrays: &'t mut Vec<ArrayDecl>` (`parser.rs:806`) whose doc says interning "grows during
  type-expression resolution" and "persists across REPL lines", and
  `parse_array_type_expr` already interns at `parser.rs:1552`. `Parser::resolve_type`
  (`parser.rs:1729`) resolves a single type name via `resolve_type_name_in_module`
  (`:1731`) plus the qualified-name export check (`type_is_exported`, `:1751`).
  `assemble_module` runs the type pre-pass over every file before any body parses
  (`driver.rs:176-181`) and shares one `arrays` registry across the closure
  (`driver.rs:194`), so a parse-time-interned id needs no later remap and a type declared
  later in the file resolves fine. The element token is read by
  `expect_word_any_spanned` (`parser.rs:1714`), whose failure arm is the located
  `parse error: expected a word, found …` at `:1721-1724` (there is no `fn parse_word`).
- `parse_term`'s `Token::LBracket` arm (`parser.rs:2026`) is unconditional; there is no `;`
  arm in the match and a bare `;` falls to `other => Err(...)` at `parser.rs:2042`. Probed:
  `[ 1 ; 2 ]` yields `parse error: unexpected token Semicolon at line 1, col 19`, and
  `[ 1 2 drop` (no `]`, no `;`) yields
  `unexpected end of input, expected`]`(unterminated quotation)`. **"Unterminated
  quotation" only fires at EOF**, so a `;`-containing unterminated quotation has never
  reported it. `quotation_type_ahead` (`parser.rs:1564-1582`) is the depth-scan template
  and returns `false` at EOF (`:1580`).
- Four `TermKind` matches are **exhaustive with no catch-all** and will fail to compile on
  a new variant: `check_term` (`check.rs:8144-9060`), `poly_term` (`check.rs:5159-5289`,
  whose `Quotation` arm at `:5283-5289` is the "not yet supported in a polymorphic body"
  diagnostic template), `lower_term` (`ir.rs:3231-3272`), and `resolve::rewrite_terms`
  (`resolve.rs:458`). Every other walker has a catch-all: `alpha_rename_locals`'s
  `other => other.clone()` (`ast.rs:1224`), `repl.rs:296`/`:2117`,
  `resolve::collect_calls` (`:742`).
- `begin_loop`/`finalize_loop` (`ir.rs:3102`/`3167`). `finalize_loop` clears only
  `carried_slots`/`back_edges` (`:3170-3171`), leaving `header`/`entry_block`/
  `alloca_home` set, so both existing loop-opening sites pair `save_loop_state`
  (`ir.rs:3016`) with `restore_loop_state` (`:3029`): the `times` arm (`:3449`/`:3532`)
  and `lower_self_tail_combinator` (`:3293`/`:3335`). The `times` arm also resets
  `self.terminated` after `start_block(exit)` with an explicit warning that otherwise
  "every term after the `times` is silently dropped" (`ir.rs:3520-3523`), and it passes the
  **whole stack** as carried params (`:3457`) -- which the new loops must *not* copy.
- `PtrOffset` (`ir.rs:1060-1061`) takes a **compile-time** immediate byte offset (backend
  `add base, <imm>`, `backend/qbe.rs:1150-1152`) and cannot advance by a loop variable.
  `ElemAddr` (`ir.rs:1067`, helper `elem_addr` `:4073`) takes a **runtime index**, lowered
  as `mul index, stride` + `add base` (`backend/qbe.rs:1157-1165`), so `stride = 1` gives
  byte addressing with no signature change. `FieldStore` picks its width from the *value's*
  `IrType` (`backend/qbe.rs:1178-1181` -> `field_store_op`, `:398-420`), so an
  `Int { bits: 8 }` zero stores exactly `storeb`.
- `ArrayLayout` sets `size = stride * count`, `align = elem_align.max(1)`
  (`ir.rs:1002-1010`), so an array is **not** word-padded: `[i8 10]` is size 10 / align 1
  and a scalar enum element is size 1 / align 1 (`ir.rs:893-909`). `Instr::Alloc` emits a
  bare `alloc4`/`alloc8`/`alloc16` stack bump (`backend/qbe.rs:1163-1167`, `alloc_op`
  `:424`) -- **no zeroing anywhere**, which is why the zero goldens need a dirty-frame
  preamble.
- **Which element types are safe to zero (D3's basis), and the set is exhaustive.** `Type`
  has 12 variants (`ast.rs:821-879`). `is_copy`'s `_ => true` (`check.rs:509`) makes
  `Type::Str`, `Type::Cstr` and `Type::Quotation` `Copy`, and none is a `Type::Ref`, so
  neither existing gate excludes them; all three are 8-byte **pointer-shaped**
  (`IrType::Ptr | OwnedCell | Str | Cstr | Code => 8`, `ir.rs:555`). A `Str` is the address
  of a static `{ptr,len}` descriptor whose consumers dereference it (`StrLen` emits
  `add src, STR_LEN_OFFSET` then `loadl`, `backend/qbe.rs:1186-1191`; `StrPtr` `loadl`s at
  `:1193`), and a quotation carries a code pointer, so an all-zero slot is a null pointer
  whose first read faults. `Type::Str`'s own doc names the invariant this would break:
  `Copy`, invisible to `contains_reference`, "and constructible only by a literal (R11),
  which is what makes both of those sound" (`ast.rs:855-861`). Both holes are reachable
  today, probed: `"hi" 3 fill` builds a `[str 3]`, and with
  `type: Boxed f [ i64 -- i64 ] ;`, `[ 1 + ] Boxed 3 fill` builds a struct-with-quotation
  element. Everything else is zero-safe: `Int`/`Usize`/`Isize` zero is a valid integer,
  `Float` zero is `+0.0`, `Enum` zero is tag 0 which names a real variant (tags are
  declaration-order from 0, `ir.rs:729`; empty enums rejected at `parser.rs:1814`; `bool`
  zeroes to its variant 0), `OwnedCell` is already non-`Copy`, a shared `Ref` is caught
  transitively by `contains_reference`, and `Struct`/`Array` are D3's recursion. Bundle
  structs are IR-internal and excluded from the construction registry (`ir.rs:712`). So
  `{Str, Cstr, Quotation}` plus the two existing gates leaves no hole.
- A struct/enum construction word's id comes from a **name-keyed registry lookup at
  lowering time** (`StructWord::Construct(id)` registered `ir.rs:714`, consumed `:4862`;
  `EnumWord::Construct(id, vi)` registered `:730`, consumed `:4936`) -- so it is *not* a
  precedent for a term carrying its own id, and D1 below settles the payload question
  directly instead.

## Decisions

**D1 -- the constructor is a body-level term `[ Type ; Count ]`, a new `TermKind` carrying a
parse-time-interned `Type::Array(id)`.** The parser resolves the element name via
`Parser::resolve_type` (`parser.rs:1729`), validates `Count`, and interns the whole array
shape through its own `&mut Vec<ArrayDecl>` exactly as `parse_array_type_expr` already does
(`parser.rs:1552`), so the term carries a finished `Type::Array(id)`. Lowering therefore
needs no `array_id_of` structural search and no `.expect`, and checking needs no
name resolution or interning. (The alternative -- carry element+count and intern in
`check.rs` -- cannot put an id on the term, since the AST is immutable during checking, and
would leave lowering calling `array_id_of` like `fill` does.)
**Consequence, deliberately accepted:** because interning takes a `u32`, `Count` must be
validated *before* interning, so `Count` is a grammar-level literal in `1..=u32::MAX` and
an out-of-range count is a located **parse** error, not a check-time one. A compound
element type (`[ [i64 3] ; 4 ]`) is likewise a located parse error with no new logic: the
element read expects one word token and lands on `expect_word_any_spanned`'s
"expected a word, found LBracket" (`parser.rs:1721-1724`).
**Disambiguation:** at `parse_term`'s `Token::LBracket` arm, add one lookahead mirroring
`quotation_type_ahead`'s depth scan (`parser.rs:1564-1582`): a top-depth `;` before the
matching `]` (depth returning to 0) means array constructor, otherwise quotation literal.
Once the `;` is seen the parse **commits** to the constructor -- that is what makes the
element/count parse errors above located and specific. The accepted cost is that a
malformed quotation containing a stray `;` (`[ 1 2 ; …`) now reports a constructor-shaped
error instead of today's "unexpected token Semicolon"; both are located parse errors and
the commit is required by the exit criteria above. An unterminated quotation with **no**
`;` keeps today's "unterminated quotation" message, because the scan returns `false` at
EOF (`parser.rs:1580`).

**D2 -- the shared gate is type-directed only, and the constructor adds a zero-validity
check.** One helper, called from both `check_array_word`'s `"fill"` arm and the new
constructor's `check_term` arm, owns exactly the three checks that read a `Type`:
`contains_reference`, `is_copy`, and (for the constructor only) D3's zero-validity
predicate. It does **not** own the quotation or literal-count checks, which read `Slot`
fields (`element.quot`, `count.int_val`, `check.rs:10238-10244`) and which the constructor
does not even have operands for -- those stay in `fill`'s arm. Nor does it own the count
range check, which D1 moved to parse time for the constructor; `fill` keeps
`fill_count_out_of_range_error` (`check.rs:9466`) untouched at its own call site.
Signature: `(ctx, span, site: &str, element: Type, structs, enums, arrays)`, with
`#[allow(clippy::too_many_arguments)]` if it reaches 8 (the codebase's existing answer,
e.g. `check.rs:5294`), since "green" is `clippy -- -D warnings`.
Diagnostics take **two different shapes** and must not share one string:
`constructed_reference_error` already takes a noun phrase (`fill` passes
`"the element`fill`would store"`, `check.rs:10257`) and needs none of this;
`fill_of_linear_element_error` (`:9485`) embeds a bare `` `fill` `` and gains a `site`
parameter rendered as a bare code span, with `fill`'s call site passing `"fill"` so its
rendered text stays byte-identical.

**D3 -- contents are zero-initialized, and the element type must be provably zero-safe.**
"Unspecified contents" was unsound (an uninitialized `[ Bool ; 4 ]` lets the checker treat
an arbitrary byte as a valid `Bool`), but zeroing alone is *also* unsound for
pointer-shaped `Copy` types (grounding fact above; `str`, `cstr` and quotations are all
`Copy`, invisible to `contains_reference`, and null-dereference on first read). Therefore:

- **The gate (checker):** reject an element type that transitively contains `Type::Str`,
  `Type::Cstr` or `Type::Quotation`, recursing through struct fields, **all** enum variant
  fields, and array elements, with a located diagnostic naming the offending inner type and
  the path to it. Recursion over all variants rather than just variant 0 is deliberately
  conservative (only variant 0's payload is readable in an all-zero value, but rejecting
  any variant's pointer-shaped payload avoids a subtle tag-gating argument for a case with
  no known use). This is element-type-directed logic; only the *fill loop* is type-agnostic.
- **The lowering:** one `Alloc` (`alloc_array`, `ir.rs:4028`, using the term's own interned
  id per D1), then a `begin_loop`/`finalize_loop`-bounded loop zeroing exactly
  `ArrayLayout::size` bytes, **byte-granular**: `ElemAddr` with `stride = 1` (**not**
  `PtrOffset`, whose offset is a compile-time immediate) and a `FieldStore` of a `Const`
  typed as an 8-bit int. Byte granularity is chosen because an array is not word-padded
  (`[i8 10]` is size 10 / align 1), so a word-granular loop would write past the
  allocation; a word/tail split is a later optimization. The runtime cost (one iteration
  per byte) is a deliberate trade for one obviously-correct path, and it is a *runtime*
  cost only -- code size stays O(1) in `Count`, which is the defect being closed.
- **Carried parameters:** pass **only the induction index** to `begin_loop`, with
  `stage_aggregates: false`. Do not copy the `times` arm's `mem::take(&mut self.stack)`
  (`ir.rs:3457`): the destination `Alloc` and everything else must reach the loop body and
  the exit block by dominance. Passing the array as a carried aggregate would route it
  through the back-edge staging blit (`ir.rs:3117-3147`, `finalize_loop` pass 2 at
  `:3193`), blitting a stale snapshot over each iteration's stores.
- **Loop-state hygiene:** wrap the loop in `save_loop_state`/`restore_loop_state`
  (`ir.rs:3016`/`3029`), and reset `self.terminated` after `start_block(exit)` as the
  `times` arm does (`ir.rs:3520-3523`). Without the former, `finalize_loop` leaves
  `header`/`alloca_home` set and a later `Alloc` hoists into a dead preheader
  (`push_alloc`, `ir.rs:2990-3001`), a later self-tail call seals a `Jmp` to this loop's
  header with the wrong phi row (`ir.rs:3795-3801`), and the combinator self-tail
  interception at `ir.rs:3385` misfires. Without the latter, every term after the loop is
  silently dropped.

**D4 (replaces the brief's D4) -- `fill` stays a builtin; only its lowering changes.**
`lower_array_word`'s `"fill"` arm (`ir.rs:4100-4118`) changes from `n` unrolled
`field_ptr`+`store_elem` pairs to one `Alloc` plus a `begin_loop`/`finalize_loop`-bounded
loop storing the seed `n` times via `elem_addr` (`ir.rs:4073`) + `store_elem`
(`ir.rs:4082`, whose `Blit` arm already accepts a runtime destination).
`check_array_word`'s `"fill"` arm's type-checking is **not touched** beyond D2's `site`
parameter: same literal-count restriction, same gates, same `surviving`-set forwarding,
byte-identical rendered diagnostics. `fill` keeps accepting `str`/`cstr`/quotation-carrying
elements -- it replicates a real seed and never mints one from zeroed memory, so D3's extra
gate is the constructor's alone. **All four hazards in D3's last two bullets apply here
identically and for the first time**, since `fill` has never opened a loop.
Phase 4 also has two committed artifacts to deal with, both named in its phase text: the
unit test `lower_fill_allocs_and_unrolls_n_stores` (`ir.rs:6384`, asserts `FieldStore`
count == 4 and `Blit` count == 0) encodes the behaviour being removed and must be replaced,
and `tests/qbe_baseline/` is a byte-identical committed QBE IL snapshot corpus (29 files,
14 of the examples use `fill`) that must be deliberately regenerated, not blindly.
The runtime profile changes (unrolled stores become a counted loop); accepted, and the exit
criteria check identical program output rather than runtime.

**D5 -- a concrete element inside a combinator body is in scope and works for free.** A
combinator is monomorphized and checked by the ordinary concrete `check_word`
(`check_poly_combinator_standalone`, `check.rs:4951`), then term-spliced at concrete call
sites, so a concrete `[ i64 ; 4 ]` (like a concrete `fill`, which already works there --
probed) is accepted and lowered with no extra work. This slice therefore does **not** claim
"unreachable from any polymorphic word": out of scope is a *type-variable* element or
count, and reachability through `poly_walk`/`check_poly_body`. Pin the working case with a
golden so it is decided rather than accidental.

## Open questions -- resolved

- **Where `fill`'s rewritten definition would live: dissolved.** `fill` is not rewritten as
  a word (D4); no new library file, `lib/arrays.sth` untouched.
- **Does `Count` accept a bound length-variable `'N`: no, and moot** -- no polymorphic
  type-variable path is in scope.
- **Is a name-dispatched guard (`check_destructure_drop_guard`-style) the right template:
  no.** D2 gives the concrete signature, and it is type-directed rather than name-directed.
  `poly_copy_gate` (`check.rs:5500`) is not touched this slice.

## Out of scope (do not resurrect)

- A bound type-variable element (`'T`) or bound length-variable count (`'N`), and
  reachability through `poly_walk`/`check_poly_body`. (A *concrete* element inside a
  combinator body is in scope, D5.)
- Linear-element, bare-reference-element, or quotation-element arrays.
- An element type that is or contains `str`/`cstr`/a quotation, **for the new constructor
  only** (D3's gate). `fill` continues to accept them.
- `lib/arrays.sth`'s own fate; runtime/heap allocation; a compound/nested element type; a
  word/tail-split or `memset`-style optimization of the zero-fill loop.
- Any change to `check_array_word`'s `"fill"` arm's **type-checking control flow**; D2's
  `site` parameter is the one sanctioned exception and `fill`'s rendered messages must stay
  byte-identical.

## Exit criteria

- `[ i64 ; 10 ]` compiles, runs, and yields a `Copy` `[i64 10]`: `len` folds to `10` and,
  **after a dirty-frame preamble** (a preceding word that `fill`s a same-or-larger array
  with a nonzero seed in the same frame region and returns), index `9` reads `0`. Without
  the preamble this criterion is a placebo: `Alloc` is a bare stack bump with no zeroing
  (`backend/qbe.rs:1163-1167`), so stack residue supplies zeros for free.
- `[ Bool ; 4 ]`, after the same preamble, prints exactly its variant-0 value at all four
  slots (assert the concrete expected output, not "a valid `Bool`" -- a garbage byte prints
  as `true` and nothing would fail).
- `[ i8 ; 10 ]` (size 10 / align 1) runs correctly after the preamble with index `9`
  reading `0` and a live neighbouring value intact. The *deterministic* overrun guard is
  the IR assertion below, since QBE rounds and orders frame slots itself.
- Each of these is a **located** rejection naming the offending inner type: `[ str ; 4 ]`;
  a struct with a `str` field; a **depth-2** struct (struct -> struct -> `str`); a struct
  with an **array-of-`str`** field (the only reachable exercise of the predicate's array
  arm, since array shapes are unnameable and a compound element is a parse error); an enum
  carrying a `str` on a **non-zero** variant; and a struct with a quotation field. A linear
  element and a bare-reference element are likewise located rejections naming the
  constructor's site rather than `fill`.
- A non-literal count, an out-of-range count (`0`, `> u32::MAX`), and a compound element
  type are each located **parse** errors (D1 moved count validation to parse time).
- The constructor's lowering emits **exactly one** `Instr::Alloc`, with size/align matching
  the layout; its zero-init loop's emitted `ElemAddr` has `stride == 1` and its loop bound
  `Const` equals `ArrayLayout::size`; and its instruction count is equal at `Count` 4 and
  64 while being **greater than a small floor** (so an empty lowering cannot satisfy it).
- `fill`'s re-lowering emits one `Alloc` plus a loop with instruction count equal at `N` 4
  and 64 (same floor). A `fill` at count 8 with a **scalar** seed has its last slot equal to
  the seed, and a `fill` at count 8 with an **aggregate (struct) seed** has its last slot's
  fields equal to the seed (the `store_elem` `Blit` path under the new loop).
- Every existing `fill`-using example produces **identical program output** to a baseline
  captured from the **pre-change** binary and committed in phase 4's first commit, before
  `ir.rs:4102` is touched. `tests/qbe_baseline/` is regenerated deliberately as a reviewed
  step, not to make a red test pass.
- A word containing the constructor (or a `fill`) followed by a `times` loop compiles and
  runs correctly, and a **tail-recursive** word containing one does too, and terms after
  the loop are not dropped -- the loop-state and `terminated` guards.
- A concrete `[ i64 ; 4 ]` inside a **combinator body** compiles and runs (D5).
- The compile-cost claim is committed as a golden rather than a manual measurement: a
  `fill` at `N = 10000` emits an instruction count within a constant of the `N = 4` case.
  The wall-clock re-measurement of `docs/phase4-slice6a-spec.md:353`'s 10k/100k numbers is
  recorded in the phase's commit message, compile-time only.

## Test coverage (per CLAUDE.md conventions)

- **Parser (unit, beside `parser.rs`):** `array_constructor_with_concrete_type_parses`
  (assert the term carries an interned `Type::Array`);
  `array_constructor_interns_the_array_shape_once` (a second identical constructor reuses
  the id); `array_constructor_type_declared_later_in_file_resolves`;
  `array_constructor_missing_count_is_parse_error`;
  `array_constructor_extra_token_after_count_is_parse_error`;
  `array_constructor_non_literal_count_is_parse_error`;
  `array_constructor_zero_count_is_parse_error`;
  `array_constructor_over_u32_max_count_is_parse_error`;
  `array_constructor_compound_element_type_is_parse_error`;
  `quotation_without_a_semicolon_still_parses_as_a_quotation`;
  `unterminated_quotation_without_a_semicolon_still_reports_unterminated` (the EOF path;
  note a `;`-containing unterminated quotation never reported "unterminated" and must not
  be asserted to);
  `repl_input_with_a_constructor_is_not_complete_until_the_definition_ends` (see the REPL
  fix in phase 1).
- **Concrete check (unit, beside `check.rs`):** `array_constructor_i64_ten_yields_slot`;
  `array_constructor_str_element_is_rejected`;
  `array_constructor_struct_containing_str_element_is_rejected`;
  `array_constructor_depth_two_struct_containing_str_is_rejected` (proves recursion, not
  one-level field iteration); `array_constructor_struct_with_array_of_str_field_is_rejected`
  (the predicate's array arm); `array_constructor_enum_with_str_on_a_nonzero_variant_is_rejected`
  (pins the conservative all-variant recursion);
  `array_constructor_struct_containing_quotation_element_is_rejected`;
  `array_constructor_bare_reference_element_is_rejected`;
  `array_constructor_linear_element_is_rejected`;
  `fill_still_accepts_a_str_element` (pins D4's looser gate);
  `fill_diagnostics_unchanged_after_site_parameterization` (assert the **full rendered
  string** byte-for-byte, not `contains("fill")`).
- **Lowering (unit, beside `ir.rs`):** `array_constructor_emits_exactly_one_alloc_of_correct_size`
  (count the `Alloc`s and assert size/align);
  `array_constructor_zero_init_uses_stride_one_and_bounds_by_layout_size` (assert the
  `ElemAddr`'s stride operand `== 1` and the loop-bound `Const` `== ArrayLayout::size` --
  these catch the two live errors, a stride of 8 and a bound of `count`, which an
  instruction-*kind* assertion would not);
  `array_constructor_instruction_count_is_independent_of_count` (equal at 4 and 64, and
  above a floor); `fill_lowering_instruction_count_is_independent_of_n` (same);
  `fill_lowering_uses_elem_addr_after_relowering` (it uses `field_ptr`/`PtrOffset` today,
  so this is a real transition assertion); `fill_lowering_preserves_surviving_set`.
  Existing `lower_fill_allocs_and_unrolls_n_stores` (`ir.rs:6384`) is **replaced** by these,
  not deleted silently -- its name encodes the retired behaviour.
  Note the instruction-list idiom is `count(func, pred)` (`ir.rs:5632`) over `IrFunc`
  blocks; `header`/`alloca_home` live on the consumed `FuncBuilder` (`ir.rs:2858`) and are
  **not** reachable from `lower_src`'s `IrModule` (`ir.rs:5285`), so loop-state hygiene is
  asserted by observable consequence (the goldens below), not by reading those fields.
- **Goldens:** each zero-init golden carries the dirty-frame preamble; `[ i64 ; 10 ]`;
  `[ i8 ; 10 ]` plus a live neighbour; `[ Bool ; 4 ]` printing its variant-0 value four
  times; `fill` at count 8 with a scalar seed (last slot); `fill` at count 8 with a struct
  seed (last slot's fields); a constructor followed by a `times`, asserting output *after*
  the loop appears (the `terminated` guard); a tail-recursive word containing a `fill`; a
  concrete `[ i64 ; 4 ]` inside a combinator body; and the pre-change-baselined corpus
  regression over all 14 `fill`-using examples.
- **Located-error goldens:** every rejection in the exit criteria, source-in ->
  diagnostic-out.
- **Mutation discipline:** prove each guard fails when its target is deleted: the
  `;`-lookahead, the parse-time count validation, the Copy gate, the reference gate, **the
  zero-validity predicate at each recursion depth** (delete the struct-field recursion and
  the depth-2 test must fail; delete the array-element recursion and the
  array-of-`str` test must fail), the `Alloc` count/size assertions, the stride and bound
  assertions, both instruction-count-independence assertions, and the `save_loop_state`/
  `terminated` resets (delete each and name which golden fails).

## Risks

- **The zero goldens are placebos without the dirty-frame preamble.** `Alloc` never zeroes,
  so fresh stack residue often reads as 0 and the tests would pass *flakily* with the
  entire zero-fill loop deleted. The preamble is load-bearing, not decoration. A single
  `[i64;N]` dirtier only reliably overlaps other `[i64;N']` stack slots; the `[i8;10]` and
  `[bool;4]` probes need their own byte-granular dirtier or they pass on incidental
  stack-zero residue instead of the zero-init loop (mutation-tested in phase 3's review).
- **The exit `terminated = false` reset is unreachable in phase 3's fixed loop body**
  (nothing in the `ArrayCtor` arm ever sets `terminated = true`), so no golden can exercise
  it there; it earns coverage only once phase 4 splices `fill`'s own body through the same
  template shape. Kept for template parity, not because phase 3 can prove it load-bearing.
- **Loop-state hygiene is new exposure for both lowerings** and fails silently in the
  *surrounding* code, not at the loop. `examples/combinator_in_times.sth:13` does place a
  `fill` before a `times`, so the corpus offers partial cover, but the dedicated
  tail-recursive and after-the-loop goldens are the real guards.
- **The zero-validity predicate is easy to under-implement**: rejecting bare `str`/`cstr`
  while missing the depth-2, array-through-struct, or non-zero-variant paths leaves the
  same null-pointer hole one level down. Those three tests are the discriminating ones.
- **Phase 4's baseline ordering is a trap**: writing the expected corpus output from the
  post-change binary makes the regression tautological. Capture and commit pre-change
  first.
- **Two doc comments go stale** and should be updated in the same phases:
  `store_elem`'s "`fill`'s unrolled stores are its only caller" (`ir.rs:4080-4081`) and
  `elem_addr`'s "every caller is a reference projection" (`ir.rs:4070-4072`).
- **Phases 3 and 4 build two structurally identical loops.** Kept separate so the phases
  stay independent; the cost is that the four hazards must be got right twice. If the
  second one drifts, extract a shared `counted_store_loop` helper rather than patching one
  side.

## Phases

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Parser + plumbing. Add the [ Type ; Count ] TermKind carrying a parse-time-interned Type::Array(id): resolve the element name via Parser::resolve_type (parser.rs:1729), validate Count as a literal in 1..=u32::MAX (out-of-range is a located PARSE error, since interning takes a u32), then intern the shape through the parser's own &mut Vec<ArrayDecl> exactly as parse_array_type_expr does (parser.rs:1552). Add the semicolon-lookahead at parse_term's Token::LBracket arm (parser.rs:2026) mirroring quotation_type_ahead's depth scan (parser.rs:1564-1582): a top-depth semicolon before the matching bracket means constructor and the parse COMMITS there; EOF returns false so an unterminated quotation with no semicolon keeps today's message. Add the four exhaustive-match arms this variant forces, since none has a catch-all: check_term (check.rs:8144), lower_term (ir.rs:3231) and resolve::rewrite_terms (resolve.rs:458) get stubs/no-ops for now, and poly_term (check.rs:5159) gets a real located diagnostic modelled on its Quotation arm (check.rs:5283-5289) saying the constructor is not yet supported in a polymorphic body -- that is a user-visible message, so it needs its own test. Fix repl.rs input_is_complete (repl.rs:2949-2966), which clears open_def on any Token::Semicolon regardless of bracket depth: only clear it at bracket_depth == 0, or a first-line ': f ( -- ) [ i64 ; 4 ]' is judged Complete and submitted unterminated. Tests: the parser list in the spec's Test coverage section, plus the poly-body diagnostic and the REPL completeness case.",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "Shared type-directed gate plus the concrete check. Extract a helper (ctx, span, site: &str, element: Type, structs, enums, arrays) owning exactly contains_reference, is_copy, and the new zero-validity predicate; add #[allow(clippy::too_many_arguments)] if it reaches 8 args. It does NOT own the quotation or literal-count checks (they read Slot fields, check.rs:10238-10244) nor the count range check (parse-time per phase 1) -- those stay in fill's arm. Give fill_of_linear_element_error (check.rs:9485) a site parameter rendered as a bare code span, with fill passing \"fill\" so its text is byte-identical; constructed_reference_error already takes a noun phrase and needs no change. The zero-validity predicate, used by the CONSTRUCTOR ONLY, rejects an element transitively containing Type::Str, Type::Cstr or Type::Quotation, recursing through struct fields, ALL enum variant fields, and array elements, naming the offending inner type and the path. Replace check_term's phase-1 stub with the real arm: call the helper, push the Slot from the term's interned Type::Array. Tests: the concrete-check list in the spec, notably the depth-2 struct, the struct-with-array-of-str, and the enum-with-str-on-a-nonzero-variant cases; fill still accepts a str element; fill's diagnostics byte-identical; and the mutation guards per recursion depth.",
      "difficulty": "standard"
    },
    {
      "phase": 3,
      "focus": "Constructor lowering. Replace lower_term's phase-1 stub: one Instr::Alloc via alloc_array (ir.rs:4028) using the term's own interned id (no array_id_of), then a begin_loop/finalize_loop-bounded byte-granular zero-fill of exactly ArrayLayout::size bytes using ElemAddr with stride 1 (NOT PtrOffset, whose offset is a compile-time immediate, backend/qbe.rs:1150-1152) and a FieldStore of an 8-bit-typed Const zero. Pass ONLY the induction index to begin_loop with stage_aggregates: false -- do not copy the times arm's mem::take(&mut self.stack) (ir.rs:3457); the destination Alloc reaches the body by dominance, and carrying it would route it through the back-edge staging blit (ir.rs:3117-3147). Wrap the loop in save_loop_state/restore_loop_state (ir.rs:3016/3029) and reset self.terminated after start_block(exit) as the times arm does (ir.rs:3520-3523). Goldens (each with the dirty-frame preamble, since Alloc never zeroes): [ i64 ; 10 ] with index 9 reading 0; [ i8 ; 10 ] plus a live neighbour; [ Bool ; 4 ] printing its variant-0 value four times; a constructor followed by a times, asserting output after the loop; a concrete [ i64 ; 4 ] inside a combinator body (D5). Lowering tests: exactly one Alloc of correct size/align; ElemAddr stride == 1 and loop bound == ArrayLayout::size; instruction count equal at Count 4 and 64 and above a floor. Update elem_addr's stale doc comment (ir.rs:4070-4072).",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "Re-lower fill only. FIRST COMMIT, before touching ir.rs: capture and commit the expected stdout of all 14 fill-using examples from the PRE-change binary, or the regression is tautological. Then change lower_array_word's fill arm (ir.rs:4100-4118) from n unrolled field_ptr+store_elem pairs to one Alloc plus a begin_loop/finalize_loop-bounded loop storing the seed n times via elem_addr (ir.rs:4073) + store_elem (ir.rs:4082, whose Blit arm accepts a runtime destination). Note this is a switch from compile-time PtrOffset to runtime ElemAddr -- fill uses field_ptr today. All four hazards from phase 3 apply here for the first time, since fill has never opened a loop: index-only carried params, stage_aggregates false, save/restore loop state, reset terminated. Leave check_array_word's fill arm's type-checking control flow untouched beyond phase 2's site parameter, and preserve the Copy-seed replication and surviving-set forwarding. Replace the unit test lower_fill_allocs_and_unrolls_n_stores (ir.rs:6384, asserts FieldStore==4/Blit==0) whose name encodes the retired behaviour, with: instruction count equal at N 4 and 64 above a floor, and uses elem_addr after re-lowering. Goldens: fill at count 8 with a scalar seed (last slot equals seed); fill at count 8 with a struct seed (last slot's fields equal the seed); a tail-recursive word containing a fill; the pre-change-baselined corpus regression. Regenerate tests/qbe_baseline/ (29 files, 14 fill-using) deliberately as a reviewed step, never blindly to green a red test. Commit the compile-cost re-measurement (docs/phase4-slice6a-spec.md:353's 10k/100k, compile-time only) in the message, and add the N=10000-vs-N=4 instruction-count golden as its durable proxy. Update store_elem's stale doc comment (ir.rs:4080-4081).",
      "difficulty": "hard"
    }
  ]
}
```
