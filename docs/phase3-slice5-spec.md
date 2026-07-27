# Phase 3 Slice 5 — Reference types, places, escape checking (spec)

Design input: [the brief](./phase3-slice5-brief.md). Base: `main` @ `a66c47a`, 700 tests green.
Second draft: addresses review round 1 (soundness, criteria, consistency — 19 findings) and
five decisions made by the project owner in response, cited inline as **D1**-**D5**. Third
draft: three further amendments, cited inline as **Amendment A/B/C**.

At the base commit, ROADMAP.md:438-442 named this slice "second-class references + parameter
conventions (`let`/`inout`/`sink`/`set`) + escape checking", title on 438. The conventions half
is deleted rather than built (R14); the slice is reference types, places, and escape checking.
(The first draft cited line 452, which was Slice 7 — resources as linear values +
user-definable destructor bodies — and would have rewritten the wrong slice's title; corrected
in the prior round throughout this document and in the brief.) **Amendment A applies the
correction directly to ROADMAP.md, in this same revision**, rather than deferring it to a
delivery phase: the title now reads "Second-class references + places + escape checking", with
the Hylo parameter-convention framing removed, and it now spans ROADMAP.md:438-443 (one line
longer than the base commit's 438-442, which shifts every citation below it in the file by one
line — re-verified and corrected throughout this revision).

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

**R2 — A borrow is taken from a local, postfix.** A **place**, for this slice, is a local name.
`&` yields `&T` and `&!` yields `&!T`, both postfix, matching `^`'s existing postfix form
(`rest ^`). Applied to anything that is not a local name — a computed value, a literal, a word
result, or a projection expression — it is a located compile error.

The first draft additionally let a place be "a projection path rooted at a local name"
(`a Buf>len &!`, meaning "borrow just this field"). Round 1 (soundness minor 8) showed that
form is unreachable as specified: `&`/`&!` cannot be ordinary words dispatched on the stack top,
because naming a linear local moves it (`moves.take`, src/check.rs:1670) before any such word could run, and
nothing in the codebase inspects the *preceding* term the way that form would require. **D3/D4**
resolves this by retiring the projection-path form outright rather than building the backward
parser folding it would need: **R3**'s accessor family projects through an *already-borrowed*
reference, so the same result (`&!usize` for one field of `a`) is reached as `a &! Buf&|>len`
— borrow the whole local first, then project through the reference with an ordinary word. A
place is therefore exactly a local name, R2's postfix operator is applied to it exactly once
per borrow, and every deeper reference is a projection (R3), not a second application of `&`/`&!`.

The rejected stack-value alternative (`& ( T -- T &T )`) is recorded because it is close: a
plain word leaving the value below its own reference is tighter for a single borrow and is
trackable (the virtual stack is a compile-time `Vec<Value>` and shuffles are permutations
preserving the `Value` id). It loses because it cannot use locals at all (naming a linear local
moves it) and because the two-borrow call degenerates to `& rot & rot swap`. It is purely
additive later.

**R3 — Accessor words project through a reference, mutability inherited, reference-ness
explicit in the spelling (D3/D4).** `fill` and `len` are **not renamed and not changed**, full
stop. `get` and `set` keep their existing value-form signatures **unchanged through this
slice's own phases (1-3)**: no signature here changes meaning, which is what keeps R20's
additive-work claim true. Amendment B marks `get`/`set` themselves **superseded** by this
slice's reference-mode accessors (R21) — a separate, later, mechanical concern with its own
delivery phase (4) and its own stated fallback, not a change to phases 1-3's typing rules. A
parallel family of reference-mode accessor words is added instead — every one of them takes an
already-reference-typed value (from R2 or from another projection) and yields a *narrower*
reference, never a plain value:

| shape | word | effect | on `&!` receiver |
|---|---|---|---|
| struct field | `S&|>fi` (one per struct/field, mirrors the existing `S|>fi` Copy-peek naming) | `( &S -- &Ti )` | `( &!S -- &!Ti )` |
| array element | `&|>` (generic, like `get`) | `( &[T N] usize -- &T )` | `( &![T N] usize -- &!T )`, same runtime bounds trap as `get` |
| cell payload | `&^` (generic, like `^|>`; the first draft used the reversed spelling, flipped here so every reference-mode accessor leads with the ampersand, uniformly) | `( &^T -- &T )` | `( &!^T -- &!T )` |

Every reference-mode accessor's arity is fixed and readable from its spelling (one reference in,
one narrower reference out); only the shared-vs-mutable qualifier is inferred from the receiver.
This is the whole of D3/D4's "reference-ness explicit, mutability inherited" rule, and it is
what closes the two forks the first draft left open:

- **B1 fixed**: the first draft typed `@` only as `( &T -- T )`, so `b Buf>len` (yielding
  `&!usize` under the old `S>fi`-on-a-reference reading) had nowhere to go. `S&|>fi` never
  exists in an ambiguous arity, and R4 below types `@` for both `&T` and `&!T` directly, so no
  reference ever needs an implicit `&!T -> &T` coercion (which would in any case have collided
  with R5's "no `&` while an `&!` is live" — taking the coercion *is* taking a `&`).
- **B2 fixed**: the first draft read `get`'s existing two-output, non-consuming value form
  ("`get` on `&[T N]`... yields `&T`") as if it were the same word with one output instead. It
  is not the same word. `&|>` is a distinct, fixed-arity word; `get` keeps its own signature
  unchanged and is never applied to a reference.

**Projection through a `&!` consumes the parent reference (D3/D4).** `S&|>fi`, `&|>`, and `&^`
each take their reference argument off the stack the way every word takes its arguments off the
stack — ordinary consumption, not a special rule. Combined with R5's reborrow allowance (naming
a `&!` *local* does not move it, so the same local can be projected from again once the derived
reference is gone), this makes a mutable projection chain linear by construction: at any moment
there is exactly one live reference in a chain rooted at a given borrow, because deriving the
next one used up the previous one. This is what retires the nested-borrow exemption the first
draft's R5/R7 pairing needed (**B4**): R7's disjointness scan never has to reason about a
reference *and* the local it was reborrowed from being simultaneously "live", because a
reference derived by projection is a different, newer `Value` than the reborrow it consumed,
and the reborrow is gone the instant the projection runs. It also means **R11's root list needs
no reference-typed case** (**B3**): the only thing R2's postfix `&`/`&!` is ever applied to is
a plain aggregate local, never to a value that is already a reference, so R11's whitelist
(struct/enum/array/cell) stays exactly as first drafted.

