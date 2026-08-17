# Phase 7 Slice 2: static storage and global sets (spec)

Module-level static storage as a *place* (never owned, moved, or dropped;
constant-initialised; reached only through the existing `&`/`&!` sigil), plus
the per-word **global set**: which statics a word touches and in what mode,
inferred over the intra-module call graph and *declared* on exported words,
checked for exact match. This is the plain, non-embedded half of DESIGN.md's
*Embedded* section: no MMIO overlay, no `volatile`, no fixed address, no ISR
(all Phase 9).

## D1 — declaration grammar

A fifth arm of `parse_bodies`'s top-level dispatch, symmetric with `extern:`:

```text
static-decl := "static:" NAME Type ( "=" literal )? ";"
literal     := int-lit | bool-lit | str-lit
```

- One static per declaration, no batch form.
- `= literal` is a **single literal only**: int, bool, or string. No
  arithmetic, no reference to another static, no aggregate literal. This is
  DESIGN.md's `Preelaborate` tier, which falls out of "no comptime interpreter".
- The initialiser may be **elided**, meaning the type's zero: `0`, `false`, and
  for `str` the empty string.
- **Scalar types only** (`i64`, `u32`, `bool`, `str`). The rejection is
  **allow-list-based, not struct-detection-based**: the parser is single-pass
  with no type table at declaration time (a `type:` may follow a `static:` in
  the same file), so it accepts the fixed scalar keyword set and rejects
  everything else with one located "non-scalar type" error naming both the
  static and the type. A genuine struct type and a mistyped or
  forward-referenced user type are indistinguishable here and get that same
  error.
- `NAME` obeys the reserved-name / access-word rejections `parse_worddef` and
  `parse_extern_decl` already apply.

## D2 — the global clause

Its own keyword-headed clause, right after the effect's closing `)` and before
the body, the same slot family as `inline`, not nested in the stack-effect
parens (an in-parens placement reads as part of the stack shape, which it is
not):

```text
worddef      := ":" NAME "inline"? "(" effect ")" global-clause? body ";"
global-clause:= "global:" entry ( "," entry )*
entry        := NAME mode
mode         := "r" | "w"

: tick ( -- i64 ) global: COUNT w, LIMIT r  ...body... ;
```

- `parse_effect` / `parse_poly_effect` are **unchanged**. `parse_worddef` peeks
  for `global:` after the `)`, mirroring its existing `declares_inline` peek.
  An effect with no clause parses byte-for-byte as before.
- Stored as `WordDef.declared_globals: Option<Vec<GlobalEntry>>`. `None` = no
  clause; `Some(vec![])` is not representable (a bare `global:` is a located
  parse error).
- `mode` is declared but always verified against the inferred mode: mode is
  derived from the body, never independently authored.
- The comma **is** load-bearing here, unlike everywhere else in the grammar:
  entries are whitespace-separated with no other delimiter, so `COUNT w LIMIT r`
  is otherwise indistinguishable from a clause that ended after `COUNT w` with
  the rest falling into the body. The lexer has no comma token (this is the
  first comma in Sooth), so a glued trailing comma (`w,`) is stripped with
  `strip_suffix(',')` and a free-standing `,` is accepted separately. A missing
  comma is a **located parse error** naming the second entry: letting the clause
  end early would report the dropped entry as an unknown word in the body, the
  silent truncation this language exists to eliminate.
- Not a general "compiler annotation" mechanism. One consumer with a checked,
  structured payload does not justify generic marker syntax; `inline` is the
  project's pattern for a word-level marker.

## D4 — AST

No new `Type` variant: a static's ref is exactly `&T`/`&!T` via `intern_ref_type`.

```rust
pub struct StaticDecl { name: String, ty: Type, init: StaticInit, module: u32, span: Span }
pub enum StaticInit { Zero, Int(i64), Bool(bool), Str(String) }
pub struct GlobalEntry { name: String, mode: GlobalMode /* R | W */, span: Span }
```

