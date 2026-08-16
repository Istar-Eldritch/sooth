# Phase 7 Slice 2: static storage and global sets (spec)

Module-level static storage as a *place* (never owned, moved, or dropped;
constant-initialised; reached only through the existing `&`/`&!` sigil), plus
the per-word **global set** that keeps it honest: which statics a word touches
and in what mode, inferred over the intra-module call graph and *declared* on
exported words, checked for exact match. This is the plain, non-embedded half
of DESIGN.md's *Embedded* section: no MMIO overlay, no `volatile`, no fixed
address, no ISR (all Phase 9, where their target-facing consumer lives).

Recon in [`slice2-brief.md`](./slice2-brief.md) holds as written; the source
line references below were re-read against `main` at `5ee2796` (`cargo test`
green). Where this spec extends the brief's file list, it says so explicitly
(see *Scope note: lowering* and *Files touched*).

## Codebase map (verified)

- **Top-level dispatch** is one `while` in `parse_bodies`
  (`src/parser.rs:349-368`): peek, branch on `type:`/`extern:`/`import:`/
  `export:`, else `parse_worddef`. A `static:` arm slots in as a fifth branch,
  symmetric with `extern:` (`parse_extern_decl`, `src/parser.rs:1482`).
- **`ParsedBodies`** (`src/parser.rs:298`) collects `words`/`externs`/field
  bodies/`exports`; the driver (`assemble_module`, `src/driver.rs:170`) drains
  each into the `Module` (`src/ast.rs:18`), then runs
  `check_exported_signatures` **pre-mangle** (`src/driver.rs:326`,
  `src/check/declarations.rs:242`), then `resolve_modules`
  (`src/resolve.rs:338`), then (in the caller) `check::check`
  (`src/driver.rs:373`, `src/check.rs:440`) **post-mangle**.
- **The `&`/`&!` sigil already means "second-class ref to a named place,"**
  today scoped to bound locals. The borrow is *typed* at one site,
  `src/check/word_families.rs:180-219` (`lower_reference_word`'s checker twin,
  the final arm of the borrow-word family): it looks `rest` up as a local, and
  on success emits `intern_ref_type(refs, local_ty, mutable)` plus
  `prov.borrow(rest, mutable, span)`. The three rejection paths beside it are
  `borrow_of_non_place_error` (`:1064`), `borrow_of_scalar_local_error`
  (`:1087`, "a scalar has no address"), and `borrow_of_reference_local_error`
  (`:1098`). Three other sites strip the sigil to recover the bare name:
  `ast::rename_call` (`src/ast.rs:1620`, inlining a captured local),
  `resolve::strip_ref_sigil` (`src/resolve.rs:154`, rewriting an
  import-qualified call), `check::engine::call_local` (`src/check/engine.rs:541`).
  None has a "not a local, but a declared static" case today.
- **No call-graph / fixpoint infrastructure exists.** The nearest relative is
  the colour-DFS cycle finder in `check/drop_graph.rs`
  (`check_tail_call_cycles`, `:378`, `find_tail_cycle`) and
  `check/combinators.rs`'s `check_combinator_cycles` (`:220`); both build a
  name-keyed adjacency over `words` and DFS it. The global-set fixpoint follows
  that adjacency-building shape but computes a monotone set fixpoint, not a
  cycle verdict.
- **`export:` is a separate per-module list** (`ParsedBodies.exports`,
  `ModuleInfo.exports`), raw names + spans, cross-referenced against the word
  table by `check_exported_signatures` **pre-mangle**. That pass is the natural
  home for the global-set boundary check (both need raw names and the raw
  export list).
- **Data emission precedent:** string literals lower through
  `Instr::StrLit` and emit `data $strb{idx}` / `data $strd{idx}` in the QBE
  preamble (`src/backend/qbe.rs:650-700`); a borrow lowers to a pointer via
  `push_reference` (`src/ir/func_builder/word_families.rs:9`, called from
  `lower_reference_word` at `:26-70`). A static's
  data symbol and address load follow these two shapes directly.

## D1 — declaration grammar

A fifth arm of `parse_bodies`'s dispatch loop, symmetric with `extern:`:

```text
static-decl := "static:" NAME Type ( "=" literal )? ";"
literal     := int-lit | bool-lit | str-lit
```

- One static per declaration (no batch form), matching every other top-level
  form.
- `= literal` is a **single literal only** (D3): an integer, bool, or
  string literal. No arithmetic, no reference to another static, no
  struct-literal aggregate. Recon confirms no const-expression or
  struct-literal-initialiser machinery exists to reuse; a `Term`-level
  expression here would be new surface nobody asked for.
- The initialiser may be **elided**, meaning the type's zero value
  (`static: COUNT i64 ;` is `0`; `bool` is `false`, and `str`'s zero value is
  the **empty string** `""`). This is DESIGN.md's
  `Preelaborate` tier (constants/zero only), which falls straight out of "no
  comptime interpreter".
- **Scalar types only this slice** (`i64`, `u32`, `bool`, and `str` where its
  literal already exists): see OQ1. A struct-typed static is rejected at the
  declaration with a located error naming the type, deferred to Phase 9. The
  rejection is **allow-list-based, not struct-detection-based**: the parser is
  single-pass and has no type table at declaration-parse time (a `type:` may be
  declared *after* a `static:` in the same file), so it accepts only the fixed
  scalar keyword set (`i64`/`u32`/`bool`/`str`) and rejects everything else with
  the same "not a scalar" error. A mistyped or forward-referenced user type is
  therefore indistinguishable from a genuine struct type at this layer, and both
  get that one error — the correct behaviour, but worth stating precisely.