**R4 — Access through a reference: `@`, `!`, `+!`.**

- `@` fetches, typed for **both** `&T -> T` and `&!T -> T`, consuming the reference either way.
  Because both are covered directly, there is no implicit `&!T -> &T` demotion rule to write
  (closes B1 for good, not just for the dogfood's literal line).
- `!` stores, `( &!T T -- )` only (storing through a shared reference is meaningless: a shared
  reference carries no exclusivity, so there is nothing to protect a concurrent reader from).
- `+!` adds in place, `( &!T T -- )`, `T` an integer type. Sugar for fetch-add-store, kept
  because the alternative spelling needs two sequential borrows of the same place plus a `swap`.
- **All three are restricted to a `Copy` *scalar* `T`, not merely `Copy` `T` (round 1 minor
  11).** `is_copy` makes an all-scalar-field struct like `Vec2 { x i64 y i64 }` `Copy`, so
  `v & @` type-checks under a bare Copy restriction with `T = Vec2`, an aggregate. R12's `@`
  lowering is `FieldLoad`, and `field_load_op` (src/backend/qbe.rs:295) `unreachable!`s on an
  aggregate at line 318 ("an aggregate field is copied by blit, not scalar-loaded");
  `field_store_op` (src/backend/qbe.rs:323) mirrors it at line 338. Fetching or storing a Copy
  *aggregate* through a reference is therefore a located compile error, not merely an
  unimplemented case — the panic is a real reachable path without this restriction, since no
  criterion needed a Copy-aggregate case and nothing else in the checker excludes it.
- Fetching or storing a **linear** `T` through a reference is a separate, pre-existing rejection
  from the plain Copy check: it would either produce a second owner of one object (fetch) or
  silently leak the overwritten value (store, since nothing auto-drops) — both soundness rules,
  not scope decisions. `S<fi`'s drop-on-overwrite (docs/phase3-slice1-spec.md:60,
  src/ir.rs:2465) is the precedent for lifting the store restriction later; left out here
  because no criterion needs it.

**R5 — Exclusivity is the entire aliasing rule.** At most one live `&!` to a place, and no `&`
to a place while an `&!` to it is live. Everything else is a consequence, not a separate rule,
and must not be implemented as one:

- `&T` is `Copy` (shared references carry no exclusivity constraint).
- `&!T` is not `Copy`.
- `dup` on a `&!` is rejected **by R5**, since it would produce two live mutable references to
  one place.
- Naming a `&!` local is a **reborrow**, not a move. Without this a mutable helper would kill
  its own parameter on first use, and the dogfood's `push-byte` names `b` three times.
- Two live `&!` rooted at *different* places never conflict — R5 is per-place, and nothing about
  it is a single global "one mutable reference at a time" counter. `copy-byte`'s two-borrow
  call (`&!Buf` into `dst`, `&Buf` into `src`, two different locals) exercises exactly this.