`Module` and `ParsedBodies` each gain `statics: Vec<StaticDecl>`;
`assemble_module` drains one into the other. `StaticDecl.span` is the name's,
not the keyword's, matching `WordDef.span` so the duplicate-declaration error
points at the name it names.

## Resolution

A bare name after sigil-stripping resolves in order: (a) a bound local,
(b) a static of the **accessing module**, (c) whatever an unresolved name means
today.

### R1 — borrow-typing takes a static branch

In the borrow-word family arm of `check/word_families.rs`, when `rest` is not a
bound local, look it up in the accessing module's static table before the
existing rejection paths. A hit pushes `intern_ref_type(refs, T, mutable)`
exactly as a local borrow does, with static-rooted provenance (R3).

A **scalar** static *is* borrowable, unlike a scalar local: a static has a
data-symbol address. `borrow_of_scalar_local_error` is therefore reached only
when `rest` is neither a local nor a static. Diagnostics demangle the static
name before printing it.

### R2 — statics are module-private and unconditionally mangled

A static is never exported or imported; only the `global:` clause on an exported
word crosses a module boundary. `resolve_modules` learns `statics` as a name
category: it mangles each `StaticDecl.name` and rewrites every `&NAME`/`&!NAME`
whose core name is a module static (the `strip_ref_sigil` fallthrough). A core
name that is neither a local, a module type/word, nor a module static is left
untouched.

"Exactly as words are" does **not** extend to `resolve::mangle`'s exemption
list. Statics use their own `resolve::mangle_static`, unconditional. Every
exemption `mangle` makes (`main`, `drop`, every `lib/core.sth` prelude word)
exists so a *word* of that name stays reachable by bare name from a module that
did not declare it; a static is reachable no such way, so the exemptions only
ever collided at the assembler. Both the decl loop and the `&NAME` rewrite use
it, and *only* those two: the word-rewrite arm just below the static arm needs
the exemptions, and blanket-replacing the call unresolves every `if`.

`ast::rename_call` needs no change: a static ref is not a bound local, so it
already falls through.

### R3 — a static-rooted borrow keeps exclusivity, skips only the disposal scans

The borrow checker's exclusivity/aliasing scans are **`owned_root`-keyed, not
type-keyed** (`live_mutable_borrow_of` and its immutable twin test
`d.owned_root.as_deref() == Some(place)`). A static-rooted `Deriv` therefore
sets `owned_root: Some(<static name>)` and `place` to the same, exactly as an
owned local does: reporting `None` (as a reference parameter's reborrow does)
would silently disable mutable-aliasing detection for permanent shared state,
the case that needs it most. Two live `&!COUNT`, or a live `&!COUNT` beside a
`&COUNT`, is the existing `conflicting_borrow_error` /
`aliased_place_borrow_error`, unchanged.

The disposal/consume/leak exemption is **vacuous in code, by design**: no branch
enacts it. A static's borrow is `&T`/`&!T`, already non-linear, and the static
itself never reaches the stack, so those scans have nothing to reach for. A
later reader finding no static case there should not "restore" one.

`stored_reference_error` / `check_no_stored_references` apply **unchanged**: that
rule is type-keyed (`contains_reference`), so a static-rooted ref may not be put
in a struct field, array, cell, or another static.

One `owned_root`-keyed scan is neither exclusivity nor disposal and needs its
own carve-out: `check_reference_across_back_edge` rejects any reference with a
set `owned_root` that crosses a self-tail-call back-edge, because a *local*'s
storage does not survive the next iteration. A static's data-segment storage
does, so the scan skips a borrow **recorded** as static-rooted via a
`static_root` flag on the `Deriv`, set at the borrow site and never re-derived
by looking the root name up in the static table: locals are not mangled and
statics are, so a local spelled `COUNT__m0` would answer that lookup and inherit
the exemption.

An escaping closure capturing `&!COUNT` is **admitted**: `ref_root_is_in_frame`
finds no static in the local scope and classifies the borrow `OuterRooted`,
which is the semantically right answer (data-segment storage outlives every
frame). No static carve-out is needed in that classification.

**Recorded gap (later slice): exclusivity is per-body, so two cross-body holes
stay open.** The `owned_root` scans run over one word body, which is all a local
root ever needs, since a local cannot be named from another body. A static can.
Neither a caller holding a live `&!COUNT` across a call to a callee taking its
own `&!COUNT`, nor a `&!COUNT` escaping in a materialized closure's `env` and
coexisting with a second live borrow, is caught. The natural home is the R4/R5
fixpoint, which already knows which statics each callee writes.

## Global-set analysis (`src/check/globals.rs`)

Runs from `assemble_module` **pre-mangle**, beside `check_exported_signatures`,
where word names, static names and the raw `export:` list all still agree.

### R4 — direct sets and the intra-module call graph

A word's **direct set** maps `static-name -> mode` over every `&NAME`/`&!NAME`
term in the body that names a module static, at any depth, recursing into
quotation literals (which is also how `if` arms are reached, `if` being a
library word over two quotations). Mode is `w` if any `&!NAME` occurs, else `r`.

- The walk applies the static filter itself: sigil-strip plus an exact lookup in
  the module's static table. A `capture_names`-style walk without it would
  miscount plain word calls as static accesses, because that traversal
  over-includes word names and relies on a downstream scope intersection that
  does not exist here.
- `shadowed` follows the language's own scoping (a bind's extent is the rest of
  its block; a nested quotation inherits outer binds by value), so a local
  shadowing a static keeps the static out of the set, and a call to a local is
  not read as a word call.
