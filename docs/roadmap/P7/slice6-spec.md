# P7.S6 — surface syntax unification (spec)

Input: [`slice6-brief.md`](./slice6-brief.md), taken as decided design **except** for the
brief's worked bound-migration examples, which are arithmetically wrong and are corrected
here (see R6, "the bound-bearing occurrence is a slot").

**Nature of the change: parser, tests, and one emitted-symbol rendering.** No AST shape,
checker, lowering, IR or backend *logic* change. `PolySig`, `PolyType`, `RawTy`,
`Type::Array`, `ArrayDecl` and every downstream consumer keep their shapes; the token
stream that reaches them is spelled differently and the parser produces the identical
tree. Two non-parser edits: the array type's rendered name (`ArrayDecl::name_static`, R8)
and the diagnostic prose that spells the array shape. The `name_static` edit is **not**
display-only — it changes emitted QBE symbol names for array-typed monomorphs; ruled on in
R8.

Four surface changes:

1. the array type is named: `['T 'N]` → `array['T 'N]`;
2. `type:`/`trait:` bind their type variables in brackets: `type: Box['T]`, `trait: Ord['T]`;
3. a word's bounds move out of its effect into a bracket: `: max['T: Copy Ord] ( 'T 'T 'T -- 'T )`;
4. an `impl:` target's array spelling follows (1): `impl: Show for array['T 'N]`.

---

## 1. Codebase map (re-verified line by line against HEAD `9c13878`)

### The bare-`[` disambiguator (change 1)

| What | Where (verified) |
| --- | --- |
| `quotation_type_ahead` — the forward scan to the matching `]` for a depth-1 `--` | `src/parser.rs:4150` |
| its **six** direct callers (all figures are the `quotation_type_ahead()` *call* line, not the enclosing `fn` line) | `3251` (`parse_poly_slot`), **`3933` (`parse_slot`)**, `3995` (`parse_type_expr`), `4039` (`quotation_effect_opens_here`), `4468` (`parse_field_type_expr`), `5242` (`parse_generic_field_shape`) |
| `quotation_effect_opens_here` (`fn` at `4038`) callers (the `owning [ … ]` readers) | `src/parser.rs:3353`, `4027`, `4076` (inside `split_owning_cell_word`) |
| `parse_slot` — the concrete slot reader (ordinary word effects, `extern:` params), incl. its `owning`-named-slot `:`-lookahead exemption at `3966–3968` and its name-then-optional-`:type` read at `3972–3975` | `src/parser.rs:3921` |
| `parse_poly_slot` (its `[` arm at 3250–3256) | `src/parser.rs:3237` |
| `parse_poly_array` (poly `[ elem count ]`) — invalid-length message at `3637` | `src/parser.rs:3617` |
| `parse_array_type_expr` (concrete `[ elem count ]`) | `src/parser.rs:4133` |
| `parse_generic_field_array` | `src/parser.rs:5419` |
| `parse_generic_field_shape`'s own `&` arm (`5267`; glued-header-application sub-arm at `5298–5315`, via `poly_generic_header`) and `^` arm (`5323`) | `src/parser.rs:5228` |
| `parse_poly_slot`'s own `&` arm (`5267`-analogue at `3265–3296`) and `^` arm (`3298–3339`) — each handles **only** an empty or `'`-prefixed remainder and then falls through to the concrete readers | `src/parser.rs:3237` |
| `parse_ref_type_expr` — the `&`/`&!` splitter; `&`-with-empty-remainder recurses into `parse_type_expr` at `4116`, a `^`-led remainder goes to `split_owning_cell_word` at `4118`, any other non-empty remainder goes to `resolve_type_or_apply` at `4120` | `src/parser.rs:4103` |
| `split_owning_cell_word` — the `^`-run splitter; empty remainder recurses at `4065`, `owning` remainder at `4066`, else `resolve_type_or_apply` at `4089` | `src/parser.rs:4054` |
| array-length diagnostics spelling `[T N]` in prose | `src/parser.rs:3637`, `4411`, `4418`, `4423` |
| `parse_quotation_effect_rows` / `parse_quot_type_list` | `src/parser.rs:4184` / `4196` |
| `parse_poly_quotation` / `..._inner` / `parse_poly_quot_list` | `src/parser.rs:3506` / `3521` / `3574` |
| reserved-name gate (`Slice`/`!Slice` arm at `219`, `owning` arm at `229`) | `src/parser.rs:212`; `SLICE_TYPE_NAME` = `242`, `MUT_SLICE_TYPE_NAME` = `247` |
| `OWNING_QUOTATION_KEYWORD` | `src/ast.rs:2468` |
| `resolve_type_or_apply` — where `Slice[T]` is intercepted by name ahead of every user registry | `src/parser.rs:5030` (Slice arm `5035–5037`) |
| `parse_type_arguments` (application-site argument list, no glue requirement) | `src/parser.rs:5148` |
| `ArrayDecl` / `name_static` / `intern_array_type` (the `format!("[{} {}]", …)` at `1463`) | `src/ast.rs:1278` / `1281` / `1455` |
| `instantiation_symbol` — builds the emitted symbol from `ty.name()` | `src/ast.rs:2176` (the `sanitize(ty.name())` at `2184`) |
| `type_arg_key`, array arm spelling `[elem count]` | `src/ast.rs:734`, array arm at `758–760` |
| the three diagnostic renderers that build the array shape by hand with `format!("[{} {}]", …)`, bypassing `name_static` | `src/check/poly.rs:8460` `poly_type_str` (array arm `8472`), `src/parser.rs:1985` `generic_field_type_str` (arm `1996`), `src/parser.rs:386` `poly_type_shape_str` (arm `395`) |
| REPL's `<[T N]>` array placeholder render | `src/repl.rs:759` |

### `type:` / `trait:` headers (change 2)

| What | Where (verified) |
| --- | --- |
| `header_ty_var_count` — counts postfix `'`-prefixed tokens | `src/parser.rs:105` |
| its three callers (call lines) | `src/parser.rs:79` (`prepass_type_decls`), `4583` (`current_typedef_is_generic`, `fn` at `4582`), `4868` (`parse_generic_typedefs`) |
| `parse_generic_header` / `parse_generic_header_vars` | `src/parser.rs:4760` / `5184` |
| `duplicate_generic_ty_var_error` (raised in `parse_generic_header_vars`) | `src/parser.rs:1862` |
| `skip_typedef` | `src/parser.rs:4966` |
| `current_typedef_is_enum` → `body_has_pipe_before_semicolon(pos+2)` | `src/parser.rs:4574` |
| `parse_trait_decl` — its **own inline postfix peek** at `2495–2508`, whose neither-form case is **two** arms: a located `Some((tok, span))` error at `2501–2506` and a separate `None`/EOF arm at `2507` (`self.eof_error("a type variable (`'T`)")`), second-variable check at `2509` (`multi_variable_trait_error`, `356`). **It does not call `header_ty_var_count`**, so R5a's OR does not reach it; see R5b | `src/parser.rs:2491` |
| REPL `type:`-line paths and the generic-header REPL gate | `src/parser.rs:1318` (`parse_typedef_line`), `1401` (`parse_enum_typedef_line`), `1370` (`reject_generic_typedef_in_repl`), `1384` (`typedef_line_is_enum`), `1392` (`enum_variant_names`) |

### Word-definition bounds (change 3)

| What | Where (verified) |
| --- | --- |
| `parse_worddef` — `inline` peek `2302`, `expect(LParen)` `2306`, poly/concrete fork `2310` | `src/parser.rs:2289` |
| `effect_has_variable` — the poly/concrete pre-scan | `src/parser.rs:3143` |
| `parse_poly_effect` | `src/parser.rs:3176` |
| `parse_poly_ty_var` — bound arm (glued and standalone `:`) at `3478–3492`, **returns `Ok(RawTy::Var(id))` at `3500`** | `src/parser.rs:3462` |
| `bound_on_use_error` | `src/parser.rs:1796` |
| `parse_capabilities` (the greedy bound list; `None => break` fallthrough at `3702`) | `src/parser.rs:3665`; callers `parse_poly_ty_var:3486`, `parse_impl_bounds:2763` |
| `impl_target_bound_error` — raised **post hoc** in `parse_impl_target` at `2737`, on `!builder.bounds.is_empty()` (`2733`) *after* `parse_poly_slot` returns, **not** inside `parse_poly_ty_var` | `src/parser.rs:420` |
| `PolyBuilder` (`bounds`, `forbid_bounds` at `1540`, `intern_ty_var`) | `src/parser.rs:1515` |
| trait member declaration (`inline` peek `2550`, `parse_trait_member_effect` `2605`) | `src/parser.rs:2520`– |
| REPL word-def path | `src/parser.rs:1257` (`parse_line_with_structs`) |

### `impl:` target (change 4)

`parse_impl_target` at `src/parser.rs:2728` (sets `forbid_bounds: true`, routes through
`parse_poly_slot`, rejects a glued bound via `impl_target_bound_error`);
`parse_impl_bounds` (S4b's `where`-clause) at `src/parser.rs:2763`.

### Not affected

`tree-sitter-sooth/grammar.js` tokenises brackets generically (`bracket_group`), so
`array[` needs no grammar change: `array` is an ordinary word and `[ … ]` an ordinary
bracket group. `docs/book/` is uncompiled and already teaches rejected syntax (a known,
separately-tracked defect); out of scope here.

---

## 2. Requirements

### R1 — `array` is the array type's name

At every type-position reader, an array type is written `array[ <elem> <count> ]`:

- `array[i64 4]`, `array['T 'N]`, `array[array[u8 3] 2]`, `&array['T 4]`, `^array[i64 2]`;
- `count` is still a decimal literal or a length variable, with the existing bounds checks
  and their diagnostics (R8);
- the reader accepts `array` followed by `[` with or without intervening space, matching
  `parse_type_arguments`' treatment of `Slice[T]`/`Box[i64]` (no glue rule). Nothing is
  ambiguous, because `array` is reserved (R3).

**The six bracket-dispatch sites** — cited by their `quotation_type_ahead()` call line, the
same convention as the codebase map: `parse_poly_slot:3251`, `parse_slot:3933`,
`parse_type_expr:3995`, `quotation_effect_opens_here:4039`, `parse_field_type_expr:4468`,
`parse_generic_field_shape:5242`. Each becomes: a bare `[` opens a **quotation effect**;
the word `array` followed by `[` opens an **array**. The existing array parsers
(`parse_poly_array`, `parse_array_type_expr`, `parse_generic_field_array`) keep their
bodies and are entered past the `array` token, consuming the `[` exactly as today.

**R1a — `&array[…]` / `^array[…]` need their own interception; the six sites cannot
reach them.** `&` and `^` are *not* lexer delimiters, so `&array[i64 4]` lexes as
`Word("&array")`, `LBracket`, … — the `&array` word never passes a `[`-dispatch site.
Today `parse_ref_type_expr` (`4103`) hands the non-`^` non-empty remainder `array` to
`resolve_type_or_apply` (`4120`), and `split_owning_cell_word` (`4054`) does the same at
`4089`; both would report "unknown type `array`".

The mechanism is a **name-keyed interception in the two splitters and their poly-path
equivalents** (`parse_poly_slot`'s `&`/`^` arms), recognising an `array` remainder ahead of
the user type registry the same way `resolve_type_or_apply:5035` recognises `Slice`/`!Slice`
and `split_owning_cell_word:4066` recognises `OWNING_QUOTATION_KEYWORD`:

**Concrete path.**

- in `parse_ref_type_expr` (`4103`), when `remainder == ARRAY_TYPE_NAME`, read the array
  from the ongoing token stream (the `[` is the next token) via `parse_array_type_expr`
  (`4133`) instead of calling `resolve_type_or_apply` (`4120`);
- likewise in `split_owning_cell_word` (`4054`, remainder arm `4089`), beside its existing
  `owning` arm (`4066`).

**Poly path — dispatch into `parse_poly_array`, not the concrete array reader.**
`parse_poly_slot`'s `&` arm (`3265–3296`) and `^` arm (`3298–3339`) each handle exactly two
remainder shapes today — empty (bare sigil, recurses into `parse_poly_slot`) and
`'`-prefixed (glued variable, interned from the substring) — and **fall through to the
concrete readers** (`parse_ref_type_expr` / `parse_owning_cell_type_expr` via
`parse_type_expr`) for every other remainder. A concrete reader resolves the array's element
concretely, so `&array['T 'N]` / `^array[i64 2]` inside a `PolySig` would die blaming `'T`
as an unknown type — the exact misreport those arms exist to prevent. Requirement:

- each poly arm gains a third case, `remainder == ARRAY_TYPE_NAME` **and** the next token
  is `[`, which advances past the sigil word and calls **`parse_poly_array` (`3617`)**,
  threading `builder` and `word_is_output` through, then wraps the result in
  `RawTy::Ref` / `RawTy::OwnedCell` exactly as the two existing cases do;
- this case must be tested **before** the arm's fall-through to the concrete readers, i.e.
  in the same `if let Some((Token::Word(w), span)) = self.peek()` block as the empty and
  `'`-prefixed cases, not after it. Ordering here is the whole point of R1a: placed after
  the fall-through it is dead code.

Today's bare-sigil spelling `&['T 4]` already reaches `parse_poly_array` through the
empty-remainder recursion, so this is a new *glued* shape, not a regression — but see the
next paragraph, where the generic-field twin **is** a regression.

**R1a (continued) — `parse_generic_field_shape` is a third site with its own `&`/`^` arms.**
`parse_generic_field_shape` (`5228`) does not share `parse_poly_slot`'s arms; it carries its
own `&` arm (`5267`) and `^` arm (`5323`). Both handle the empty and `'`-prefixed
remainders, and both then handle *"a run glued to a generic header that is then applied"* by
calling `poly_generic_header(&remainder, …)` (`5298–5315` for `&`). `poly_generic_header`
looks the remainder up in the user struct/enum registries, which will never contain `array`,
so `&array['T 4]` in a generic struct field falls past it into the concrete parser and is
reported as an unknown type.

This is a **regression, not merely an unsupported new shape**: today's spelling `&['T 4]`
parses in that position via the bare-sigil recursion at `5271–5275`
(`parse_generic_field_shape` → `LBracket` → `parse_generic_field_array`), so migrating a
generic struct field to `array[…]` would break a field that builds at HEAD. Requirement:
both arms gain the `remainder == ARRAY_TYPE_NAME` + next-token-`[` case, dispatching into
**`parse_generic_field_array` (`5419`)** with `decl_name`/`ty_vars`/`used` threaded through,
folded by the arm's existing `fold_field_ref` / owning-run wrap. Placed **before** the
`poly_generic_header` case, since `array` must be recognised ahead of the user registry
exactly as `resolve_type_or_apply:5035` recognises `Slice`.

All three sites are explicit phase-1 scope with their own tests
(`parse_ref_type_expr_named_array_parses`, `split_owning_cell_word_named_array_parses`,
`parse_poly_slot_ref_named_array_parses`, `parse_poly_slot_owned_cell_named_array_parses`,
`parse_generic_field_shape_ref_named_array_parses`,
`parse_generic_field_shape_owned_cell_named_array_parses`).

**R1a — the generic-field co-assertion is transitional, and its phase-4 disposition is
named.** The two generic-field tests carry a co-assertion that today's `&['T 4]` field
spelling *still builds*, which is what proves the new arm did not wrongly dispatch into
`poly_generic_header` or a concrete reader. That co-assertion is only valid while the legacy
bare-`[`-as-array path survives, i.e. phases 1–3. In phase 4 the bare `[` becomes a
quotation unconditionally, so `&['T 4]` in a generic field routes into
`parse_generic_field_shape`'s quotation arm (`5241`) and is rejected. Its successor, phase 4
only: **`parse_generic_field_shape_bare_bracket_after_retirement_is_a_quotation_error`**,
listed beside R10's other phase-4 successors. Two assertions, because the arm's ty-var scan
runs ahead of the reader:

- `&['T 4]` reports `quotation_field_ty_var_error` — `quotation_effect_ty_var_ahead` (`5247`)
  sees the declaration's `'T` inside the bracket and fires before
  `parse_quotation_type_expr` (`5250`) can reach R4a's validator;
- the concrete twin `&[i64 4]` reports R4a's missing-`--` error, which is the path through
  the validator.

So the flip is pass → *rejected with a pinned message*, not pass → fail.

**R1b — ruling: a slot *named* `array` needs no special-case code.** Earlier drafts asked
for a `:`-lookahead exemption mirroring `parse_slot`'s `owning` exemption at `3966–3968`.
That exemption is **not needed**, and specifying one would ship an unreachable mechanism
plus a placebo test for it. Two independent reasons, both verified at HEAD:

- R1's dispatch predicate is *the word `array` **followed by `[`***. In `array : i64` the
  next token is the `:` word, so no dispatch site is entered. `owning` needed its guard
  because `owning_quotation_ahead` keys on the *keyword alone*; R1's predicate already
  carries the discriminator in its shape.
- `parse_slot`'s name-then-optional-`:type` read at `3972–3975` consumes `array` as a
  **slot name** and never resolves it as a type, so R2's error cannot reach it (see R2's
  pinned raise site below).

So: **no code**, but keep the coverage. `parse_slot_named_array_with_type_annotation_parses`
stays in the test plan as an ordinary regression test that `array : i64` is a named slot —
documented there as *not* mutation-testable, because it guards no gate.

### R2 — `array` not followed by `[` is a located error

`array` in a type position with no following `[` reports a located error naming the
required form (`array[T N]`), not "unknown type `array`". One error function
(`array_without_bracket_error`), in the style of `generic_arity_error`.

**Pinned raise site: `resolve_type_or_apply` (`5030`), and nowhere else.** That function is
the single funnel through which a bare word is resolved *as a type name* — reached from
`parse_slot:3982`, `parse_type_expr`, `parse_field_type_expr`, and (after R1a) from the
fall-through of `parse_ref_type_expr:4120` and `split_owning_cell_word:4089` once their
`array` interception declines for want of a `[`. So one arm, keyed on
`name == ARRAY_TYPE_NAME`, placed beside the existing `Slice`/`!Slice` arm at `5035–5037`,
covers every reader including R1a's splitter and generic-field arms: `&array --` reports
"`array` must be followed by `[T N]`", not "unknown type `array`". Because a *named slot*
never calls `resolve_type_or_apply` on its name half, this raise site is also what makes
R1b's exemption unnecessary rather than merely untested.

### R3 — `array` is a reserved type/variant name

`reject_reserved_name` (`src/parser.rs:212`) gains an `array` arm alongside the
`Slice`/`!Slice` arm (`219`) and the `owning` arm (`229`), gated on `kind` being `type` or
`variant`, with a message in the same shape ("reserved for the array type syntax
(`array[T N]`)"). A new `pub const ARRAY_TYPE_NAME: &str = "array";` sits beside
`SLICE_TYPE_NAME` (`242`) and is the one spelling every reader compares against. A word,
local, field or **slot** named `array` stays legal (R1b), exactly as for `owning`/`Slice`.

### R4 — a bare `[` in a type position is a quotation effect, unconditionally

`quotation_type_ahead` (`4150`) is deleted and its six call sites collapse to a single
`LBracket` peek. `quotation_effect_opens_here` (`fn` at `4038`, its `quotation_type_ahead()`
call at `4039`) reduces to "the next token is `[`" and stays as the one named predicate the
`owning` readers (`3353`, `4027`, `4076`) call.

Consequence to be handled, not left implicit: today a quotation reader is only entered once
a depth-1 `--` is known to exist — an invariant `parse_quotation_effect_rows`' doc comment
at `4174–4175` states outright. After R4, a bracket with no `--` (`( [ i64 i64 ] -- )`)
reaches the quotation reader and must produce a located error naming the missing `--` and
the `array[T N]` alternative.

**R4a — the detection point.** The obvious placement (make `parse_quot_type_list`/
`parse_poly_quot_list` stop on a top-depth `]`) **cannot work**: those loops dispatch every
unmatched token to `parse_type_expr`/`parse_poly_slot`, which fails on a bare count token
(`4` in `[ i64 4 ]` is `Token::Int`, hitting `parse_type_expr`'s fallthrough) *before* the
loop can observe the `]`. The diagnostic would never fire from there.

Instead, the scan moves inside the readers as a **validator**: `quotation_type_ahead`'s
matching-bracket walk is rewritten as

```rust
fn require_top_depth_arrow(&self, depth_base: i32) -> Result<(), String>
```

— the same walk over `self.tokens` from `self.pos`, seeded at `depth_base` instead of `0`,
returning `Ok(())` where the old one returned `true` and the located
`quotation_effect_missing_arrow_error` where it returned `false`. `depth_base` is not
cosmetic: it is the fix for the entry-point mismatch below, and the parameter must be
explicit rather than defaulted, so no caller can be added without ruling on it.

**R4a(i) — the poly reader is entered PAST its bracket, so its depth base is 1, not 0.**
`parse_poly_quotation_inner` (`3521`) is documented at `3515–3519` as *"positioned just past
its opening bracket"*, and all **three** of its callers have already consumed that bracket:

| Caller | Line | How the bracket was consumed |
| --- | --- | --- |
| `parse_poly_slot`'s `~[` arm | `3248` | `self.pos += 1` over the single `Token::TildeLBracket` (Slice 10a R1) |
| `parse_poly_slot`'s `owning` arm | `3360` | `self.pos += 1` over the `Token::LBracket`, after `quotation_effect_opens_here` |
| `parse_poly_quotation` | `3512` | `self.expect(Token::LBracket)?` at `3511` |

Earlier drafts said the validator is called *"positioned on the `[`"* at `3521`. That is
false for every one of the three callers, and implementing it literally at `depth_base: 0`
makes a legal `~[ 'T -- Bool ]` walk with depth `0`, hit the closing `]` before any `--`,
decrement to `-1`, never satisfy the `depth == 0` stop, run to EOF and return the missing-`--`
error — i.e. a false rejection of every inline combinator in `lib/combinators.sth`.

Ruling, one mechanism, uniform across all three callers: **call the validator once, on the
first line of `parse_poly_quotation_inner`, as `self.require_top_depth_arrow(1)?`.** Do not
call it in the three callers. Depth base `1` states "we are already inside one bracket", so
the walk's `--`-at-depth-1 test and its `depth == 0` stop both land where they do on the
concrete path. The concrete path is the only `depth_base: 0` caller: it is
`parse_quotation_effect_rows` (`4184`), which *is* positioned on the `[` (its doc comment at
`4181` says so and its body opens with `expect(Token::LBracket)` at `4185`), so the
validator goes **before** that `expect`, as `self.require_top_depth_arrow(0)?`.

Exactly two call sites, therefore: `parse_quotation_effect_rows` at base `0`,
`parse_poly_quotation_inner` at base `1`. The four list readers
(`parse_quot_type_list:4196`, `parse_poly_quot_list:3574`) stay unchanged.

**R4a(iii) — the error must not offer `array[T N]` to an author who wrote `~[`.** The base-1
site serves *both* openers: `parse_poly_slot`'s `~[` arm (`3248`, a `Token::TildeLBracket`)
and the two `Token::LBracket` callers (`3360`, `3512`). But `~[` has **no array reading at
all** — `parse_slot` (`3927`), `parse_type_expr` (`3992`), `parse_field_type_expr` (`4465`)
and `parse_generic_field_shape` (`5235`) each reject a bare `Token::TildeLBracket` outright
via `tilde_quotation_position_error` — so "or write `array[T N]`" sends that author somewhere
the parser refuses. Requirement: `quotation_effect_missing_arrow_error` takes an
`opened_with_tilde: bool` and **drops the `array[T N]` clause when it is true**, keeping the
missing-`--` half. `require_top_depth_arrow` computes it, so no caller has to:

```rust
let opened_with_tilde = depth_base > 0
    && matches!(self.tokens.get(self.pos - 1), Some((Token::TildeLBracket, _)));
```

(sound because every base-1 caller consumed exactly one opener token immediately before
entry, per R4a(i)'s table; at base `0` the parser is *on* the `[`, so the flag is false by
construction). The two named missing-arrow tests are pinned to **different opener spellings**
so both entry points are covered independently:

- `parse_quotation_effect_missing_arrow_is_error` — plain `[`, base 0, fixture
  `: f ( [ i64 4 ] -- ) drop ;`, asserting the message **does** name `array[T N]`;
- `parse_poly_quotation_missing_arrow_is_error` — `~[`, base 1, fixture
  `: f ( ~[ i64 4 ] -- ) drop ;`, asserting the message names the missing `--` and **does
  not** mention `array[T N]`.

Mutation guard: hardcode the flag to `false` and the second test must fail.

**R4a(ii) — the depth walk must count `Token::TildeLBracket`.** `quotation_type_ahead`
(`4150–4168`) increments only on `Token::LBracket` and decrements only on
`Token::RBracket`; `Token::TildeLBracket` is a single token that opens a bracket and is
**invisible to the counter**, while its matching `]` still decrements. This is already
observable at HEAD: `: f ( [ ~[ i64 -- i64 ] 4 ] -- ) drop ;` misroutes, because the inner
quotation's `--` is seen at depth 1 and the outer array bracket is read as a quotation
effect — `parse error: expected a word, found Int(4)`. (The all-`[` twin
`: f ( [ [ i64 -- i64 ] 4 ] -- ) drop ;` parses as an array, correctly, because the nested
`[` *is* counted.)

Carried into R4a unchanged, that blindness makes the validator **fail open**: any bracket
containing a nested `~[ … ]` passes vacuously on the inner quotation's `--`, then fails
further down with a worse diagnostic. Requirement: `require_top_depth_arrow` increments on
`Token::TildeLBracket` as well as `Token::LBracket`. Named test,
`require_top_depth_arrow_counts_a_nested_tilde_bracket`, fixture
`: f ( [ ~[ i64 -- i64 ] 4 ] -- ) drop ;`, asserting R4a's missing-`--` message naming
`array[T N]` — the outer opener is a plain `[`, so per R4a(iii) the array clause is present,
and it is the correct advice for that program, since its author meant
`array[ ~[ i64 -- i64 ] 4 ]`. Mutation guard: revert the `TildeLBracket` increment and this
test must fail (it will otherwise pass on the vacuous path).

This keeps one copy of the depth walk and guarantees the error fires ahead of any type-expr
failure. R4a is the one place in the slice where a mistake changes parsing behaviour rather
than spelling.

### R5 — `type:` and `trait:` bind their variables in brackets

- `type: Box['T] val 'T ;`, `type: Result['T 'E] | Ok 'T | Err 'E ;`, `trait: Ord['T]`.
- For `type:` and `:`, the bracket is **optional**: a bare name with no following `[` is a
  concrete (non-generic) declaration, unchanged (`type: Ordering | Less | Equal | Greater ;`).
- For `trait:`, the bracket is **mandatory at slice exit** — i.e. after phase 4, not from
  phase 2. There is no such thing as a non-generic trait, so a `trait:` carrying *neither*
  form keeps today's located "expected a type variable" error (`parse_trait_decl:2503–2507`),
  retargeted to name the bracket form (`trait: Name['T]`). Which of the two *present* forms
  is accepted when is R5b's subject, and R5b governs: phases 2–3 accept both.
- An empty bracket (`type: Box[]`, `trait: Ord[]`) is a located error.
- A duplicate variable in one bracket keeps `duplicate_generic_ty_var_error` (`1862`).
- `trait:` still admits exactly one variable; a second inside the bracket keeps
  `multi_variable_trait_error` (`356`).
- The bracket's contents are `'`-prefixed words only (no bounds on a `type:`/`trait:`
  header — bounds are a word-definition feature, R6). A non-`'`, non-`]` token inside a
  header bracket is a located error naming the expected form, never a silent break.
- The REPL's generic-`type:` rejection (`reject_generic_typedef_in_repl:1370`) fires on the
  bracketed form, so a REPL `type: Box['T] …` still gets its "not supported in the REPL yet"
  message and not a nonsense unknown-type error.
- `skip_typedef` (`4966`) and `body_has_pipe_before_semicolon`/`scan_variant_names` must
  remain correct with header brackets present (the bracket contains only `'`-prefixed
  words, so no `Pipe`/`Semicolon` enters the scanned range; assert with a test rather than
  assuming).

**R5a — `header_ty_var_count`'s replacement must accept BOTH forms until phase 4.**
`prepass_type_decls:79` uses `header_ty_var_count(tokens, i+2) > 0` to decide *not* to
register a generic header into the concrete registries. `current_typedef_is_generic:4583`
and `parse_generic_typedefs:4868` use it for the same question. If phase 2 replaces this
with a bracket-only lookahead, then for the whole of phase 2 — before phase 3 migrates the
corpus — **every** postfix header in `lib/`, `examples/`, `tests/` and the `src/` fixture
strings is misclassified as concrete and mis-parsed, turning phase 2 red corpus-wide.

Requirement: `header_ty_var_count` is replaced by
`header_is_generic(tokens, start) -> bool`, returning **`bracket_follows(tokens, start) ||
header_ty_var_count(tokens, start) > 0`** — the OR of the new and legacy shapes. All three
call sites (`79`, `4583`, `4868`) use `header_is_generic`, and all three keep the OR for
the whole of phases 2 and 3. Narrowing to bracket-only happens in **phase 4 and no
earlier**, at all three sites together (R10).

Dual acceptance applies to the type-variable **reader**, not only to this classifier:
`parse_generic_header_vars` (`src/parser.rs:5184`) keeps its postfix-reading loop
(`5189–5195`, `while` a `'`-prefixed word) as a **second arm** beside the new bracket reader
for the whole of phases 2 and 3. Replacing the reader outright would satisfy
`header_is_generic` and still break every un-migrated header, `lib/result.sth:1` and
`lib/option.sth:1` among them, three phases before phase 3 migrates them.

**R5b — `trait:` needs its own dual acceptance; R5a's OR does not reach it.**
`parse_trait_decl` (`2491`) **does not call `header_ty_var_count`**. It carries its own
inline peek at `2495–2508`: a `'`-prefixed word directly after the trait name is consumed as
the header variable, and *anything else* — including a `[` — hits the located error at
`2503–2507`. So threading `header_is_generic` through R5a's three `type:`-side callers
leaves `trait:` untouched, and a phase 2 that made the bracket mandatory for `trait:`
immediately would go red across the corpus three phases before phase 3 migrates it: §3's
inventory counts **217 postfix `trait:` occurrences in 92 files**, including `lib/cmp.sth:38`
(imported by the P7 golden tests) and `examples/traits.sth:25` and `:29`.

Requirement, mirroring R5a's `type:` treatment exactly: for the whole of **phases 2 and 3**,
`parse_trait_decl` accepts **either** form after the trait name —

- a `Token::LBracket` → the new bracketed header, read by the same bracket parser R5 gives
  `type:` (one variable only; a second inside the bracket keeps
  `multi_variable_trait_error`, `356`);
- a `'`-prefixed word → the legacy postfix variable, parsed by today's code at `2496–2500`
  and today's second-variable check at `2509`, byte-for-byte unchanged;
- **neither** → the existing located error, unchanged in *when it fires*, retargeted only in
  its message text to name `trait: Name['T]`. This case has **two** arms and both are
  retargeted, message text only: the `Some((tok, span))` arm at `2501–2506` ("expected a type
  variable (`'T`) after `trait: {name}`, found {tok:?}") and the `None`/EOF arm at `2507`
  (`self.eof_error("a type variable (`'T`)")`, different text and reached on a truncated
  source). Retargeting only the first would leave a `trait: Ord` at EOF still advising the
  postfix form.

The legacy disjunct is removed in **phase 4 and no earlier**, at which point the postfix
word becomes R10's `postfix_header_var_error`. Named guard test for the transitional window:
`parse_trait_decl_accepts_both_bracket_and_postfix_during_migration` (asserting both
`trait: Ord['T] …` and `trait: Ord 'T …` produce the same `TraitDecl`), the exact twin of
R5a's `header_is_generic_accepts_both_bracket_and_postfix_during_migration`, and
`parse_trait_decl_with_neither_form_is_still_an_error` pinning the third case.

Phase-vs-exit summary, stated once so nothing has to infer it: exit criterion 4's
"mandatory bracket for `trait:`" is a **slice-exit** (post-phase-4) statement. Within phases
2–3 both forms parse; only phase 4 makes the bracket the sole accepted spelling.

### R6 — a word's bounds live in a bracket, and only there

Grammar: `: <name> [inline] [ '<var>[: <bound>…] … ] ( <effect> ) [global: …] <body> ;`
— the bracket sits after the optional `inline` keyword and before `(`, so
`: mymax inline ['T: Copy Ord] ( 'T 'T -- 'T )` reads left to right. The same bracket is
admitted on a `trait:` member declaration (`2520`–), in the same slot relative to its own
`inline` peek (`2550`); a trait member's implicit header variable is unchanged.

**The bound-bearing occurrence is a stack slot, and stays one.** `parse_poly_ty_var`
(`3462`) parses `'T: Ord` and returns `Ok(RawTy::Var(id))` at `3500` — the token bearing
the bound *is an input slot*, not a bound-only annotation. So `: eq ( 'T: Ord 'T -- Bool )`
(`lib/cmp.sth:139`) has **two** inputs, and `: max ( 'T: Copy Ord 'T 'T -- 'T )` has three.

Migration rule, therefore: **moving a bound into the bracket never removes a slot.** The
bracket *adds* the bound declaration; the effect keeps every mention of the variable, with
the bound-bearing slot's `'T:`-prefix stripped back to a bare `'T`. Arity is preserved
exactly:

- `: eq ( 'T: Ord 'T -- Bool )` → `: eq['T: Ord] ( 'T 'T -- Bool )` (2 inputs, before and after)
- `: max ( 'T: Copy Ord 'T 'T -- 'T )` → `: max['T: Copy Ord] ( 'T 'T 'T -- 'T )` (3 inputs)
- `: f ( 'T: Show i64 -- 'T )` → `: f['T: Show] ( 'T i64 -- 'T )` (2 inputs)

A migration that deletes the bound-bearing slot changes the word's arity and is a bug, not
a re-spelling. Phase 3 must diff input counts, not just text.

Two rules that keep this a spelling change:

- **The bracket is only required for bounds.** An unbounded variable still binds at its
  first mention in the effect: `: swap2 ( 'T 'U -- 'U 'T )` is unchanged and carries no
  bracket. This keeps the migration surface to bound-bearing words rather than every
  polymorphic word.
- **Ids stay effect-derived.** The bracket parser must *not* pre-intern its variables into
  `PolyBuilder` via `intern_ty_var`: that would number ids in bracket order rather than
  effect first-mention order, changing `PolySig.ty_var_names` order and therefore
  `instantiation_symbol` output (`sooth_mono_…__t0_…`) for any word whose bracket order
  differs from its effect order. Instead, parse the bracket into a local
  `Vec<(name, Span, Vec<Bound>)>` side table, parse the effect exactly as today, then
  attach each entry's bounds to the id the effect interned. A bracket-declared variable
  that never appears in the effect is a located error (it would otherwise leave a bound on
  a variable with no slot, a shape the checker has no path for).

**R6a — the bracket's bound-list grammar.** Inside the bracket:

```text
bracket    := '[' var_decl+ ']'
var_decl   := TYVAR [ ':' bound_list ]        -- TYVAR is a `'`-prefixed word;
                                                 the `:` may be glued (`'T:`) or spaced
bound_list := bound+                          -- one or more capability/trait names
```

Termination is positional and total, with no "next slot" fallback:

- a `bound_list` ends at the next `'`-prefixed word (the next `var_decl`) or at `]`;
- `['T: Copy 'U: Ord]` therefore parses as two `var_decl`s, `'T` bounded by `Copy` and
  `'U` by `Ord`; `['T 'U: Ord]` is legal (`'T` unbounded, declared for documentation, but
  see the unused-variable rule — it must still appear in the effect);
- a token inside the bracket that is neither `'`-prefixed nor `]` is a **located error**
  naming the expected form. Specifically, inside a bound list an unrecognised name is an
  unknown-capability error, not a silent break.

This last point changes `parse_capabilities` (`3665`). Its `None => break` arm at `3702`
exists precisely so a greedy bound list can stop before the enclosing effect's *next input
slot* — a situation that cannot arise inside a bracket, where the only things that can
follow a bound are another `'`-var or `]`. `parse_capabilities` gains a bracket mode (a
flag, or a caller-supplied terminator predicate) in which the `None` arm errors instead of
breaking. `parse_impl_bounds:2763` (the `where`-clause caller) keeps today's behaviour
unchanged.

**Consequence for two existing tests** (both verified present at HEAD):

- `parse_capabilities_stops_before_a_following_type_slot` (`src/parser.rs:9730`) — its
  subject (the greedy list ending at the enclosing effect's next slot) is *destroyed* by
  the bracket grammar. **Retarget**, do not retire: rename to
  `parse_bound_bracket_ends_at_close_and_effect_follows`, fixture
  `: f['T: Show] ( 'T i64 -- 'T ) ;`, same two assertions (`sig.bounds == [(0, User(1))]`,
  `sig.inputs == [Var(0), Concrete(I64)]`). Arity is identical either way and that is the
  point: the old fixture's `'T: Show` token *was* the first input slot, so both spellings
  have two inputs. The migration only strips the `: Show` off that slot and restates it in
  the bracket; the `sig.inputs` assertion is unchanged, byte for byte, from the old test.
- `parse_capabilities_unbound_qualifier_after_a_bound_is_the_next_slot` (`9756`) — its
  subject (an unresolvable qualifier past bound #1 falling through to the next slot rather
  than raising the bound-qualifier error) is destroyed outright: inside a bracket there is
  no next slot, so `['T: Copy2 q::Point]` must now *error*. **Retire and replace** with
  `parse_bound_bracket_unknown_name_after_a_bound_is_an_error`, asserting the located
  unknown-capability/unbound-qualifier error, plus a companion
  `parse_bound_bracket_qualified_type_in_effect_still_resolves_as_a_slot` keeping the
  original's real-world case alive on the effect side (`: f['T: Copy2] ( 'T q::Point -- 'T )`).
  `parse_capabilities_rejects_an_unbound_qualifier_in_a_bound` (`9746`) survives, migrated
  to bracket spelling.

Poly/concrete routing: a word carrying a non-empty bracket takes the `PolySig` path
regardless of `effect_has_variable` (`3143`), which is unchanged and still decides the
bracketless case.

### R7 — a bound inside an effect is an error

`( 'T: Ord 'T -- )` is rejected with a located error naming the bracket form. Mechanically:
`parse_poly_ty_var` (`3462`) keeps its glued-colon (`'T:`) and standalone-`:` *detection* at
`3468`/`3478–3480` but no longer calls `parse_capabilities` (`3486`) — a detected bound in an
effect is now an immediate located error, not a parsed one.

**R7a — which error, at which of the two call sites.** The bound-detection logic has exactly
two entry paths, and moving the rejection *into* `parse_poly_ty_var` moves the decision with
it, so the choice must be made there rather than after the fact:

| Path | `forbid_bounds` | Error raised, and from where |
| --- | --- | --- |
| word-def / trait-member effect (`parse_poly_effect:3176` → `parse_poly_slot:3237` → `parse_poly_ty_var`) | `false` | the new `bound_in_effect_error`, raised **inside `parse_poly_ty_var`** the moment `bound_follows` is true |
| `impl:` target (`parse_impl_target:2728` → `parse_poly_slot` → `parse_poly_ty_var`) | `true` | `impl_target_bound_error` (`420`), also raised **inside `parse_poly_ty_var`**, selected by `builder.forbid_bounds` |

So `parse_poly_ty_var`'s rejection is `return Err(if builder.forbid_bounds {
impl_target_bound_error() } else { bound_in_effect_error(&name, span) })`. The
`forbid_bounds` flag (`1540`) survives precisely because it is this selector; it also keeps
its existing narrowing role at `3478–3480` (under `forbid_bounds`, only a *glued* `'T:`
counts as a bound, so a trait member body's standalone `:` is not mistaken for one —
unchanged).

**Consequence that must be handled, not left implicit:** `parse_impl_target`'s post-hoc
check at `2733–2738` (`if !builder.bounds.is_empty() { return Err(impl_target_bound_error())
}`) becomes **dead code** — nothing can push into `builder.bounds` from an effect any more,
so the vector is always empty there. Delete those six lines along with the diagnostic's old
raise site, keeping `impl_target_bound_error` itself alive at its new raise site above. Left
in place it is an unreachable branch that a reader would take for the impl path's real gate;
removed without moving the diagnostic first, `impl: Show for 'T: Copy` would report the
word-def message. Named test: `parse_impl_target_bound_on_var_is_error` (`10386`, already
present) must keep passing **and** keep asserting `impl_target_bound_error`'s text, not the
new word-def text — that assertion is the guard for this whole item, so pin the message.

The variable itself still interns and still yields its `RawTy::Var(id)` slot in the
accepted (bracket) spelling — R7 only removes the ability to *write a bound* there.

`bound_on_use_error` (`1796`) becomes unreachable once bounds cannot be written in an effect
at all (it fired only on a non-binding occurrence that carried bounds, at `3494`). Delete the
function rather than leaving a dead diagnostic; the bracket's own duplicate/unused-variable
errors cover what remains. Two attached items, both verified at HEAD:

- **there is no *dedicated* test for it, but one test pins its message and must be
  retargeted, not deleted**: `parse_trait_decl_member_bound_reports_bound_on_use_not_unknown_capability`
  (`src/parser.rs:9064`) asserts `"must be written at its binding"` on the fixture
  `trait: Show 'T : show ( 'T: Copy -- ) ; ;`. That is a bound in a trait-member effect,
  i.e. `forbid_bounds == false`, so under R7a it now raises `bound_in_effect_error`. Phase 4
  retargets the assertion to that message and renames the test
  `parse_trait_decl_member_bound_in_effect_is_error`; its real subject (a member-signature
  bound must not misreport "unknown capability `Copy`") survives intact. Phase 3 migrates its
  fixture's postfix header separately;
- `parse_impl_bounds`' doc comment (`src/parser.rs:2759`) names `bound_on_use_error` in prose
  to explain why the `where`-clause does not reuse `parse_poly_ty_var`. Deleting the function
  leaves that reference dangling; reword it in the same commit. `parse_capabilities` (`3665`) is left with two live callers: the
new bracket parser (R6a, bracket mode) and `parse_impl_bounds` (`2763`).

### R8 — `impl:` target arrays, and the rendered array spelling

- `impl: Show for array['T 'N]`; falls out of R1 through `parse_poly_slot`.
  `parse_impl_target`'s `forbid_bounds: true` and its row-variable rejection are unchanged.
- `intern_array_type` (`src/ast.rs:1463`) mints `array[i64 4]` instead of `[i64 4]`, so
  every diagnostic and pretty-printer that renders an array picks the new spelling up
  through `name_static` with no per-site change. The REPL's `<[T N]>` placeholder
  (`src/repl.rs:759`) follows automatically (it interpolates `name`).

**R8a — ruling: this changes emitted symbol names, and we accept it.** `Type::name()`
returns `name_static` for `Type::Array`, and `instantiation_symbol` (`src/ast.rs:2176`)
builds the emitted QBE symbol from `sanitize(ty.name())` at `2184`. Renaming `[i64 4]` →
`array[i64 4]` therefore changes the symbol of every monomorph instantiated at an array
type (`sooth_mono_w__t0__i64_4_` → `sooth_mono_w__t0_array_i64_4_`). This is **not**
display-only and the spec does not claim otherwise.

Blast radius, measured at HEAD: no test pins an array-typed `sooth_mono_*` symbol. Every
pinned symbol found by
`grep -rho "sooth_mono_[A-Za-z0-9_]*" tests/ src/ | sort -u` instantiates at a scalar or
struct type (`__t0_i64`, `__t0_str`, `__t0_Bool`, `__t0_Pt`, …); the `sooth_mono_*` symbols
in `tests/qbe_baseline/array_*.ssa` are `lt`/`gt`/`eq` at `i64`, i.e. unaffected.

Ruling: **accept the rename** (option (a)); do not re-key `instantiation_symbol`. Phase 3
must re-run that grep after migrating and update any array-typed pinned symbol it finds
(expected: none, but the grep is a required phase-3 step, not an assumption).

- Diagnostic prose that hardcodes the old shape is updated: `src/parser.rs:3637`, `4411`,
  `4418`, `4423` (array-length errors).

**R8b — three renderers build the array shape by hand and bypass `name_static` entirely.**
The `name_static` change propagates only to code that asks a `Type` for its name. Three
functions instead `format!("[{} {}]", …)` from a `PolyType`, so they are *not* carried along,
and — because they spell no literal `[T N]` or `[elem count]` — they are also invisible to
phase 3's doc-comment sweep. All three verified at HEAD:

| Renderer | Array arm | Ruling |
| --- | --- | --- |
| `poly_type_str`, `src/check/poly.rs:8460` | `8472` | **Change to `array[…]`.** It is the user-facing poly diagnostic renderer ("a variable by its declared spelling … an array structurally"), and R2/R8's whole claim is that a diagnostic's array spelling is copy-pasteable source. Exactly **one** existing test pins an array shape and needs migrating: `poly_type_str_renders_a_reference` (`src/check/poly.rs:12254`), whose last assertion renders `&['T 4]` → `&array['T 4]`. The other two tests in that family are *not* affected and were falsely cited by earlier drafts: `poly_type_str_renders_a_generic_application` (`12271`) asserts `"Result['T 'E]"`, a `PolyType::Generic` that never reaches the array arm, and `poly_type_str_renders_slice` (`12412`, assertions at `12417–12423`) asserts `"Slice[i64]"`/`"!Slice[i64]"`, a `PolyType::Concrete` over an interned slice. Neither renders an array; neither changes under this slice. |
| `generic_field_type_str`, `src/parser.rs:1985` | `1996` | **Change to `array[…]`.** Its doc comment calls it "the surface spelling of a generic `type:` field's type, in the declaration's own variable spellings" — user-facing by construction. |
| `poly_type_shape_str`, `src/parser.rs:386` | `395` | **Exempt, left as `[…]`.** Verified against its doc comment at `382–385`: it renders "a `PolyType` target shape for the synthesized member word name … positional ids (`'T0`, `'N0`) since the synth name is a compiler-internal spelling, **never shown to the user**". Same reasoning as `type_arg_key` below: it is an identity key, and changing it renames synthesized `member;Trait;Type` words for no benefit. |

Phase 3's sweep must therefore include a second grep,
`grep -rn 'format!("\[{} {}\]"' src/` (5 hits at HEAD: the three above plus
`src/ast.rs:760` `type_arg_key` and `src/ast.rs:1462` `intern_array_type` itself), and
reconcile every hit against this table. Without it exit criterion 7 ("no diagnostic prose
spells the old shape") is asserted rather than enforced — the `[T N]`/`[elem count]` grep
cannot see any of them.

- **Doc comments spelling `[T N]`/`[elem count]`: the list below is representative, not
  exhaustive.** The exhaustive gate is the phase-3 sweep command
  `grep -rn '\[T N\]\|\[elem count\]\|\[ elem count \]' src/` plus a full `cargo test` run,
  not this list. Known non-test sites at HEAD: `src/ast.rs:1273`, `2284`;
  `src/parser.rs:1253`, `3614`, `3987`, `4125`, `4400`, `4460`, `5415`;
  `src/ir/layout.rs:248`, `427`; `src/backend/qbe.rs:279`, `285`;
  `src/ir/func_builder/word_families.rs:461`; `src/repl.rs:759`, `4323`.
- `type_arg_key`'s array arm (`src/ast.rs:758`) is **left as `[elem count]`**: it is the
  generic-instantiation registry key (identity, deduped and compared), not a surface
  spelling, and changing it would change registry names for no stated benefit. The
  resulting cosmetic divergence (`Box[[i64 4]]` in an instantiation name vs `array[i64 4]`
  elsewhere) is a known wart; do not "fix" it inside this slice.

### R9 — the migration is exhaustive and semantics-free

Every `.sth` file in `lib/`, `examples/` (including `examples/experiments/`) and every test
fixture — in `tests/*.rs` and in the `#[cfg(test)] mod tests` blocks under `src/` — reads in
the new syntax. Expected-diagnostic assertions that quote an array spelling or an in-effect
bound are updated to what the new parser emits. No test is deleted, weakened, or retargeted
to a different subject as part of the migration; if a migrated fixture stops reproducing its
subject, stop and report it rather than adjusting the assertion. (R6a's two
`parse_capabilities` tests are the one sanctioned retarget/retire, ruled on above and
scheduled in phase 2, not swept up silently in phase 3.)

Scratch file exclusion: `examples/*.tmpsth` (nine gitignored leftovers at HEAD, matched by
`.gitignore`'s `/examples/*` rule) — do not migrate or commit any of them. The rule is the
glob, not a single named file.

### R10 — the legacy spellings stop parsing (phase 4)

At phase 4 exit, each legacy form is a **located** error naming its replacement:

- a bare `[` in a type position is a quotation effect, and one with no top-depth `--` gets
  R4a's error (`quotation_effect_missing_arrow_error`);
- a bound inside an effect gets R7's error;
- a **postfix `type:`/`trait:` header variable** (`type: Box 'T …`) gets its own located
  error naming the bracket form (`postfix_header_var_error`), raised where
  `header_is_generic`'s legacy disjunct is removed. Concretely: `header_is_generic`
  narrows to `bracket_follows` at all three sites (`79`, `4583`, `4868`), and
  `parse_generic_header`/`parse_typedef` gain a check that a `'`-prefixed
  token directly following the declared name is this error rather than falling through to
  a concrete-declaration mis-parse. Separately, and *not* covered by that narrowing,
  `parse_trait_decl` drops R5b's postfix disjunct: its `'`-prefixed arm (today `2496–2500`)
  becomes the same `postfix_header_var_error`, while its neither-form arm keeps the
  "expected a type variable" error. Named test:
  `parse_typedef_postfix_header_var_is_error` and
  `parse_trait_decl_postfix_header_var_is_error`.

---

## 3. Migration inventory (measured, with the exact commands)

All figures below are reproducible at HEAD `9c13878` with these four patterns
(PCRE, `grep -P`). Publishing them is the point: phase 3's sweep is driven by the commands,
not by the numbers.

```sh
# A. array spellings  (negative lookbehind excludes `Result['T 'E]`-style applications
#    and Rust indexing `v[i]`)
ARR="(?<![A-Za-z0-9_])\[ ?[&^!]*\[?[A-Za-z'][A-Za-z0-9_']*\]? +('[A-Za-z][A-Za-z0-9]*|[0-9]+) ?\]"
# B. a bound at a binding occurrence
BND="'[A-Za-z][A-Za-z0-9]*:(?!:)"
# C. postfix `type:` header      D. postfix `trait:` header
TYH="type: +[A-Za-z][A-Za-z0-9_]* +'"
TRH="trait: +[A-Za-z][A-Za-z0-9_]* +'"

for P in "$ARR" "$BND" "$TYH" "$TRH"; do
  grep -rPo "$P" tests src --include=*.rs | wc -l
  grep -rPo "$P" lib examples --include=*.sth | wc -l
done
```

| Pattern | `tests/*.rs` | `src/**/*.rs` | `lib/*.sth` | `examples/**/*.sth` | total |
| --- | --- | --- | --- | --- | --- |
| A arrays | 224 (20 files) | 363 (22 files) | 5 (1) | 28 (14) | **620** |
| B bounds | 183 (29 files) | 316 (20 files) | 9 (2) | 14 (6) | **522** |
| C postfix `type:` | 53 (8 files) | 129 (7 files) | 2 (2) | 0 | **184** |
| D postfix `trait:` | 87 (9 files) | 127 (7 files) | 1 (1) | 2 (1) | **217** |
| **all four** | | | | | **1 543 in 92 files** |

The 92-file figure is
`{A∪B∪C∪D over tests src --include=*.rs, and over lib examples --include=*.sth} | sort -u | wc -l`.

**The `src/` bulk is fixture strings, not code.** Of the 935 `src/` occurrences, **837 sit
at or after the file's first `#[cfg(test)]` line**, derived by:

```sh
# $ALL is the union of the four patterns defined above; it must be assigned,
# and the count must be `grep -oP | wc -l` (occurrences), not `grep -cP`
# (matching *lines*, which under-counts to 706).
ALL="$ARR|$BND|$TYH|$TRH"
for f in $(grep -rlP "$ALL" src --include=*.rs); do
  s=$(grep -n '#\[cfg(test)\]' "$f" | head -1 | cut -d: -f1)
  awk -v s="${s:-999999}" 'NR>=s' "$f" | grep -oP "$ALL"
done | wc -l
```

Heaviest: `src/check/poly.rs` 387 (360 in tests), `src/parser.rs` 272 (226),
`src/check/declarations.rs` 67 (66), `src/driver.rs` 42 (42). The ~98 non-test remainder is
doc comments plus R8's diagnostic prose.

`tests/*.rs` heaviest files: `phase7_slice4.rs`, `phase7_slice3e.rs`, `phase7_slice4b.rs`,
`phase7_slice3r.rs`, `phase4_combinators.rs`, `phase7_slice3b_follow.rs`,
`phase7_slice3n.rs`, `phase0.rs`, `phase3_refs.rs`.

**Exactly five postfix `.sth` header sites exist** (verified by pattern C∪D over `lib`
and `examples`):

```text
lib/result.sth:1     type: Result 'T 'E | Ok 'T | Err 'E ;
lib/option.sth:1     type: Option 'T | None | Some 'T ;
lib/cmp.sth:38       trait: Ord 'T
examples/traits.sth:25  trait: Order 'T
examples/traits.sth:29  trait: Show 'T
```

(The brief and earlier drafts counted four, missing `lib/result.sth`.) The header bulk is
Rust-side fixtures.

Concrete (non-generic) array slots read by `parse_slot` — e.g. `examples/inplace_fold.sth`
(5 occurrences) — are in scope for phase 3 and are the reason `parse_slot` must be taught
`array[` in phase 1 (R1, six sites).

Two migration shapes deserve care because they hoist rather than re-spell, and both must
preserve arity per R6:

- a bound nested inside a structural type — `examples/poly_borrow_first.sth:6`
  `( ['T: Copy 4] -- 'T )` → `: first['T: Copy] ( array['T 4] -- 'T )`;
  `examples/experiments/binary_search.sth` is **explicitly excluded**: its own first line
  reads `\ bin_search implemented with hypothetical grammar and functionalities`, and its
  line 14 `Slice['T: Ord 'N]` passes two type arguments to `Slice`, which takes exactly one
  (`resolve_type_or_apply:5035`), so the file does not parse at HEAD and is compiled by no
  test. Leave it byte-for-byte as-is. Do not invent a migration target for it;
- `lib/combinators.sth:66`
  `: filter inline ( ['T: Copy 'N] ~[ 'T -- Bool ] -- ['T 'N] usize )`
  → `: filter inline ['T: Copy] ( array['T 'N] ~[ 'T -- Bool ] -- array['T 'N] usize )`.

`tests/corpus_stdout/*.txt` holds program stdout only (no type spellings), so it should
need no edit; verify rather than assume. `tests/qbe_baseline/*.ssa` holds emitted symbols
and is covered by R8a's grep step.

---

## 4. Test plan

Per CLAUDE.md: parser unit tests beside the stage code, happy path plus an error case,
`thing_condition_expected` naming, and the exit criteria as goldens.

New/updated `src/parser.rs` unit tests:

- `parse_poly_slot_named_array_parses` / `..._nested_named_array_parses`
- `parse_slot_named_array_parses` and `parse_slot_named_array_with_type_annotation_parses`
  (R1b: a slot *named* `array`, `array : i64`). Flagged in the mutation plan as **not
  mutation-testable** — R1b rules that no special-case code exists, so these guard no gate
  and there is nothing to revert. Keep them as ordinary regression coverage.
- `parse_ref_type_expr_named_array_parses`, `split_owning_cell_word_named_array_parses`,
  `parse_poly_slot_ref_named_array_parses`,
  `parse_poly_slot_owned_cell_named_array_parses`,
  `parse_generic_field_shape_ref_named_array_parses`,
  `parse_generic_field_shape_owned_cell_named_array_parses` (R1a, all five interception
  sites). The two generic-field tests are **regression** tests, not new-feature tests: assert
  alongside them that today's `&['T 4]` field spelling still builds, so the pair fails if the
  arm dispatches into `poly_generic_header` or a concrete reader instead of
  `parse_generic_field_array`. That co-assertion is **phases 1–3 only**; its phase-4 successor
  is `parse_generic_field_shape_bare_bracket_after_retirement_is_a_quotation_error`, spelled
  out in R1a.
- phase 4 only: `parse_poly_slot_bare_bracket_is_quotation` (an array-shaped bare bracket,
  `[ i64 4 ]`, is a quotation-effect error, not an array)
- `parse_type_expr_array_without_bracket_is_error`,
  `parse_ref_type_expr_array_without_bracket_is_error` (R2, both entry shapes)
- phase 4 only: `parse_quotation_effect_missing_arrow_is_error` (R4a, concrete reader, depth
  base 0, opener `[`, message **names** `array[T N]`) and
  `parse_poly_quotation_missing_arrow_is_error` (poly reader, depth base 1, opener `~[`,
  message names the missing `--` and **not** `array[T N]`, per R4a(iii)) — the two spellings
  are pinned separately so both entry points get independent coverage. Plus
  `parse_poly_quotation_legal_inline_effect_still_parses`, the guard against R4a(i)'s failure
  mode, which must use a `~[ 'T -- Bool ]` parameter and would fail outright if the validator
  ran at depth base 0
- phase 4 only: `require_top_depth_arrow_counts_a_nested_tilde_bracket` (R4a(ii), fixture
  `: f ( [ ~[ i64 -- i64 ] 4 ] -- ) drop ;`, pinning the missing-`--` message text)
- `reject_reserved_name_array_type_is_error` / `..._variant_is_error`; and a word named
  `array` still parses (R3)
- `parse_typedef_generic_header_brackets_parses`, `..._empty_bracket_is_error`,
  `..._duplicate_var_is_error`, `..._bare_name_is_concrete`,
  `..._non_var_token_in_bracket_is_error`
- `parse_trait_decl_bracket_header_parses`, `..._two_bracket_vars_is_error`,
  `parse_trait_decl_with_neither_form_is_still_an_error` (the `2503–2507` arm, unchanged in
  timing)
- `header_is_generic_accepts_both_bracket_and_postfix_during_migration` (R5a's OR, asserted
  in phases 2–3) and its `trait:` twin
  `parse_trait_decl_accepts_both_bracket_and_postfix_during_migration` (R5b, likewise phases
  2–3); `header_is_generic_rejects_postfix_after_retirement` and
  `parse_trait_decl_postfix_header_var_is_error` are the phase-4 counterparts that replace
  them
- `parse_worddef_bound_bracket_parses`, `..._bracket_after_inline_parses`,
  `..._bound_in_effect_is_error` (R7), `..._bracket_var_unused_in_effect_is_error`,
  `..._bracket_var_id_order_follows_effect` (R6's id-order rule, asserted on
  `PolySig.ty_var_names`),
  `parse_worddef_bound_bracket_preserves_effect_arity` (R6's arity rule: `: eq['T: Ord]
  ( 'T 'T -- Bool )` has two inputs)
- `parse_bound_bracket_ends_at_close_and_effect_follows`,
  `parse_bound_bracket_unknown_name_after_a_bound_is_an_error`,
  `parse_bound_bracket_qualified_type_in_effect_still_resolves_as_a_slot`,
  `parse_bound_bracket_multiple_bound_vars_parse` (`['T: Copy 'U: Ord]`, R6a)
- `parse_impl_target_named_array_parses`
- `repl_generic_typedef_bracket_header_is_rejected` (R5's REPL gate)
- `repl_word_def_bound_bracket_parses` and `repl_word_def_bound_in_effect_is_error`
  (`parse_line_with_structs:1257` — the REPL word-def path, which this project has a
  documented history of regressing separately from the file path)
- `skip_typedef_with_bracket_header_skips_whole_decl` and an enum-detection test for a
  bracketed header (R5's scan-correctness caveat)
- phase 3 only: `intern_array_type_renders_named_array` (R8's `name_static` change),
  `array_index_out_of_bounds_error_names_the_new_spelling` (one migrated diagnostic
  message, per CLAUDE.md's "diagnostics are behaviour"), and R8b's two renderer tests —
  `poly_type_str_renders_a_reference` (`src/check/poly.rs:12254`, migrated to expect
  `&array['T 4]`) and a new `generic_field_type_str_renders_named_array`. Assert *no* change
  to `poly_type_shape_str`'s output (the ruled-on exemption) so a well-meaning sweep cannot
  quietly rename synthesized member words
- phase 4 only: `parse_typedef_postfix_header_var_is_error`,
  `parse_trait_decl_postfix_header_var_is_error` (R10),
  `parse_generic_field_shape_bare_bracket_after_retirement_is_a_quotation_error` (R1a's
  co-assertion successor) and `parse_trait_decl_member_bound_in_effect_is_error` (R7's
  retarget of the `bound_on_use_error` assertion at `src/parser.rs:9064`)

Goldens: `examples/gcd.sth`, `examples/factorial.sth`, the `lib/cmp.sth` trait/impl family
and `lib/combinators.sth` build and run unchanged in the new syntax (existing golden tests,
migrated sources).

Mutation-test the new gates before declaring phase 4 done (this project has shipped placebo
tests repeatedly): revert R2's error, R4a's missing-`--` error, R7's in-effect-bound error,
R6's unused-bracket-variable error, R6a's bracket-mode unknown-name error, R10's
postfix-header error and R3's reserved-name arm one at a time and confirm a named test
fails for each. Note especially that a test asserting "a bare `[ i64 4 ]` is rejected" can
pass for the wrong reason (an older blocker upstream), so pin the message text. Commit
before mutation testing.

---

## 5. Phase sequencing rationale

Every phase must leave `cargo fmt --check && cargo clippy -- -D warnings && cargo test`
green, and the mechanism change and the ~1 543-occurrence migration cannot land in the same
commit without a wall of unrelated churn. So the parser accepts **both** spellings for the
duration of phases 1–2, the corpus migrates in phase 3 against a parser that already
accepts the new form, and phase 4 retires the old forms and deletes `quotation_type_ahead`.
Transitional dual acceptance is a within-slice scaffold, not a kept convenience: phase 4's
exit is that no legacy spelling parses.

During phases 1–3 the legacy bare-`[` array path keeps `quotation_type_ahead` as its
disambiguator, and `header_is_generic` keeps R5a's OR, so behaviour for un-migrated sources
is bit-identical.

## 6. Exit criteria

1. No `[` opens a **type** without a preceding name (`array[`, `Slice[`, `!Slice[`, `Box[`,
   `Result[`), including behind `&`, `&!` and `^` (R1a). The slice's own R5/R6 brackets are
   binding sites, not types, and are preceded by a declaration or word name
   (`type: Box['T]`, `: max['T: Copy Ord] ( … )`).
2. `quotation_type_ahead` does not exist; a bare `[` in a type position opens a quotation
   effect, and a bracket with no top-depth `--` yields a located error naming `--` and
   `array[T N]`, raised from `require_top_depth_arrow` — at depth base `0` from
   `parse_quotation_effect_rows` (positioned on the `[`) and depth base `1` from
   `parse_poly_quotation_inner` (entered past it), counting `Token::TildeLBracket` (R4a).
3. `array` is reserved as a `type:`/`variant` name, unparseable as an array without its
   bracket (R2's raise site is `resolve_type_or_apply`), and still legal as a slot/field/word
   name with no special-case code (R1b).
4. **At slice exit (post phase 4):** `type:` and `:` bind type variables in an **optional**
   bracketed list (a bare name is a concrete declaration); `trait:` binds its single variable
   in a **mandatory** bracket. During phases 2–3 both `type:` and `trait:` accept the bracket
   *or* the legacy postfix variable (R5a, R5b) — this criterion is not a phase-2 gate.
5. A word's bounds parse only in its bracket; a bound inside an effect is a located error;
   the bracket never changes a word's effect arity (R6).
6. `impl: Show for array['T 'N]` parses; `impl:` targets still forbid bounds and rows.
7. `ArrayDecl::name_static` renders `array[i64 4]`; `poly_type_str` and
   `generic_field_type_str` follow (R8b), `poly_type_shape_str` and `type_arg_key` are the
   two ruled-on exemptions; both phase-3 sweeps (`'\[T N\]\|\[elem count\]'` and
   `'format!("\[{} {}\]"'`) come back reconciled; the array-typed-monomorph symbol grep
   (R8a) is clean.
8. Every `lib/*.sth`, `examples/**/*.sth`, `tests/*.rs` fixture and `src/` unit-test fixture
   reads in the new syntax; no test deleted or weakened beyond R6a's two ruled-on cases.
9. A postfix `type:`/`trait:` header variable is a located error naming the bracket form
   (R10).
10. `cargo fmt --check && cargo clippy -- -D warnings && cargo test` green; P7 goldens
    (`gcd.sth`, `factorial.sth`, `lib/cmp.sth`, `lib/combinators.sth`) pass.
11. Re-run CLAUDE.md's split signals against `src/parser.rs` at slice exit (the file loses
    one function and gains a bracket-header parser, a bound-bracket parser and R1a's
    splitter arms; record the verdict).

## 7. Risks

- **R4a is the only behaviour-changing edit, and its entry point is the trap.**
  `parse_poly_quotation_inner` is entered *past* its bracket by all three callers, so a
  validator seeded at depth 0 there false-rejects every legal inline combinator in
  `lib/combinators.sth`; and a walk that ignores `Token::TildeLBracket` fails *open*,
  passing vacuously on a nested quotation's `--`. Both are pinned by named tests
  (`parse_poly_quotation_legal_inline_effect_still_parses`,
  `require_top_depth_arrow_counts_a_nested_tilde_bracket`).
- **R5a's OR and R5b's `trait:` twin are both load-bearing, and they are separate
  mechanisms.** `parse_trait_decl` never calls `header_ty_var_count`, so fixing only R5a's
  three `type:` callers leaves 217 postfix `trait:` occurrences — `lib/cmp.sth:38` among them,
  imported by the P7 goldens — failing for the whole of phase 2. Narrowing either mechanism
  before phase 4 turns the corpus red mid-sequence. The two
  `..._accepts_both_bracket_and_postfix_during_migration` tests are the guards for phases 2–3.
- **R7's diagnostic move is a two-site change.** Rejecting the bound inside
  `parse_poly_ty_var` without keying on `forbid_bounds` gives the `impl:` path the word-def
  message and leaves `parse_impl_target:2733–2738` as an unreachable branch that reads like
  the real gate. `parse_impl_target_bound_on_var_is_error` must pin the message text.
- **Arity drift (R6).** The bound-bearing token is a slot. A migration that drops it
  silently changes word arity; `parse_worddef_bound_bracket_preserves_effect_arity` plus a
  phase-3 input-count diff are the guards.
- **Id-order drift (R6).** Pre-interning bracket variables would renumber `PolySig` ids and
  change monomorph symbol names. The side-table rule plus
  `parse_worddef_bracket_var_id_order_follows_effect` is the guard.
- **Symbol rename (R8a).** Accepted, blast radius measured as zero today, but phase 3 must
  re-run the `sooth_mono_*` grep rather than trusting this measurement.
- **`&array`/`^array` unreachability (R1a).** The lexer does not delimit `&`/`^`, so the
  six `[`-dispatch sites cannot serve these spellings. Missing this ships a syntax that
  silently reports "unknown type `array`".
- **Migration blindness.** With 837 of the src-side occurrences inside `#[cfg(test)]`
  blocks, a `tests/`-only sweep looks complete and is not. Phase 3 must sweep `src/` too,
  with the published commands.
- **Silent test weakening.** A migrated fixture whose subject no longer reproduces must be
  reported, not adjusted.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "R1/R1a/R1b/R2/R3: the named array type. Add ARRAY_TYPE_NAME beside SLICE_TYPE_NAME (src/parser.rs:242) and an `array` arm in reject_reserved_name (212). Teach all SIX bracket-dispatch sites, cited by their quotation_type_ahead() call line (parse_poly_slot:3251, parse_slot:3933, parse_type_expr:3995, quotation_effect_opens_here:4039, parse_field_type_expr:4468, parse_generic_field_shape:5242), to enter the existing array readers on `array` + `[`. Then R1a's name-keyed interception, recognising `array` ahead of the user type registry exactly as resolve_type_or_apply:5035 recognises Slice, at FIVE arms across THREE functions: parse_ref_type_expr (4103) and split_owning_cell_word (4054) dispatch into parse_array_type_expr (4133) instead of resolve_type_or_apply (4120 / 4089); parse_poly_slot's `&` arm (3265-3296) and `^` arm (3298-3339) dispatch into parse_poly_array (3617), NOT into the concrete array reader, since a concrete reader cannot hold a type-variable element -- and this case must sit inside the same peek block as the existing empty-remainder and `'`-prefixed cases, BEFORE the fall-through to parse_type_expr, or it is dead code; parse_generic_field_shape's own `&` arm (5267) and `^` arm (5323) dispatch into parse_generic_field_array (5419), placed BEFORE their poly_generic_header case (5298-5315 for `&`), which looks `array` up in the user registries and misreports `unknown type`. The generic-field arms are a REGRESSION fix, not a new shape: `&['T 4]` builds today via the bare-sigil recursion at 5271-5275, so assert that spelling still builds beside each new one -- that co-assertion is phases 1-3 only and its phase-4 successor is parse_generic_field_shape_bare_bracket_after_retirement_is_a_quotation_error (R1a). R2: ONE raise site, an `array`-with-no-following-`[` arm in resolve_type_or_apply (5030) beside the Slice arm at 5035-5037 -- every reader funnels through it, including R1a's arms when their `[` check declines, so `&array --` reports array_without_bracket_error rather than `unknown type array` (note parse_ref_type_expr has THREE remainder branches: empty recurses at 4116, a `^`-led remainder goes to split_owning_cell_word at 4118, everything else resolves at 4120). R1b needs NO code: R1's dispatch predicate already requires a following `[`, and parse_slot's name-then-`:type` read at 3972-3975 never resolves a slot name as a type, so `array : i64` cannot trip R2; add parse_slot_named_array_with_type_annotation_parses as plain regression coverage and do NOT add a `:`-lookahead exemption. The legacy bare-`[` path and quotation_type_ahead stay in place so un-migrated sources parse bit-identically. `impl: Show for array['T 'N]` falls out via parse_poly_slot (R8 first half). Parser unit tests for every new spelling plus the R2/R3 error cases; do not touch ArrayDecl::name_static yet.",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "R5/R5a/R5b/R6/R6a: bracket binding sites, accepted alongside the legacy postfix forms. Add a bracketed type-variable list to `type:`/`trait:` headers (parse_generic_header:4760, parse_generic_header_vars:5184, parse_trait_decl:2491). DUAL ACCEPTANCE IS THE RULE FOR THIS PHASE, for BOTH declaration forms: the bracket is optional for `type:` permanently, and for `trait:` the bracket is mandatory only at SLICE EXIT (post phase 4), never in this phase. Concretely for `trait:`, per R5b: parse_trait_decl does NOT call header_ty_var_count -- it has its own inline peek at 2495-2508 -- so R5a's OR does not reach it and it needs the equivalent treatment of its own. After the trait name it accepts EITHER a Token::LBracket (the new bracketed header, one variable, second-variable case keeping multi_variable_trait_error at 356) OR a `'`-prefixed word (today's legacy postfix code at 2496-2500 plus today's second-variable check at 2509, byte-for-byte unchanged); with NEITHER form present the existing located error still fires, unchanged in timing and retargeted only in message text to name `trait: Name['T]` -- and that neither-form case is TWO arms, the Some((tok, span)) arm at 2501-2506 and the None/EOF arm at 2507 (self.eof_error), both retargeted in message text only. Making the bracket mandatory here instead would go red corpus-wide: section 3 counts 217 postfix `trait:` occurrences in 92 files including lib/cmp.sth:38 (imported by the P7 goldens) and examples/traits.sth:25 and :29, none of which migrate until phase 3. Named guard test parse_trait_decl_accepts_both_bracket_and_postfix_during_migration, plus parse_trait_decl_with_neither_form_is_still_an_error. On the `type:` side, replace header_ty_var_count with header_is_generic = bracket_follows(...) || header_ty_var_count(...) > 0 and route all three callers (79, 4583, 4868) through it; dual acceptance also applies to the type-variable READER, so parse_generic_header_vars (5184) keeps its postfix loop (5189-5195) as a second arm beside the new bracket reader through phase 3 -- replacing the reader outright satisfies the classifier requirement while still breaking lib/result.sth:1 and lib/option.sth:1; the OR must survive phases 2 and 3 untouched, or the un-migrated corpus is misclassified as concrete and phase 2 goes red corpus-wide. Keep duplicate-var, single-trait-var and REPL-generic-gate (1370) diagnostics; a non-`'`/non-`]` token inside a header bracket is a located error. Verify skip_typedef (4966) and the pipe/variant scans stay correct with a bracket present. Add the word-definition and trait-member bound bracket after the `inline` peek (parse_worddef:2302, trait member 2550), parsed into a side table and attached to effect-derived ids after parse_poly_effect (never pre-interned), with located errors for an empty bracket and for a bracket variable unused in the effect. Give parse_capabilities (3665) a bracket mode where its `None => break` fallthrough (3702) errors instead, since a bracket has no next-slot to fall through to; parse_impl_bounds (2763) keeps today's behaviour. Retarget parse_capabilities_stops_before_a_following_type_slot (9730) and retire/replace parse_capabilities_unbound_qualifier_after_a_bound_is_the_next_slot (9756) per R6a. Bounds inside an effect still parse in this phase. Unit tests for both spellings, including the id-order and arity assertions and the REPL word-def path (parse_line_with_structs:1257).",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "R9 plus R8's rendering: the corpus migration, no semantic edits. Using the published grep commands in section 3, rewrite every fixture and source to the new spelling: tests/*.rs, the `#[cfg(test)] mod tests` fixtures under src/ (837 occurrences, notably check/poly.rs, parser.rs, check/declarations.rs, driver.rs), lib/*.sth and examples/**/*.sth including experiments; ~1543 occurrences in 92 files. Apply R6's arity rule: a bound moving into a bracket leaves its slot behind (`: eq ( 'T: Ord 'T -- Bool )` -> `: eq['T: Ord] ( 'T 'T -- Bool )`, still two inputs); diff input counts, do not just re-spell. Migrate all five postfix .sth headers (lib/result.sth:1, lib/option.sth:1, lib/cmp.sth:38, examples/traits.sth:25 and :29). Change ArrayDecl::name_static via intern_array_type (src/ast.rs:1463) to `array[i64 4]`, then re-run `grep -rho \"sooth_mono_[A-Za-z0-9_]*\" tests/ src/ | sort -u` and update any array-typed pinned symbol (expected none, but the grep is required, not optional, since instantiation_symbol at src/ast.rs:2184 builds from ty.name()). Update the array-length diagnostic prose (src/parser.rs:3637, 4411, 4418, 4423) and sweep doc comments with `grep -rn '\\[T N\\]\\|\\[elem count\\]' src/` rather than trusting R8's representative list. Hoist the nested-bound cases (poly_borrow_first.sth, poly_borrow_setat.sth, lib/combinators.sth filter, experiments/arrays.sth) into bound brackets; LEAVE examples/experiments/binary_search.sth BYTE-FOR-BYTE AS-IS -- its own header line says `hypothetical grammar`, its line 14 passes two type arguments to Slice which takes exactly one, so it does not parse at HEAD and is compiled by no test; do not invent a migration target for it. Per R8b, also change the two hand-rolled user-facing array renderers that bypass name_static -- poly_type_str (src/check/poly.rs:8460, array arm 8472) and generic_field_type_str (src/parser.rs:1985, arm 1996) -- migrating poly_type_str's ONE pinned array expectation, at src/check/poly.rs:12254 (poly_type_str_renders_a_reference, `&['T 4]` -> `&array['T 4]`); do NOT touch poly_type_str_renders_a_generic_application (12271, asserts `Result['T 'E]`, a PolyType::Generic that never reaches the array arm) or poly_type_str_renders_slice (12412, assertions 12417-12423, asserts `Slice[i64]`/`!Slice[i64]` over an interned slice) -- neither renders an array and neither changes under this slice; and LEAVE poly_type_shape_str (src/parser.rs:386, arm 395) alone, verified compiler-internal by its doc comment at 382-385 (`never shown to the user`), since it keys synthesized member word names. Run a SECOND sweep, `grep -rn 'format!(\"\\[{} {}\\]\"' src/` (5 hits at HEAD), and reconcile each against R8b's table -- the `[T N]`/`[elem count]` grep is blind to all of them, so without this second sweep exit criterion 7 is asserted rather than enforced. Skip every gitignored examples/*.tmpsth (nine at HEAD). Exit criteria for this phase: intern_array_type_renders_named_array and array_index_out_of_bounds_error_names_the_new_spelling are named, passing tests; the sooth_mono grep is clean; full green check passes. Delete or weaken no test: report any fixture whose subject stops reproducing.",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "R4/R4a/R7/R10 retirement plus verification. Delete quotation_type_ahead (src/parser.rs:4150) and every legacy bare-`[`-as-array path; reduce quotation_effect_opens_here (fn at 4038, its quotation_type_ahead() call at 4039, callers 3353 / 4027 / 4076) to a single LBracket peek. Rewrite the deleted depth walk as require_top_depth_arrow(depth_base: i32) -> Result<(), String> -- same walk from self.pos, seeded at depth_base instead of 0 -- and give it EXACTLY TWO call sites at TWO DIFFERENT depth bases. (a) parse_quotation_effect_rows (4184) IS positioned on the `[` (doc comment 4181, expect(LBracket) at 4185), so call it there BEFORE that expect, with depth_base 0. (b) parse_poly_quotation_inner (3521) is entered PAST its bracket by all THREE of its callers -- parse_poly_slot's `~[` arm (3248, TildeLBracket consumed), parse_poly_slot's owning arm (3360, LBracket consumed), and parse_poly_quotation (3512, after expect(LBracket) at 3511) -- as its own doc comment at 3515-3519 states, so call it on that function's FIRST LINE with depth_base 1, and do NOT add calls in the three callers. depth_base 1 is not optional bookkeeping: at base 0 a legal `~[ 'T -- Bool ]` meets the closing `]`, goes to depth -1, never satisfies the depth==0 stop, runs to EOF and false-rejects every inline combinator in lib/combinators.sth -- so pin parse_poly_quotation_legal_inline_effect_still_parses as the guard. The walk must ALSO increment on Token::TildeLBracket, not only Token::LBracket: today's quotation_type_ahead (4150-4168) counts neither, which is already observable at HEAD (`: f ( [ ~[ i64 -- i64 ] 4 ] -- ) drop ;` misroutes to `expected a word, found Int(4)`) and which would make the validator fail OPEN, passing vacuously on a nested quotation's `--`; guard with require_top_depth_arrow_counts_a_nested_tilde_bracket on that exact fixture, pinning the message. R4a(iii): quotation_effect_missing_arrow_error takes an opened_with_tilde flag and DROPS its `array[T N]` clause when set, because the base-1 site serves the `~[` opener too and `~[` has no array reading anywhere (parse_slot:3927, parse_type_expr:3992, parse_field_type_expr:4465, parse_generic_field_shape:5235 all reject a bare TildeLBracket via tilde_quotation_position_error), so that advice would send the author somewhere the parser refuses; require_top_depth_arrow computes the flag itself as `depth_base > 0 && matches!(self.tokens.get(self.pos - 1), Some((Token::TildeLBracket, _)))`, sound because every base-1 caller consumed exactly one opener token immediately before entry. Pin the two missing-arrow tests to DIFFERENT openers: parse_quotation_effect_missing_arrow_is_error uses `[` (base 0) and asserts the message names `array[T N]`; parse_poly_quotation_missing_arrow_is_error uses `~[` (base 1) and asserts it does NOT. Do NOT try to detect the missing `--` inside parse_quot_type_list (4196) or parse_poly_quot_list (3574), which dispatch a bare count token to parse_type_expr and fail before the loop can see the `]`. R7/R7a: reject a bound inside an effect from INSIDE parse_poly_ty_var (3462) the moment bound_follows is true (3478-3480), selecting the error on builder.forbid_bounds -- false (word-def / trait-member effect) gives the new bound_in_effect_error, true (impl: target) gives impl_target_bound_error (420) -- and then DELETE parse_impl_target's now-dead post-hoc check at 2733-2738, which can no longer fire because nothing pushes into builder.bounds from an effect any more. Move the diagnostic before deleting that check, never after, or `impl: Show for 'T: Copy` reports the word-def message; parse_impl_target_bound_on_var_is_error (10386) must keep passing AND keep pinning impl_target_bound_error's text. Delete the now-unreachable bound_on_use_error (1796, raised at 3494). No DEDICATED test exists for it, so there is nothing to delete -- but parse_trait_decl_member_bound_reports_bound_on_use_not_unknown_capability (9064) DOES pin its message (`must be written at its binding`) on a trait-member effect, i.e. forbid_bounds == false, so RETARGET it to bound_in_effect_error and rename it parse_trait_decl_member_bound_in_effect_is_error rather than dropping it; its subject (not misreporting `unknown capability Copy`) must survive. Also reword parse_impl_bounds' doc comment (2759), which names bound_on_use_error in prose and would be left dangling. Narrow header_is_generic to bracket_follows at all three sites (79, 4583, 4868) and add R10's located postfix_header_var_error at parse_generic_header/parse_typedef; separately drop R5b's postfix disjunct from parse_trait_decl, turning its `'`-prefixed arm (2496-2500) into the same error while its neither-form arm keeps `expected a type variable`. Named tests parse_typedef_postfix_header_var_is_error and parse_trait_decl_postfix_header_var_is_error. Commit, then mutation-test the new gates (R2, R3, R4a, R6-unused-var, R6a bracket-mode unknown name, R7, R10) one at a time pinning message text, confirm the P7 goldens, run the full green check, re-run CLAUDE.md's split signals against src/parser.rs, and update ROADMAP/P7 slice status.",
      "difficulty": "hard"
    }
  ]
}
```