**R6 — The borrow check fires at consumption points, not via a liveness pass.** When a place is
moved, dropped, or borrowed in a way R5 forbids, scan the virtual stack (`stack: Vec<Value>`)
and the locals map (`locals: HashMap<String, Value>`) for a slot holding a reference derived
from that place, and reject with a located error naming both the place and the conflicting
borrow. Both structures are exact at compile time, so this is a scan, not an analysis. A
reference is **live** from the instruction that creates it until the term that consumes its
slot; a reference-typed *local* is live for the whole word body (see R8 for what happens to it
at the body's end, since it is neither `Copy` nor linear).

Rejected alternative: last-use (NLL-style) liveness. More precise, materially more machinery,
and no criterion in this slice needs the precision.

**R7 — Path disjointness is not modeled.** Two references derived from the same local conflict
under R5 even when they project into disjoint fields, if both are simultaneously live (R6). The
measured cost is one `swap` in the dogfood's `push-byte`, sequencing the two projections so the
first is fully consumed (down to a plain value or a further-derived reference that is itself
consumed) before the second is taken — never holding both at once. This is a stated limitation
with its own criterion, so it is behaviour rather than an accident, and it is additive later.

**R8 — Escape is prevented structurally, by five positional rejections over transitive
containment.** A type that **transitively contains** a reference — the reference itself, a
struct with a reference-typed field (directly or nested), an enum variant carrying one, an
array of them, or a cell over one — is a located compile error in: a struct field declaration,
an enum variant payload declaration, an array element (via `fill`), a cell payload (via `^`),
and the **output** side of a declared effect signature. A reference on the **input** side of an
effect is fine (R2 already establishes the only source of a reference is a local borrow inside
some frame, so a parameter reference is unremarkable) and is accepted, tested alongside the
rejections rather than left implicit.

Round 1 (soundness B5) found the first draft's three-position version had two holes precisely
because it enumerated *positions* rather than closing over *type constructors*: `check_owned_cell_word`'s
`"^"` arm (src/check.rs:2225) interns a cell over any payload type with no filter, so `a &! ^`
built `^&!Buf` — not itself a reference type, so the old three-position check missed it, and it
is legal in a field and on an output side once built. `check_array_word`'s `"fill"` arm
(src/check.rs:2133, the Copy check at 2146) accepts any `Copy` element, and R5 makes `&T` `Copy`,
so `r 4 fill` built `[&Buf 4]` with the same consequence. Both holes close by rejecting *at the
construction site* (`^`'s and `fill`'s own arms reject a payload/element that transitively
contains a reference) rather than only at declaration sites, which is why the rule is now five
positions phrased over containment instead of three phrased over syntax.

Combined with place-only creation (R2) and R11 (only an aggregate local can be a borrow root, so
a reference can never be the *only* handle to something whose lifetime it controls), a reference
cannot outlive its referent, so no lifetime apparatus is needed.

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
cleanly. The existing type unification already does most of the work, since a live reference on
the stack is part of the stack shape; this requirement is the small remainder covering a borrow
held in a local, tested on both the disagreement and the agreement side so an over-broad
"any borrow crossing an `if` is an error" implementation cannot pass by accident. Rejected
alternative: a `MaybeBorrowed` lattice element mirroring Slice 1's `MaybeMoved`. The conservative
rule is smaller and no criterion needs the imprecision.

**R11 — Only aggregate locals may be borrowed.** The root of a place (R2: a local name) must be
a local of struct, enum, array, or cell type. A local of scalar type is a located compile error
("borrow a field or an aggregate"). This deletes the spill obligation from the brief's D5
entirely: by recon 3 scalars are SSA temporaries with no address, and giving them memory homes
is real work no criterion needs. A projection whose *result* is scalar (`b Buf&|>len` yielding
`&!usize`) is unaffected, since the referent is a field inside an aggregate that already has a
slot, and — per R3's consequence above — the list stays exactly struct/enum/array/cell, with no
reference-typed case needed, since R2 is never applied to an already-reference value.

**R12 — No new IR instruction; a reference is always `IrType::Ptr`.** Struct-field projection
is `PtrOffset`, array-element projection is `ElemAddr`, cell projection (`&^`) is a `Load` of
the stored pointer, `@` is `FieldLoad`, `!` is `FieldStore`, `+!` is `FieldLoad` + `Bin(Add)` +
`FieldStore`. `Ptr` stays opaque; no pointer arithmetic is exposed to the surface language.

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
only `ir_type_of` gains the two new `Type` arms. Cell projection's `Load` (R3, `&^`) loads a
pointer value whose `IrType` is `Ptr`, exactly the existing `OwnedCell` shape (src/ir.rs:1337),
never an `Int`, so `Load`'s doc comment ("`dst: Int = *ptr`", src/ir.rs:743) describes the
*instruction*, not a constraint on the destination's `IrType` — the same reading Slice 2's own
cell-unwrap lowering already relies on.

**R13 — Mutation through a reference emits no rebuild.** The measurable form of the recon-2
table: the emitted body of the dogfood's `push-byte` contains no `alloc` and no `blit`.

**R14 — No parameter-convention keywords.** `let`, `inout`, `sink`, and `set` are not added.
The reference type is the convention: `&Buf` is what `let Buf` would have meant and `&!Buf` is
what `inout Buf` would have meant. `sink` is the unannotated default, so **no existing signature
changes meaning and no existing code migrates**. `set` is cut twice over: stack returns are a
better out-parameter than a mutable hole, and `set` is already a user-callable array word in
`examples/stack.sth`.

**R15 — Top-of-scope locals are not relaxed.** Recon 4 makes mid-body `| |` a parse error, so a
projection cannot be named where it is most wanted. The dogfood works without it at the cost of
one `swap`, the same cost already accepted under R7. Relaxing binding is a parser and scoping
change orthogonal to references and would widen the slice.

**R16 — The question ROADMAP.md's parked design question answers, answered.** (The first draft
cited line 447, which is mid-sentence inside the parked question; the question and its "Design
question this slice's brief must answer" marker ran 443-449, marker on 443, at the base commit
— now 444-450, marker on 444, after Amendment A's one-line-longer Slice 5 title/body edit
above it, re-verified against the current file.) `inout` projections **do** subsume a reified
take/fill pair (`S/fi` yielding a residual `∂S/∂fi`, refilled exactly once) for every
statically known path, because a projection is the same residual made implicit and lexically
bounded, and it covers whole-value borrows too. No residual form is added. Reified residuals
remain worth having only where the focus must escape, which is Slice 3's zipper; R8 forbids
storing a reference, so the zipper waits for Slice 6's RC rather than for a residual type. This
answer is recorded in delivery phase 3's changes list so it lands with the ROADMAP correction
rather than only living in this prose.

**R17 — Reference-mode enum elimination (D1).** When a word's declared top input is a reference
to an enum (`&Enum` or `&!Enum`), the existing clause-style whole-word form (`| Variant ... |
Variant ... ;`) applies in **reference mode**, same syntax, three differences from the
value-mode form:

- The scrutinee is **borrowed, not consumed**: reading the discriminant through the reference is
  a tag `FieldLoad` (no new IR instruction, consistent with R12), and the enum value itself is
  never freed or moved by the dispatch.
- Each clause's payload bindings are **references inheriting the scrutinee's mutability**: a
  `Cons v i64 next ^List` clause under a `&!List` scrutinee binds `v : &!i64` and
  `next : &!^List`, exactly as a struct-field projection under `&!` would (R3).
- **No clause may consume a payload binding.** A payload binding is a reference like any other;
  moving it out (rather than projecting through it or feeding it to `@`/`!`/`+!`) is a located
  error, the same rule R4 already applies to a fetched/stored `T`.

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
      next &^ walk
  ;
```

`v`'s reborrow is fully consumed by `+!` before `next` is named, and `next &^` derives a
`&!List` whose provenance traces back to `walk`'s own parameter (R9's ancestor-frame case), so
the recursive `walk` call is a legal back-edge exactly as it would be for a struct.

The mode follows the declared scrutinee type, which is explicit in the signature (`&!List` vs
`List`), so choosing reference mode is never implicit or type-directed in a way that would
violate D4's "reference-ness explicit in the spelling" rule — the spelling that is explicit here
is the *signature*, and clause syntax itself is unchanged either way.

### Test discipline (binding)

**R18 — Every criterion is a runnable golden**, source in to expected stdout or source in to
expected diagnostic, with two reasoned exceptions: R13 asserts on the emitted module (a runtime
golden cannot distinguish "mutated in place" from "rebuilt correctly", and eliminating the
rebuild is the point of the slice), and R12's `IrType::Ptr` mapping is exercised indirectly by
every other golden rather than asserted on its own (asserting an internal `IrType` choice
directly would pin an implementation detail no external behaviour depends on; R13's and
criterion 14's structural/behavioural assertions are what would actually break if the mapping
regressed). Both structural assertions are unit tests over `backend::qbe::emit`'s output and
must assert against a single named function body (via `func_body`, mirroring the existing
`emitted_alloc_shim_has_null_trap` pattern), never a whole-module IL string match. New
lexer/parser/check/ir code carries its own unit tests beside it (`#[cfg(test)] mod tests`) in
addition to the goldens listed below, per CLAUDE.md's existing convention.