- A ref threaded in as an ordinary parameter contributes **nothing**: only a term
  naming a static directly accrues it.
- Call-graph edges are **intra-module only**. An imported callee contributes
  nothing (its statics are private and unnameable here). An overloaded name
  edges to every same-module candidate: which one a call site picks is
  type-directed, and the union is the safe answer.
- Combinators need no special case: a combinator is inlined at its call site, so
  its quotation body's accesses are already counted in the enclosing word's walk.

**Soundness scope: literal quotations only.** The pass approximates inlining by
traversing quotation literals *textually* at their definition site, so (1) a
static named inside a quotation *value* produced and threaded elsewhere is
invisible to it, and (2) a literal's accesses are attributed to the word that
textually **contains** it, not the word that eventually calls it, so a closure
factory returning `[ &!COUNT incr ]` accrues `COUNT: w`. Both are bounded to
escaping quotations, which DESIGN.md already excludes from the RT subset; the
analysis is sound *on* the non-escaping subset.

### R5 — the fixpoint

A word's **inferred set** is its direct set unioned with every intra-module
callee's inferred set, mode-joined (`r ⊔ w = w`). Worklist form, not recursion:
relax every word until a full pass changes nothing. The lattice (subsets of the
module's statics × `{r,w}`) is finite and the update monotone, so it converges
with no cycle-breaking case and mutual recursion needs no visited guard. A
monotone relaxation propagates at least one call-graph level per pass, so the
run asserts a bound of `words.len() + 2` passes: a regression that stops
converging fails **red** rather than hanging CI. The pass count is returned so a
test can pin the bound.

### R6 — exact match at the boundary

- **Exported word:** the clause is **mandatory whenever the inferred set is
  non-empty**, and must then equal it **exactly**. An exported word touching no
  static needs no clause: the empty set has no spelling, since `Some(vec![])` is
  unrepresentable and a bare `global:` is a parse error. (Every export in the
  existing corpus, and every injected prelude word, is that case.)
  - Clause absent: located error at the word listing what it touches and handing
    back the exact clause text to paste.
  - Clause present but disagreeing: one located-error family, each message naming
    the static and the disagreement, at the entry's span (or the word's, for a
    missing entry):
    - **extra**: a declared entry the inferred set lacks;
    - **missing**: an inferred entry the clause lacks;
    - **wrong mode**: a declared mode differing from the inferred one;
    - **no such static**: a declared entry naming nothing declared in the module.
      Checked *before* the inferred-set comparison and kept distinct from
      **extra**, because a typo is an unresolved name, not a claim about an
      untouched static.