- The `NAME` obeys the same reserved-name / access-word rejections
  `parse_worddef` / `parse_extern_decl` already apply
  (`reject_reserved_name`, `ACCESS_WORDS`).

## D2 — the global clause, its own trailing keyword clause

The clause is **not** nested inside the stack-effect parens (settled after
review: an in-parens placement, leading or trailing, reads as if it were part
of the stack shape — a value pushed or popped — when it is neither). It is its
own clause, headed by its own colon-suffixed keyword, sitting right after the
effect's closing `)` and before the body — the same slot family as the
existing `inline` keyword, not a third line (the whole declaration stays one
line):

```text
worddef      := ":" NAME "inline"? "(" effect ")" global-clause? body ";"
effect       := slot* "--" slot*
global-clause:= "global:" entry ( "," entry )*
entry        := NAME mode
mode         := "r" | "w"
```

```
: tick ( -- i64 ) global: COUNT w, LIMIT r ;
  ...body... ;
```

- `parse_effect` (`src/parser.rs:1565`) and `parse_poly_effect` (`:1614`)
  are **unchanged** — no new stop-token, no grammar change inside the effect
  reader at all. Instead `parse_worddef` (`:1317`) learns a new optional peek
  for `global:` immediately after consuming the effect's closing `)`, mirroring
  exactly how it already peeks for `declares_inline` before the `(`
  (`:1328-1334`). An effect with no `global:` parses byte-for-byte as today,
  and every existing call site that builds an effect is untouched.
- The clause is stored on `WordDef` as `declared_globals: Option<Vec<GlobalEntry>>`
  (D4). `None` = no clause written; `Some(vec![])` is **not** representable
  (a bare `global:` with no entry is a located parse error).
- `mode` is `r` or `w`. It is **declared**, but the checker still verifies it
  against the inferred mode (decision 5: mode is derived, never independently
  authored): a declared `r` on a static the body writes is a wrong-mode error.
- The comma **is** load-bearing here, unlike everywhere else in the grammar:
  entries are whitespace-separated tokens with no other delimiter between them,
  so without a separator `COUNT w LIMIT r` cannot be told apart from a clause
  that silently ended after `COUNT w` with `LIMIT` and `r` left as body terms.
  The lexer has no comma token (this is the first comma anywhere in Sooth), so
  a trailing comma lexes glued to the preceding word (`w,`); the parser strips
  it with `strip_suffix(',')` and separately accepts a free-standing `,` for
  the case where whitespace intervenes. A missing comma **is** a located parse
  error: leaving the clause to end early would report the dropped entry as an
  unknown word in the body, a silent truncation of the exact kind this language
  exists to eliminate, so on ending the clause the parser looks ahead one entry
  and rejects a following NAME + `r`/`w` pair. `global:` itself still needs no
  punctuation to mark its own start or end — that boundary is the keyword, the same way `--` already
  separates inputs from outputs with no extra token either side of it.
- **Explicitly rejected:** a general "compiler annotation" mechanism with
  `global:` as its first instance. One consumer with a checked, structured
  payload (a NAME-mode list the checker verifies exactly, not an inert tag)
  doesn't justify a generic marker syntax; `inline` already demonstrates this
  project's actual pattern for a word-level marker (a narrow, bespoke keyword,
  not a general mechanism). A plausible second consumer (Phase 9's ISR
  symbol/section export) has a different enough shape — a name/string pair, not
  a checked NAME-mode list — that generalizing now wouldn't even validate the
  abstraction.

## D4 — AST