**R19 — Every diagnostic criterion asserts the specific error**, not merely that compilation
failed. Turning silent failure into a sharp error is the point, so the error text and its
location are part of the spec.

**R20 — Two migration claims, kept distinct so the additive one stays falsifiable on its own
(Amendment C).** The task that produced this revision originally called this requirement "zero
migration"; Amendment B's `get`/`set` removal (R21, delivery phase 4) falsifies that literal
claim by design, so the requirement is restated as two separate claims instead of loosened
until it says nothing:

- **The reference feature itself is purely additive** and changes no existing signature's
  meaning (R14). Demonstrated, not asserted, by a concrete mechanism: delivery phase 3 runs
  `git diff --name-status a66c47a -- examples/ tests/phase0.rs tests/phase1.rs` and asserts
  every line is an addition (`A`), never a modification (`M`), of a pre-existing file — an
  added file (the dogfood, criterion tests) is fine, an edited one is the regression this
  exists to catch. This claim **closes at phase 3** and does not depend on phase 4 happening at
  all; it stays true even if phase 4's fallback is taken and `get`/`set` never move.
- **`get`/`set`'s removal is a separate, subsequent, mechanical migration** (R21, delivery
  phase 4), and its diff is *expected*, not a violation of the claim above. Phase 4 has its own
  regression check, different in kind from phase 3's: the suite must still pass, but the diff
  over `examples/` and `tests/phase{0,1}.rs` is expected to be non-empty, itemized by phase 4's
  own call-site audit (migrated to `&|> @`/`&|> !`, or deleted as redundant with an existing
  reference-mode golden), and reviewed as a real diff rather than waved through by a check that
  only counts additions.

Conflating the two would let a mechanical vocabulary change quietly stand in for "no signature
changed meaning", which is the opposite of what R14 is for; keeping them distinct is what makes
the additive property falsifiable on its own regardless of whether phase 4 ever lands.

### Superseded vocabulary (Amendment B)

**R21 — `get` and `set` are superseded by `&|> @` and `&|> !`.** Not renamed, not changed, in
phases 1-3 (R3): marked superseded here, with their replacements documented, and migrated away
from and removed in delivery phase 4 below, if that phase is reached (R20's fallback). The case
for supersession:

- `get ( [T N] usize -- [T N] T )` is non-consuming and two-output because Slice 1 gave it no
  other way to leave the array live; every call site that only wants to read one element pays
  for it with an immediate `swap drop` to discard the re-pushed array. `examples/vm.sth:58`
  (`vm Vm>prog vm Vm>pc get swap drop`) and `:95` (`vm vm Vm>mem addr get swap drop`) are the
  measured cost: two words of pure plumbing at every read. `&|> @` (borrow the array once,
  project to the element, fetch) reads the same value with no re-pushed array to discard,
  because the reference the read consumes is a narrower reference, not the array itself.
- `set ( [T N] usize T -- [T N] )` writes by taking the whole array and handing back a whole
  new one — functionally correct, and exactly the rebuild-per-mutation cost R13 exists to
  eliminate for structs, just for arrays instead. `&|> !` (borrow the array, project to the
  element, store) mutates the one element in place.
- Net vocabulary shrinks: two words with an awkward arity (`get`'s two outputs, `set`'s
  whole-array threading) collapse into compositions of the same primitives (`&|>`, `@`, `!`)
  every other accessor in this slice already uses. This is the same argument R13 makes for
  structs, applied to arrays a slice late because arrays predate references.

`fill` (constructs an array from a Copy element and a count) and `len` (reads the compile-time
constant size) have no reference-mode replacement to be superseded by — neither reads nor writes
a single element — and stay untouched, not merely deferred; R21 names only `get`/`set`.

## Load-bearing invariants (must survive)

- Backend stays QBE; no LLVM. `Ptr[T]` stays opaque, never assumed to be a `u64`. R12 adds no
  instruction, maps every reference to the existing `IrType::Ptr`, and R2 exposes no pointer
  arithmetic, so a WASM lowering stays possible.
- The linear spine holds: exactly-once, no auto-drop, forgetting is a compile error. References
  do not weaken it, because they never own: R4's Copy-*scalar* restriction on `@`/`!`/`+!` is
  what stops a borrow from manufacturing a second owner or leaking an overwritten one, and R8
  stops a reference outliving its referent. `&!T`'s own disposal (neither `Copy` nor linear) is
  stated explicitly in R8 rather than left to fall through the existing two categories silently.
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
  in Phase 3, and generics are Phase 4; Phase 4's planned ad-hoc dispatch (ROADMAP.md:488-490
  after Amendment A's one-line Slice 5 shift — static overloading over statically-known input
  types, plus open multimethods) is expected to
  eventually subsume both the reference type constructors themselves and R3's explicit
  reference-mode accessor spellings, once a word can be overloaded on whether its receiver is
  `T`, `&T`, or `&!T` rather than needing a distinct name per case. That expectation is recorded
  here as a **revisit trigger**: when Phase 4's dispatch work lands, re-examine whether `&`/`&!`
  and the `S&|>fi`/`&|>`/`&^` family should collapse into overloads of `S>fi`/`get`/`^|>`.

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
  narrower exclusivity rule this slice adds, so the paragraph engages Phase 3 Slice 5 directly
  instead of reading as flatly contradicted by it. Phase 1-3's checker/IR work (below) proceeds
  against this already-amended DESIGN.md; no phase needs to touch it again.

## Delivery phases

