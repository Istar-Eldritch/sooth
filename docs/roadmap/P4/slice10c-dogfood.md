# Phase 4 Slice 10c dogfood: hand-written enum eliminators

Paper-design exercise. No source was modified. Every claim is backed by a
`file:line` citation into the real tree (HEAD `2825c10`) or by a scratch
`cargo run -- build` probe whose exact output is quoted.

## TL;DR (the fatal finding first)

**Hand-written eliminators are not viable for any enum other than `bool`, and
even `bool`'s cannot retire `if`.** Two independent, load-bearing rules form a
pincer:

1. The *only* construct that dispatches on an enum is a **clause body**
   (`WordBody::Clauses`). A term body has no enum dispatch except the primitive
   `if`, which works on `bool` alone (`src/check/audits.rs`, `src/ir/.../control_flow.rs`).
2. A clause-bodied word may **not take a quotation parameter** at all
   (`src/check/audits.rs:340`), and any `~` in a signature forces the word
   polymorphic (`src/parser.rs:1234`), and a polymorphic word with a clause
   body is rejected outright (`src/check/poly.rs:158`, `:253`).

So: to dispatch on an enum you need a clause body; a clause body cannot receive
the branch quotations. To receive branch quotations you need a term-body
combinator; a term body cannot dispatch on a non-`bool` enum. There is no third
option (no discriminant-read word, no non-clause payload extraction: the enum
word table only has `Construct`, `src/ir/layout.rs:264`,`:480`).

If eliminators are required, they must be **compiler-generated**. The compiler
already owns the exact machinery (`lower_clauses` does the N-way discriminant
dispatch + join, `src/ir/func_builder/control_flow.rs`); what is missing is a
front-end path that feeds it *branch quotations* instead of clause syntax.

Everything below is the detailed answer to the seven questions.

---

## Q1 — `Variant[ ... ]`: new type, or a tag on the existing quotation literal?

**Recommendation: a compile-time tag on the quotation *literal term*, never a
new `Type` variant, and never a new quotation type.**

Why not a new `Type`:

- `Type` is deliberately closed. Its own doc says the polymorphic/variable
  forms "gains **no** variant (S1)" and live only in `PolyType`
  (`src/ast.rs:855-864`, the `Type` enum doc). Adding a `Type::Branch` would
  ripple into `is_copy`, `ir_type_of`, `is_quotation_type`, and the four
  materialization boundaries that reject `~` "by type inequality *before* the
  boundary" (`src/ast.rs:920-938`). Each of those is a default-deny site that
  becomes a new place to forget the branch case.
- A quotation *literal* already has **no type of its own**. `TermKind::Quotation(Vec<Term>)`
  (`src/ast.rs:1204`) is a bare term list. Its type (`Type::Quotation` vs
  `Type::InlineQuotation`) is decided by the *callee's declared parameter* at
  the call site and carried on the checker's `Slot.quot` as `QuotRef::Known(id)`
  — there is explicitly "**No 'statically known' bit**" on the type itself
  (`src/ast.rs:889-900`, the `Type::Quotation` doc; D6). Knownness is a checker
  side-table, not a type.

So the variant tag is a **syntactic annotation on the literal**: give
`TermKind::Quotation` an optional `tag: Option<String>` field (or add a sibling
`TermKind::TaggedBranch(String, Vec<Term>)`). It is set by the parser when it
sees `Ident[` (see Q5), read only by the (hypothetical) eliminator-argument
checker to pick which variant this literal serves, and **erased before
lowering** at the same boundary the `~` marker is — a branch literal has no
runtime representation, exactly like `Type::InlineQuotation`
(`src/ast.rs:911-919`). Since no eliminator can actually be written (Q2), the
tag would in practice have no consumer at all today.

---

## Q2 — THE CRUX: can a hand-written eliminator be written?

**No, for two independent reasons, plus a third that bites even if the first
two were lifted.** Ruben's hypothesis is confirmed and sharpened.

### Wall A — enum dispatch is clause-only; branches can't enter a clause body

Probed directly:

```
$ cat p_bool_ord.sth
: Bool? ( [ -- ] [ -- ] bool -- )
| True   drop call
| False  swap drop call ;
$ cargo run -- build p_bool_ord.sth
error: the quotation-taking word `Bool?` has a clause body; a quotation
parameter is only supported on a word with a term body this slice (its body is
inlined at each call site, and a clause body cannot be spliced), and a runtime
quotation value is slice 7
```