- **Private word:** the clause is optional; if present it is checked for the same
  exact match. Forbidding it would need its own rejection error for no gain.

Match is **exact, not superset**: DESIGN.md's blame-localisation wants a
declaration that ratchets, so an over-declared static is as much an error as an
under-declared one.

`check_static_decls` (in `check/declarations.rs`, same pre-mangle slot) rejects
a repeat declaration and a name a word/extern/type of the same module already
holds.

The REPL rejects a `global:` clause outright: `check_globals` runs only in
`assemble_module`, and a live session declares no statics, so the clause would be
accepted and never checked (the same treatment `export:` gets there).

## Lowering

Minimal scalar-static lowering, so an agreeing program builds and runs (Phase 7
S4's allocator state depends on statics actually storing something):

- `Module.statics` lowers into `IrModule.statics: Vec<StaticData>` (mangled
  symbol, size, `StaticValue::{Int, Str}`), with the elided initialiser resolved
  to its type's zero (`str`'s zero being the empty descriptor).
- The QBE preamble emits `data $SYM = { <class> <v> }` per static, interning a
  `str` static's content alongside the existing string literals.
- `Instr::StaticAddr(Value, symbol)` copies the data symbol's address into a
  `Ptr`, the `FuncAddr` shape, no load of the value, so the borrow stays a place
  the following `@`/`!`/`+!` reads through. It is consumed by the existing
  `push_reference`, which records the referent for dispatch.

The volatile aspect, fixed-address MMIO overlay, and bit-level register layout
stay Phase 9: this is plain compiler-allocated storage only.

## Out of scope

- MMIO (`volatile`, `at <addr>`, bit-level register layout) and ISR
  symbol/section export (Phase 9).