1. **Reference types, places, projection, access, and every escape/root rejection needed for
   this phase's lowering to be total.** `&T`/`&!T` in the type system; `&`/`&!` as postfix
   borrow operators on a local (R2); the `S&|>fi`/`&|>`/`&^` accessor family projecting through
   a reference with inherited mutability (R3); `@`/`!`/`+!` with R4's Copy-*scalar* restriction,
   typing `@` for both `&T` and `&!T`; R11's scalar-local rejection; R8's five transitive-
   containment rejections (struct field, enum payload, array element via `fill`, cell payload
   via `^`, effect output) paired with the input-side accept-case; R8's reference-`drop`-is-a-
   no-op rule; R12's lowering, including the `&T`/`&!T` -> `IrType::Ptr` mapping. Checking here
   is types plus these specific soundness rules, not yet the borrow-conflict machinery (R5-R7,
   R9-R10) — those are phase 2/3. **Round 1 (criteria E3) moved R11, R8's rejections, and the
   drop-no-op rule here from a later phase**: without them, phase 1's lowering has cases with
   nothing to lower to (an unaddressable scalar local) or a silent soundness hole a later
   phase's diagnostics would only report on, never prevent, at this commit. State explicitly in
   the phase-1 commit message: **at this commit, R5/R6/R7/R9/R10 do not exist yet, so a program
   using two conflicting borrows, or a borrow crossing a back-edge unsafely, is *accepted* by
   the phase-1 compiler.** That is deliberate and temporary, not an oversight to be rediscovered
   at review.
   Exit: criteria 1 through 6, 13, and 15 (accept/reject at the type level; escape at the five
   positions; drop-as-no-op; the accessor family; `push-byte`/`byte-at` compile, run, and
   produce the right bytes; `push-byte`'s emitted body contains no `alloc`/`blit` while a
   rebuild-style control word in the same test module still does; a callee's mutation through a
   `&!` parameter is visible to the caller).
2. **The borrow rules and their diagnostics.** R5 exclusivity (including the different-places
   accept-case), R6's consumption-point scan (over both the stack and the locals map, and over
   the moved/dropped/conflicting-borrow trio of consumption points), R7's disjointness
   rejection and its sequenced-workaround accept-case. Every rejection lands with its located
   error and its diagnostic golden.
   Exit: criteria 7 through 9.
3. **Loops, joins, reference-mode enum elimination, the full dogfood, and the documentation
   corrections.** R9's back-edge rules from both sides; R10's join rule with both the
   disagreement and agreement accept-case; R17's reference-mode clause elimination (typing was
   phase 1's accessor-family work in spirit, but the back-edge interaction that makes it worth
   having is exercised here); the full dogfood end to end including `walk`; recording R16's
   answer into ROADMAP.md's parked design question (the DESIGN.md:134/208-214 amendment, D2,
   and ROADMAP.md:438-443's title/body correction, Amendment A, are already applied and need no
   further phase-3 work — only the design-question passage at ROADMAP.md:444-450 still needs
   R16's answer written into it); R20's additive-work regression check.
   Exit: criteria 10, 11, 12, 14, and 16.
4. **`get`/`set` migration and removal (Amendment B), or the stated fallback.** Migrate every
   existing `get`/`set` call site to `&|> @`/`&|> !` (R21), then delete both words. Re-verified
   scope (grepping the whole word, excluding comments and test-name string literals like
   `"get-drops-rest"`/`"set-drops-overwritten"`, which the brief-stage estimate of 28/7/6 `get`
   and 15 `set` did not exclude, and which undercounts `tests/phase1.rs` in particular, whose
   REPL-session goldens pack many clauses — and many `get`/`set` calls — onto one source line):
   `get` appears 20 times in `tests/phase0.rs`, 5 in `tests/phase1.rs`, 2 in
   `examples/stack.sth`, and 3 in `examples/vm.sth` (30 total); `set` appears 17 times in
   `tests/phase0.rs`, 16 in `tests/phase1.rs`, 1 in `examples/stack.sth`, and 15 in
   `examples/vm.sth` (49 total, and this file's count for `vm.sth` alone matches the brief-stage
   estimate exactly). Not every call site migrates the same way: a test written specifically to
   exercise `get`'s or `set`'s own behaviour (its bounds trap, its non-consuming shape, its
   whole-array copy-back) is **deleted**, not rewritten, once the equivalent behaviour is
   already covered by this slice's own reference-mode goldens (criteria 3 and 4); a call site
   where `get`/`set` is incidental plumbing inside a larger test or example is **rewritten** to
   use `&|> @`/`&|> !`.

   **Recorded blocker, to be solved here rather than discovered here:** `examples/vm.sth`'s
   assembler (`build`) threads its `[Op 13]` array purely on the stack through a chain of
   thirteen `set` calls (`Halt 13 fill`, then twelve `index value set`) — the array is never
   named as a local anywhere in `build`. `&|> !` requires a place (R2/R11: a reference is taken
   from a local), and R15 declines to relax top-of-scope-only binding, so this call site cannot
   migrate token-for-token. `build` must be restructured to bind the array as a local: `Op`'s
   variants carry only scalar payload fields (`i64`/`usize`), so `[Op 13]` is `Copy`
   (`is_copy`, src/check.rs:156, recurses element-wise), and each of the twelve replacement
   calls can take a fresh, independent `&!` borrow of the same local, none overlapping —
   `| arr | ... arr &! 0 >usize 0 >usize Load &|> ! ...`, one borrow per instruction. Since
   `examples/vm.sth` is Phase 2's exit dogfood, this restructuring is the highest-risk part of
   the migration and the most likely trigger for the fallback below.

   **Stated fallback, a real decision point, not an aspiration:** if phases 1-3 run long, this
   phase is deferred to Phase 3 Slice 6 and `get`/`set` **stay**, superseded in documentation
   (R21) only. This is not a failure state: R21 already gives every future reader the
   replacement mapping regardless of whether phase 4 ever lands, and R20's additive-work claim
   does not depend on phase 4 happening at all.
   Exit: criterion 17, or an explicit deferral recorded in the phase-3 commit if the fallback
   is taken instead.

## Criterion → test map

Goldens live in `tests/phase0.rs`, except the two structural criteria (13, 14), which assert on
emitted code and belong in unit tests beside `backend/qbe.rs`, R17's typing-only criterion (12),
which may live beside R17's own checker code if no runtime observation is needed for the accept
half, and criterion 17, which is conditional on delivery phase 4 being reached at all (R20's
fallback) and is a call-site audit plus a suite-still-passes check, not a single golden.