That is `clause_bodied_quotation_word_error`, `src/check/audits.rs:340`. It
fires for an *ordinary* `[ ... ]` branch. For a `~` branch a *different* guard
fires first, because `~` forces the word polymorphic
(`effect_has_variable` returns `true` on `TildeLBracket`, `src/parser.rs:1234`)
and:

```
$ cat p_bool_elim.sth
: Bool? ( ~[ -- ] ~[ -- ] bool -- )
| True   drop call
| False  swap drop call ;
$ cargo run -- build p_bool_elim.sth
error: `Bool?` combines a clause-style body with a polymorphic signature,
which is not supported
```

`src/check/poly.rs:158` (native) / `:253` (REPL). Both directions are closed.

There is **no other enum dispatch**. A term body can only branch with the
builtin `if`, which is `bool`-typed (`TermKind::If`, parsed at
`src/parser.rs:2222`, lowered at `control_flow.rs:170`). There is no
discriminant-read word and no way to extract a payload outside a clause: the
enum word table is `EnumWord::Construct(id, vi)` only (`src/ir/layout.rs:264`,
`:480`), and `=` refuses enum operands:

```
$ cargo run -- build p_ord_termcomb.sth   # tries `o Less =` inside a term body
error: `=` requires two operands of the same numeric type, found `Ordering`
and `Ordering`
```

So `Ordering?`, `Shape?`, and every user-enum eliminator are unwritable full
stop, regardless of stack shape.

### Wall B — the stack-shuffle depth ceiling (the part Ruben predicted)

Suppose Wall A were lifted and a clause word *could* take erased `~` branches.
The clause-body checker puts the branches **below** the payload:
`below = inputs-except-scrutinee`, then "pushes the variant's fields (first
field deepest) atop" (`src/check/word_entry.rs`, `check_clause_body`, the
`initial = below.to_vec()` + `initial.push(field_ty)` block ~`:405-415`). So the
arm for variant *i* sees, bottom→top: `b0 b1 … b_{k-1} payload…`, and must
`call b_i` while discarding the other k−1 branches.

The entire shuffle vocabulary is `dup drop swap over rot` (`src/check.rs:1821`,
`:1835`, `:1860`, `:1867`, `:1889`) — there is no `nip`/`pick`/`roll`:

```
$ cargo run -- build p_nip.sth
error: unknown word `nip` in `try` (line 1)
```

`rot` touches `n-3` (`src/check.rs:1892`), `swap` `n-2`, `over` copies `n-2`.
**Nothing reaches stack depth ≥ 4.** Any value 4-or-more deep is *frozen* until
the things above it are consumed. Working the virtual stacks:

- **`Bool?`** (k=2, no payload). `initial = [t, e]` (e on top).
  - `| True  drop call` → `[t,e]` --drop--> `[t]` --call--> runs `t`. ✓
  - `| False swap drop call` → `[t,e]` --swap--> `[e,t]` --drop--> `[e]` --call--> runs `e`. ✓
  - **Exactly Ruben's hypothesis.** Writable.
- **Ordering** (k=3, no payload). `initial=[b0,b1,b2]`.
  - `| Less    drop drop call` (keep b0). ✓
  - `| Equal   drop swap drop call` (keep b1). ✓
  - `| Greater rot drop swap drop call` (keep b2 — must `rot` to reach the two
    below it). ✓ but already fiddly.
- **Shape** (k=2, but `Rect` payload is `f64 f64`). `Rect` arm:
  `initial=[b0, b1, w, h]`, depths top→down: h(1) w(2) b1(3) **b0(4)**. The arm
  must discard `b0`, which is frozen at depth 4. `rot`+`swap`+`drop` can churn
  the top three but can never touch position 4. **Unwritable.** Probed:

  ```
  $ cargo run -- build p_depth4.sth   # ( ~ ~ i64 i64 -- i64 ) rot drop
  error: stack effect mismatch ... body leaves 3 values ...
  ```

- **5-variant** (no payload). `b0` is at depth 5 in four of the arms. Frozen.
  **Unwritable.**

**Precise break point:** an arm is unwritable as soon as
`payload_width + variant_count − 1 ≥ 4`. That is sharper than "position-dependent
order falls apart": the real killer is a hard depth-3 ceiling on the shuffle
ops. It bites at 4 variants with no payload, or at **2 variants the moment one
carries a 2-field payload** (`Shape`).