- Cross-module / link-time global-set composition under separate compilation.
- Non-literal, arithmetic, or cross-static initialisers.
- Struct-typed statics and their initialisers (Phase 9).
- Cross-body static exclusivity (R3's recorded gap): needs the R4/R5 set.
- Making `check_static_decls`'s collision rule symmetric across modules.
  `colliding_name_kind` compares against words of the static's *own* module and
  `parser::prelude_words` gives every `lib/core.sth` word module 0, so
  `static: if` errors in the entry file and is silently accepted in every
  imported one. Harmless at codegen now that `mangle_static` is unconditional;
  squaring it belongs with whatever slice gives prelude words real hygiene.

## Files touched

- `src/ast.rs`: `StaticDecl` / `StaticInit` / `GlobalEntry` / `GlobalMode`;
  `Module.statics`; `WordDef.declared_globals`. No `Type` variant.
- `src/parser.rs`: the `static:` dispatch arm and `parse_static_decl`;
  `parse_global_clause` and its peek in `parse_worddef`; `ParsedBodies.statics`.
- `src/driver.rs`: drain `bodies.statics`; invoke `check_static_decls` and
  `check::globals` pre-mangle.
- `src/resolve.rs`: `mangle_static` and the static name category.
- `src/check/word_families.rs`, `src/check/engine.rs`, `src/check/captures.rs`,
  `src/check/poly.rs`, `src/check/word_entry.rs`, `src/check.rs`: the static
  borrow branch, the static table threaded through the checker contexts, the
  `static_root` flag and the back-edge exemption.
- `src/check/declarations.rs`: `check_static_decls`.
- `src/check/globals.rs` (new): direct sets, fixpoint, boundary check.
- `src/repl.rs`: reject `global:` in a session line.
- `src/ir.rs`, `src/ir/layout.rs`, `src/ir/driver.rs`,
  `src/ir/func_builder/*`, `src/backend/qbe.rs`: `StaticData` / `StaticValue` /
  `Instr::StaticAddr`, the data preamble, the `&STATIC` lowering arm.

## Exit

A module with private static state exports a word whose declared global set the
checker verifies **exactly** against the inferred one; a mismatch (missing,
wrong mode, extra, or no-such-static) is a located error naming the static. An
exported word touching a static with no entry for it is a located compile error
naming the static. A static accessed through `&`/`&!` reuses the existing borrow
machinery: it is ref-typed, cannot be stored, and two live `&!` borrows in one
body conflict exactly as for a local aggregate. An agreeing static-using program
builds and runs.

The "no new `Type` variant" constraint is enforced by code review of
`src/ast.rs`: no runtime test can assert a variant's absence.

## Tests

Parser (`src/parser.rs`): `parse_static_scalar_with_initializer_ok`,
`parse_static_decl_span_points_at_the_name`,
`parse_static_zero_elided_initializer_ok`, `parse_static_bool_elided_zero_ok`,
`parse_static_str_elided_zero_is_empty_ok`,
`parse_static_bool_and_str_initializer_ok`, `parse_static_struct_type_is_error`,
`parse_global_clause_records_entries`,
`parse_global_clause_accepts_a_free_standing_comma`,
`parse_global_clause_missing_comma_is_error` (asserts it names the *second*
entry, not a clause that ended early), `parse_global_clause_empty_is_error`,
`parse_global_clause_invalid_mode_is_error`,
`parse_effect_without_global_clause_unchanged` (the additive guard),
`parse_global_clause_on_poly_effect_ok`. The existing
`reserved_reference_name_is_error_at_every_declaration_site` and
`redefining_an_access_word_is_error` gain the `static:` site.

Analysis (`src/check/globals.rs`): `direct_set_counts_named_static_not_ref_parameter`
(exact set, not non-empty), `mode_is_write_if_any_mutable_borrow`,
`fixpoint_unions_callee_sets`, `direct_set_ignores_imported_callee` (the
load-bearing negative a positive-only union test never exercises),
`fixpoint_converges_on_mutual_recursion` (bounded pass count, so a regression
fails red rather than hanging), plus module scoping, clause bodies, mode-join
order and call shadowing. Boundary: `exact_match_missing_entry_is_error`,
`exact_match_wrong_mode_is_error`, `exact_match_extra_entry_is_error`,
`no_such_static_entry_is_distinct_error`,
`private_word_clause_optional_absent_ok`,
`private_word_clause_checked_when_present`. All assert the exact message text,
never `is_err()`.

Borrow (`src/check/word_families.rs`): `borrow_of_scalar_static_is_ref_typed`
with its twin `borrow_of_scalar_local_still_error` (proves the branch, not the
absence of an error); `two_live_mutable_static_borrows_conflict` (mutation
witness for R3's `owned_root`); `local_shadowing_a_static_resolves_to_the_local`;
`storing_a_static_ref_in_a_cell_is_error` (mutation witness for the type-keyed
store rule; a struct-field spelling would be a placebo, since a field typed
`&!i64` is rejected at the type declaration with no static involved).

Goldens (`tests/phase7_slice2.rs`): `exported_word_global_set_mismatch_diagnostic`,
`undeclared_static_access_diagnostic`, `static_ref_escape_diagnostic`,
`duplicate_static_declaration_diagnostic`,
`static_name_collides_with_word_or_type_diagnostic`,
`agreeing_static_program_builds_and_runs`,
`two_modules_declaring_the_same_static_get_distinct_storage`,
`statics_named_like_mangle_exempt_words_get_distinct_storage`,
`a_library_static_named_main_does_not_collide_with_the_entry_symbol`,
`static_ref_named_inside_an_escaping_quotation_no_ice`,
`static_ref_captured_into_an_escaping_closure_env_no_ice`,
`every_scalar_static_type_round_trips_through_its_storage`.

Regression: every effect with no `global:` clause and every program with no
`static:` declaration parses and checks byte-for-byte as before.