| # | criterion | test | phase |
|---|---|---|---|
| 1 | `&`/`&!` on a local yield reference-typed values; applied to a literal, an arithmetic result, or a word result they are located errors | `borrow_of_place_is_accepted`, `borrow_of_non_place_is_error` | 1 |
| 2 | borrowing a scalar local is a located error; borrowing a scalar *field* through a projection (the field, not the local, is scalar) is accepted | `borrow_of_scalar_local_is_error`, `borrow_of_scalar_field_is_accepted` | 1 |
| 3 | projection reads correctly through all three shapes: struct field (`S&|>fi`); array element, incl. the bounds trap (`&|>`); cell payload (`&^`); mutability is inherited, so storing through a projection derived from a *shared* reference is a located error | `projection_through_field_element_and_cell_reads_correctly`, `element_projection_out_of_bounds_still_traps`, `store_through_shared_reference_is_error` | 1 |
| 4 | `@`, `!`, `+!` read/write/increment through a reference; `@`/`!` on a linear `T` are located errors; `@`/`!` on a `Copy` **aggregate** `T` are located errors, not a compiler panic | `access_through_reference_reads_and_writes`, `increment_through_mutable_reference_adds_in_place`, `fetch_or_store_of_linear_payload_is_error`, `fetch_or_store_of_copy_aggregate_is_error` | 1 |
| 5 | escape: a reference in a struct field, an enum variant payload, an array element (`fill`), a cell payload (`^`), and on an effect's output side are five located errors; a reference on an effect's *input* side is accepted; `drop` of a reference frees nothing | `reference_in_struct_field_is_error`, `reference_in_enum_payload_is_error`, `reference_as_array_element_is_error`, `reference_in_cell_payload_is_error`, `reference_returned_from_word_is_error`, `reference_in_effect_input_is_accepted`, `drop_of_reference_frees_nothing` | 1 |
| 6 | **structural**: the emitted body of `push-byte` contains no `alloc` and no `blit` and does contain the address-arithmetic-plus-store shape, under an instruction-count ceiling; a rebuild-style control word in the same module still contains `alloc`/`blit`, proving the assertion is not vacuous | `mutation_through_reference_emits_no_rebuild`, `rebuild_style_equivalent_still_emits_alloc_and_blit` | 1 |
| 7 | exclusivity: two live `&!` to *one* place, a `&` taken while an `&!` to it is live, and `dup` on a `&!` are three located errors; two live `&!` to *different* places is accepted; `&` is `Copy` (names twice, accepted) and naming a `&!` local twice is accepted as a reborrow | `two_live_mutable_borrows_is_error`, `shared_borrow_alongside_mutable_is_error`, `dup_of_mutable_reference_is_error`, `two_live_mutable_borrows_to_different_places_is_accepted`, `shared_reference_is_copy`, `naming_mutable_reference_local_reborrows` | 2 |
| 8 | consuming a place while a borrow of it is live is a located error naming both the place and the borrow, whether the conflicting borrow sits on the virtual stack or in the locals map; disposing a borrowed place is likewise a located error; the same place consumed, or disposed, *after* its borrow is gone is accepted | `move_of_place_borrowed_on_stack_is_error`, `move_of_place_borrowed_in_locals_is_error`, `dispose_of_borrowed_place_is_error`, `move_after_borrow_ends_is_accepted` | 2 |
| 9 | two references into disjoint fields of one place, held simultaneously, are rejected (stated limitation); sequencing them (fully consuming the first before taking the second) is accepted | `disjoint_field_borrows_are_conservatively_rejected`, `sequenced_borrows_of_two_fields_are_accepted` | 2 |
| 10 | a reference parameter crosses a self-tail-call back-edge and mutates in constant stack over 1,000,000 nodes; a reference to a current-scope local crossing a back-edge, and a currently-borrowed local being loop-carried, are two located errors | `reference_parameter_crosses_back_edge_in_constant_stack`, `reference_to_local_across_back_edge_is_error`, `borrowed_local_carried_across_back_edge_is_error` | 3 |
| 11 | a borrow live on one arm of an `if` and not the other is a located error at the join; a borrow live on both arms, or on neither, joins cleanly | `borrow_on_one_arm_only_is_error`, `borrow_live_on_both_arms_is_accepted` | 3 |
| 12 | reference-mode clause elimination: a word whose declared top input is `&Enum`/`&!Enum` may dispatch clause-style; payload bindings are references inheriting the scrutinee's mutability; a clause body that consumes (moves out) a payload binding is a located error | `reference_mode_clause_binds_payload_as_reference`, `reference_mode_clause_consuming_payload_is_error` | 3 (typing groundwork in 1, exercised end to end here) |
| 13 | **structural**: a mutation a callee makes through a `&!` parameter is visible in the caller (proves `&!T` lowers to `IrType::Ptr`, not a by-value aggregate) | `mutation_through_reference_parameter_is_visible_to_caller` | 1 |
| 14 | the dogfood runs end to end and prints the expected byte, including the two-borrow `copy-byte` call and the `walk` word over `&!List` | `reference_dogfood_prints_expected_bytes` | 3 |
| 15 | a leftover reference on the *stack* without a `drop` is a surplus-value error; a reference *local* that is never explicitly dropped is accepted (it expires silently at scope end) | `unused_reference_is_surplus_value_error`, `reference_local_expires_without_drop` | 1 |
| 16 | no regression: the full existing suite passes, and `git diff --name-status a66c47a -- examples/ tests/phase0.rs tests/phase1.rs` shows only additions, no modifications, demonstrating R14/R20's additive-work claim concretely rather than by assertion | existing suite, unmodified; `regression_diff_shows_only_additions` | 3 |
| 17 | `get` and `set` are removed and every migrated call site uses `&|> @`/`&|> !` instead; the existing suite passes, with the migration diff over `examples/` and `tests/phase{0,1}.rs` itemized as either a call-site rewrite or the deletion of a now-redundant `get`/`set`-specific test, never a silent change to what a test proves. Under R20's fallback both words survive unmigrated and this criterion is explicitly not attempted, recorded in the phase-3/4 commit rather than left ambiguous | `get_and_set_are_removed_and_call_sites_migrated` (fallback: not attempted) | 4 |

## Dogfood, as this revision specifies it

The brief carries the original dogfood source, and the brief is edited in this revision only
for its three stale `ROADMAP.md` citations (two instances of line 452, one of line 447, both
corrected per B1 above; per the pipeline's scope for a citation-only pass) — it is **not**
updated for D3/D4's new accessor spellings, so its literal source is now stale (`^&`, plain
`Buf>data`/`get` applied to a reference, `new`/`dispose`). This section is the authoritative,
current version; an implementer works from here, not from the brief's literal code.