### Verdict

Confirmed and refuted-in-part: `Bool?` is writable exactly as hypothesised;
the general case collapses, but earlier and for a crisper reason than
"position-dependent discard" — it is the depth-3 shuffle ceiling, on top of the
two front-end walls that stop any non-`bool` eliminator before the stack even
matters.

---

## Q3 — does the mechanism work against today's rules? (probed, not reasoned)

**(a) Can a clause-bodied word bind locals *before* dispatching?** **No.**
`parse_worddef` picks `WordBody::Clauses` iff `at_clause_start()` (a `|`
immediately followed by a registered variant name) and otherwise
`WordBody::Terms` — a hard XOR (`src/parser.rs:1002-1008`, `at_clause_start`
`:1166`). A clause body's *only* binding is each clause's own `| names |`
**after** the variant match (`parse_clauses`, `:1185-1195`). So Ruben's
`binary_search` sketch (`| b idx |` first, then dispatch on `Ordering`) parses
as a **Terms** body, and a Terms body has no `Ordering` dispatch. It is
unparseable/uncheckable today, and the new eliminator form does **not** fix it
(Q6).

**(b) Do `drop`/`swap`/… work on an erased `~`? Is dropping one legal under the
linear spine?** Probed:

```
: try ( ~[ -- ] -- )            drop        ;   → builds (exit 0)
: try ( ~[ -- ] ~[ -- ] -- )    swap drop drop ; → builds (exit 0)
: try ( ~[ -- ] -- )            dup drop drop ;  → builds (exit 0)   # so ~ is Copy
: try ( ~[ -- ] ~[ -- ] -- )    nip drop    ;   → error: unknown word `nip`
```