`src/ast.rs` gains a static-declaration node and its registry, plus one field
on `WordDef`. **No new `Type` variant** (decision 3: a static's ref is exactly
`&T`/`&!T` for the static's declared `T`, via `intern_ref_type`).

```rust
pub struct StaticDecl {
    pub name: String,
    pub ty: Type,
    pub init: StaticInit,
    pub module: u32,
    pub span: Span,
}

pub enum StaticInit {
    Zero,
    Int(i64),
    Bool(bool),
    Str(String),
}

pub struct GlobalEntry {
    pub name: String,
    pub mode: GlobalMode, // R | W
    pub span: Span,       // the entry's NAME token, for the exact-match diagnostic
}
```

- `Module` gains `statics: Vec<StaticDecl>`; `ParsedBodies` gains
  `statics: Vec<StaticDecl>`; `assemble_module` drains one into the other
  (`words.extend`/`externs.extend` twin) before `check_exported_signatures`.
- `WordDef` gains `declared_globals: Option<Vec<GlobalEntry>>`. The arity change
  on `WordDef`'s construction is confined to `parse_worddef` and any test
  constructor; it is **not** the tree-wide pattern-padding P6 Slice 1 needed,
  because `WordDef` is a named-field struct (`..Default`-free construction sites
  set the new field to `None`), whereas `TermKind::Quotation` was a tuple
  variant. Re-grep `WordDef {` construction sites at implementation time and set
  `declared_globals: None` at each.

## Resolution — the third case (recon 3, decision 3)

A bare name after sigil-stripping resolves, in order: (a) a bound local
(today's only case, unchanged), (b) a static declared in the **accessing
module**, (c) whatever an unresolved name means today (word call or error).

### R1 — borrow-typing extends with a static branch

At `src/check/word_families.rs:180-219`, when `rest` is not a bound local, look
it up in the accessing module's static table before falling to the existing
rejection paths:

- If `rest` names a module static of type `T`, push
  `intern_ref_type(refs, T, mutable)` (exactly as a local borrow does) with a
  **static-rooted** provenance (R3).
- A **scalar** static *is* borrowable, unlike a scalar local
  (`borrow_of_scalar_local_error`, `:1087`): a static has a data-symbol address,
  a scalar local does not. So the scalar-local rejection is reached only when
  `rest` is neither a local nor a static.
- `&STATIC` / `&!STATIC` both resolve the same static; `mutable` flows from the
  sigil into `intern_ref_type` and into the inferred mode (R5).

### R2 — statics are module-private and module-mangled

A static is never exported or imported (there is no `export:`-of-a-static this
slice; a static's *ref* never crosses a module boundary, only the global
*clause* on an exported word does). Two modules may each declare `COUNT`, so at
codegen each static's data symbol is **module-scoped mangled**, exactly as
words are:

- `resolve::resolve_modules` (`src/resolve.rs:338`) learns `statics` as a name
  category: it mangles each `StaticDecl.name` per module and rewrites every
  `&NAME`/`&!NAME` reference whose core name is a module static (extending
  `strip_ref_sigil`'s fallthrough at `src/resolve.rs:154`, recon 3). A core name
  that is neither a local, a module type/word, nor a module static is left
  untouched, exactly as today.
- `ast::rename_call` (`src/ast.rs:1620`, inlining) needs **no change**: a static
  ref is not a bound local, so it already falls through unchanged. Confirm this
  with a note in the diff, do not edit it.

### R3 — a static-rooted borrow keeps exclusivity, skips only the disposal scans

A static is never owned, moved, or dropped, but two live `&!` borrows to the
same static are exactly as illegal as two live `&!` borrows to the same local
aggregate. The borrow checker's exclusivity/aliasing scans are
**`owned_root`-keyed, not type-keyed**: `check_reference_word`'s `live_deriv`
predicate and the conflict scans in `src/check/engine.rs`
(`live_mutable_borrow_of` `:972` and its immutable twin `:957`) both test
`d.owned_root.as_deref() == Some(place)`. A `Deriv` with `owned_root: None`
matches none of them, so an earlier design that reported **no** `owned_root` for
a static (modelled on a reference *parameter*'s reborrow,
`src/check/engine.rs:56`) would silently disable mutable-aliasing detection for
permanent shared state — the case that needs it most. That is wrong.

So a static-rooted `Deriv` sets **`owned_root: Some(<static name>)`** (and
`place` to the same static name), exactly as an owned local does. The
exclusivity/conflict scans then fire for statics verbatim: two simultaneously
live `&!COUNT`, or a live `&!COUNT` beside a `&COUNT`, is the existing
`conflicting_borrow_error` / `aliased_place_borrow_error`, unchanged.

What *is* skipped is narrower, and keyed on the fact that a static is never a
linear owned value: only the **disposal / consume / leak scans** are exempted
for a static root.

- No "forgotten disposal" surplus-value error can fire for a static ref (it is
  a `&T`/`&!T`, already non-linear), and no move-state / consume error can fire
  for the static itself (it is never moved or dropped).
- The existing "a reference cannot be stored" rule (`stored_reference_error`,
  `src/check/declarations.rs:590`; `check_no_stored_references` at `:531`)
  applies **unchanged** — a ref rooted in a static may not be put in a struct
  field, array, cell, or another static, because that rule *is* type-keyed
  (`contains_reference`), independent of what the ref points at.

That disposal/consume exemption is **vacuous in code, by design**: no branch
enacts it. A static's borrow is a `&T`/`&!T`, already non-linear, and the static
itself never reaches the stack, so the surplus/disposal and move-state scans
have nothing to reach for in the first place. The exemption states what cannot
fire, it is not a carve-out to implement — a later reader who finds no static
case in those scans should not "restore" one.

One `owned_root`-keyed scan is neither exclusivity nor disposal, and the
exemption list above needs its own carve-out for it: `check_reference_across_back_edge`
(`src/check.rs`) rejects any reference whose `owned_root` is set when it
crosses a self-tail-call back-edge, because a *local*'s storage does not
survive to the next iteration. A static's data-segment storage does, so this
scan skips a borrow *recorded* as static-rooted — a `static_root` flag set on
the `Deriv` at the borrow site, never re-derived by looking the root name up in
the static table, because locals are not mangled and statics are: a local
spelled `COUNT__m0` in a module declaring `static: COUNT` answers that lookup
and would inherit the exemption. A freshly borrowed `&!COUNT`
passed to a self-tail call
(`: spin ( &!i64 i64 -- ) | c n | c 1 +! n 0 > ~[ &!COUNT n 1 - spin ] ~[ ] if ;`)
is accepted, while the same call passing a reference rooted in an ordinary
local is rejected exactly as before, with the message naming that local "a
local of this frame".

So the mechanism is reused wholesale (a `Deriv` with a real `owned_root`, the
same borrow-typing arm, the same type-keyed store rule); the only carve-out is
that the disposal/consume scans treat a static root as nothing to dispose. The
spec must **not** claim "every borrow rule is type-keyed, applies verbatim": the
exclusivity rules are `owned_root`-keyed, and statics get a real `owned_root`
precisely so those rules keep working.

## Global-set analysis (recon 4, decision 5/6, OQ3)

A new pass, `check::globals` (its own module under `src/check/`), invoked from
`check_exported_signatures`'s caller in `assemble_module` **pre-mangle** (raw
static names, raw word names, raw export list all agree there). It has two
stages.

### R4 — direct sets and the intra-module call graph

For each word, its **direct set** is the map `static-name -> mode` over every
`&NAME`/`&!NAME` term in the body that names a module static, at any depth,
recursing into `if` arms and nested quotation literals (reusing
`capture_names`-style traversal, `src/check/engine.rs:557`). Mode is `w` if any
`&!NAME` occurs, else `r` (decision 5; `&!` subsumes `&`).

The reused traversal shape **must filter** to `&`/`&!`-sigilled names that
resolve to a module static. `capture_names` itself over-includes ordinary word
names — its own doc comment says there is "no way to tell a local reference from
a word call at this syntactic layer," which is harmless there because
`capture_alive_names` only intersects the set against actual scope bindings.
Here there is no such downstream intersection, so the direct-set walk has to
apply the static filter itself, or it will miscount plain word calls as static
accesses. This filter is required, not incidental.

- A ref threaded in as an ordinary parameter (`uart-init ( &!Uart -- )`)
  contributes **nothing** (decision 6): only a term *naming a static directly*
  accrues it. The traversal counts `&NAME` where `NAME` is a static, never a
  `&`-typed parameter slot.
- The call graph edges are `word -> each word it calls`, **intra-module only**.
  A call to an imported word contributes nothing to the caller's set this slice
  (the callee's statics are private to another module and unnameable here); this
  is DESIGN.md's "inferred *within* a module," and the cross-module composition
  is the separate-compilation question the brief lists as out of scope. Builtin
  and combinator calls: a combinator is inlined at its call site, so its
  quotation body's direct accesses are already counted in the enclosing word's
  own traversal (recon 4's "direct-access-or-calls-a-word-that-does" closes
  under inlining exactly as the rest of DESIGN.md's argument does).

### R4/R5 soundness scope: literal quotations only

The "combinator inlining keeps it monomorphic" argument (DESIGN.md) holds for
this pass only over **non-escaping, literal** quotations. `check::globals` runs
**pre-mangle in the checker**, not at IR-lowering inlining time
(`src/ir/calls.rs`), and it approximates inlining by traversing quotation
literals **textually** at their definition site. Two consequences the pass does
not fully close, both bounded but worth stating rather than asserting away:

1. The traversal reproduces the inlined result only for a quotation that appears
   as a literal at the call site. A static named inside a quotation *value* that
   is produced and threaded elsewhere (an escaping closure) is not a literal the
   traversal can see through. DESIGN.md already excludes escaping quotations
   from the RT subset, so the gap is bounded to code outside that subset — but
   the analysis is sound only *on* the non-escaping subset, and should say so.
2. The traversal attributes a literal's static access to the word that
   textually **contains** the literal, not the word that eventually calls it. A
   word that merely builds and returns `[ &!COUNT incr ]` (returning the
   closure rather than calling it) accrues `COUNT: w` though it never itself
   touches `COUNT`. Combined with R6's **exact-match** rule, an exported
   closure-factory can be forced to declare a static it never directly accesses,
   or error. Again bounded to escaping quotations, but a real behavioural corner
   where exact-match and the textual attribution collide; do not claim full
   soundness here without this qualification.

### R5 — the fixpoint (OQ3 resolved)

A word's **inferred set** is its direct set unioned with every intra-module
callee's inferred set, mode-joined (`r ⊔ w = w`). The lattice is finite (subsets
of the module's statics × `{r,w}`) and the update is monotone, so a worklist
iteration to a fixed point converges; mutual recursion is handled by iterating
to convergence rather than recursing, with an on-stack/visited guard as in
`find_tail_cycle` (`src/check/drop_graph.rs`) only needed if a plain recursive
formulation is chosen. Spec the **worklist** form (relax every word's set until
no set changes in a full pass): it needs no cycle-breaking special case and is
the smaller thing to prove terminating. A direct `a -> b -> a` cycle converges
because both sets grow monotonically to the union of the two direct sets and
then stop.

### R6 — exact-match at the boundary (recon 6, decision 7)

For each word, using the raw export list of its owning module:

- **Exported word:** the `global:` clause is **mandatory whenever the inferred
  set is non-empty**, and must then equal it **exactly** (same names, each with
  the exact inferred mode). An exported word that touches no static needs no
  clause: the empty set has no spelling, since `Some(vec![])` is unrepresentable
  by D4 and a bare `global:` is a parse error. (Every export in the existing
  corpus, and every injected prelude word, is that case.)
  - Clause absent -> located error at the word: *"exported word `W` must
    declare its global set (line L): it touches NAME (mode), ..."*, which hands
    back the exact clause text to write.
  - Clause present but disagreeing -> a **single located-error family**
    (`global_set_mismatch_error`) at the offending entry's span (or the word
    span for a missing entry), covering these disagreements, each naming the
    static and the disagreement:
    - a declared entry the inferred set lacks (**extra**);
    - an inferred entry the clause lacks (**missing**);
    - a declared mode that differs from the inferred (**wrong mode**);
    - a declared entry naming a static that **does not exist in the module**
      (**no such static**) — a *distinct* case with its own message, not folded
      into **extra**. An **extra** entry names a real module static the word
      does not touch; a **no such static** entry is a typo or dangling name that
      resolves to nothing, and lumping it under "extra" would surface a
      confusing "you declared a static you don't touch" for what is really an
      unresolved name. This case is checked against the module's static table
      before the inferred-set comparison.
- **Private word:** the clause is **optional** (OQ2); if present it is checked
  for the same exact match (cheap uniform consistency, no special-cased
  "written but ignored" state); if absent, never an error. Forbidding it on a
  private word would need its own rejection error, strictly more surface than
  checking it uniformly.

Match is **exact, not superset**: DESIGN.md's blame-localisation argument wants
a declaration that ratchets, so an over-declared static (claimed but never
touched) is as much an error as an under-declared one.

## OQ resolutions (summary)

- **OQ1 -> D1/D3:** scalar statics only (`i64`/`u32`/`bool`/`str`); a
  struct-typed static is a located declaration error, deferred to Phase 9. No
  struct-literal-initialiser machinery exists to reuse, and a struct static with
  no fixed address and no MMIO consumer has no motivating client yet.
- **OQ2 -> R6:** the clause is allowed on a private word and checked-if-present;
  forbidding it costs a bespoke rejection error for no gain.
- **OQ3 -> R5:** a monotone finite-lattice worklist fixpoint over the
  intra-module call graph; mutual recursion converges by iteration, no SCC
  machinery required.

## Scope note: lowering (deviation from the brief, flagged)

The brief's "Sequencing" lists touched files as `parser`/`ast`/`resolve`/
`check` only, and the phase's own S2 exit is stated in **checker** terms (a
declared global set the checker verifies; an undeclared access is a compile
error). Taken literally, the exit witnesses are all *diagnostic* goldens
(source in -> error out) plus checker unit tests, which need **no** `src/ir` /
`src/backend` change: a program that fails the boundary check never reaches
lowering.

But an *agreeing* program (correct global set, valid static access) checks and
then has nothing to lower against: a static with no emitted storage and no
address for `&STATIC` is a declarable-but-unbuildable feature, i.e. the
half-finished state. Phase 7 S4 (allocator rework) also *depends on* S2 for
"the allocator's own state," which is only real once a static actually stores
something at runtime.

I therefore include **minimal scalar-static lowering** as Phase 4 below
(a `data $NAME` symbol per static in the QBE preamble, mirroring the string
data at `src/backend/qbe.rs:650-700`; a new `Instr::StaticAddr(Value, symbol)`
pushing the static's address as a `Ptr`, consumed by the existing
`push_reference`, `src/ir/func_builder/word_families.rs`). This widens the file
list into `src/ir` and `src/backend`, which the brief did not list.

**This is a flagged judgment call, not a settled one.** You asked (via the
brief) for a checker/parser slice; I think the slice is half-built without
storage and that S4 forces it anyway, so I have folded minimal lowering in. If
you would rather ship S2 as checker-only and land lowering with S4 (or as its
own slice), drop Phase 4 and scope the exit witnesses to diagnostic goldens
plus a check-passes unit test for the agreeing case. Tell me which.

**Dropping Phase 4 stopped being free once Phase 2 landed.** Every
static-borrowing program now type-checks and reaches lowering, where
`lower_reference_word` still asserts its operand is a bound local
(`src/ir/func_builder/word_families.rs:66`): `&!COUNT 1 +!` panics with
`checked: a borrow's operand is a local`, where before this slice it produced a
located error naming the static. Every shape reaches it — `i64`/`bool`/`str`
statics, a borrow inside an inline quotation, a self-tail loop, a closure
factory. So the choice is now between keeping Phase 4 and having whoever drops
it add a located "static lowering not implemented" rejection in its place; there
is no option that leaves the compiler panic-free without one of the two. Not
patched in Phase 2 on purpose: the guard belongs to whichever phase settles
this, not to the phase that exposed it.

The volatile aspect, fixed-address MMIO overlay, and bit-level register layout
stay Phase 9 regardless: this is plain compiler-allocated storage only.

## Out of scope

- MMIO: `volatile`, `at <addr>` fixed-address overlay, bit-level register
  layout (Phase 9).
- ISR symbol/section export and the ISR/mainline shared-state wrapper (Phase 9).
- Cross-module / link-time global-set composition under separate compilation
  (brief: moot here, no ISR; R4 is intra-module).
- Any non-literal, arithmetic, or cross-static initialiser (D3).
- Struct-typed (aggregate) statics and their initialisers (OQ1; Phase 9).
- `Copy`-marker interaction beyond "a static is its own carve-out": DESIGN.md
  settles this, nothing new here.

## Files touched

- `src/ast.rs`: `StaticDecl` / `StaticInit` / `GlobalEntry` / `GlobalMode`; the
  `Module.statics` field; the `WordDef.declared_globals` field. **No `Type`
  variant** (D4). Note (no edit) that `rename_call` needs no change (R2).
- `src/parser.rs`: the `static:` dispatch arm + `parse_static_decl` (D1); the
  `global:` clause reader in `parse_worddef`, right after the effect's closing
  `)`, mirroring the existing `declares_inline` peek (D2 — `parse_effect`/
  `parse_poly_effect` themselves are unchanged); the `ParsedBodies.statics`
  field; parser unit tests.
- `src/driver.rs`: drain `bodies.statics` into `Module.statics`; invoke the new
  `check::globals` pass pre-mangle beside `check_exported_signatures`.
- `src/resolve.rs`: teach `resolve_modules` the static name category — mangle
  each `StaticDecl.name` and rewrite `&NAME`/`&!NAME` static references
  (`strip_ref_sigil` fallthrough, R2).
- `src/check/word_families.rs`: the static branch in the borrow-typing arm
  (R1); a scalar static is borrowable.
- `src/check/declarations.rs`: the `static:` declaration name rules — a repeat
  declaration, and a name a word/extern/type of the same module already holds,
  both located errors (`check_static_decls`). They live beside
  `check_exported_signatures` rather than in `globals.rs`: same pre-mangle slot,
  same declaration-check responsibility, and no part of the global-set analysis.
- `src/check/globals.rs` (new): direct-set traversal (R4), the worklist fixpoint
  (R5), and the boundary exact-match check + diagnostics (R6). New module under
  `src/check/` because it is a self-contained analysis with its own imports and
  no call into the per-word borrow walk — the growth-structure "module doing one
  thing, import divergence" signal points to its own file rather than growing
  `check.rs`.
- `src/check/engine.rs` / `src/check/declarations.rs`: static-rooted provenance
  carries a real `owned_root` (the static name) so the exclusivity/conflict
  scans keep firing, and skips **only** the disposal/consume scans (R3); confirm
  `stored_reference_error` applies unchanged.
- **(Phase 4, flagged)** `src/ir/*` and `src/backend/qbe.rs`: `Module.statics`
  into the IR module; `Instr::StaticAddr`; the `data $NAME` preamble emission;
  the `&STATIC` lowering arm in `lower_reference_word`.

## Exit

A module with private static state exports a word whose declared global set the
checker verifies **exactly** against the inferred one; a mismatch (missing
entry, wrong mode, an extra entry the inferred set does not contain, or a
declared entry naming a static that doesn't exist in the module) is a
located error naming the static (R6). An undeclared static access inside a
module (an exported word touching a static with no `global:` entry for it) is a
located compile error naming the static (R6). A static accessed only through
`&`/`&!` reuses the existing borrow machinery: it is ref-typed, cannot escape or
be stored, and two live `&!` borrows to it conflict exactly as for a local
aggregate (the exclusivity scans are `owned_root`-keyed and a static carries a
real `owned_root`); the only carve-out is that the disposal/consume scans treat
a static root as nothing to dispose (R1/R3). With Phase 4 (if retained), an
agreeing static-using program builds and runs.

The "no new `Type` variant" constraint (D4) is enforced by **code review of
`src/ast.rs`**: no runtime test can assert a variant's absence.

## Tests (goldens + unit)

Parser (`src/parser.rs` `#[cfg(test)]`):

- `parse_static_scalar_with_initializer_ok` — `static: LIMIT i64 = 10 ;`
  parses to a `StaticDecl` with `StaticInit::Int(10)`.
- `parse_static_decl_span_points_at_the_name` — `StaticDecl.span` is the
  name's, not the `static:` keyword's, matching `WordDef.span` so Phase 3's
  duplicate-declaration error points at the name it names.
- The existing `reserved_reference_name_is_error_at_every_declaration_site` and
  `redefining_an_access_word_is_error` gain the `static:` site, covering the
  two `NAME` rejections this declaration inherits (`reject_reserved_name`,
  `ACCESS_WORDS`).
- `parse_static_zero_elided_initializer_ok` — `static: COUNT i64 ;` parses with
  `StaticInit::Zero`.
- `parse_static_bool_elided_zero_ok` — `static: FLAG bool ;` parses with
  `StaticInit::Zero`; a build/lowering of the elided value is `false`
  (D1/D3 zero value).
- `parse_static_str_elided_zero_is_empty_ok` — `static: NAME str ;` parses with
  `StaticInit::Zero`, whose `str` zero value is the empty string `""` (D1).
- `parse_static_bool_and_str_initializer_ok` — `static: FLAG bool = true ;` and
  `static: TAG str = "x" ;` parse with `StaticInit::Bool(true)` /
  `StaticInit::Str("x")`.
- `parse_static_struct_type_is_error` — `static: U Uart ;` is a located error
  naming **both** the static and the type (the offending name is the static's;
  the type is what disqualified it). The rejection is **allow-list-based**: the
  parser accepts only the fixed scalar keyword set (`i64`/`u32`/`bool`/`str`)
  with no type table at parse time, so a genuine struct type and a
  mistyped/forward-referenced user type produce the *same* "non-scalar type"
  error (D1; OQ1 deferral surfaces at the declaration).
- `parse_global_clause_records_entries` — `( -- i64 ) global: COUNT w, LIMIT r`
  parses to two `GlobalEntry` with the right modes.
- `parse_global_clause_accepts_a_free_standing_comma` — the same clause written
  `COUNT w , LIMIT r` (separator spaced off its mode token) yields the same
  entries as the glued `w,` form.
- `parse_global_clause_missing_comma_is_error` — `global: COUNT w LIMIT r` is a
  located parse error naming `LIMIT`, not a clause that silently ends after the
  first entry.
- `parse_global_clause_empty_is_error` — a bare `global:` with no entry is a
  located parse error.
- `parse_effect_without_global_clause_unchanged` — `( i64 -- i64 )` parses with
  `declared_globals: None` (additive guard, the regression guarantee).
- `parse_global_clause_on_poly_effect_ok` — the clause reads the same after a
  variable-bearing effect (`parse_poly_effect` path).

Checker unit (`src/check/globals.rs` `#[cfg(test)]`, constructing `WordDef`s +
`StaticDecl`s directly so the analysis is exercised without a full build):

- `direct_set_counts_named_static_not_ref_parameter` — a body writing `&!COUNT`
  accrues `COUNT: w`; a body receiving a `&!` parameter and not naming a static
  accrues nothing (decision 6). Assert the exact set, not just non-empty.
- `mode_is_write_if_any_mutable_borrow` — `&COUNT` then `&!COUNT` yields
  `COUNT: w` (mode join).
- `fixpoint_unions_callee_sets` — `a` calls `b`, `b` writes `COUNT`; `a`'s
  inferred set contains `COUNT: w` though `a` names no static directly.
- `direct_set_ignores_imported_callee` — module A's word calls an **imported**
  word from module B that touches B's static; A's inferred set gains **nothing**
  from B (R4 is intra-module only). Assert A's set is exactly its own direct
  accesses — the load-bearing negative that a positive-only
  `fixpoint_unions_callee_sets` never exercises.
- `fixpoint_converges_on_mutual_recursion` — `a` calls `b`, `b` calls `a`, each
  names one static; both inferred sets equal the union of the two direct sets.
  Assert convergence within a **bounded** number of worklist passes (a 2-word
  cycle converges in ≤ 3 passes; assert the pass counter never exceeds a small
  fixed `N`) so a regression to unguarded recursion surfaces as a bounded-loop
  failure/panic, **not an infinite hang** that stalls CI (the OQ3 guard's
  witness must fail *red*, never wedge the suite).
- `exact_match_missing_entry_is_error` — an exported word touching `COUNT` with
  no clause entry for it: assert the **exact** missing-entry message naming
  `COUNT`, not `is_err()`.
- `exact_match_wrong_mode_is_error` — declared `COUNT r`, body writes it: assert
  the exact wrong-mode message.
- `exact_match_extra_entry_is_error` — declared `COUNT w`, body never touches
  `COUNT`: assert the exact extra-entry message (exact-not-superset). `COUNT`
  here **is** a real module static the word simply does not touch.
- `no_such_static_entry_is_distinct_error` — a `global:` entry naming
  `NOPE`, which is **not** declared in the module at all: assert the dedicated
  **no such static** message (an unresolved-name diagnostic), *not* the
  extra-entry message. This proves the fourth R6 case is its own branch and a
  typo'd static name is not misreported as "declared but untouched."
- `private_word_clause_optional_absent_ok` — a private word touching `COUNT`
  with no clause checks.
- `private_word_clause_checked_when_present` — a private word with a *wrong*
  clause is the same exact-match error (decision 7).

Checker/borrow unit (`src/check/word_families.rs` `#[cfg(test)]`):

- `borrow_of_scalar_static_is_ref_typed` — `&!LIMIT` on `static: LIMIT i64`
  types as `&!i64` (a scalar static is borrowable though a scalar local is not).
- `borrow_of_scalar_local_still_error` — the twin: a scalar **local** still hits
  `borrow_of_scalar_local_error`. This proves the static *branch*, not merely
  the absence of an error — a bug that made everything borrowable would pass the
  test above but fail this one.
- `two_live_mutable_static_borrows_conflict` — two simultaneously-live `&!COUNT`
  (or a live `&!COUNT` beside a live `&COUNT`) to the same static in one body is
  the existing `conflicting_borrow_error` / `aliased_place_borrow_error`,
  unchanged. This is the **mutation witness** for R3's exclusivity: it must fail
  if a static-rooted `Deriv` is given `owned_root: None`, which would silently
  disable the `owned_root`-keyed conflict scan for permanent shared state.
- `local_shadowing_a_static_resolves_to_the_local` — a bound local named `COUNT`
  in a module that also declares `static: COUNT`: `&COUNT` resolves to the
  **local** (R1's resolution order: local, then static, then word/builtin).
  Proves the ordering, not just asserts it.
- `storing_a_static_ref_in_a_cell_is_error` — `&!COUNT ^` is the existing
  `stored_reference_error`, unchanged (R3): the *store* rule is type-keyed
  (`contains_reference`). This is a **mutation witness** for R3 — it must fail
  if the static branch accidentally routes around `check_no_stored_references`.
  A *struct-field* spelling of the same test would be a placebo: a field typed
  `&!i64` is rejected at the type declaration itself, in a program with no
  static anywhere, so it never routes a static-rooted ref through the rule.

Goldens (`tests/phase7_slice2.rs`, source in -> diagnostic / build out — the
phase exit criteria):

- `exported_word_global_set_mismatch_diagnostic` — a two-word module whose
  exported word declares a global set disagreeing with what it (transitively)
  touches; asserts the located exact-match error naming the static (Exit case 1).
- `undeclared_static_access_diagnostic` — an exported word touching a static
  with no `global:` entry for it; asserts the located "must declare" error
  naming the static (Exit case 2).
- `static_ref_escape_diagnostic` — a program trying to store a static ref;
  asserts the unchanged `stored_reference_error` (Exit: reuses the type-keyed
  store rule).
- `static_ref_captured_into_escaping_closure_no_ice` — a static-rooted `&!COUNT`
  captured into a quotation that then **materializes** (`(code, env)` value) and
  escapes the word: asserts it hits the **existing** capture/escape rejection
  (`contains_reference` / `stored_reference_error` path) with a *located* error.
  **High-risk golden:** this codebase has live, unguarded materialized-quotation
  ICEs and row-combinator quotation crashes, so "a static ref behaves like any
  other ref" is most likely to be false or to crash exactly here. This golden's
  job is to prove **no new ICE** — a located diagnostic, not a backend panic —
  not merely that *some* error fires.
  **Resolved after Phase 2 (measured, not argued): assert *acceptance*.**
  `ref_root_is_in_frame` (`src/check/captures.rs`) looks a borrow's
  `owned_root` up in the *local* scope, finds no static there, and classifies
  it `OuterRooted`; a closure factory
  (`: mk ( -- [ i64 -- i64 ] ) [ &!COUNT @ + ] ;`) therefore checks clean with
  no checker ICE. That is the semantically right answer — data-segment storage
  outlives every frame, so the local-rooted hazard this rule guards does not
  exist for a static — so the golden asserts the program is admitted, and the
  classification needs **no** static carve-out. What it must still pin down is
  that admission is not a *panic* downstream: today the same program dies in
  lowering (see the Phase 4 note above), so this golden only becomes
  meaningful once that is settled.
- `duplicate_static_declaration_diagnostic` — two `static: COUNT ...` in one
  module is a located error at the second declaration.
- `static_name_collides_with_word_or_type_diagnostic` — a `static: COUNT` whose
  name already names a word or type in the same module is a located error
  (same name-category collision the prepass already enforces for words/types).
- **(Phase 4, if retained)** `agreeing_static_program_builds_and_runs` — a
  module with a private static counter, an exported word with the correct global
  clause, incrementing it through `&!`; builds and runs to the expected output.

Regression: the full existing suite stays green; every effect with no `global:`
clause and every program with no `static:` declaration parses and checks
byte-for-byte as before (the additive guarantee).

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "AST + parser: StaticDecl/StaticInit/GlobalEntry/GlobalMode and the Module.statics + ParsedBodies.statics + WordDef.declared_globals fields (D4, no Type variant); the static: dispatch arm and parse_static_decl (D1, scalar-only, elided-zero, struct-type rejection per OQ1); the global: clause as its own trailing keyword clause in parse_worddef right after the effect's closing ')', mirroring the existing declares_inline peek -- parse_effect/parse_poly_effect are unchanged (D2, bare global: is a parse error); set WordDef { declared_globals: None } at every construction site (named-field, not tuple-pattern padding); driver drains bodies.statics into Module.statics. Parser unit tests including the additive no-clause / no-static guards. Phase must compile and stay green standalone with the analysis not yet wired.",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "Resolution + borrow-typing: teach resolve_modules the static name category (mangle StaticDecl.name per module, rewrite &NAME/&!NAME static references via strip_ref_sigil's fallthrough, R2); add the static branch to the borrow-typing arm in check/word_families.rs so &STATIC/&!STATIC types as &T/&!T and a scalar static is borrowable (R1); static-rooted provenance carries a real owned_root (the static name) so exclusivity/conflict scans keep firing and skips ONLY the disposal/consume scans, while stored_reference_error applies unchanged (R3). Confirm rename_call needs no change (note in diff). Borrow unit tests + the stored-ref mutation witness.",
      "effort": "M",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "The global-set analysis in a new src/check/globals.rs: direct-set traversal counting only directly-named statics, recursing if-arms and quotation literals (R4, decision 6); the intra-module call-graph adjacency and the monotone worklist fixpoint to convergence with mode-join (R5, OQ3); the boundary exact-match check with the mandatory-on-export / optional-on-private rule and the single located mismatch-error family covering missing/wrong-mode/extra plus the distinct no-such-static case (R6, decision 7). Wired pre-mangle in assemble_module beside check_exported_signatures. Checker unit tests (incl. the bounded-iteration mutual-recursion convergence witness, which must fail red rather than hang, and the intra-module-only negative test) and the exit diagnostic goldens.",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "FLAGGED (see Scope note): minimal scalar-static lowering so an agreeing static-using program builds and runs. Module.statics into the IR module; a data $NAME symbol per static in the QBE preamble mirroring the string data emission (src/backend/qbe.rs:650-700), zero/const-initialised per StaticInit; a new Instr::StaticAddr(Value, symbol) pushing the static's address as a Ptr, consumed by push_reference; the &STATIC arm in lower_reference_word. The agreeing-builds-and-runs golden. DROP THIS PHASE if S2 is to ship checker-only with lowering deferred to S4; scope the exit to diagnostic goldens + a check-passes unit test in that case.",
      "effort": "M",
      "difficulty": "hard"
    }
  ]
}
```
