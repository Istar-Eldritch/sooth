# Phase 3 Slice 6 — Reference types, places, escape checking (spec)

Design input: [the brief](./phase3-slice6-brief.md). Base: `main` @ `a66c47a`, 700 tests green.
Second draft: addresses review round 1 (soundness, criteria, consistency — three reports, roughly
54 findings between them (18 soundness + 21 criteria + 15 consistency), not the 19 an earlier
draft's header understated it as, N-13 round 3 audit) and
five decisions made by the project owner in response, cited inline as **D1**-**D5**. Third
draft: three further amendments, cited inline as **Amendment A/B/C**. Fourth draft: addresses
review round 2 (soundness — 7 blockers plus numerous minors/nits, sectioned A-G in that
review) and four further decisions made by the project owner in response, cited inline as
**Decision A**-**Decision D**; round-2 findings are cited by the review's own labels (e.g.
**A1**, **C2**, **G1**). Fifth draft: addresses review round 3 (soundness, criteria, and an
audit of the prior two fix passes) and a further set of owner decisions: **Decision E**
switches the borrow operator from postfix to prefix (`&a`/`&!a`, not `a &`/`a &!`); delivery
phase 4 (`get`/`set` migration and removal) is cut, its criterion and test retired, and
R20/R21 reworked so both words stay documented as superseded with their removal deferred to a
later slice; Decision D's justification is replaced, since it no longer exists to unblock a
phase this revision cuts; a newly-inserted ROADMAP.md slice, **general locals** (mid-body
`| names |` binding plus REPL-line locals), is recorded as this slice's prerequisite, which is
also why this slice renumbers from **Slice 5 to Slice 6**; and the aggregate-local aliasing
hole round 3 found twice independently is recorded as an explicitly open question, not decided
here. Round-3 findings are cited by the review's own labels (e.g. **A1**, **N-6**).

At the base commit, ROADMAP.md:438-442 named this slice "second-class references + parameter
conventions (`let`/`inout`/`sink`/`set`) + escape checking", title on 438. The conventions half
is deleted rather than built (R14); the slice is reference types, places, and escape checking.
(The first draft cited line 452, which was Slice 7 — resources as linear values +
user-definable destructor bodies — and would have rewritten the wrong slice's title; corrected
in the prior round throughout this document and in the brief.) **Amendment A applies the
correction directly to ROADMAP.md**, rather than deferring it to a delivery phase: the title
read "Second-class references + places + escape checking", with the Hylo parameter-convention
framing removed, and spanned ROADMAP.md:438-443 (one line longer than the base commit's
438-442, which shifted every citation below it in the file by one line at the time). **This
fifth draft's own ROADMAP.md edit inserts a new slice, "General locals," immediately before
this one** (see the prerequisite note below) — itself a prerequisite for this slice — which
renumbers this slice from Slice 5 to **Slice 6** and shifts every ROADMAP.md citation in this
document by a further 16 lines: the title/body now spans ROADMAP.md:454-459 and the parked
design question spans 460-466, both re-verified against the current file rather than carried
forward by arithmetic.

## Context: what is already true on the base commit

Measured by building and running programs, not by reading code (brief, recon 1 to 4).

- The name-it-repeatedly idiom `examples/stack.sth` uses for functional setters
  (`s s Stack>items s Stack>top x set Stack<items`) is a **hard compile error** the moment the
  aggregate is linear: `use after move in bump ... local b of type Buf was moved`. The idiom is
  Copy-only by language rule, not by convention.
- The destructure-and-rebuild tower is the only alternative today and its cost grows with
  nesting depth. Independently re-measured for this revision (three probes, `type: L1 v i64 ;`,
  `type: L2 v L1 tag i64 ;`, `type: L3 v L2 tag i64 ;`, each `bump` incrementing the innermost
  `v` and rebuilding every level with `S>fi`/`S<fi`), counting QBE instruction lines in
  `$bump`'s emitted body — excluding the `export function` header line, the `@start` label, and
  the closing `}` — depth 1 is 8 instructions / 1 `alloc` / 0 `blit`; depth 2 is 16 / 2 / 1;
  depth 3 is 25 / 3 / 2. `alloc` and `blit` grow by exactly +1 per level (one fresh frame slot
  and one child-blit per rebuilt level, as expected). The instruction count's own per-level
  delta is *not* a constant +8: it is +8 then +9 under this probe, because the second field of
  `L3` is reached one intervening level deeper than the second field of `L2`, so the
  destructure/rebuild plumbing cost is itself shape-dependent. Neither the first draft's stated
  absolutes (13/21/29) nor a first-round reviewer's independent measurement (12/20/28)
  reproduce here either — three attempts at the same qualitative probe, three different
  absolute tables. The counting convention is stated above precisely so a fourth attempt can
  reproduce *this* table; the load-bearing claim is qualitative and does not depend on any of
  the three tables' absolutes: the rebuild's cost is `alloc` + `blit` and grows with depth,
  a reference has neither, and R13's criterion asserts the qualitative absence directly against
  `push-byte`'s own emitted body, never against this table's counts.
- Aggregates already have addresses (`:Outer %v0` arrives as a pointer, rebuilds are `alloc8`
  slots). Scalars are SSA temporaries (`%v8 =l copy 1`), and QBE has no address-of for a
  temporary, so borrowing a bare scalar would mean creating a home that does not exist.
- Locals are top-of-scope only: `a 1 + | b |` is `parse error: unexpected token Pipe`.

The IR already has every operation this slice needs. `PtrOffset` (src/ir.rs:736), `ElemAddr`
(src/ir.rs:742), `FieldLoad` (src/ir.rs:758), `FieldStore` (src/ir.rs:763) all exist, and
`ElemAddr`'s own doc comment already calls its result "an opaque element place".

## Requirements

### Surface: reference types, places, projection, access

**R1 — Two reference types, `&T` and `&!T`.** Shared and mutable. Neither owns; neither is
linear. `&T` is `Copy`. `&!T` is neither `Copy` nor linear — see R8 for how the exactly-once
machinery treats it. Both are constructed only by R2 (a fresh borrow of a local) or by
projecting through an existing reference (R3, R17); they are ordinary types in type position,
including inside an effect signature's *input* side (R8 forbids the output side and every
other storage position).

**R2 — A borrow is taken from a local, prefix (Decision E).** A **place**, for this slice, is
a local name. `&` yields `&T` and `&!` yields `&!T`, both **prefix**: `&a`/`&!a`, not
`a &`/`a &!`. Applied to anything that is not a local name — a computed value, a literal, a
word result, or a projection expression — it is a located compile error.

**Decision E: prefix, not postfix (resolving round 2 soundness minor 8).** The second draft
made `&`/`&!` postfix, matching `^`'s existing postfix form (`rest ^`). Round 2 (soundness
minor 8) showed a postfix `&`/`&!` cannot be an ordinary word: naming a linear local moves it
(`moves.take`, src/check.rs:1670) before any following word could run, so a postfix `<local>
&!` would need the parser to fold backward into one place term, and nothing in the codebase
inspects a preceding term that way. Prefix removes the problem entirely: `&a` and `&!a` each
lex as a **single token** (measured: both produce one `unknown word` diagnostic on the base
commit, never a token-sequence parse error), so the checker resolves a borrow in one step,
exactly like any other word — no backward-folding parser machinery is needed. Prefix is also
more concatenative, not less: `&!a` is an atom that pushes a borrow and consumes nothing from
the stack, exactly as `a` is an atom that pushes a value, whereas postfix `a &!` was the form
that broke the model, needing to inspect the term that preceded it. Two details worth stating
explicitly: the sigil binds tightly, so `& a` (with a space) is **two tokens**, not a borrow —
and the same tightness runs the other way (round 1 nit 16): whitespace matters on *both* sides,
so a local immediately followed by another word with no space, e.g. `a&!` typed as one run, is
a *single* unknown-word token (`a&!`), not the two tokens `a` then `&!`; the sigil must always
be its own separate token, glued to nothing; and a reference-typed **parameter** needs no sigil
at all — if `b : &!Buf` arrived as an input,
the body writes `b`, not `&!b`, since `b` already names a reference. The sigil unambiguously
means "borrow an *owned* local", never "this local's static type happens to be a reference".
This does not change any projection spelling (`T&>fi`, `T&!>fi`, `&>`, `&!>`, `&^`, `&!^`),
which stay exactly as Decision A defined them — only the base borrow operator moves from
postfix to prefix.

The first draft additionally let a place be "a projection path rooted at a local name"
(applying `Buf>len &!` after naming `a`, meaning "borrow just this field"). Round 1 (soundness
minor 8) showed that form is unreachable as specified, for the same underlying reason Decision
E now fixes at the root for the base operator too: nothing in the codebase inspects a
preceding term the way a postfix accessor-path form would require. **D3/D4** resolves this by
retiring the projection-path form outright rather than building the backward parser folding it
would need: **R3**'s accessor family projects through an *already-borrowed* reference, so the
same result (`&!usize` for one field of `a`) is reached as `&!a Buf&!>len` — borrow the whole
local first, then project through the reference with an ordinary word. A place is therefore
exactly a local name, R2's prefix operator is applied to it exactly once per borrow, and every
deeper reference is a projection (R3), not a second application of `&`/`&!`.

The rejected stack-value alternative (`& ( T -- T &T )`) is recorded because it is close: a
plain word leaving the value below its own reference is tighter for a single borrow and is
trackable (the virtual stack is a compile-time `Vec<Value>` and shuffles are permutations
preserving the `Value` id). It loses because it cannot use locals at all (naming a linear local
moves it) and because the two-borrow call degenerates to `& rot & rot swap`. It is purely
additive later.

**R3 — Accessor words project through a reference, with a distinct spelling per shape *and*
per mutability (D3/D4, Decision A).** `fill` and `len` are **not renamed and not changed**, full
stop. `get` and `set` keep their existing value-form signatures **unchanged through this
slice's own phases (1-3)**: no signature here changes meaning, which is what keeps R20's
additive-work claim true. Amendment B marks `get`/`set` themselves **superseded** by this
slice's reference-mode accessors (R21) — a separate, later, mechanical concern with its own
delivery phase (4) and its own stated fallback, not a change to phases 1-3's typing rules. A
parallel family of reference-mode accessor words is added instead — every one of them takes an
already-reference-typed value (from R2 or from another projection) and yields a *narrower*
reference, never a plain value.

**Decision A: one spelling per shape *per mutability*, and the pipe is dropped.** Round 2 (A5)
caught the second draft's single-spelling-per-shape design lying about its own arity: `S&|>fi`
echoed `S|>fi`, the existing **non-consuming, two-output** Copy-peek (`( S -- S field )`), but
the reference-mode accessor is a **one-output, consuming** operation — the exact confusion the
B2 fix (below) was written to kill for `get`, reintroduced one level up by the naming itself.
The fix is to split every accessor by mutability into two distinct, fixed-arity words and drop
the `|`, which also removes any interaction with the lexer's `| locals |` delimiter
special-casing, and moots round 2's nit F4 along the way: the second draft's `&|>` spelling
lexed a bare typo `x &|` as `Word("&")` + `Pipe` (a misleading "unexpected token Pipe" error
for what was probably meant as the accessor), and dropping `|` from every spelling removes the
character the typo could even be made of — there is no `&`-then-`Pipe` shape left to mistype
into:

| shape | shared word | shared effect | mutable word | mutable effect |
|---|---|---|---|---|
| struct field | `T&>fi` (one per struct/field) | `( &S -- &Ti )` | `T&!>fi` (one per struct/field) | `( &!S -- &!Ti )` |
| array element | `&>` (generic, like `get`) | `( &[T N] usize -- &T )` | `&!>` (generic, like `get`) | `( &![T N] usize -- &!T )`, same runtime bounds trap as `get` |
| cell payload | `&^` (generic, like `^|>`) | `( &^T -- &T )` | `&!^` (generic, like `^|>`) | `( &!^T -- &!T )` |

All six spellings lex as single tokens on the base commit (measured, one probe per spelling:
`: main ( -- ) <w> ;` for `&>`, `&!>`, `&^`, `&!^`, and a struct-specific probe for
`Buf&>len`/`Buf&!>len`, each producing exactly one `unknown word` diagnostic rather than a
token-sequence parse error). This supersedes the second draft's "mutability inherited from the
receiver" rule outright, not merely its spelling: mutability is now explicit in the token
itself, so a reader gets the whole signature — reference-ness *and* mutability *and* arity —
from the word alone, with nothing left to infer from context. Every reference-mode accessor's
arity is fixed and readable from its spelling (one reference in, one narrower reference out) and
now so is its mutability. This closes the two forks the first draft left open, and A5's naming
fork besides:

- **B1 fixed**: the first draft typed `@` only as `( &T -- T )`, so `b Buf>len` (yielding
  `&!usize` under the old `S>fi`-on-a-reference reading) had nowhere to go. `T&>fi`/`T&!>fi`
  never exist in an ambiguous arity, and R4 below types `@` for both `&T` and `&!T` directly, so
  no reference ever needs an implicit `&!T -> &T` coercion (which would in any case have
  collided with R5's "no `&` while an `&!` is live" — taking the coercion *is* taking a `&`).
- **B2 fixed**: the first draft read `get`'s existing two-output, non-consuming value form
  ("`get` on `&[T N]`... yields `&T`") as if it were the same word with one output instead. It
  is not the same word. `&>`/`&!>` are distinct, fixed-arity words; `get` keeps its own
  signature unchanged and is never applied to a reference.
- **A5 fixed**: `T&>fi`/`T&!>fi` no longer echo `S|>fi`'s name for a differently-shaped
  operation. The two families now share only the struct/field-name convention, not an implied
  arity.

**Projection through a `&!` consumes the parent reference, and — separately — suspends the
place it was reborrowed from (Decision B, replacing the second draft's "linear by
construction" claim).** `T&>fi`/`T&!>fi`, `&>`/`&!>`, and `&^`/`&!^` each take their reference
argument off the stack the way every word takes its arguments off the stack — ordinary
consumption, not a special rule. Round 2 (C2) showed that consuming the parent *value* is not
by itself enough to make a mutable projection chain linear, because naming a `&!` local is a
**reborrow**, and a reborrow manufactures a fresh parent independent of whatever was derived
from the previous one:

```forth
: two-live ( &!Buf -- )
  | b |
  b Buf&!>len        \ [&!usize] — reborrow #1 consumed by the projection, derived ref live
  b Buf&!>len        \ [&!usize, &!usize] — reborrow #2, while #1's derived ref is still live
  1 +! 1 +! ;
```

Each reborrow of `b` *is* consumed by its own projection, exactly as the second draft's
mechanism describes — and the program still manufactures two simultaneously live `&!usize`
into the same field, because nothing about "the reborrow got consumed" says anything about
what it was *turned into* still being live. The rule this slice actually needs is stated over
the **place**, not the reborrow value: **taking a mutable reference derived from a place
suspends that place for as long as any reference derived from it (by any number of projection
steps) is live; naming the place again during that window is a located error**, symmetric with
R5's existing rule that consuming a place while a borrow of it is live is an error. R6's
consumption-point scan is restated below to key on the place and its outstanding derivations —
whether anything currently traces its provenance back to that place — rather than on whether
some stack slot or locals-map entry literally *is* the reborrow's own `Value`; `two-live` is
rejected the moment the second `b` is named, because the first reborrow's derived `&!usize` is
still outstanding against `b`'s place.

This narrows what the second draft called "exclusivity falls out structurally" (**B4**) to the
claim round 2 says it can actually support: **R7's disjointness scan never has to reason about
a reborrow and a reference derived from *that same* reborrow being simultaneously live**,
because the suspend rule above already rejects taking a second reborrow while the first's
derivation is outstanding — R7 only ever sees one live derivation chain per place at a time.
The broader claim ("retires the nested-borrow exemption", "linear by construction") is cut; the
linearity is enforced, by the suspend rule, not free of enforcement. `push-byte`'s own hand-trace
(below) is the positive case: each of its three reborrows of `b` is fully consumed — down to a
plain value or a further-derived reference that is itself fully consumed — before the next one
is taken, so the place is never suspended at the point of a subsequent reborrow, and the program
is accepted.

**The suspend rule is mutable-only (Decision B).** `&T` is `Copy`, so a *shared* projection
composes freely: a live `&T` parent alongside a reference derived from it (`&a dup Buf&>len`,
say) is fine and must stay legal — shared references carry no exclusivity, so there is nothing
for a suspend to protect. Only a `&!`-rooted derivation suspends its place.

It also still means **R11's root list needs no reference-typed case** (**B3**): the only thing
R2's prefix `&`/`&!` is ever applied to is a plain aggregate local, never to a value that is
already a reference, so R11's whitelist (struct/enum/array/cell) stays exactly as first
drafted — that part of the second draft's reasoning did not depend on the linearity claim C2
falsifies, and survives it.

**Name reservation (round 2, F2).** Nothing in the base commit reserves an `&`-led name: a
local named `&!` compiles and runs today (measured: `: f ( i64 -- ) | &! | &! drop ;` produces a
working object file, only failing to *link* for the unrelated reason that the probe defines no
`main` word), and a `type:` named `&!Buf` is accepted by the parser, then fails at the QBE
backend with an unescaped-symbol error (`invalid character & (38)`) — a pre-existing, separate
bug this slice does not fix, but whose exposure this reservation prevents. This slice adds the
identical machinery the owning-cell syntax already has: `is_reserved_caret_name`/
`reserved_caret_name_error` (src/parser.rs:85-103) reject any name beginning with `^` at every
declaration site (`type:` name, `:` word name, local binding, the REPL's own `type:` path); the
same shape is added for `&`, rejecting any name beginning with `&` at the same four sites.
Separately, `@`, `!`, and `+!` are exact-name builtins, not prefix-reserved, and — unlike `^` —
nothing in the base commit protects *any* builtin exact name from shadowing today (measured:
`: drop ( i64 -- ) . ;` and `: ! ( i64 -- ) drop ;` both compile with no diagnostic), so this is
genuinely new machinery, not a mirror of an existing check: a `:` word declaration using exactly
`@`, `!`, or `+!` as its name is a located shadowing-rejection error, added specifically because
those three names would otherwise silently change meaning to every future caller the moment
this slice introduces them as builtins.

**Type-position splitting is three cases, not one (round 2, F3).** `&`, `!`, and `^` are not
lexer delimiters, but `[` is, so `&T`/`&!T` in type position spells three different token
shapes and needs three parsing cases: a bare `&!Buf` arrives as one `Word` token and must split
*within* itself (measured: lexes as a single unknown-word token, never a token sequence); a
composed `&!^List` also arrives as one `Word` token, splits within itself down to the remainder
`^List`, and that remainder must be handed to the *existing* caret splitter
(`parse_owning_cell_type_expr`, src/parser.rs:583-611), not to `resolve_type` directly, since
`^List` is itself an owning-cell type expression one level down; `&![u8 64]` splits *across*
tokens (`Word("&!")` then `LBracket`, measured directly) because `[` already is a delimiter, so
recursing into the ongoing token stream — exactly `parse_owning_cell_type_expr`'s existing
empty-remainder case — handles it without new machinery. The `&`/`&!` splitter mirrors
`parse_owning_cell_type_expr`'s two-case shape (empty remainder recurses into the stream;
non-empty remainder resolves the substring) and adds the one case `^`'s splitter does not need:
a non-empty remainder that itself begins with `^` is handed to the caret splitter, not to
`resolve_type`. This is not hypothetical: nothing in the dogfood ever spells `&!^List` as
literal source text, but `walk`'s `Cons` clause binds `next` at exactly that type by inference
(R17), so any signature a programmer writes by hand in this shape (`: helper ( &!^List -- )
... ;`) must parse the same way, and the splitter earns its third case the first time R17 is
used at all, not only in a contrived example.

**R4 — Access through a reference: `@`, `!`, `+!`.**

- `@` fetches, typed for **both** `&T -> T` and `&!T -> T`, consuming the reference either way.
  Because both are covered directly, there is no implicit `&!T -> &T` demotion rule to write
  (closes B1 for good, not just for the dogfood's literal line).
- `!` stores, `( &!T T -- )` only (storing through a shared reference is meaningless: a shared
  reference carries no exclusivity, so there is nothing to protect a concurrent reader from).
- `+!` adds in place, `( &!T T -- )`, `T` an integer type. Sugar for fetch-add-store, kept
  because the alternative spelling needs two sequential borrows of the same place plus a `swap`.
  **Round 2 (minor A3): `T` is inferred from the receiver, and D8's bare-literal coercion
  applies exactly as it does anywhere else `T` is inferred rather than declared** — typing
  `+!`/`!` via `match_slot`/`unify_pair` (the same mechanism an ordinary declared-parameter
  call site uses) means the coercion carve-out (`usize`/`isize` bare literals only, no bare
  `i64`-to-narrower coercion) is not a new rule for this position, it is the existing rule
  applied where `T` happens to come from a reference's pointee rather than from a `:` word's
  declared signature. This is why `b Buf&!>len 1 +!` (`T = usize`, inferred) accepts the bare
  literal `1` while `push-byte`'s `x !` needed `72 >u8` upstream (`T = u8`, inferred, and `u8`
  is not on the carve-out) — both follow from the same rule, stated here explicitly so a
  reader does not have to infer it from the dogfood alone.
- **Restricted to `Copy` `T`; a Copy *aggregate* is a real, first-class case, not a rejection
  (Decision D).** **Justification (this revision): `dup` on a Copy aggregate already lowers to
  exactly this, and it works today.** `lower_call`'s `"dup"` arm (src/ir.rs:1714-1726) allocs a
  fresh slot and blits the bytes for a Copy `Struct`/`Enum`/`Array` — its own comment reads "A
  struct or enum is copied by value: alloc a fresh slot and blit the bytes, so mutating the copy
  leaves the original intact" — and this machinery is exercised on every `dup` of a Copy
  aggregate in the suite today (verified: `1 2 V dup V> . . V> . .` prints `2 1 2 1`, i.e. the
  duplicate and the original are independent). Without Decision D, `dup` can duplicate a Copy
  struct but `@` cannot — an arbitrary hole with no principled explanation, in an operation this
  slice itself introduces. The restriction was never about safety: `is_copy` returns `false` for
  anything transitively containing a cell, so no linear value can ever reach this path; the only
  reason `@`/`!` rejected a Copy aggregate was that they were wired to the scalar lowering path
  (`FieldLoad`/`FieldStore`, which `unreachable!` on an aggregate at src/backend/qbe.rs:318 and
  :338) and never to the aggregate one Decision D adds.

  `is_copy` makes an all-scalar-field struct like `Vec2 { x i64 y i64 }`, or an all-scalar-payload
  enum like `examples/vm.sth`'s `Op`, `Copy`. The second draft (round 1 minor 11) closed the
  `field_load_op`/`field_store_op` `unreachable!` by restricting `@`/`!`/`+!` to Copy-*scalar* `T`
  only, rejecting a Copy aggregate as a located compile error — but that made `@`/`!` a strict
  subset of `get`/`set`, which read/write a Copy aggregate element (`examples/vm.sth`'s
  `[Op 13]`) without difficulty, so `get`/`set` could not actually be retired (R21's own
  headline example, `vm.sth:58`'s `get`, is exactly this case) — though R21's removal is now
  deferred (see below), the hole in `@`/`!` would remain arbitrary regardless of whether `get`/
  `set` are ever retired. Decision D lifts the restriction for the aggregate case instead of
  leaving it rejected: `@` on a Copy *aggregate* `T` lowers to `Alloc` (a fresh destination slot,
  sized and aligned from `T`'s layout) followed by `Blit` (a byte-copy from the field/element
  address into that slot); `!` on a Copy aggregate `T` lowers to `Blit` alone (a byte-copy from
  the stored value's address into the field/element address). Both instructions already exist
  (`Alloc` src/ir.rs:750, `Blit` src/ir.rs:754) and are already used for exactly this shape of
  copy by `dup`'s own Copy-aggregate arm above, so R12's no-new-`Instr`-variant claim survives
  unchanged — this is a new lowering arm over an existing instruction pair, not new IR. There is
  no linearity story to write for this case, and this is not an open question left for later:
  the value is `Copy` by construction, and duplicating a `Copy` value is safe by definition, full
  stop. This also does not disturb the no-`alloc`/no-`blit` structural criterion (criterion 6,
  `push-byte`'s own body): that criterion is about `push-byte`'s element type `u8`, which is
  scalar regardless of this change, so the ceiling it asserts is unaffected. The restriction that
  remains is scalar-vs-aggregate no longer matters; what still matters, unchanged, is
  Copy-vs-linear (next bullet) — `@`/`!`/`+!` stay rejected on a linear `T` exactly as before,
  aggregate or not.

  **New criterion, closing round 3's A5/soundness finding: the aliasing test.** A behavioural
  golden that only fetches and stores without a subsequent mutation cannot tell an `Alloc`+`Blit`
  fetch apart from a lowering that skips the `Alloc` and returns the field address directly —
  both read and write correctly until the source is mutated after the fetch. Add a golden that
  fetches a Copy aggregate through `&`, mutates the *original* through `&!`, then prints the
  *fetched copy* and asserts it still reads its **pre-mutation** value — the whole point of the
  `Alloc` is that the fetched value is independent of the referent from that moment on.
- Fetching or storing a **linear** `T` through a reference is a separate, pre-existing rejection
  from the plain Copy check: it would either produce a second owner of one object (fetch) or
  silently leak the overwritten value (store, since nothing auto-drops) — both soundness rules,
  not scope decisions. `S<fi`'s drop-on-overwrite (docs/phase3-slice1-spec.md:60,
  src/ir.rs:2465) is the precedent for lifting the store restriction later; left out here
  because no criterion needs it.

**R5 — Exclusivity is the entire aliasing rule, and it is symmetric (Decision C, resolving
round 2 C1).** At most one live `&!` to a place; no `&` to a place while a `&!` to it is live;
and — the direction the second draft omitted — no `&!` to a place while a `&` to it is live.
Round 2 (C1) found the second draft stated only the first two: "no `&` to a place while an `&!`
to it is live" has no converse, so `&a &!a` was legal by the letter of the rule, producing a
live shared reference and a live mutable reference to the same place simultaneously — exactly
the aliasing R5 exists to prevent, and worse once `dup` is added (`&a dup` makes two live
shared refs, then `&!a` makes the violation obvious). R5 now states both orders explicitly.
Criterion 7's single test name (`shared_borrow_alongside_mutable_is_error`) is order-ambiguous
and would almost certainly be written in the order the second draft already covered; it is
split below into one test per order so the fix has independent evidence, not just prose.

**N-9 (round 3 audit): "must not be implemented as one" no longer holds, precisely.** The
place-suspend rule for a mutable reborrow's outstanding derivations (Decision B) is genuinely
separate machinery, not a consequence of the bullets below — it is stated in R3, restated as a
provenance scan in R6, and is what makes exclusivity per-place rather than per-borrow-instant.
Read this heading as: exclusivity plus the suspend rule together are the aliasing rule for
values **reached by borrowing a place**; everything below this paragraph other than the
suspend rule is a consequence of that pair, checked at the same consumption points (R6), and
need not be implemented separately. R5 does **not** cover two aggregate *values* that alias one
address with no borrow ever taken — see "Open question: aggregate-local aliasing" below, which
this revision leaves unresolved.

Consequences of exclusivity plus the suspend rule, not further separate rules:

- `&T` is `Copy` (shared references carry no exclusivity constraint).
- `&!T` is not `Copy`.
- `dup` on a `&!` is rejected **by R5**, since it would produce two live mutable references to
  one place.
- Naming a `&!` local is a **reborrow**, not a move. Without this a mutable helper would kill
  its own parameter on first use, and the dogfood's `push-byte` names `b` three times. A
  reborrow is itself subject to the suspend rule above (Decision B): naming `b` again while a
  reference derived from its *previous* reborrow is still outstanding is the same R5 violation,
  not a separate case.
- Two live `&!` rooted at *different* places never conflict — R5 is per-place, and nothing about
  it is a single global "one mutable reference at a time" counter. `copy-byte`'s two-borrow
  call (`&!Buf` into `dst`, `&Buf` into `src`, two different locals) exercises exactly this.

**R6 — The borrow check fires at consumption points, keyed on the place and its outstanding
derivations, not via a liveness pass (restated for Decision B).** When a place is moved,
dropped, or (re)borrowed in a way R5 forbids, the check must answer "does anything currently on
the virtual stack (`stack: Vec<Value>`) or in the locals map (`locals: HashMap<String, Value>`)
trace its provenance back to this place, through any number of projection steps", not merely
"is some slot's `Value` literally equal to a reference taken directly from this place". Round 2
(C2) is why the stronger form is necessary: a projection's result is a *new* `Value`, and a
naive scan for "the reborrow itself" misses a `&!usize` two projection steps removed from a
place that is nonetheless still live against it (the `two-live` example above). Provenance is
cheap to track — R3's projections already know their own operand — and every place that can
conflict (move, dispose, new borrow) consults the same predicate. Both `stack` and `locals` are
exact compile-time structures, so this is a scan over threaded provenance, not an analysis.
Reject with a located error naming both the place and the conflicting borrow. A reference is
**live** from the instruction that creates it until the term that consumes its slot; a
reference-typed *local* is live for the whole word body (see R8 for what happens to it at the
body's end, since it is neither `Copy` nor linear).

Rejected alternative: last-use (NLL-style) liveness. More precise, materially more machinery,
and no criterion in this slice needs the precision.

**R7 — Path disjointness is not modeled.** Two references derived from the same local conflict
under R5 even when they project into disjoint fields, if both are simultaneously live (R6). The
measured cost is one `swap` in the dogfood's `push-byte`, sequencing the two projections so the
first is fully consumed (down to a plain value or a further-derived reference that is itself
consumed) before the second is taken — never holding both at once. This is a stated limitation
with its own criterion, so it is behaviour rather than an accident, and it is additive later.
R17's reference-mode clause payload bindings are a narrow, named exemption from this rule, not
a second case of it — see R17 for why fan-out from one scrutinee is sound where this rule is
conservative.

**R8 — Escape is prevented structurally, by six positional rejections over transitive
containment (five over compiled code, one over the REPL's cross-line storage — D4 below).** A
type that **transitively contains** a reference — the reference itself, a struct with a
reference-typed field (directly or nested), an enum variant carrying one, an array of them, or
a cell over one — is a located compile error in: a struct field declaration, an enum variant
payload declaration, an array element (via `fill`), a cell payload (via `^`), the **output**
side of a declared effect signature, and (D4) a value surviving to the end of a REPL line that
would be carried into the session's inter-line stack. A reference on the **input** side of an
effect is fine (R2 already establishes the only source of a reference is a local borrow inside
some frame, so a parameter reference is unremarkable) and is accepted, narrowly (D3 below),
tested alongside the rejections rather than left implicit.

Round 1 (soundness B5) found the first draft's three-position version had two holes precisely
because it enumerated *positions* rather than closing over *type constructors*: `check_owned_cell_word`'s
`"^"` arm (src/check.rs:2225) interns a cell over any payload type with no filter, so `&!a ^`
built `^&!Buf` — not itself a reference type, so the old three-position check missed it, and it
is legal in a field and on an output side once built. `check_array_word`'s `"fill"` arm
(src/check.rs:2133, the Copy check at 2146) accepts any `Copy` element, and R5 makes `&T` `Copy`,
so `r 4 fill` built `[&Buf 4]` with the same consequence. Both holes close by rejecting *at the
construction site* (`^`'s and `fill`'s own arms reject a payload/element that transitively
contains a reference) rather than only at declaration sites, which is why the rule is now five
positions over compiled code (six counting D4's REPL case below) phrased over containment
instead of three phrased over syntax.

Combined with place-only creation (R2) and R11 (only an aggregate local can be a borrow root, so
a reference can never be the *only* handle to something whose lifetime it controls), a reference
cannot outlive its referent, so no lifetime apparatus is needed.

**Three round-2 hardening notes on R8 (D2, D3, D4), none of which change R8's outcome:**

- **D2 — `set` is a second array-element construction site R8 doesn't name in its own prose.**
  R8 phrases the array rejection as "an array element (via `fill`)", but `set ( [T N] usize T
  -- [T N] )` also writes an element and survives phases 1-3 unmentioned. It is closed only
  *transitively*: `fill`'s own construction-site rejection already means a `[&Buf 4]` value can
  never exist to call `set` on in the first place, so there is nothing for `set` to leak. State
  that reasoning here rather than leaving `set` looking like an unguarded second site: R8's
  rejection is at every *construction* site (`fill`, `^`), and `set` is a *mutation* site over an
  already-legal array, which cannot be reference-typed if it was never constructible as one.
- **D3 — narrow the input-side accept-case to a literal top-level reference.** R8's accept-case
  ("a reference on the input side of an effect is fine") is phrased over the type at the top
  level, but read as literally as the rejection bullets above it, `: evil ( [&Buf 4] -- )` and
  `: evil2 ( ^&Buf -- )` would also be accepted declarations, since nothing in R8's own text
  narrows the carve-out below the position level for the accept side specifically. Both are
  uninhabited today — D1's construction-site rejections mean no value of either type can ever
  be built to pass in — so there is no live exploit, but the carve-out is stated narrowly here
  regardless: an effect's *input* side accepts a type that **is itself** `&T`/`&!T` at the top
  level, not a type that merely contains one nested inside an array or cell, so the accept-case
  stays closed if a future slice adds another aggregate constructor that this slice's two
  construction-site rejections don't already cover.
- **N-11 (round 3 audit): the output-side ban has a consequence worth stating plainly.** Because
  R8 rejects a reference on the **output** side of any declared effect, a projection can never be
  factored into its own helper word: `: len-of ( &!Buf -- &!usize ) Buf&!>len ;` is rejected, not
  because the projection itself is wrong, but because `&!usize` on the output side is exactly
  what R8 bans. This is a real, load-bearing limitation, not a bug — every projection in this
  slice's dogfood is written inline for this reason — and it belongs here, at the rule that
  causes it, rather than left for an implementer to discover by trying to factor one out.
- **D4 — the REPL's carried stack is a storage position R8 must reject explicitly, not by
  accident.** `Session` persists the inter-line stack as raw 8-byte cells plus a per-slot `Type`
  (src/repl.rs:246-255). A reference landing there would be a dangling pointer silently
  surviving into the next line — R12 maps a reference to `IrType::Ptr`, an 8-byte cell, so
  nothing about the storage format would reject it. It is unreachable *today* only by an
  accident this slice must not rely on: `Ctx::Line` has no locals at all
  (`Ctx::Line { .. } => None`, src/check.rs:285/306), so R2's local-only place can never fire
  inside a REPL line, and R8 is never asked the question. That is a fact about the REPL's
  current shape, not a guarantee — R8 gains an explicit sixth rejection: a reference-typed value
  surviving to the end of a REPL line (i.e. a value that would be carried into `Session`'s
  buffer) is a located error, with its own golden, rather than a safety property that happens to
  hold today for an unrelated reason.

*On DESIGN.md's "no borrow checker" claim.* At the base commit, DESIGN.md:134 read "There is no
borrow checker" and DESIGN.md:211-212 read that a borrow checker "is the worst possible fit
here and is deliberately avoided", in the same breath as DESIGN.md:210's "cannot escape their
scope... no lifetime system is needed", which the paragraph above relies on. R5 (per-place
exclusivity), R6 (a scan for a conflicting borrow at every consumption point), R9 (back-edge
rules), and R10 (join agreement) are, taken together, a borrow checker minus lifetimes. That is
a real distinction, not a rhetorical dodge: what those passages rule out is a *lifetime* system
— lifetime variables, region annotations, anything that tracks how long a reference is allowed
to live relative to a named binding — and this slice adds none of that. What it adds is
narrower: a per-place aliasing rule checked at the point each place is consumed, which is
possible with no lifetime apparatus *because* R8 already guarantees a reference cannot escape
the frame that created it. DESIGN.md is amended (below, and applied in this same revision) to
say this explicitly rather than leaving a reader to reconcile the apparent contradiction
unassisted; DESIGN.md:134 now reads "There is no lifetime-tracking borrow checker" and
DESIGN.md:208-214's paragraph states the per-place-exclusivity-vs-lifetime-system distinction
directly.

A leftover reference is a surplus value like any other and requires an explicit `drop` — **at
the stack level only** (round 1 consistency B3). `&!T` is neither `Copy` nor linear, a third
category the exactly-once machinery does not otherwise have, so this slice states its treatment
explicitly rather than leaving it implied: the surplus-value check (which already fires off the
declared effect's output side, the same mechanism that catches a forgotten `int`) applies to it
exactly as it applies to any non-`Copy` value left on the stack. A reference-typed **local**,
by contrast, is never surplus-checked: it simply expires silently at the end of the word body
(R6's "live for the whole word body" already frames it this way), matching the fact that a
parameter is never itself "left over" — `push-byte`'s `b`, and `copy-byte`'s `dst`/`src`, are
never explicitly dropped and this is correct, not an oversight. `drop` applied to a reference
(local or stack value) emits no destructor call: it never owned anything.

**R9 — Loops: the referent must outlive the iteration, from both sides.** A reference
**parameter** (arriving in the word's input effect) may cross a self-tail-call back-edge, since
its referent lives in an ancestor frame and outlives every iteration. This is what keeps
`: walk ( &!List -- ) ... walk ;` legal, and that case is why the feature is worth having — see
R17 for how a clause-style `walk` over an enum reference is spelled at all, which the first
draft could not do. A reference *derived by projection* from a parameter (R3) inherits the same
permission, since its provenance traces back to the parameter's ancestor-frame referent, not to
anything created in the current frame. Two located errors guard the other side: a reference
derived from a **current-scope local** may not cross a back-edge, and a **borrowed local** may
not itself be loop-carried, because locals rebind at the header (`header_phis`,
src/ir.rs:1496) and either would alias a reused slot. "Borrowed" here means *currently*
borrowed at the point of the back-edge (R6's live-until-consumed definition), not "ever
borrowed during this iteration" — a borrow that already ended before the back-edge leaves
nothing live to alias and is unaffected.

**R10 — Branch joins: borrow state must agree.** A place borrowed on one arm and not the other
is a located error at the join; a place borrowed identically on both arms, or on neither, joins
cleanly. **N-10 (round 3 audit): the case this actually catches, restated, since "a borrow held
in a local" was shown vacuous given entry-only binding (a reference-typed local is a parameter,
identical on both arms by construction).** The existing type unification checks that both arms
leave the *same types* on the stack, and would already reject two arms whose stacks disagree in
shape (a live reference on one arm, none on the other). What it does **not** check is *which
place* a live reference's suspension is attributed to: two arms can each leave a stack of
identical shape (say, both a live `&!usize`) while each arm's value suspends a **different**
place — one arm derives its reference from local `x`, the other from local `y` — and type
unification alone has nothing to say about that, since it only compares types, not provenance.
R10 is the rule that the *suspended-place* bookkeeping must also agree across arms, not just the
stack's types; that is real content the type-only unification does not supply, tested on both
the disagreement and the agreement side so an over-broad "any borrow crossing an `if` is an
error" implementation cannot pass by accident. Rejected alternative: a `MaybeBorrowed` lattice
element mirroring Slice 1's `MaybeMoved`. The conservative
rule is smaller and no criterion needs the imprecision.

**R11 — Only aggregate or cell locals may be borrowed** (N-12: retitled from "aggregate" alone,
since the body always included cell — the title just didn't say so). The root of a place (R2: a
local name) must be a local of struct, enum, array, or cell type. A local of scalar type is a located compile error
("borrow a field or an aggregate"). This deletes the spill obligation from the brief's D5
entirely: by recon 3 scalars are SSA temporaries with no address, and giving them memory homes
is real work no criterion needs. A projection whose *result* is scalar (`b Buf&!>len` yielding
`&!usize`) is unaffected, since the referent is a field inside an aggregate that already has a
slot, and — per R3's consequence above — the list stays exactly struct/enum/array/cell, with no
reference-typed case needed, since R2 is never applied to an already-reference value.

**R12 — No new IR instruction; a reference is always `IrType::Ptr`.** Struct-field projection
is `PtrOffset`, array-element projection is `ElemAddr`, cell projection (`&^`/`&!^`, N-14: both
spellings, not just the shared one) is a `Load` of the stored pointer, `@` is `FieldLoad`, `!` is
`FieldStore`, `+!` is `FieldLoad` + `Bin(Add)` + `FieldStore`. `Ptr` stays opaque; no pointer
arithmetic is exposed to the surface language.

Round 1 (soundness B6) found the first draft never said what `IrType` a reference maps to, and
the tempting default is silently wrong: `ir_type_of` (src/ir.rs:154) is a total `Type -> IrType`
map, and mapping `&!Buf` to `IrType::Struct(id)` would make `qbe_abi_ty` (src/backend/qbe.rs:264)
spell it `:Buf` in ABI positions, which QBE's C-ABI classification passes **by value** —
measured directly, a callee storing into a `:Buf` parameter does not affect the caller. Under
that mapping `push-byte ( &!Buf u8 -- )` would mutate a caller-side temporary and the dogfood
would silently print an unmutated byte. `&T` and `&!T` map to `IrType::Ptr` instead, always,
including in ABI positions: `IrType::Ptr` already exists (src/ir.rs:131) exactly for this shape
("a native pointer under QBE, a linear-memory offset under a future WASM lowering") and both
`width` and `qbe_abi_ty` already spell it `l` (src/backend/qbe.rs:248, and via the `_ =>
width(ty)` fallthrough at src/backend/qbe.rs:264+) with no change needed to either function —
only `ir_type_of` gains the two new `Type` arms. Cell projection's `Load` (R3, `&^`/`&!^`) loads
a pointer value whose `IrType` is `Ptr`, exactly the existing `OwnedCell` shape (src/ir.rs:1337),
never an `Int`, so `Load`'s doc comment ("`dst: Int = *ptr`", src/ir.rs:743) describes the
*instruction*, not a constraint on the destination's `IrType` — the same reading Slice 2's own
cell-unwrap lowering already relies on.

**Round 2 (E1): "only `ir_type_of` gains two arms" understates the work, and the missing part
is a soundness answer, not a mechanical one.** `Type` is matched exhaustively at many sites
(`is_copy`, every `is_linear`-shaped predicate, the surplus-value check, and further sites this
spec does not enumerate), and adding `&T`/`&!T` as new `Type` variants means each of those
matches gains an arm whose *answer* the checker depends on for correctness, not merely an arm
that must exist to satisfy exhaustiveness: `is_copy` must return `true` for `&T` and `false` for
`&!T` (this is R5's "`&T` is `Copy`, `&!T` is not" bullet, restated as a checker obligation, not
just prose), and every `is_linear`-shaped predicate must return `false` for both (a reference is
neither `Copy` nor linear, R8's third-category treatment above) — getting either wrong would
silently misclassify a reference as duplicable-and-droppable or as needing the linear
drop-tracking machinery, either of which is a soundness bug, not a missing case. Separately, and
consistent with the tripwire paragraph in the invariants section below: a reference needs an
interned `RefId` registry — an `(inner: Type, mutable: bool)` pair, mirroring `Array`'s and
`OwnedCell`'s existing `(Id, &'static str)` registries — the moment a parameterized reference
type needs to render its own name (an error message, a `T&>fi`/`T&!>fi` accessor's generated
name). Phase 1's changes list is updated below to name `is_copy`/the linearity predicates and
the `RefId` registry explicitly, rather than leaving them implied by "`ir_type_of` gains two
arms".

**R13 — Mutation through a reference emits no rebuild.** The measurable form of the recon-2
table: the emitted body of the dogfood's `push-byte` contains no `alloc` and no `blit`.
**Round 2 (nit E2): criterion 6's instruction-count ceiling must budget for the bounds guard.**
`push-byte`'s array-element projection (`&!>`) has a *computed* index (from `Buf&!>len @`, not
a literal), so `bounds_check` (src/ir.rs:2341-2358) emits a `Cmp`, a `Jnz`, a trap block, and a
`Call sooth_oob_trap` in addition to the address-arithmetic-plus-store shape — a literal index
would skip the guard entirely. This does not affect the no-`alloc`/no-`blit` assertion itself,
only the numeric ceiling a phase-1 implementer sets for "how many instructions is too many":
set the ceiling from `push-byte`'s own measured shape (bounds guard included), not from an
idealized reference-only body that has none.

**R14 — No parameter-convention keywords.** `let`, `inout`, `sink`, and `set` are not added.
The reference type is the convention: `&Buf` is what `let Buf` would have meant and `&!Buf` is
what `inout Buf` would have meant. `sink` is the unannotated default, so **no existing signature
changes meaning and no existing code migrates**. `set` is cut twice over: stack returns are a
better out-parameter than a mutable hole, and `set` is already a user-callable array word in
`examples/stack.sth`. **Round 2 (minor G3): the "stack returns" half of that argument is
currently unsupported by the compiler, and this slice does not fix it.** A user-defined word
with two outputs is a **reachable panic** on the base commit today (measured: `: w ( -- i64 i64
) 1 2 ;` called from `main` panics in `print` at src/ir.rs:1825, and a two-output struct
destructure panics in `drop` at src/ir.rs:1749) — `get`'s own two outputs work only because
`get` lowers inline as a checker/IR special case, not as an ordinary multi-output `Call`. This
is a pre-existing gap, out of this slice's scope to fix, but R14's cited alternative should not
be read as already available; the argument for cutting `set` stands on its other, independent
leg (`set` is already a user-callable array word) regardless.

**R15 — Top-of-scope locals are not relaxed.** Recon 4 makes mid-body `| |` a parse error, so a
projection cannot be named where it is most wanted. The dogfood works without it at the cost of
one `swap`, the same cost already accepted under R7. Relaxing binding is a parser and scoping
change orthogonal to references and would widen the slice.

**R16 — The question ROADMAP.md's parked design question answers, answered.** (The first draft
cited line 447, which is mid-sentence inside the parked question; the question and its "Design
question this slice's brief must answer" marker ran 443-449, marker on 443, at the base commit;
Amendment A's one-line-longer title/body edit shifted it to 444-450, marker on 444; this
revision's own general-locals insertion shifts it a further 16 lines, to **460-466, marker on
460**, re-verified against the current file rather than carried forward by arithmetic.) `inout`
projections **do** subsume a reified
take/fill pair (`S/fi` yielding a residual `∂S/∂fi`, refilled exactly once) for every
statically known path, because a projection is the same residual made implicit and lexically
bounded, and it covers whole-value borrows too. No residual form is added. Reified residuals
remain worth having only where the focus must escape, which is Slice 3's zipper; R8 forbids
storing a reference, so the zipper waits for Slice 6's RC rather than for a residual type. This
answer is recorded in delivery phase 3's changes list so it lands with the ROADMAP correction
rather than only living in this prose.

**R17 — Reference-mode enum elimination (D1).** When a word's declared top input is a reference
to an enum (`&Enum` or `&!Enum`), the existing clause-style whole-word form (`| Variant ... |
Variant ... ;`) applies in **reference mode**, same syntax, four differences from the
value-mode form (the fourth added this round, Decision B/round 2 B1-B3):

- The scrutinee is **borrowed and consumed by the dispatch** (round 2 B3, see below), not owned
  and not freed: reading the discriminant through the reference is a tag `FieldLoad` (no new IR
  instruction, consistent with R12), and the enum value itself is never freed or moved by the
  dispatch — only the reference *value* is consumed, the same way any reference argument to any
  word is consumed by that word.
- Each clause's payload bindings are **references inheriting the scrutinee's mutability**: a
  `Cons v i64 next ^List` clause under a `&!List` scrutinee binds `v : &!i64` and
  `next : &!^List`, exactly as a struct-field projection under `&!` would (R3).
- **No clause may consume a payload binding.** A payload binding is a reference like any other;
  moving it out (rather than projecting through it or feeding it to `@`/`!`/`+!`) is a located
  error, the same rule R4 already applies to a fetched/stored `T`.
- **A single clause's payload bindings are exempt from R7 (round 2 B1/B2, Decision B).** A
  clause binds every field of one variant **simultaneously**, in one dispatch step, not by
  repeated reborrow-then-project — there is no root local name to reborrow from at all, since
  clause-style words have no word-entry `| … |` and the scrutinee is an anonymous input. Round 2
  found this is a genuine fan-out that neither R5/R6's suspend rule nor R7's disjointness rule,
  as stated for the reborrow-and-project case, has a rule for: `Cons`'s `v : &!i64` and
  `next : &!^List` are two simultaneously live `&!` rooted at one referent, which R7 would
  otherwise conservatively reject. The exemption is narrow and stated explicitly, not a silent
  gap: the fields bound by one clause are **statically disjoint** fields of one variant (the
  checker knows the full field layout of `Cons` at the point it binds `v`/`next`, the same static
  knowledge R7 defers exploiting for the *general* projection case), so binding them all as
  simultaneously-live references is sound by construction, and is exempted from R7 on exactly
  that basis — disjointness that is *statically known*, not merely asserted. This is a principled,
  narrow carve-out for one dispatch mechanism, not a hole in R7's general conservatism; R7 itself
  is unchanged for the reborrow-and-project case everything else in this spec uses.

This resolves the first draft's flagship gap (round 1 soundness B7): `: walk ( &!List -- ) ...
walk ;` is repeatedly the motivating example for R9's back-edge rule, but clause-style
elimination required the top input to *be* an enum, and a reference is not one, so there was no
surface syntax for the case the whole feature exists to serve. Under R17 the motivating program
is now:

```forth
: walk ( &!List -- )
  | Nil
  | Cons | v next |
      v 1 +!
      next &!^ walk
  ;
```

`v`'s reborrow is fully consumed by `+!` before `next` is named, and `next &!^` derives a
`&!List` whose provenance traces back to `walk`'s own parameter (R9's ancestor-frame case), so
the recursive `walk` call is a legal back-edge exactly as it would be for a struct.

**Round 2 B3: the scrutinee reference slot is consumed at dispatch, not left as a surplus
value.** Value-mode clause elimination removes the scrutinee from the stack before binding
payloads (`stack_below = params[..params.len() - 1]`, src/ir.rs:2679); reference mode does the
same with the reference value, so every clause body of `walk ( &!List -- )` starts from `[]`
below its own payload bindings and the word's declared `( &!List -- )` is checked exactly as
value mode's `( List -- )` would be — if reference mode instead left the scrutinee reference on
the stack through every clause, the surplus-value check (R8) would fire at every clause exit
(the measured shape for a leftover value is "body leaves 1 values, but ( … ) declares 0
outputs"), which is not what R17 specifies. Consuming the scrutinee reference is consistent with
R3 ("projection... consumes its reference argument") and with dispatch being, mechanically, one
projection per payload field off the same reference operand.

**Round 2 B4: `lower_clauses`'s `EnumId` must be threaded from the checked frontend type, not
re-derived from the lowered scrutinee's `IrType`.** `lower_clauses` currently derives the
`EnumId` via `self.value_type(scrutinee)` with `_ => unreachable!("checked: a clause word's top
input is an enum")` (src/ir.rs:2680-2682). Under R12 a `&!List` scrutinee lowers to
`IrType::Ptr`, not `IrType::Enum(id)`, so this `unreachable!` becomes a **reachable panic** the
first time reference-mode dispatch reaches this function, unless the `EnumId` is threaded down
from the already-checked frontend `Type` (which knows the concrete enum regardless of whether it
arrived by value or by reference) rather than re-derived from the lowered IR value. This is
added to phase 3's changes list below (R17's own phase) as required lowering work, not left as
an implicit consequence of "no new IR instruction".

**Round 2 B4 (minor): a one-variant enum's dispatch never reads the tag, so a golden asserting
the tag `FieldLoad` needs at least two variants.** `dispatch_on_tag` short-circuits to a bare
`Jmp` with no tag read when the enum has exactly one variant (src/ir.rs:2436). `List`
(`examples/list.sth`) has two (`Nil`/`Cons`), so `walk`'s own golden is not vacuous on this
axis, but the phase-3 test author should not construct a *new*, minimal one-variant enum for a
tag-read assertion and expect it to observe a `FieldLoad` — noted here so the gap is not
rediscovered as a test failure.

The mode follows the declared scrutinee type, which is explicit in the signature (`&!List` vs
`List`), so choosing reference mode is never implicit or type-directed in a way that would
violate D4's "reference-ness explicit in the spelling" rule — the spelling that is explicit here
is the *signature*, and clause syntax itself is unchanged either way.

### Test discipline (binding)

**R18 — Every criterion is a runnable golden**, source in to expected stdout or source in to
expected diagnostic, with **one** reasoned exception: R13 (no-rebuild, criterion 6) asserts on
the emitted module, since a runtime golden cannot distinguish "mutated in place" from "rebuilt
correctly", and eliminating the rebuild is the point of the slice. **N-2 (round 3 audit): R12's
`IrType::Ptr` mapping is not a second exception — it is directly asserted, and the prior draft's
claim that it is only "exercised indirectly" contradicted its own criterion table.** Criterion
13 (`mutation_through_reference_parameter_is_visible_to_caller`) exists precisely to assert the
mapping on its own: a wrong mapping (`&!T` lowered to a by-value aggregate rather than
`IrType::Ptr`) would make a callee's mutation invisible to the caller, and criterion 13's own
function-body assertion over the emitted IL is what would actually break if the mapping
regressed — there is no second, softer criterion standing in for it. Both structural criteria
(6, 13) are unit tests over `backend::qbe::emit`'s output and must assert against a single
named function body (via `func_body`, mirroring the existing `emitted_alloc_shim_has_null_trap`
pattern), never a whole-module IL string match. New lexer/parser/check/ir code carries its own
unit tests beside it (`#[cfg(test)] mod tests`) in addition to the goldens listed below, per
CLAUDE.md's existing convention.

**R19 — Every diagnostic criterion asserts the specific error**, not merely that compilation
failed. Turning silent failure into a sharp error is the point, so the error text and its
location are part of the spec.

**R20 — The reference feature itself is purely additive**, and changes no existing signature's
meaning (R14). Demonstrated, not asserted, by a concrete mechanism: delivery phase 3 runs
`git diff --name-status a66c47a -- examples/ tests/phase0.rs tests/phase1.rs` and asserts every
line is an addition (`A`), never a modification (`M`), of a pre-existing file — an added file
(the dogfood, this slice's own new test file) is fine, an edited one is the regression this
exists to catch.

**This is now the whole claim, not one of two (fifth draft: delivery phase 4 is cut).** Earlier
drafts split this into two claims — the feature's own additive property, closing at phase 3, and
a separate, scheduled `get`/`set` migration-and-removal (Amendment B, delivery phase 4) whose
expected non-additive diff had to be kept from contaminating the first claim. That split is now
unnecessary: this revision cuts the `get`/`set` migration outright rather than scheduling it as
a fourth delivery phase of this slice, or leaving it behind a stated fallback (R21 below gives
the reason, and it is not the one earlier drafts gave). `get`/`set` stay exactly as they are, in
every phase this slice delivers; only their documentation changes. There is no second migration
to keep distinct from the first any more, so there is only one claim, and it is exactly what the
git-diff check above demonstrates.

**Round 2 (minor G2), now moot.** That finding worried that `regression_diff_shows_only_additions`
would need retiring once a delivery phase 4 modified `examples/stack.sth`/`vm.sth` and
`tests/phase{0,1}.rs`. With phase 4 cut from this slice entirely — not deferred behind a
fallback, cut — the concern does not arise here: nothing this slice delivers ever modifies a
pre-existing file, so the test stays green indefinitely as part of this slice. It resurfaces
only when some later slice actually performs the `get`/`set` migration (R21), at which point
that slice's own commit is where the retirement belongs, not this one's.

### Superseded vocabulary (Amendment B)

**R21 — `get` and `set` are superseded by `&!> @` (or `&> @` for a read-only borrow) and
`&!> !`.** Not renamed, not changed, anywhere in this slice (R3): marked superseded here, with
their replacements documented, and their migration and removal **explicitly deferred to a later
slice**, not scheduled as a delivery phase of this one. The case for supersession:

- `get ( [T N] usize -- [T N] T )` is non-consuming and two-output because Slice 1 gave it no
  other way to leave the array live; every call site that only wants to read one element pays
  for it with an immediate `swap drop` to discard the re-pushed array. `examples/vm.sth:58`
  (`vm Vm>prog vm Vm>pc get swap drop`) and `:95` (`vm vm Vm>mem addr get swap drop`) are the
  measured cost: two words of pure plumbing at every read. `&> @` (borrow the array once,
  project to the element, fetch) reads the same value with no re-pushed array to discard,
  because the reference the read consumes is a narrower reference, not the array itself.
- `set ( [T N] usize T -- [T N] )` writes by taking the whole array and handing back a whole
  new one — functionally correct, and exactly the rebuild-per-mutation cost R13 exists to
  eliminate for structs, just for arrays instead. `&!> !` (borrow the array mutably, project to
  the element, store) mutates the one element in place.
- **Net vocabulary shrinks: true only once Decision D lands (round 2 G1).** Round 2 found this
  bullet false as first drafted: `examples/vm.sth`'s `prog`/`build` arrays hold `Op`, an
  all-scalar-payload enum and therefore a `Copy` *aggregate*, and the second draft's R4
  restricted `@`/`!` to Copy *scalar* `T` only — so `&>`/`&!>` composed with `@`/`!` were
  strictly *less* expressive than `get`/`set`, which read/write any Copy element including an
  aggregate one, and `get`/`set` could not in fact be retired. Decision D lifts exactly this
  restriction (R4 above): `@`/`!` on a Copy aggregate now lower via `Alloc`+`Blit`/`Blit` alone,
  so `&>`/`&!>` composed with `@`/`!` are no longer a narrower tool than `get`/`set` for any
  array this codebase has, and the vocabulary genuinely shrinks: two words with an awkward arity
  (`get`'s two outputs, `set`'s whole-array threading) collapse into compositions of the same
  primitives (`&>`/`&!>`, `@`, `!`) every other accessor in this slice already uses. This is the
  same argument R13 makes for structs, applied to arrays a slice late because arrays predate
  references.

`fill` (constructs an array from a Copy element and a count) and `len` (reads the compile-time
constant size) have no reference-mode replacement to be superseded by — neither reads nor writes
a single element — and stay untouched, not merely deferred; R21 names only `get`/`set`.

**Why the migration is deferred, and it is not the reason earlier drafts gave.** Earlier drafts
treated this as a scope/risk tradeoff (`examples/vm.sth`'s `build` needing restructuring to bind
its array as a local before `&!> !` has a place to project from, R15 declining to relax
top-of-scope binding). That restructuring is real, but it is not what actually blocks the
migration. **A bare REPL line has no locals at all** (`Ctx::Line { .. } => None`,
src/check.rs:285 and :306): R2 makes a local the only place a borrow can be taken from, so a
REPL line can never form a place, never take a borrow, and never use `&>`/`&!>`/`@`/`!` — the
entire replacement vocabulary — full stop. Verified empirically: `0 4 fill | a | ...` typed as a
bare REPL line is `parse error: unexpected token Pipe`, while the identical body typed inside a
`:` word definition at the REPL compiles cleanly. Removing `get`/`set` today would therefore make
**array element access impossible at REPL line scope** — not merely awkward, unreachable.
`tests/phase1.rs:585-591` is a live REPL golden whose stated purpose is exercising
`fill`/`get`/`set`/`len` at REPL scope (the VM dogfood driven entirely from REPL lines);
deleting `get`/`set` with no replacement reachable from a bare line would delete that capability,
not migrate it.

This is exactly the blocker the newly-inserted ROADMAP.md general-locals slice (this slice's own
prerequisite, see "Prerequisite" below) removes: once a REPL line can bind `| names |` the way a
word body already can, a line can form a place, take a borrow, and use `&>`/`&!>`/`@`/`!` like
any other scope, and the migration this section documents becomes possible for the first time.
That is why the migration is **deferred, not abandoned**: it has a concrete unblocking event, a
later slice's own exit criterion, rather than a vague "maybe later".

### Prerequisite: the general-locals slice (ROADMAP.md, Phase 3 Slice 5)

This slice now has a real prerequisite recorded in ROADMAP.md: **general locals** (mid-body
`| names |` binding, plus locals at the REPL line), inserted immediately before this slice and
renumbering it from Slice 5 to Slice 6. **This spec does not assume it lands first, and the
dogfood below still targets the current language** (top-of-scope-only binding, R15, unchanged)
— but once mid-body binding exists, three things in this document get simpler, recorded here so
they are not rediscovered independently later:

- **The dogfood's `run` helper disappears.** `run`'s only reason to exist is to give `main`'s
  two `Buf` values a binding site (R15, round-2 A1); `main` itself declares no inputs, so it
  cannot bind locals under today's entry-only rule. With mid-body binding, `main` can bind `a`
  and `b` directly wherever `new new` leaves them, and `run` folds back into `main`.
- **`push-byte` can name its intermediate projection instead of re-deriving it.** `push-byte`
  currently reborrows `b` three times (R7's one-`swap` cost included) because a projection's
  result cannot be named where it is produced. Mid-body binding lets the array reference
  produced by `Buf&!>data &!^` be named once and reused, rather than reborrowing `b` a third
  time to reach `Buf&!>len` again.
- **R7's disjointness workaround stops needing its extra `swap`.** The `swap` in `push-byte`
  exists to sequence two projections of `b` so only one is ever live at a time (R7 is
  conservative about disjoint fields). Naming each projection's result as it is produced removes
  the need to reorder the stack to keep them sequenced.

None of this is implemented here; it is recorded so the general-locals slice's own brief does
not have to rediscover why it matters to this one.

## Open question: aggregate-local aliasing (not resolved this revision)

Round 3 found, independently in two places, that **naming an aggregate local does not copy
it**: `lower_call` pushes the *same* `Value` — a pointer to one frame slot — when a local is
named ("i64 is Copy; reuse the value id", src/ir.rs:1709), including for a struct/array/enum
local, while `dup` deep-copies via `Alloc`+`Blit` (src/ir.rs:1714-1726, this revision's own
Decision D justification above). Independently, a non-consuming aggregate projection
(`S|>fi`'s `Peek`, src/ir.rs:2378-2386; `get` on an array element, src/ir.rs:2036-2039) pushes
the **interior address, with no copy**, on the stated justification that "the owning aggregate
is consumed by the getter/destructure/clause" — which is false for a non-consuming peek. Either
way, **two distinct locals can denote one region of memory** today. This is pre-existing and
currently invisible, because nothing mutates in place; this slice's `!`/`+!` make it observable
for the first time, so this slice is where the question has to be decided, not merely noted:

```forth
type: V x i64 y i64 ;  type: S a V b i64 ;
: f ( V V -- ) | p q | p V> . . q V> . . ;
: main ( -- )
  1 2 V 3 S
  S|>a swap S|>a swap drop
  f ;
```

verified on this commit to print `2 1 2 1`: `p` and `q` are two aliases of one `V`, and
mutating through one after this slice's `!` lands would be observed through the other, with no
rule in R5/R6 noticing, since neither `p` nor `q` was ever borrowed from a *place* — they are
two plain values that happen to share one address.

**R5's claim to be "the entire aliasing rule" does not hold until this is settled** (see the
N-9 note at R5, above): R5 governs borrows taken from places; this hole is about two aggregate
*values* sharing an address with no borrow ever taken. Three candidate resolutions are recorded,
none chosen:

1. **Naming an aggregate local materialises a copy.** Closes the hole at the point of naming,
   at the cost of a real, performance-visible `Alloc`+`Blit` every time an aggregate local is
   named — a cost this slice otherwise works hard to avoid (R13).
2. **R5 extends to track outstanding aggregate copies of a place**, not just borrows of one, so
   `p`/`q` above would be rejected as two live aliases of the same place the moment both are
   named. More machinery than R5 as currently stated, and its interaction with `dup` (which
   *does* copy) needs working out.
3. **Borrow roots are restricted** so an aliasable local (one that arrived by a non-consuming
   peek of another place, rather than being bound at word entry from the stack) cannot be a
   borrow root at all — narrower than either of the above, and it only closes the hole where a
   reference is later taken, not the aliasing itself.

This question **gates implementation**: phase 1 cannot ship R4's `!`/`+!` without an answer,
since they are exactly what makes the aliasing observable. Recorded here, explicitly undecided,
rather than silently shipped alongside R5 as if it were already covered.

## Load-bearing invariants (must survive)

- Backend stays QBE; no LLVM. `Ptr[T]` stays opaque, never assumed to be a `u64`. R12 adds no
  instruction, maps every reference to the existing `IrType::Ptr`, and R2 exposes no pointer
  arithmetic, so a WASM lowering stays possible.
- The linear spine holds: exactly-once, no auto-drop, forgetting is a compile error. References
  do not weaken it, because they never own: R4's Copy restriction on `@`/`!`/`+!` (now covering
  a Copy aggregate as well as a Copy scalar, Decision D) is what stops a borrow from
  manufacturing a second owner or leaking an overwritten one — the restriction that matters is
  Copy-vs-linear, not scalar-vs-aggregate — and R8 stops a reference outliving its referent.
  `&!T`'s own disposal (neither `Copy` nor linear) is stated explicitly in R8 rather than left
  to fall through the existing two categories silently.
- `core` stays `no_std`. No in-process JIT, no comptime interpreter.
- **The Slice 2 tripwire is acknowledged as tripped, deliberately, not argued away (D5).**
  docs/phase3-slice2-spec.md:9 reads "Second ad-hoc constructor after arrays. A third is the
  signal to switch to Phase 4 generics instead." `&T` and `&!T` are the third and fourth. The
  first draft's invariants section claimed otherwise ("not a third `^`-style payload-interned
  constructor... no registry entry"), which does not survive scrutiny: `Type` is `#[derive(...,
  Copy)]` (src/ast.rs:306), which is exactly why `Array` and `OwnedCell` are `(Id, &'static str)`
  pairs backed by an interned registry rather than a boxed payload, and `&T`/`&!T` will need the
  identical treatment (a `RefId`, an interned `(inner, mutable)` registry) the moment a
  parameterized reference type needs to render its own name — the *only* things genuinely absent
  are ownership, allocation, and a destructor, which is a real difference but not the tripwire's
  actual criterion. The sequencing argument, stated honestly instead: references are needed now,
  in Phase 3, and generics are Phase 4; Phase 4's planned ad-hoc dispatch (ROADMAP.md:504-508;
  round 2's citation audit (G4) corrected 488-490 to 488-492, round 3's audit (N-6) corrected
  that to 488-492 exactly, and this revision's own ROADMAP.md edits — Amendment A's one-line
  Slice title shift plus this draft's 16-line general-locals insertion — shift it a further 16
  lines to 504-508, re-verified against the current file rather than carried forward by
  arithmetic) — static overloading over statically-known input types, plus
  open multimethods) is expected to
  eventually subsume both the reference type constructors themselves and R3's explicit
  reference-mode accessor spellings, once a word can be overloaded on whether its receiver is
  `T`, `&T`, or `&!T` rather than needing a distinct name per case. That expectation is recorded
  here as a **revisit trigger**: when Phase 4's dispatch work lands, re-examine whether `&`/`&!`
  and the `T&>fi`/`T&!>fi`/`&>`/`&!>`/`&^`/`&!^` family should collapse into overloads of
  `S>fi`/`get`/`^|>`.

## DESIGN.md amendment (D2)

**Already applied, in this same spec revision** (not deferred to a phase-3 implementer): the
inconsistency this section resolves was found against documentation, not code, so nothing
blocks fixing it immediately rather than waiting on phase 1-3's checker/IR work. DESIGN.md:134
and DESIGN.md:208-214 (line numbers as of the base commit, before this edit) were amended to
distinguish what stays ruled out (a
*lifetime* system: lifetime variables, region annotations, anything binding a reference's
validity to a named scope) from what this slice adds (a per-place aliasing rule checked at
consumption points, with no lifetime tracking of any kind). The amendment is narrow and stays in
the surrounding prose's existing voice:

- Line 134, "There is no borrow checker." became "There is no lifetime-tracking borrow
  checker." — a two-word qualification, not a rewrite; the passage it introduces (resource
  hand-back via ordinary stack threading) needs no borrow checker of any kind and is unaffected.
- The "References are second-class" paragraph (base-commit lines 208-214, now 208-221 after the
  amendment) gained several sentences distinguishing the ruled-out *lifetime* apparatus from the
  narrower exclusivity rule this slice adds, so the paragraph engages Phase 3 Slice 6 directly
  instead of reading as flatly contradicted by it. Phase 1-3's checker/IR work (below) proceeds
  against this already-amended DESIGN.md; no phase needs to touch it again.

## Delivery phases

1. **Reference types, places, projection, access, and every escape/root rejection needed for
   this phase's lowering to be total.** `&T`/`&!T` in the type system, including a `RefId`
   registry (an interned `(inner, mutable)` pair, mirroring `Array`/`OwnedCell`, round 2 E1) and
   the two soundness answers (`is_copy` true for `&T`/false for `&!T`; every `is_linear`-shaped
   predicate false for both) rather than leaving them as unstated consequences of "`ir_type_of`
   gains two arms"; `&`/`&!` as prefix borrow operators on a local (R2, Decision E), with the `^`-style
   name reservation mirrored for `&`-led names and a dedicated shadowing rejection for the
   exact names `@`/`!`/`+!` (round 2 F2); the type-position splitter for `&T`/`&!T`, its three
   cases including handing a `^`-led remainder to the existing caret splitter (round 2 F3); the
   `T&>fi`/`T&!>fi`/`&>`/`&!>`/`&^`/`&!^` accessor family, one spelling per shape *per
   mutability* (Decision A), projecting through a reference and suspending its root place for
   the mutable forms (R3, Decision B); `@`/`!`/`+!` with R4's Copy restriction, now covering a
   Copy aggregate via `Alloc`+`Blit`/`Blit` as well as a Copy scalar (Decision D), typing `@`
   for both `&T` and `&!T`; R11's scalar-local rejection; R8's six transitive-containment
   rejections (struct field, enum payload, array element via `fill`, cell payload via `^`,
   effect output, and a reference surviving to the end of a REPL line, round 2 D4) paired with
   the input-side accept-case narrowed to a top-level reference type (round 2 D3); R8's
   reference-`drop`-is-a-no-op rule; R12's lowering, including the `&T`/`&!T` -> `IrType::Ptr`
   mapping. Checking here is types plus these specific soundness rules, not yet the
   borrow-conflict machinery (R5-R7, R9-R10) — those are phase 2/3. **Round 1 (criteria E3)
   moved R11, R8's rejections, and the drop-no-op rule here from a later phase**: without them,
   phase 1's lowering has cases with nothing to lower to (an unaddressable scalar local) or a
   silent soundness hole a later phase's diagnostics would only report on, never prevent, at
   this commit. State explicitly in the phase-1 commit message: **at this commit,
   R5/R6/R7/R9/R10 do not exist yet, so a program using two conflicting borrows, or a borrow
   crossing a back-edge unsafely, is *accepted* by the phase-1 compiler.** That is deliberate
   and temporary, not an oversight to be rediscovered at review.
   Exit: criteria 1 through 6, 13, and 15 (accept/reject at the type level; escape at the six
   positions; drop-as-no-op; the accessor family; `push-byte`/`byte-at` compile, run, and
   produce the right bytes; `push-byte`'s emitted body contains no `alloc`/`blit` while a
   rebuild-style control word in the same test module still does; a callee's mutation through a
   `&!` parameter is visible to the caller).
2. **The borrow rules and their diagnostics.** R5 exclusivity in both symmetric directions
   (Decision C: `&` while a `&!` is live, and `&!` while a `&` is live, each its own test, plus
   the different-places accept-case), R6's consumption-point scan keyed on a place's outstanding
   derivations rather than literal `Value` identity (Decision B, so a reborrow taken while a
   projection derived from the *previous* reborrow is still live is rejected, over both the
   stack and the locals map, and over the moved/dropped/conflicting-borrow trio of consumption
   points), R7's disjointness rejection and its sequenced-workaround accept-case. Every
   rejection lands with its located error and its diagnostic golden.
   Exit: criteria 7 through 9.
3. **Loops, joins, reference-mode enum elimination, the full dogfood, and the documentation
   corrections.** R9's back-edge rules from both sides; R10's join rule with both the
   disagreement and agreement accept-case; R17's reference-mode clause elimination (typing was
   phase 1's accessor-family work in spirit, but the back-edge interaction that makes it worth
   having is exercised here), including threading the scrutinee's `EnumId` from the checked
   frontend `Type` into `lower_clauses` rather than re-deriving it from the lowered `IrType`
   (round 2 B4, closing the reachable `unreachable!` a `&!Enum` scrutinee would otherwise hit),
   the R7 exemption for one clause's statically-disjoint payload bindings (round 2 B1/B2), and
   consuming the scrutinee reference at dispatch rather than leaving it a surplus value (round 2
   B3); the full dogfood end to end including `walk`; recording R16's answer into ROADMAP.md's
   parked design question (the DESIGN.md:134/208-214 amendment, D2, and ROADMAP.md:454-459's
   title/body correction, Amendment A, are already applied and need no further phase-3 work —
   only the design-question passage at ROADMAP.md:460-466 still needs R16's answer written into
   it); R20's additive-work regression check.
   Exit: criteria 10, 11, 12, 14, and 16.

**This slice ends at phase 3. There is no delivery phase 4.** Earlier drafts scheduled the
`get`/`set` migration and removal here as a fourth phase (Amendment B), with a stated fallback
if it ran long. This revision cuts it outright rather than carrying a fallback: R21 explains why
it cannot even be attempted yet (a bare REPL line has no locals to form a place with, so the
entire replacement vocabulary is unreachable at REPL scope), which is a harder blocker than the
scope/risk tradeoff (`examples/vm.sth`'s `build` needing restructuring) earlier drafts gave, and
no amount of care within *this* slice removes it. The migration is recorded (R21) and left for
the slice that removes the actual blocker (ROADMAP.md's general-locals slice, this slice's own
prerequisite).

## Criterion → test map

**Goldens for this slice live in a new file, `tests/phase3_refs.rs`, not `tests/phase0.rs`
(round 3 criteria A1).** Criterion 16 below asserts `tests/phase0.rs` is never modified from the
base commit; adding a golden to it would make that assertion false by the same commit that adds
the golden, so every new runtime test in this slice lives in its own file instead, which the
git-diff watch list (examples/, tests/phase0.rs, tests/phase1.rs) does not track at all — an
addition invisible to the check is as good as one the check explicitly allows. The two
**structural** criteria (**6, 13**), which assert on emitted code, belong in unit tests beside
`backend/qbe.rs` instead, mirroring the existing `emitted_alloc_shim_has_null_trap` pattern.
R17's typing-only criterion (12) may live beside R17's own checker code if no runtime
observation is needed for the accept half.

| # | criterion | test | phase |
|---|---|---|---|
| 1 | `&`/`&!` on a local yield reference-typed values; applied to a literal, an arithmetic result, or a word result they are located errors | `borrow_of_place_is_accepted`, `borrow_of_non_place_is_error` | 1 |
| 2 | borrowing a scalar local is a located error; borrowing a scalar *field* through a projection (the field, not the local, is scalar) is accepted | `borrow_of_scalar_local_is_error`, `borrow_of_scalar_field_is_accepted` | 1 |
| 3 | projection reads correctly through all three shapes with the correct spelling per mutability: struct field (`T&>fi`/`T&!>fi`); array element, incl. the bounds trap (`&>`/`&!>`); cell payload (`&^`/`&!^`); storing through the *shared*-spelled projection (`&^`/`&>`/`T&>fi`'s result) is a located error | `projection_through_field_element_and_cell_reads_correctly`, `element_projection_out_of_bounds_still_traps`, `store_through_shared_reference_is_error` | 1 |
| 4 | `@`, `!`, `+!` read/write/increment through a reference; `@`/`!` on a linear `T` are located errors; `@`/`!` on a `Copy` **aggregate** `T` (Decision D) read/write correctly via `Alloc`+`Blit`/`Blit`, not a panic and not an error; the fetched copy is independent of its referent, proven by mutating the source after the fetch | `access_through_reference_reads_and_writes`, `increment_through_mutable_reference_adds_in_place`, `fetch_or_store_of_linear_referent_is_error`, `fetch_or_store_of_copy_aggregate_reads_and_writes`, `fetch_of_copy_aggregate_survives_source_mutation` | 1 |
| 5 | escape: a reference in a struct field, an enum variant payload, an array element (`fill`), a cell payload (`^`), on an effect's output side, and (D4) surviving to the end of a REPL line, are six located errors; a reference on an effect's *input* side is accepted; `drop` of a reference frees nothing | `reference_in_struct_field_is_error`, `reference_in_enum_payload_is_error`, `reference_as_array_element_is_error`, `reference_in_cell_payload_is_error`, `reference_returned_from_word_is_error`, `reference_surviving_repl_line_is_error`, `reference_in_effect_input_is_accepted`, `drop_of_reference_frees_nothing` | 1 |
| 6 | **structural**: the emitted body of `push-byte` contains no `alloc` and no `blit` and does contain the address-arithmetic-plus-store shape, under an instruction-count ceiling; a rebuild-style control word in the same module still contains `alloc`/`blit`, proving the assertion is not vacuous. Pinned to the mangled symbol (`qbe_name` rewrites `-` to `_`, src/backend/qbe.rs:186): assert against `func_body(&il, "export function $push_byte(")` (the header form, src/backend/qbe.rs:578), never `"push-byte"` literally, which cannot match and would make `func_body` panic rather than assert (round 3 criteria A3) | `mutation_through_reference_emits_no_rebuild`, `rebuild_style_equivalent_still_emits_alloc_and_blit` | 1 |
| 7 | exclusivity, both directions (Decision C): two live `&!` to *one* place, a `&` taken while a `&!` to it is live, a `&!` taken while a `&` to it is live, and `dup` on a `&!` are four located errors; a reborrow taken while a reference *derived by projection* from the previous reborrow is still live is a located error (Decision B, the `two-live` shape); two live `&!` to *different* places is accepted; `&` is `Copy` (names twice, accepted) and naming a `&!` local twice, once the prior derivation is fully consumed, is accepted as a reborrow | `two_live_mutable_borrows_is_error`, `shared_borrow_while_mutable_live_is_error`, `mutable_borrow_while_shared_live_is_error`, `reborrow_while_projected_reference_still_live_is_error`, `dup_of_mutable_reference_is_error`, `two_live_mutable_borrows_to_different_places_is_accepted`, `shared_reference_is_copy`, `naming_mutable_reference_local_reborrows` | 2 |
| 8 | consuming a place while a borrow of it is live is a located error naming both the place and the borrow, whether the conflicting borrow sits on the virtual stack or in the locals map; disposing a borrowed place is likewise a located error; the same place consumed, or disposed, *after* its borrow is gone is accepted | `move_of_place_borrowed_on_stack_is_error`, `move_of_place_borrowed_in_locals_is_error`, `dispose_of_borrowed_place_is_error`, `move_after_borrow_ends_is_accepted` | 2 |
| 9 | two references into disjoint fields of one place, held simultaneously, are rejected (stated limitation); sequencing them (fully consuming the first before taking the second) is accepted | `disjoint_field_borrows_are_conservatively_rejected`, `sequenced_borrows_of_two_fields_are_accepted` | 2 |
| 10 | a reference parameter crosses a self-tail-call back-edge and mutates in constant stack over 1,000,000 nodes; a reference to a current-scope local crossing a back-edge, and a currently-borrowed local being loop-carried, are two located errors | `reference_parameter_crosses_back_edge_in_constant_stack`, `reference_to_local_across_back_edge_is_error`, `borrowed_local_carried_across_back_edge_is_error` | 3 |
| 11 | a borrow live on one arm of an `if` and not the other is a located error at the join; a borrow live on both arms, or on neither, joins cleanly | `borrow_on_one_arm_only_is_error`, `borrow_live_on_both_arms_is_accepted` | 3 |
| 12 | reference-mode clause elimination: a word whose declared top input is `&Enum`/`&!Enum` may dispatch clause-style; a clause's payload bindings are references inheriting the scrutinee's mutability and may be simultaneously live (the statically-disjoint-fields exemption from R7, round 2 B1/B2), not rejected as a fan-out; a clause body that consumes (moves out) a payload binding is a located error | `reference_mode_clause_binds_payload_as_reference`, `reference_mode_clause_consuming_payload_is_error` | 3 (typing groundwork in 1, exercised end to end here) |
| 13 | **structural**: a mutation a callee makes through a `&!` parameter is visible in the caller (proves `&!T` lowers to `IrType::Ptr`, not a by-value aggregate) | `mutation_through_reference_parameter_is_visible_to_caller` | 1 |
| 14 | the dogfood runs end to end and prints the expected byte, including the two-borrow `copy-byte` call and the `walk` word over `&!List` | `reference_dogfood_prints_expected_bytes` | 3 |
| 15 | a leftover reference on the *stack* without a `drop` is a surplus-value error; a reference *local* that is never explicitly dropped is accepted (it expires silently at scope end) | `unused_reference_is_surplus_value_error`, `reference_local_expires_without_drop` | 1 |
| 16 | no regression: the full existing suite passes, and `git diff --name-status a66c47a -- examples/ tests/phase0.rs tests/phase1.rs` shows only additions, no modifications, demonstrating R14/R20's additive-work claim concretely rather than by assertion | existing suite, unmodified; `regression_diff_shows_only_additions` | 3 |

**There is no criterion 17.** The prior draft's criterion 17 (`get`/`set` removal) belonged to
the delivery phase 4 this revision cuts (R20/R21); it is not renumbered away, simply not part of
this slice's exit any more. 16 criteria, phases 1 through 3, is the whole of this slice's exit.

## Dogfood, as this revision specifies it

The brief carries the original dogfood source, and the brief is edited in this revision only
for its three stale `ROADMAP.md` citations (two instances of line 452, one of line 447, both
corrected per B1 above; per the pipeline's scope for a citation-only pass) — it is **not**
updated for D3/D4's or this round's Decision A accessor spellings, or for round 2's A1/A2
fixes, so its literal source is now stale in three ways (`^&`, plain `Buf>data`/`get` applied
to a reference, `new`/`dispose`; the second-draft accessor spellings; and `main`'s mid-body
`| a b |` and bare `i64` literals). This section is the authoritative, current version; an
implementer works from here, not from the brief's literal code.

**Round 2 A1 (blocker): the previous revision's `main` does not compile, and this revision
restructures it rather than patching the trace around the bug.** `main ( -- ) new new | a b |
...` binds locals *mid-body*, against zero declared inputs — R15 states plainly that locals
bind only at word entry against declared inputs, and the prior revision's own hand-trace then
asserted the opposite of what it had just stated (measured: `new new | a b | ...` produces
`parse error: unexpected token Pipe`, exactly as the recon section's own scalar-local probe
already demonstrated for a simpler program). The fix is the one R15 already implies: a helper
word that declares the two buffers as inputs. `main` becomes a two-line caller; `run` carries
the body and the local binding, now legal because `run` declares exactly two inputs and binds
exactly two locals.

**Round 2 A2 (blocker): `72`/`90` are `i64` literals where `push-byte` declares `u8`, and
need `>u8`.** D8's bare-literal coercion is gated on `is_size_type` — `usize`/`isize` only — so
there is no `i64`-to-`u8` literal coercion (measured: `72 takeu8` against a `( u8 -- )` word
is a located type-mismatch error). `new`'s own body already writes `0 >u8` correctly, which is
what makes the previous revision's two bare literals an inconsistency inside the same dogfood
rather than a new problem; both call sites below now write `72 >u8`/`90 >u8`. `byte-at`'s
index argument is unaffected: `usize` is on the bare-literal coercion carve-out, so `&a 2
byte-at` needs no conversion (verified: a bare literal against a declared `usize` parameter
type-checks with no coercion word).

```forth
type: Buf  data ^[u8 64]  len usize ;

: new ( -- Buf )
  0 >u8 64 fill ^ 0 >usize Buf ;

: push-byte ( &!Buf u8 -- )
  | b x |
  b Buf&!>len @              \ ( -- usize )
  b Buf&!>data &!^ swap      \ ( -- &![u8 64] usize )
  &!> x !                    \ ( -- ), stores x through the derived &!u8
  b Buf&!>len 1 +! ;

: byte-at ( &Buf usize -- u8 )
  | b i |
  b Buf&>data &^ i &> @ ;

: copy-byte ( &!Buf &Buf usize -- )
  | dst src i |
  dst src i byte-at push-byte ;

: run ( Buf Buf -- )
  | a b |
  &!a 72 >u8 push-byte
  &!b 90 >u8 push-byte
  &!a &b 0 copy-byte
  &a 2 byte-at .
  a drop
  b drop ;

: main ( -- )
  new new run ;
```

And R17's motivating case, over the existing `List` (`examples/list.sth`):

```forth
: walk ( &!List -- )
  | Nil
  | Cons | v next |
      v 1 +!
      next &!^ walk
  ;
```

### Hand-trace of `push-byte`, `byte-at`, `copy-byte`, `run`, and `main`

`push-byte ( &!Buf u8 -- )`, `b : &!Buf`, `x : u8`:

| term | stack after |
|---|---|
| `b` (reborrow, R5) | `[&!Buf]` |
| `Buf&!>len` (consumes the reborrow, R3) | `[&!usize]` |
| `@` (R4, typed for `&!T`) | `[usize]` |
| `b` (second reborrow — the first's derived `&!usize` was fully consumed by `@` already, so `b`'s place is not suspended, Decision B) | `[usize, &!Buf]` |
| `Buf&!>data` | `[usize, &!^[u8 64]]` |
| `&!^` (R3, mutable cell projection) | `[usize, &![u8 64]]` |
| `swap` (R7's one-`swap` cost) | `[&![u8 64], usize]` |
| `&!>` — `( &![T N] usize -- &!T )` | `[&!u8]` |
| `x` (Copy local, no move) | `[&!u8, u8]` |
| `!` (R4) | `[]` |
| `b` (third reborrow — the second's derived chain was fully consumed by `&!>` then `!` already) | `[&!Buf]` |
| `Buf&!>len` | `[&!usize]` |
| `1` | `[&!usize, i64(1)]` |
| `+!` | `[]` |

Ends `[]`, matching the declared `( &!Buf u8 -- )`. `b` is named three times; at each
subsequent naming, nothing derived from the previous reborrow is still live, so `b`'s place is
never suspended at the point of a new reborrow (Decision B) and R7's disjointness rule never
has to reason about two live derivations from `b` at once (the narrow claim R3/C2 leaves
standing). `b` expires silently at the word's end (R8) with no `drop`.

`byte-at ( &Buf usize -- u8 )`, `b : &Buf`, `i : usize`: `b` (`[&Buf]`) `Buf&>data`
(`[&^[u8 64]]`, shared struct projection) `&^` (`[&[u8 64]]`, shared cell projection) `i`
(`[&[u8 64], usize]`) `&>` (`[&u8]`, shared array-element projection) `@` (`[u8]`). Ends
`[u8]`, matching the declared output. Every projection here is shared, so Decision B's suspend
rule never engages at all (it is mutable-only); the shared family composes exactly as freely as
any other `Copy` value.

`copy-byte ( &!Buf &Buf usize -- )`, locals `dst : &!Buf`, `src : &Buf`, `i : usize`: naming
`dst src i` pushes `[&!Buf, &Buf, usize]`; `byte-at` consumes the top two (`&Buf`, `usize`,
matching its declared input), pushing `[&!Buf, u8]`; `push-byte` consumes both, pushing `[]`.
Ends `[]`, matching the declared output.

`run ( Buf Buf -- )`, locals `a : Buf`, `b : Buf` (R15: `run` declares two inputs and binds
exactly two locals, unlike the previous revision's zero-input `main`): naming `a`/`b` never
moves them outright — `Buf` is linear (its `data` field is a cell), but `&`/`&!` borrow *from*
a place without consuming the local itself (R2), so `a` and `b` remain nameable again later in
the body, up to the point each is finally `drop`ped. `&!a 72 >u8 push-byte` borrows `a` (R2,
R11: `a` is a struct local), converts the literal, calls `push-byte`; `a`'s `data` cell now
holds `72` at index 0 and `len` is `1`. `&!b 90 >u8 push-byte` does the same for `b`: index 0
is `90`, `len` is `1`. `&!a &b 0 copy-byte` borrows `a` mutably and `b` sharedly — different
places, so Decision C's symmetric rule does not fire either direction — and reads `b`'s byte 0
(`90`) into `a` at its current `len` (`1`), so `a`'s `len` becomes `2` and index 1 is `90`.
`&a 2 byte-at .` reads index 2 of `a`, which `new`'s zero-fill left untouched (only indices 0
and 1 were ever written), so it prints `0`. `a drop` and `b drop` dispose both owned buffers
(freeing their `data` cells) — this is the point `a`/`b` are actually consumed, once, matching
the linear rule. Ends `[]`, matching `run`'s declared `( Buf Buf -- )`.

`main ( -- )`: `new new` pushes `[Buf, Buf]`; `run` consumes both, matching its declared input,
leaving `[]`. Ends `[]`, matching `( -- )`; R20's regression check is unaffected since this is
a new file, not an edit to an existing one.

`walk`'s clause-mode dispatch (R17) over `&!List`: the scrutinee reference is consumed by the
dispatch itself (round 2 B3), not left as a surplus value. The `Nil` clause has no payload and
an empty body. The `Cons` clause binds `v : &!i64`, `next : &!^List` simultaneously (mutability
inherited from the `&!List` scrutinee; both live at once is R17's statically-disjoint-fields
exemption from R7, round 2 B1/B2, since `v` and `next` are two distinct, statically known fields
of one `Cons` variant, never aliasing each other). `v` (reborrow) `1` `+!` mutates the node's
value in place and leaves `[]`; `next` (reborrow) `&!^` (mutable cell projection, `T = List`)
yields `&!List` whose provenance traces back to `walk`'s own parameter, so the tail call `walk`
is a legal R9 back-edge. Every node's `v` field is incremented exactly once as the walk
recurses, in constant stack (R9, criterion 10), and `walk` never frees or moves the list it
walks — ownership stays with whoever calls it.

## Explicitly out of scope

`& ( T -- T &T )`, the stack-value borrow form (R2; purely additive, revisit if `examples/`
after Slices 6 and 8 is dominated by build-then-configure pipelines over a single value).
Path-disjoint borrows (R7). Borrowing a scalar local, and therefore the scalar spill (R11).
Mid-body local binding (R15; lands as ROADMAP.md's own general-locals slice, this slice's
prerequisite, not as part of this slice). `!` over a linear value with drop-on-overwrite (R4).
Reified take/fill residuals `∂S/∂fi` (R16). Raw or foreign pointers: `^T` is the owning pointer
and `&T`/`&!T` the borrowing one, the only client for a third is FFI at the hosted layer
(Phase 6), `*` is the multiplication word so it is not the spelling, and any future foreign
pointer must be an opaque handle with no arithmetic, since `p 8 +` would force `Ptr` to be an
integer and break the backend-neutral invariant a WASM lowering depends on. Collapsing `&`/`&!`
and the reference-mode accessor family into overloads of the value-form words (D5's revisit
trigger; waits for Phase 4's ad-hoc dispatch). Reference counting and storable references,
including the zipper (Phase 3 Slice 7). User-definable destructor bodies (Phase 3 Slice 8).
Worklist-based branching disposal (Phase 6). The `get`/`set` migration and removal itself (R21;
deferred to whatever later slice picks it up once general locals land). The aggregate-local
aliasing question above (explicitly gating, not deferred by choice — see that section).

## Outstanding round-3 findings (not applied this revision)

Recorded so they are not lost, not because they are optional. The task that produced this
revision authorized applying a subset of round 3's findings; these are the rest, listed rather
than fixed. Several are now entangled with the open aggregate-aliasing question above and
cannot be resolved independently of it.

- **Vacuous dogfood golden** (r3-soundness #1): the only printed byte (`&a 2 byte-at .`) is
  index 2, which nothing in the dogfood ever writes. Print the bytes the program actually
  produced (index 0, index 1, `len`) instead.
- **`walk` is never called** (r3-soundness #2, criterion 14): the dogfood's `main` never builds
  a `List` or calls `walk`; criterion 14 requires it. Either grow the dogfood a `List`-building
  helper with an observable printed result, or split criterion 14 and give `walk` its own
  golden.
- **Reference locals must be excluded from `Moves`, and no `is_linear` predicate exists**
  (r3-soundness #3): linearity is `!is_copy` at roughly 10 sites with differing required
  answers; `Moves::new` (src/check.rs:198-208) and the back-edge check (src/check.rs:1513-1519)
  both need an explicit reference-local exclusion or `push-byte`'s own second reborrow and R9's
  accept-case are rejected as written.
- **R6's literal provenance predicate over-rejects `push-byte`** (r3-soundness minor): needs
  "reference-typed values only" and "`@` terminates provenance" stated explicitly.
- **R5/R6's exclusivity counter vs. a reference local's own content** (r3-soundness minor):
  state that the counter counts outstanding *derivations*, never the reference the local itself
  holds.
- **R3 never states a projected field/payload type may be linear** (r3-soundness minor):
  `push-byte` needs `Buf&!>data : &!^[u8 64]`, a mutable reference to a linear field; an
  implementer mirroring R4's Copy gate onto R3 rejects it.
- **`over` is a second duplication path with the same false diagnostic as `dup`**
  (r3-soundness minor): both are rejected by the pre-existing Copy gate with a message that
  claims linearity/resource-ownership, which is false for a reference.
- **`@`'s `Alloc` is entry-block-hoisted** (r3-soundness minor): a Copy aggregate fetched by `@`
  inside a self-tail-call loop and carried across the back-edge is clobbered by the next
  iteration's fetch — pre-existing, reproducible with existing words, out of this slice's scope
  to fix but worth a note in R4 so criterion 4's golden is not accidentally written inside a
  loop.
- **`get`/`&> @` behavioural difference should be stated as deliberate** (r3-soundness minor):
  `get` on an aggregate element aliases; `&> @` copies. Record it in R21 rather than leaving a
  future reader to spot the diff unexplained.
- **The REPL escape golden has no writable program** (r3-criteria A2):
  `reference_surviving_repl_line_is_error`'s own justification (a REPL line has no locals)
  means no source text can reach the rejection it names; either drop the golden in favour of a
  `debug_assert`-style note, or make it a white-box unit test against the carried-slot path
  directly.
- **Criterion 10 mostly proves pre-existing TCO** (r3-criteria A4): needs the node count,
  `ulimit -s 1024`, an asserted post-walk value read back from a *mid-list* node, and a
  pre-change falsification note, matching `deep_list_disposes_in_constant_stack`'s own
  convention.
- **Fail-fast checking makes several bundled criteria one-third effective** (r3-criteria A7):
  criteria 1, 3, and 4 each bundle multiple rejections into one test program; only the first
  rejection in each is ever exercised. Split into one program per rejection.
- **`drop_of_reference_frees_nothing` has no stated observation** (r3-criteria A8): pin it to
  the existing alloc-trace/spy machinery (`run_owned_traced_golden`, tests/phase0.rs:2444)
  rather than leaving "program compiles and prints the expected number" as the de facto
  assertion.
- **Several rejections have no accept-case, and two rules have no test at all**
  (r3-criteria B): most pressing, `&`-led name reservation and the `@`/`!`/`+!` shadowing
  rejection ship with zero tests in any phase; `dup` on a `&!` has no accept-case proving `dup`
  on a `&T` still works.
- **Two rules exercised nowhere** (r3-criteria C2): the type-position splitter's third case
  (a `^`-led remainder) is reachable only via R17's inference, never via a hand-written
  signature; add a one-line phase-1 parser unit test.
- **Test-naming sweep** (r3-criteria E): several test names claim a retired form
  (`borrow_of_scalar_field_is_accepted` should be `projection_to_scalar_field_is_accepted`),
  assert an audit they cannot perform, or omit the outcome clause. Not applied this revision.

**Now moot, listed so it is not mistaken for still-open:** round 3's phase-4 `build`
restructuring finding (the A1 mid-body-locals bug repeated in phase 4's `build`, r3-soundness
blocker #5) no longer applies — delivery phase 4 is cut in this revision, so `build` is never
restructured here at all.

## Phases

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "reference-types-places-projection-access",
      "difficulty": "hard",
      "summary": "Add &T/&!T mapped to IrType::Ptr with a RefId registry and correct is_copy/is_linear answers, name reservation for &-led names and shadowing rejection for @/!/+!, the three-case &T/&!T type-position splitter, prefix &/&! on locals with the place-suspend rule for mutable projections, the T&>fi/T&!>fi/&>/&!>/&^/&!^ accessor family split by mutability, @/!/+! typed for both &T and &!T and covering a Copy aggregate via Alloc+Blit as well as a Copy scalar, R11's scalar-local rejection, R8's six escape rejections (five over compiled code plus the REPL carried-stack case) plus drop-as-no-op, and the surplus-value rule for a leftover reference on the stack.",
      "changes": [
        "src/lexer.rs, src/parser.rs: `&` and `&!` as prefix borrow operators on a local; `&T`/`&!T` in type position with its three splitting cases (bare, `^`-composed, `[`-delimited); `T&>fi`, `T&!>fi`, `&>`, `&!>`, `&^`, `&!^`, `@`, `!`, `+!` as words; `&`-led name reservation mirroring `is_reserved_caret_name`/`reserved_caret_name_error`; a shadowing rejection for the exact names `@`, `!`, `+!`",
        "src/check.rs: reference types in the type lattice with an interned RefId (inner, mutable) registry; is_copy true for &T/false for &!T and every is_linear-shaped predicate false for both; R2's local-only place; the T&>fi/T&!>fi/&>/&!>/&^/&!^ accessor family split by mutability, each consuming its reference argument and suspending its root place for the mutable forms; R4's Copy restriction on @/!/+!, @ typed for both &T and &!T, now covering a Copy aggregate as well as a Copy scalar; R11's scalar-local rejection; R8's six transitive-containment rejections (struct field, enum payload, fill's array element, ^'s cell payload, effect output, REPL carried-stack survival) plus the effect-input accept-case narrowed to a top-level reference type; the surplus-value rule treating a leftover &!T on the stack like any non-Copy value while a reference local expires silently",
        "src/ir.rs: ir_type_of gains &T/&!T -> IrType::Ptr; lower T&>fi/T&!>fi to PtrOffset, &>/&!> to ElemAddr, &^/&!^ to a Load of the stored pointer, @ to FieldLoad (Copy scalar) or Alloc+Blit (Copy aggregate), ! to FieldStore (Copy scalar) or Blit (Copy aggregate), +! to FieldLoad+Bin(Add)+FieldStore; drop of a reference emits no destructor call",
        "no new Instr variant (R12); Alloc and Blit already exist and are already used this way by set's own array-element lowering"
      ],
      "tests": [
        "borrow_of_place_is_accepted",
        "borrow_of_non_place_is_error",
        "borrow_of_scalar_local_is_error",
        "borrow_of_scalar_field_is_accepted",
        "projection_through_field_element_and_cell_reads_correctly",
        "element_projection_out_of_bounds_still_traps",
        "store_through_shared_reference_is_error",
        "access_through_reference_reads_and_writes",
        "increment_through_mutable_reference_adds_in_place",
        "fetch_or_store_of_linear_referent_is_error",
        "fetch_or_store_of_copy_aggregate_reads_and_writes",
        "fetch_of_copy_aggregate_survives_source_mutation",
        "reference_in_struct_field_is_error",
        "reference_in_enum_payload_is_error",
        "reference_as_array_element_is_error",
        "reference_in_cell_payload_is_error",
        "reference_returned_from_word_is_error",
        "reference_surviving_repl_line_is_error",
        "reference_in_effect_input_is_accepted",
        "drop_of_reference_frees_nothing",
        "mutation_through_reference_emits_no_rebuild",
        "rebuild_style_equivalent_still_emits_alloc_and_blit",
        "mutation_through_reference_parameter_is_visible_to_caller",
        "unused_reference_is_surplus_value_error",
        "reference_local_expires_without_drop"
      ],
      "exit": "Criteria 1 to 6, 13, and 15. The dogfood's push-byte and byte-at compile and run correctly, push-byte's emitted body contains no alloc and no blit while a rebuild-style control word in the same module still does, and a callee's mutation through a &! parameter is visible to the caller. Commit message records that R5/R6/R7/R9/R10 do not exist yet at this commit."
    },
    {
      "phase": 2,
      "focus": "borrow-rules-and-diagnostics",
      "difficulty": "hard",
      "summary": "Exclusivity as the single, symmetric per-place aliasing rule, the consumption-point scan keyed on a place's outstanding derivations rather than literal Value identity, and the disjointness rejection with its sequenced-workaround accept-case.",
      "changes": [
        "src/check.rs: R5 exclusivity, both directions (at most one live &! per place; no & alongside a live &!; no &! alongside a live &; per-place not global), with &-is-Copy and &!-is-not derived from it rather than stated separately",
        "src/check.rs: R6 consumption-point scan over the virtual stack and the locals map, keyed on a place's outstanding derivations (provenance traced through projection, not literal Value equality), firing on move, dispose, and conflicting-borrow (including a reborrow taken while a projection derived from the previous reborrow is still live), no liveness pass",
        "src/check.rs: R7 disjointness rejection as a stated limitation with its own diagnostic"
      ],
      "tests": [
        "two_live_mutable_borrows_is_error",
        "shared_borrow_while_mutable_live_is_error",
        "mutable_borrow_while_shared_live_is_error",
        "reborrow_while_projected_reference_still_live_is_error",
        "dup_of_mutable_reference_is_error",
        "two_live_mutable_borrows_to_different_places_is_accepted",
        "shared_reference_is_copy",
        "naming_mutable_reference_local_reborrows",
        "move_of_place_borrowed_on_stack_is_error",
        "move_of_place_borrowed_in_locals_is_error",
        "dispose_of_borrowed_place_is_error",
        "move_after_borrow_ends_is_accepted",
        "disjoint_field_borrows_are_conservatively_rejected",
        "sequenced_borrows_of_two_fields_are_accepted"
      ],
      "exit": "Criteria 7 to 9. Every borrow rule produces its specific located error in both R5 directions plus the reborrow-suspend case, and the accept-cases (different-places, reborrow after full consumption, Copy shared reference, move/dispose after borrow ends, sequenced disjoint fields) are all accepted."
    },
    {
      "phase": 3,
      "focus": "loops-joins-reference-mode-enums-dogfood-and-docs",
      "difficulty": "standard",
      "summary": "Back-edge rules from both sides, the branch-join rule with its accept-case, reference-mode clause elimination over an enum, the full dogfood including walk, and the additive-work regression check (DESIGN.md is already amended and ROADMAP.md's title/body already corrected, D2/Amendment A, no phase-3 action needed beyond recording R16's answer into ROADMAP.md).",
      "changes": [
        "src/check.rs: R9 back-edge rules (a reference parameter, or a reference derived from one by projection, may cross; a reference to a current-scope local may not; a currently-borrowed local may not be loop-carried)",
        "src/check.rs: R10 borrow state must agree at a branch join, both the disagreement rejection and the agreement accept-case",
        "src/check.rs: R17 reference-mode clause elimination when a word's top input is &Enum/&!Enum: consume the scrutinee reference at dispatch (not left as a surplus value), bind clause payloads as references inheriting mutability and exempt from R7's disjointness rule (statically disjoint fields of one variant), reject a clause body that consumes a payload binding",
        "src/ir.rs: lower_clauses threads the scrutinee's EnumId from the checked frontend Type rather than re-deriving it from the lowered scrutinee's IrType, closing the reachable unreachable! a &!Enum scrutinee would otherwise hit",
        "examples/refs.sth (new file): the dogfood buffer program (push-byte/byte-at/copy-byte/run/main) and the walk word over &!List with the two-borrow copy-byte call; tests/phase3_refs.rs (new file): the golden that runs it and every other criterion test for this slice, kept out of tests/phase0.rs so criterion 16's addition-only check has nothing pre-existing to modify",
        "tests/: a regression check asserting `git diff --name-status a66c47a -- examples/ tests/phase0.rs tests/phase1.rs` contains only additions (R20)",
        "ROADMAP.md: title/body already corrected by Amendment A and this revision's general-locals insertion (454-459); record the slice as done and write R16's answer into the parked design question at 460-466"
      ],
      "tests": [
        "reference_parameter_crosses_back_edge_in_constant_stack",
        "reference_to_local_across_back_edge_is_error",
        "borrowed_local_carried_across_back_edge_is_error",
        "borrow_on_one_arm_only_is_error",
        "borrow_live_on_both_arms_is_accepted",
        "reference_mode_clause_binds_payload_as_reference",
        "reference_mode_clause_consuming_payload_is_error",
        "reference_dogfood_prints_expected_bytes",
        "regression_diff_shows_only_additions"
      ],
      "exit": "Criteria 10 to 12, 14, and 16. The dogfood runs end to end, a reference parameter walks a long list in constant stack while mutating through the reference, and the full existing suite passes with the regression diff check confirming no modification to any pre-existing example or test file. This is the slice's exit: there is no phase 4."
    }
  ]
}
```