`drop` of a `~` is explicitly **allowed** — `src/check.rs:1841-1846` carves it
out ("`drop` of a compile-time-only quotation marker discards it with nothing
to dispose"). The linear spine does not forbid it. `swap`/`rot`/`over`/`dup`
all forward or copy the marker. `nip` simply does not exist.

**(c) Does `call` on a `~` bound to a local, or sitting below other values,
splice?** Probed, all build (exit 0):

```
: try ( i64 ~[ i64 -- i64 ] -- i64 )              | f | f call ;          # from a local
: try ( i64 ~[i64--i64] ~[i64--i64] -- i64 )      drop call ;             # ~ below another ~
: try ( [ i64 -- i64 ] i64 -- i64 )               | n | | f | n f call ;  # ordinary quot from a local
```

`call` on a `~` (from a local or after dropping the one above it) works;
`call` on an ordinary `[ … ]` bound to a local works too. The splice mechanics
are not the blocker — Walls A/B are.

---

## Q4 — scrutinee-first vs the three hardcoded "topmost input" sites

The three sites all assume the enum is the **last** (topmost) declared input:

- `src/check/word_entry.rs:277`: `word.effect.inputs.last()` must be the enum;
  `below = inputs[..n-1]` (`:305`).
- `src/ir/func_builder/control_flow.rs:203`: `scrutinee = *params.last()`,
  `stack_below = params[..len-1]`.
- `src/ir/func_builder/mod.rs:849` (inside the range the brief cites at
  `:772-776`, the `WordBody::Clauses` arm of `lower_word_parts`):
  `scrutinee_ty = effect.inputs.last()`.

All three are **clause-word-only**. They are never reached by a term-body
combinator.

The proposed rule (scrutinee = topmost *enum-typed* input; every input above it
must be an erased `~`, so it is topmost again at lowering) **was verified to
work — but only on the term-body-combinator route, and only for `bool`**:

```
$ cat p_scrutfirst_comb.sth
: Bool? ( bool ~[ -- ] ~[ -- ] -- )        \ scrutinee FIRST (deepest), 2 ~ above
  | e | | t | | c |
  c if t call e drop else e call t drop end ;
: main ( -- ) 3 2 > [ 42 . ] [ 99 . ] Bool? ;
$ cargo run -- build p_scrutfirst_comb.sth   → builds (exit 0)
```

The `~` values erase, so at lowering the `bool` genuinely is topmost — the
invariant survives without touching the three sites. **But this route reaches
those sites never** (it is a term combinator, not a clause word), and it only
dispatches because it uses the primitive `if`. For every eliminator I could
actually write (`Bool?` only), "all above-scrutinee inputs are erased" holds
trivially (both above-inputs are `~`). The three-site edit is only needed if
eliminators are made *clause* words — which Wall A/B forbid upstream, so the
edit is necessary-but-not-sufficient and, in isolation, dead.

---

## Q5 — lexing `Variant[`

`[` is a delimiter (`is_delimiter`, `src/lexer.rs:34`) and the **only** glued
bracket is `~[` → `TildeLBracket`, special-cased precisely because otherwise
"`~[` and `~ [` both lex as `Word("~")` + `LBracket`, discarding adjacency"
(`src/lexer.rs` Token doc, `:20-26`; glue at `:186-190`).

Therefore `Circle[` lexes as **two tokens**, `Word("Circle")` + `LBracket`,
byte-identical to the spaced `Circle [ … ]`. Confirmed by the existing test
`lex_bracket_adjacent_to_word_still_splits_expected` (`[usize]` → `LBracket
Word RBracket`). The lexer **cannot** specialise on variant-ness: which words
are variants is a parser pre-pass (`is_variant_name`, `src/parser.rs:1163`), not
known to the lexer.

So `Ident[` is **not** lexable unambiguously. Cost is one of:

- **(a) a new glue token** analogous to `TildeLBracket`. But the lexer would
  have to glue *every* `word[`, since it can't scope to variants; that collides
  with any future `arr[idx]`-style access and forces the parser to un-glue the
  non-variant cases. Rejected.
- **(b) parser-level column adjacency**: in `parse_term`, when a `Word` that is
  a registered variant name is immediately followed (same line, `col`
  contiguous) by `LBracket`, read a tagged branch. This is feasible (spans
  carry `col`) but makes **whitespace significant** — `Circle[` ≠ `Circle [` —
  which is exactly the hazard the `~[` glue comment flags. Recommend (b) with
  eyes open.

No collision with the array constructor `[ Type ; Count ]`: that begins with a
bare `LBracket` and is detected by a top-level `;` scan (`array_ctor_ahead`,
`src/parser.rs:1740`; commit-on-`;` at `parse_term`, `:2266`). A branch literal
`Circle[ … ]` is preceded by a variant word, so the two never overlap.

---

## Q6 — paper-write the dogfood

- **`if` / `unless`.** `if` → `cond then[ … ] else[ … ] Bool?`. Achievable
  **only** as a term-body combinator that uses the primitive `if` internally
  (verified above). That means `if` **cannot be retired**; the spec's framing
  ("`if` becomes the untagged convenience for `Bool?`") is inverted — `Bool?`
  must be built *on* `if`. `unless` = same with the two branch literals swapped.
- **`examples/shapes.sth` clause words.** `area` needs
  `shape Circle[ dup * 3.14159 * ] Rect[ | w h | w h * ] Shape?`.
  **Cannot be expressed:** `Shape?` needs enum dispatch (clause body) but the
  branches are quotations (Wall A), and even hypothetically the `Rect` arm hits
  the depth-4 frozen-branch wall (Wall B). `unwrap-or` on `MaybeInt` is the same
  wall. Both stay as the clause words they already are.
- **`lib/binary_search.sth`.** The new form does **not** fix Ruben's sketch.
  `Ordering?` is a 3-variant, no-`bool` eliminator → Wall A (no term-body
  dispatch; clause body can't take the three branches). Separately, the sketch
  binds `| b idx |` then dispatches — impossible in a clause body (Q3a). Stays
  unbuildable.
- **`while` (`lib/combinators.sth`).** Already an ordinary term-body combinator
  using `if` (`: while ( 'a ~[ 'a -- 'a bool ] -- 'a ) | p | p call if p while
  else end ;`). Under the proposal its `if` → `Bool?`, so it is expressible iff
  `Bool?` exists — i.e. iff `if` stays primitive. Its own self-recursion is
  fine.
- **`gcd` / `sum-to` (self-tail).** `gcd` rewritten through a `Bool?`
  combinator **builds and runs correctly** (`10 15 gcd .` → `5`, `48 60` → `12`).
  But `sum-to`:

  ```
  : sum-to ( i64 i64 -- i64 ) | acc n |
    n 0 = [ acc ] [ acc n + n 1 - sum-to ] Bool? ;
  : main ( -- ) 0 1000000 sum-to . ;
  → Segmentation fault (exit 139)
  ```

  vs the original `examples/countdown.sth` (primitive `if`): `→ 500000500000`,
  exit 0, constant stack. **The self-tail-call → loop back-edge transform does
  not survive the rewrite.** The recursive `sum-to` is now buried inside a
  branch *literal* handed to `Bool?`, not in `sum-to`'s own tail position, so
  it lowers to real recursion and blows the host stack. This is a **measured
  regression** against the locked "self-tail must survive" requirement.
- **Nested eliminator inside a branch.** Moot: no non-`bool` eliminator is
  writable, so the only nesting available is `if` inside `if`, which already
  works.
- **Shape-changing branches (`..i -- ..o`).** **Unparseable.** A quotation
  effect whose input and output rows differ is rejected at parse time —
  `quotation_row_shape_change_error` in `parse_poly_quotation_inner`
  (`src/parser.rs:1583-1588`). A branch that changes the stack shape below its
  own additions therefore cannot even be typed. And a *fully generic*
  eliminator (branches of arbitrary output shape) needs exactly `~[ ..a -- ..b ]`
  with `a ≠ b`, which is the same rejected form — so a generic `Bool?` over
  arbitrary branch outputs is impossible; each distinct branch output shape
  needs its own specialised eliminator.

---

## Q7 — what breaks / is newly required, ranked

**FATAL — the direction is unworkable as specified:**

1. **No hand-writable enum dispatch that also takes branch quotations.** Enum
   dispatch = clause body; clause body forbids quotation params
   (`audits.rs:340`) and forbids `~`/poly (`poly.rs:158`). No discriminant word,
   no non-clause payload access (`layout.rs:264`,`:480`; `=` refuses enums).
   ⇒ hand-written eliminators are impossible for every non-`bool` enum. They
   **must be compiler-generated** (or a brand-new dispatch primitive added).
2. **Self-tail → loop lost.** Measured `sum-to` segfault at 1e6 vs the original
   constant-stack `countdown`. The locked "gcd/sum-to survive" requirement
   fails when their `if` self-recursion moves into a branch literal.
3. **Shape-changing branches unparseable** (differing rows,
   `parser.rs:1583`). Rules out the shape-changing case and generic-output
   eliminators outright.
4. **Depth-3 shuffle ceiling** makes even a hypothetically-allowed clause
   eliminator unwritable once `payload_width + variants − 1 ≥ 4` (`Shape`
   already; any 4-variant enum; any 5-variant enum).
5. **`if` cannot be retired.** `Bool?` is only realisable *on top of* the
   primitive `if`; the dependency the spec asserts is backwards.

**HIGH — newly required even for a `bool`-only slice:**

1. Scrutinee-first needs the "topmost enum-typed input; all above erased" rule.
   For the term-combinator route it works without touching
   `word_entry.rs:277` / `control_flow.rs:203` / `mod.rs:849`; for a clause-word
   route those three edits are needed but insufficient (blocked by 1).
2. `Variant[` lexing needs whitespace-significant column-adjacency in the parser
   (recommended) or a new glue token (rejected). Reintroduces the exact
   whitespace hazard the `~[` glue comment warns about.

### Recommendation

Do **not** pursue hand-written eliminators. Either:

- **(A) compiler-generate** the eliminators. `lower_clauses`
  (`control_flow.rs`) already emits the N-way discriminant dispatch + M-output
  join; the only missing piece is a front-end that binds each `Variant[ … ]`
  branch literal as that variant's body and feeds it to the existing lowering.
  This also side-steps Walls A/B, the shuffle ceiling, and (by generating the
  join directly) preserves self-tail lowering if the generator threads tail
  position into each branch. It still needs the row/shape-change relaxation
  (item 3) and the lexing decision (item 7).
- **(B) keep the clause-body form.** It already works, reads cleanly, binds
  payload via `| names |`, is exhaustive-checked, and lowers to the same join.
  The scrutinee-first + branch-literal syntax buys concatenative uniformity at
  the cost of everything above; the clause form pays none of it.

Where I could not verify: I did not exercise a genuinely row-polymorphic
`Bool?` end-to-end (the row-shape-change rejection blocked the generic
signature at `parser.rs:1583`); the self-tail regression is demonstrated on the
concrete `-- i64` specialisation, which is the strongest case for the transform
surviving, and it still segfaults.