```forth
type: Buf  data ^[u8 64]  len usize ;

: new ( -- Buf )
  0 >u8 64 fill ^ 0 >usize Buf ;

: push-byte ( &!Buf u8 -- )
  | b x |
  b Buf&|>len @              \ ( -- usize )
  b Buf&|>data &^ swap       \ ( -- &!u8[64] usize )
  &|> x !                    \ ( -- ), stores x through the derived &!u8
  b Buf&|>len 1 +! ;

: byte-at ( &Buf usize -- u8 )
  | b i |
  b Buf&|>data &^ i &|> @ ;

: copy-byte ( &!Buf &Buf usize -- )
  | dst src i |
  dst src i byte-at push-byte ;

: main ( -- )
  new new | a b |
  a &! 72 push-byte
  b &! 90 push-byte
  a &! b & 0 copy-byte
  a & 2 byte-at .
  a drop
  b drop ;
```

And R17's motivating case, over the existing `List` (`examples/list.sth`):

```forth
: walk ( &!List -- )
  | Nil
  | Cons | v next |
      v 1 +!
      next &^ walk
  ;
```

### Hand-trace of `push-byte`, `byte-at`, `copy-byte`, and `main`

`push-byte ( &!Buf u8 -- )`, `b : &!Buf`, `x : u8`:

| term | stack after |
|---|---|
| `b` (reborrow, R5) | `[&!Buf]` |
| `Buf&|>len` (consumes the reborrow, R3) | `[&!usize]` |
| `@` (R4, typed for `&!T`) | `[usize]` |
| `b` (second reborrow — the first was fully consumed by `@` already) | `[usize, &!Buf]` |
| `Buf&|>data` | `[usize, &!^[u8 64]]` |
| `&^` (R3, inherits `&!`) | `[usize, &![u8 64]]` |
| `swap` (R7's one-`swap` cost) | `[&![u8 64], usize]` |
| `&|>` — `( &![T N] usize -- &!T )` | `[&!u8]` |
| `x` (Copy local, no move) | `[&!u8, u8]` |
| `!` (R4) | `[]` |
| `b` (third reborrow — the second was fully consumed by `&|>`/`!` already) | `[&!Buf]` |
| `Buf&|>len` | `[&!usize]` |
| `1` | `[&!usize, i64(1)]` |
| `+!` | `[]` |

Ends `[]`, matching the declared `( &!Buf u8 -- )`. `b` is named three times, never
simultaneously live in two derived forms (R7's discipline), and expires silently at the word's
end (R8) with no `drop`.

`byte-at ( &Buf usize -- u8 )`, `b : &Buf`, `i : usize`: `b` (`[&Buf]`) `Buf&|>data`
(`[&^[u8 64]]`, shared inherited) `&^` (`[&[u8 64]]`) `i` (`[&[u8 64], usize]`) `&|>`
(`[&u8]`, shared inherited) `@` (`[u8]`). Ends `[u8]`, matching the declared output.

`copy-byte ( &!Buf &Buf usize -- )`, locals `dst : &!Buf`, `src : &Buf`, `i : usize`: naming
`dst src i` pushes `[&!Buf, &Buf, usize]`; `byte-at` consumes the top two (`&Buf`, `usize`,
matching its declared input), pushing `[&!Buf, u8]`; `push-byte` consumes both, pushing `[]`.
Ends `[]`, matching the declared output.

`main ( -- )`: `new new` pushes `[Buf, Buf]`; `| a b |` binds `a` to the first, `b` to the
second (leftmost name to the deepest value, matching every other binding in the language),
leaving `[]`. `a &! 72 push-byte` borrows `a` (R2, R11: `a` is a struct local), pushes the u8
literal, calls `push-byte`; `a`'s `data` cell now holds `72` at index 0 and `len` is `1`. `b &!
90 push-byte` does the same for `b`: index 0 is `90`, `len` is `1`. `a &! b & 0 copy-byte`
borrows `a` mutably and `b` sharedly (R5: different places, no conflict) and reads `b`'s byte 0
(`90`) into `a` at its current `len` (`1`), so `a`'s `len` becomes `2` and index 1 is `90`. `a &
2 byte-at .` reads index 2 of `a`, which `new`'s zero-fill left untouched (only indices 0 and 1
were ever written), so it prints `0`. `a drop` and `b drop` dispose both owned buffers (freeing
their `data` cells). Ends `[]`, matching `( -- )`; R20's regression check is unaffected since
this is a new file, not an edit to an existing one.

`walk`'s clause-mode dispatch (R17) over `&!List`: the `Nil` clause has no payload and an empty
body. The `Cons` clause binds `v : &!i64`, `next : &!^List` (mutability inherited from the
`&!List` scrutinee). `v` (reborrow) `1` `+!` mutates the node's value in place and leaves `[]`;
`next` (reborrow) `&^` (inherits `&!`, T = `List`) yields `&!List` whose provenance traces back
to `walk`'s own parameter, so the tail call `walk` is a legal R9 back-edge. Every node's `v`
field is incremented exactly once as the walk recurses, in constant stack (R9, criterion 10),
and `walk` never frees or moves the list it walks — ownership stays with whoever calls it.

## Explicitly out of scope

`& ( T -- T &T )`, the stack-value borrow form (R2; purely additive, revisit if `examples/`
after Slices 5 and 7 is dominated by build-then-configure pipelines over a single value).
Path-disjoint borrows (R7). Borrowing a scalar local, and therefore the scalar spill (R11).
Mid-body local binding (R15). `!` over a linear value with drop-on-overwrite (R4). Reified
take/fill residuals `∂S/∂fi` (R16). Raw or foreign pointers: `^T` is the owning pointer and
`&T`/`&!T` the borrowing one, the only client for a third is FFI at the hosted layer (Phase 6),
`*` is the multiplication word so it is not the spelling, and any future foreign pointer must be
an opaque handle with no arithmetic, since `p 8 +` would force `Ptr` to be an integer and break
the backend-neutral invariant a WASM lowering depends on. Collapsing `&`/`&!` and the
reference-mode accessor family into overloads of the value-form words (D5's revisit trigger;
waits for Phase 4's ad-hoc dispatch). Reference counting and storable references, including the
zipper (Phase 3 Slice 6). User-definable destructor bodies (Phase 3 Slice 7). Worklist-based
branching disposal (Phase 6).

## Phases

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "reference-types-places-projection-access",
      "difficulty": "hard",
      "summary": "Add &T/&!T mapped to IrType::Ptr, postfix &/&! on locals, the S&|>fi/&|>/&^ accessor family with inherited mutability, @/!/+! with Copy-scalar restrictions typed for both &T and &!T, R11's scalar-local rejection, R8's five escape rejections plus drop-as-no-op, and the surplus-value rule for a leftover reference on the stack.",
      "changes": [
        "src/lexer.rs, src/parser.rs: `&` and `&!` as postfix borrow operators on a local; `&T`/`&!T` in type position; `S&|>fi`, `&|>`, `&^`, `@`, `!`, `+!` as words",
        "src/check.rs: reference types in the type lattice; R2's local-only place; the S&|>fi/&|>/&^ accessor family with inherited mutability, each consuming its reference argument; R4's Copy-scalar restriction on @/!/+!, @ typed for both &T and &!T; R11's scalar-local rejection; R8's five transitive-containment rejections (struct field, enum payload, fill's array element, ^'s cell payload, effect output) plus the effect-input accept-case; the surplus-value rule treating a leftover &!T on the stack like any non-Copy value while a reference local expires silently",
        "src/ir.rs: ir_type_of gains &T/&!T -> IrType::Ptr; lower S&|>fi to PtrOffset, &|> to ElemAddr, &^ to a Load of the stored pointer, @ to FieldLoad, ! to FieldStore, +! to FieldLoad+Bin(Add)+FieldStore; drop of a reference emits no destructor call",
        "no new Instr variant (R12)"
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
        "fetch_or_store_of_linear_payload_is_error",
        "fetch_or_store_of_copy_aggregate_is_error",
        "reference_in_struct_field_is_error",
        "reference_in_enum_payload_is_error",
        "reference_as_array_element_is_error",
        "reference_in_cell_payload_is_error",
        "reference_returned_from_word_is_error",
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
      "summary": "Exclusivity as the single per-place aliasing rule, the consumption-point scan over both the stack and locals map, and the disjointness rejection with its sequenced-workaround accept-case.",
      "changes": [
        "src/check.rs: R5 exclusivity (at most one live &! per place, no & alongside a live &!, per-place not global), with &-is-Copy and &!-is-not derived from it rather than stated separately",
        "src/check.rs: R6 consumption-point scan over the virtual stack and the locals map, firing on move, dispose, and conflicting-borrow, no liveness pass",
        "src/check.rs: R7 disjointness rejection as a stated limitation with its own diagnostic"
      ],
      "tests": [
        "two_live_mutable_borrows_is_error",
        "shared_borrow_alongside_mutable_is_error",
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
      "exit": "Criteria 7 to 9. Every borrow rule produces its specific located error, and the accept-cases (different-places, reborrow, Copy shared reference, move/dispose after borrow ends, sequenced disjoint fields) are all accepted."
    },
    {
      "phase": 3,
      "focus": "loops-joins-reference-mode-enums-dogfood-and-docs",
      "difficulty": "standard",
      "summary": "Back-edge rules from both sides, the branch-join rule with its accept-case, reference-mode clause elimination over an enum, the full dogfood including walk, and the additive-work regression check (DESIGN.md is already amended and ROADMAP.md's title/body already corrected, D2/Amendment A, no phase-3 action needed beyond recording R16's answer into ROADMAP.md).",
      "changes": [
        "src/check.rs: R9 back-edge rules (a reference parameter, or a reference derived from one by projection, may cross; a reference to a current-scope local may not; a currently-borrowed local may not be loop-carried)",
        "src/check.rs: R10 borrow state must agree at a branch join, both the disagreement rejection and the agreement accept-case",
        "src/check.rs: R17 reference-mode clause elimination when a word's top input is &Enum/&!Enum: borrow the scrutinee instead of consuming it, bind clause payloads as references inheriting mutability, reject a clause body that consumes a payload binding",
        "examples/ or tests/: the dogfood buffer program (push-byte/byte-at/copy-byte/main) and the walk word over &!List with the two-borrow copy-byte call",
        "tests/: a regression check asserting `git diff --name-status a66c47a -- examples/ tests/phase0.rs tests/phase1.rs` contains only additions (R20)",
        "ROADMAP.md: title/body already corrected by Amendment A (438-443); record the slice as done and write R16's answer into the parked design question at 444-450"
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
      "exit": "Criteria 10 to 12, 14, and 16. The dogfood runs end to end, a reference parameter walks a long list in constant stack while mutating through the reference, and the full existing suite passes with the regression diff check confirming no modification to any pre-existing example or test file."
    },
    {
      "phase": 4,
      "focus": "get-set-migration-and-removal",
      "difficulty": "hard",
      "summary": "Migrate every get/set call site to &|> @ / &|> ! (R21), restructuring examples/vm.sth's stack-threaded assembler to bind its array as a local, then delete both words; or, per R20's stated fallback, defer to Phase 3 Slice 6 and leave both words in place.",
      "changes": [
        "tests/phase0.rs, tests/phase1.rs: migrate get/set call sites used as incidental plumbing to &|> @ / &|> !; delete call sites that exist specifically to test get/set's own behavior (bounds trap, non-consuming shape, whole-array copy-back), since criteria 3/4's reference-mode goldens already cover the equivalent",
        "examples/stack.sth: migrate its one set and two get call sites",
        "examples/vm.sth: restructure build (currently a stack-threaded chain of thirteen set calls with no local) to bind its [Op 13] array as a local so &|> ! has a place to project from, then migrate every get/set call site",
        "src/check.rs, src/ir.rs, src/parser.rs, src/lexer.rs: remove the get and set words entirely once every call site is migrated"
      ],
      "tests": [
        "get_and_set_are_removed_and_call_sites_migrated"
      ],
      "exit": "Criterion 17. get and set no longer exist as words; every prior call site is either migrated to &|> @ / &|> ! or deleted as redundant; the full suite passes with the itemized migration diff as its only change. If the fallback is taken instead, this phase is not attempted and the deferral is recorded in the phase-3 commit."
    }
  ]
}
```
