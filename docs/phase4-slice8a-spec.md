# Phase 4 Slice 8a: static overloading, the mechanism (spec)

Retire the hand-written, type-directed builtin dispatch in `check_operator` /
`check_term` into a real overload **table**, so that a user overload of a
builtin name (`: + ( Vec2 Vec2 -- Vec2 ) ;`) becomes *reachable* at a call
site instead of being accepted, lowered, linked, and never dispatched to. This
is a refactor with exactly one new observable behaviour; everything else must
be byte-for-byte identical.

The brief (`docs/phase4-slice8a-brief.md`) is the discovery document and is
treated as authoritative. This spec does not re-derive it. It (1) answers the
brief's two open questions with concrete Rust types and states the tradeoffs
accepted, (2) turns the six settled decisions into numbered requirements, and
(3) resolves the brief's file+function citations to real `path:line` anchors as
of this writing.

## Grounding facts (verified against the tree)

All anchors are `src/check.rs` unless noted. Lines drift under concurrent work;
they were correct at spec time. Resolve by the enclosing function name if a
line has moved.

- `struct Sig { inputs, outputs }` — `check.rs:23`. `sig_of` — `check.rs:29`.
- `enum PairMatch` — `check.rs:200`; `fn unify_pair` — `check.rs:206`.
  `enum SlotMatch` / `fn match_slot` — `check.rs:165` / `check.rs:177`.
  `is_size_type` — `check.rs:155`.
- `fn builtin_table() -> HashMap<String, Sig>` returning `HashMap::new()` —
  `check.rs:224`. Seeded into the checking env in `fn check` at `check.rs:1322`
  (`struct_generated_sigs` / `enum_generated_sigs` added on top), and into the
  REPL env in `Session::typed_env` at `repl.rs:1141` (call at `repl.rs:1142`).
- `fn check_term` — `check.rs:6154`. Its builtin probe chain (the fall-through
  dispatch order) — `check.rs:6489`–`6503`: `check_access_word`, `check_shuffle`,
  `check_operator`, `check_str_word`, `check_array_word`, `check_owned_cell_word`,
  `check_struct_peek_word`, `check_struct_get_word`, then the back-edge /
  combinator-inline / poly-call interceptions, then the concrete `env` lookup
  at `check.rs:6580`.
- `fn check_operator` — `check.rs:6901`. The type-directed arms this slice
  retires: `+ - *` (`6962`), `/` (`6978`), `mod` (`6990`), `and|or|xor`
  (`7006`), `not` (`7022`), `shl|shr` (`7032`), the comparisons `= < > <= >= <>`
  (`7057` region), `max` (`7069`), `max-total` (`7091`), the `"."` arm and its
  category predicate `is_numeric() || is_bool() || matches!(Str|Cstr)` (`7106`),
  and the `>`-prefixed conversion fall-through (`_ =>` at `7117`, `7118`).
- `fn check_str_word` — `check.rs:7913`; its `"len"` arm (`str` → consuming,
  pushes `usize`) at `7925`, its `"cstr"` arm at `7935`.
- `fn check_array_word` — `check.rs:7952`; its `"len"` arm (array →
  non-consuming, folds constant `N`) at `8003`, its `"fill"` arm at `7961`.
- `fn check_shuffle` — `check.rs:8214` (`dup`/`drop`/`swap`/`over`/`rot`; **not**
  in scope, see rule 0 / out-of-scope).
- `fn check_duplicate_word_names` — `check.rs:2132` (keys `(module, name)`,
  exempts `drop`).
- `fn check_selective_imports` — `check.rs:1847`; `selective_collision_error`
  — `check.rs:1925`; `selective_collides_with_local_error` — `check.rs:1934`.
- `fn find_drop_overloads` — `check.rs:992`; `drop_overload_struct_id` —
  `check.rs:1019`. (`drop`'s bespoke registry and duplicate-check exemption stay
  untouched, dies in 8b.)
- `const BUILTIN_WORDS` — `check.rs:1578`; `fn is_builtin_word_name` —
  `check.rs:1616`; its single caller `fn check_extern_decls` — `check.rs:1540`
  (redeclaration guard at `1565`).
- Lowering mirror: `MirBuilder::lower_call`'s name-directed match in `src/ir.rs`
  — the arithmetic/`cmp`/`max`/`max-total`/`.`/`len`/`cstr` arms at
  `ir.rs:3326`–`3403`, and `self.push_instr(Instr::Print(v))` at `ir.rs:3373`.
  The `_` fall-through (combinator inline, `&`-words, `>T` conversions, struct/
  enum words, self-tail, ordinary `Instr::Call`) begins at `ir.rs:3399`.
- `Instr::Print` codegen — `src/backend/qbe.rs:1026` (15 printable `IrType`
  arms; the aggregate/cell/quotation/`Ptr` arms are `unreachable!`). `qbe_name`
  — `qbe.rs:222` (the injective sanitizer, already fixed ahead of this slice;
  regression tests at `qbe.rs:1289` / `qbe.rs:1331`).
- Types: `enum Type` — `ast.rs:690`; `is_numeric`/`is_int`/`is_float`/`is_bool`
  — `ast.rs:894`/`884`/`889`/`899`; `Type::from_name` — `ast.rs:851`.
  `enum Bound { Copy, Ord }` — `ast.rs:503`; `enum PolyType` — `ast.rs:521`;
  `struct PolySig` — `ast.rs:538`; `enum Len` — `ast.rs:511`.
- Per-call-site record precedent: `struct CallInst` (`ast.rs`, keyed by `Span`
  in `Module::instantiations`, filled in `fn check` at `check.rs:1499`–`1507`,
  read by lowering). This is the pattern the lowering hand-off (R7) reuses.
- Test helpers that call `builtin_table()` and will move to the new shape:
  `check.rs:9420` (env build), `check.rs:9541`, `check.rs:10723`.

The printable-type set `.` dispatches over today (rule 6's N rows), read off
the `check_operator` `"."` predicate cross-checked against the `Instr::Print`
codegen arms: `i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 usize isize bool str cstr`
— **15 concrete rows**, all lowering to `Instr::Print`.

## Requirements

### R0 — Scope of the retirement

Only the **overload-by-operand-type** dispatch moves into the table:
`check_operator`'s arithmetic tower, comparisons, `max`, `max-total`, and `.`.
The following stay hand-written and out of the table, each for a stated reason,
and each keeps behaving byte-for-byte:

- `check_shuffle` (`dup`/`swap`/`over`/`rot`, and `drop`): fully generic over
  every type, not an overload set; `drop` is 8b. Moving them buys nothing.
- The `>T` numeric conversions (`check_operator`'s `_` arm): dispatched by
  *parsing the target type out of the name* (`>u8`), open-ended, not an
  overload set keyed by operand type. Name-directed, so no table row can key it.
- `len`, `cstr`, `fill` (`check_str_word` / `check_array_word`): `len` and
  `fill` are generic over element and length (a `PolySig`-shaped domain, not a
  finite row set); `cstr` is a single candidate. Carved out as a unit (see R2).

`check_operator` does not fully disappear: its numeric-tower **operand-class
validation and coercion** survive as a resolution fallback (R6/Q2). What is
retired is the *dispatch-selection* match — the arms that pick a builtin by
operand type before any env lookup and thereby hide a user overload. That
selection becomes table lookup.

### R1 — No shadowing (settled decision 1)

A user overload whose input types **exactly** match an existing candidate —
builtin, local, or imported — is a located error, not a silent override. Same
shape as the duplicate-word check.

- Enforced for definitions at `check_duplicate_word_names`, whose key widens
  from `(module, name)` to `(module, name, input_types)`. Two definitions with
  identical `(module, name, input_types)` collide with the existing
  `duplicate word` message (byte-for-byte for today's identical-name tests);
  two with different input types no longer collide.
- The same check additionally rejects a definition whose `(name, input_types)`
  equals a **builtin row** in `builtin_table`. Message (new), with the word
  name and operand types interpolated:

  ```text
  error: overload of `+` (line 2, col 3) has the same input types (i64 i64) as a builtin
  ```

- **Not** an exemption class for overloadable names (that reproduces
  `find_drop_overloads`), **not** a deletion of the check (that hands back the
  linker's `symbol already defined`). One check, wider key.

### R2 — Exact match beats coercion (settled decision 2)

`unify_pair`'s literal/size-type coercion ranks strictly below an exact-type
candidate. Adding an overload cannot silently steal a call site that previously
coerced. Concretely (R6): the resolver runs an exact-input-type pass across all
candidates first; only on an exact miss does the numeric coercion fallback run.

### R3 — Overloads are imported, not carried by the type (settled decision 3)

Importing `Vec2` does not bring `+` for it. When a call names a word that has
candidates but **none** match the operands (exactly or by coercion), the result
is a located resolution error naming the operand types and the absent overload —
never a silent fallback.

- For a builtin operator (`+` on `Vec2 Vec2`), this error **is** today's
  operand-class diagnostic (`operand_pair_mismatch_error`, "requires two
  operands of the same numeric type, found `Vec2` and `Vec2`"), preserved
  byte-for-byte. The resolver routes an operator's no-match to that operator's
  existing diagnostic (R6, step 3).
- For a user-overloaded, non-operator name, the message names the operands and
  the missing overload.

### R4 — One arity per name in scope (settled decision 4)

All candidates for a name (builtin rows + local defs + imports) must agree on
input count. Disagreement is a located error where the **second** candidate
enters scope: the definition site when local, the import site when imported.
Never a call-site ambiguity resolved by ranking; the clash is rejected before
any call site is examined.

- Local: `check_duplicate_word_names` grows an arity-agreement pass over each
  name's candidate set (it cannot be caught by the R1 key, since differing
  arities produce differing keys). Message (new):

  ```text
  error: overload of `+` (line 2, col 3) takes 1 input but another `+` takes 2; all overloads of a name must agree on input count
  ```

- Import: `check_selective_imports` gains the mirror check, emitting through the
  `selective_collision_error` family.
- `: + ( Vec2 -- Vec2 ) ;` beside `: + ( Vec2 Vec2 -- Vec2 ) ;` is the rejected
  case (the ROADMAP's example).

### R5 — Concrete/generic overlap rejected, not ranked (settled decision 5)

`: + ( 'T 'T -- 'T )` beside `: + ( i64 i64 -- i64 )` (or beside a builtin
concrete row) is rejected, at whichever site the second candidate enters scope.
No specialization ordering.

- A poly word's `effect` is empty by construction (its signature lives in
  `PolySig`), so R1's textual match never fires; this is a distinct check.
- The check: if a name has both a generic (poly) candidate and any concrete
  candidate of matching arity, reject. Message (new):

  ```text
  error: generic overload `: + ( 'T 'T -- 'T )` (line 2, col 3) overlaps a concrete overload of `+`; a name cannot mix a generic and a concrete candidate
  ```

- Poly candidates live in `poly_env` (`check.rs:1364` region); the overlap
  check compares `poly_env` names against `builtin_table` rows + concrete `env`
  entries of the same name.

### R6 — Dispatch is table-driven exact-match, coercion as fallback

`check_term`'s Call handling replaces the `check_operator` probe with a single
resolution step over the merged candidate set for the name:

1. **Operand read.** Read the top `n` operand `Type`s, where `n` is the name's
   agreed arity (R4). Underflow → the existing `underflow_error`.
2. **Exact pass (R2).** If exactly one candidate's inputs equal the operand
   types, it wins:
   - a builtin row → its `BuiltinLower` effect (push its outputs);
   - a user/imported word → the ordinary word-call path (`Instr::Call`, existing
     env/poly machinery).
   This pass resolves `.`, the common `i64`/`f64` operator cases, and the new
   `Vec2 +` case. Because R1/R4/R5 already reject clashing candidates, at most
   one candidate matches; no ranking is ever needed.
3. **Coercion / diagnostic fallback (numeric operators only).** On an exact
   miss for a homogeneous numeric operator, run that operator's operand-class
   guard + `unify_pair` (the retired arm's body, minus dispatch selection):
   success → the coerced builtin row's lowering; `NeedsSizeConversion` →
   `size_conversion_needed_error` (X10, verbatim); class failure → that
   operator's specific diagnostic (`operand_pair_mismatch_error`,
   `div_requires_float_error`, `mod_requires_int_error`,
   `bitwise_pair_mismatch_error`, `bitwise_not_requires_int_error`,
   `shift_value_requires_int_error`, `shift_count_requires_i64_error`,
   `max_over_float_error`, `max_total_requires_float_error`). For `.` there is
   no coercion; an exact miss → `print_requires_printable_error`.

Quotation-operand rejection (`reject_quotation_operand`, the R11 guard at
`check_operator:6943`/`6946`) is preserved: it fires before operand types are read, so
it moves into step 1 unchanged.

### R7 — Lowering dispatches to the resolved candidate

`lower_call` (`ir.rs:3326`) must agree with the checker's resolution. Today it
re-dispatches by name; a raw `+` at a `Vec2` site would wrongly emit `Bin(Add)`.

The checker records, per call `Span`, the sites that resolved to a **user
overload of a builtin-named word**, with the resolved callee symbol — a sparse
map stored on the `Module`, mirroring `Module::instantiations` / `CallInst`.
`lower_call` consults it first: a recorded site emits `Instr::Call` to the
recorded symbol and returns; every unrecorded site takes the existing name-
directed arms unchanged. The corpus produces no records, so its lowering path
is literally untouched (byte-for-byte). Do **not** re-run resolution in
lowering; the checker is the single source of truth (the `instantiations`
precedent).

### R8 — `extern:` behaviour unchanged

An `extern:` redeclaring a builtin stays a located error
(`check_extern_redeclaring_a_builtin_is_error`). `is_builtin_word_name` /
`BUILTIN_WORDS` may read the now-populated table instead of the parallel list,
but the observable rejection must not change. Do not let an `extern:` gain an
overload path (a C symbol has one prototype).

### R9 — Corpus unchanged byte-for-byte (first-class exit)

The full existing corpus — every golden and every `examples/*.sth` — compiles
to byte-identical emitted QBE and produces byte-identical program output, with
the single exception that the new `Vec2 +` program (which does not exist in the
corpus today) newly compiles and dispatches. A golden captures a baseline of
the emitted IR/QBE for the corpus before the refactor and asserts equality
after (see Testing).

## Open question A — the table's entry type (answered)

The entry cannot be `Sig`: an entry must carry a **lowering** (`len`'s two
shipped candidates differ in codegen and consumption; `.`'s rows and a user
`. ( Vec2 -- )` differ in lowering), and one name has several candidates.

**Decision: the table holds only concrete rows; the generic builtins (`len`,
`fill`, and the `>T` conversions) are carved out and stay in their existing
hand-written dispatchers.**

```rust
/// One builtin overload candidate: the concrete input types it matches
/// (deepest-first), the concrete outputs it produces, and the codegen a call
/// resolving to it emits.
pub struct BuiltinRow {
    pub inputs: Vec<Type>,
    pub outputs: Vec<Type>,
    pub lower: BuiltinLower,
}

/// The codegen a resolved builtin row emits — one variant per distinct
/// instruction the retired arms produced. This is why an entry is not `Sig`:
/// several rows for one name share the `(inputs, outputs)` shape but differ
/// here (`.`'s 15 rows all lower `Print`; a user `.` lowers a `Call`).
pub enum BuiltinLower {
    Add, Sub, Mul, DivFloat, Mod,
    And, Or, Xor, Not, Shl, Shr,
    Cmp(CmpOp),      // = < > <= >= <>  (reuse ir's CmpOp)
    Max, MaxTotal,
    Print,
}

pub fn builtin_table() -> HashMap<String, Vec<BuiltinRow>> { /* populated */ }
```

Rows are generated compactly by iterating a small type list (e.g.
`for t in numeric_types() { rows.push(row("+", [t, t], [t], Add)) }`), not
hand-typed 200 times. `.` is its 15 concrete rows (rule 6). `shl`/`shr` are
`(T, i64 -- T)` per int `T`.

**Why concrete-only, and why carve `len`/`fill`/`>T` out.** An entry able to
hold `len`'s array candidate must carry type/length variables plus a unifier —
i.e. `PolySig`-shaped entries — and a numeric/`Ord` bound the current `Bound`
enum (`Copy`/`Ord` only) does not have. Building that apparatus into
`builtin_table` for exactly the `len`-on-array candidate (and `fill`, and the
name-parameterized conversions), which **no user is overloading this slice**, is
the premature-abstraction mistake rules 5 and 6 explicitly refuse (rule 6:
"printability was the only candidate for [a category mechanism], and it doesn't
need it"; rule 5: don't invent ordering "for a consumer that doesn't exist
yet"). A concrete-row table already delivers criterion 3 — `: + ( Vec2 Vec2 --
Vec2 )` and `: . ( Vec2 -- )` are exact concrete matches — with the smallest
shape.

**Tradeoff accepted.** The table is not yet the single home for *all* builtins;
`len`/`fill`/`cstr`/`>T`/the shuffles remain hand-written. The entry shape
deliberately does **not** preclude a later generic variant: the deferred view
type (DESIGN.md, *Slicing a buffer into a view*, gated on this slice) would add
a third, runtime-kind `len` candidate, at which point `BuiltinLower` gains a
variant and, if generic entries are wanted, `BuiltinRow` grows a sibling
generic form. That is not built now. The cost of the carve-out is that `len`
stays "dispatched by fall-through across two functions," the very shape finding
3 dislikes — accepted because absorbing it needs the generic machinery this
slice declines to build, and no exit criterion depends on it.

## Open question B — where `unify_pair` runs (answered)

**Decision: `unify_pair` runs *before* the coercion lookup, as a per-operator
fallback reached only on an exact-match miss — it does not become table rows.
The table answers only "is this operator defined for these exact types."**

Flow is R6: exact pass over all candidates first (so R2 holds for free), then,
for a numeric operator that missed, its operand-class guard + `unify_pair`
producing either the coerced concrete type (retry the builtin row) or X10's
`size_conversion_needed_error`, else the operator's specific class diagnostic.

**Why not coercion-as-rows.** Turning `unify_pair` into rows would (1) multiply
entries (every `(size-type, i64-literal)` pairing becomes explicit), and (2)
force X10's "needs an explicit conversion to `usize`" specificity to be
re-derived from a lookup *miss*, which cannot reproduce the per-operator
diagnostics (`div_requires_float` vs `mod_requires_int` vs
`operand_pair_mismatch`) byte-for-byte. R9 forbids that.

**Tradeoff accepted.** The per-operator operand-class predicates and their
diagnostics stay as code (the fallback), not table data — so `check_operator`
shrinks to that fallback rather than vanishing entirely. The table drives
*dispatch selection and user-overload interception*; operand-class validation
and coercion remain hand-written. This is the smaller, byte-faithful split, and
it honours rule 6's "no category key in the table" (the numeric classes never
become table fields).

## Out of scope (hard boundary)

- **Everything `drop`** — polymorphic `drop`, the disposal-scope /
  structural-totality constraint, the destructuring-bypass hole. All 8b. `drop`
  keeps its `find_drop_overloads` registry and its `check_duplicate_word_names`
  exemption through this slice; do not delete the exemption as tidying.
- **Dispatch on outputs / return-type overloading.** Inputs only.
- **Traits / type classes / any non-static dispatch.** Every call resolves at
  compile time or errors.
- **The deferred view type** (its runtime `len` candidate). The entry shape must
  not preclude it (Q-A), but nothing here waits on it.
- **Moving `.` (or any builtin) onto `extern:`.** `.`'s lowering stays
  `Instr::Print`; only its dispatch key changes.
- **Building generic table entries or a `Bound`-style category mechanism** (Q-A,
  rules 5/6).
- **REPL overloading of builtins as a demonstrated feature.** The REPL's
  `typed_env` and dispatch must adopt the new table shape without regressing any
  existing REPL golden; making a user builtin-overload dispatch *in the REPL* is
  not an exit criterion of this slice.

## Exit criteria

1. `check_operator` / `check_term`'s type-directed **dispatch-selection** arms
   are gone; `builtin_table` is populated with concrete rows (R0, R6, Q-A).
2. The full existing corpus (goldens + examples) is unchanged byte-for-byte
   (R9), the only new behaviour being (3).
3. `type: Vec2 x i64 y i64 ; : + ( Vec2 Vec2 -- Vec2 ) ... ;` compiles, links,
   **and dispatches** at a call site with two `Vec2` operands, printing the
   correct result (R6 check side + R7 lowering side).
4. Rule 1's exact collision, rule 3's missing-overload resolution failure, rule
   4's arity clash, and rule 5's concrete/generic overlap are each a **located**
   error with the specified message, and no definition is left silently
   unreachable.
5. `: + ( i64 i64 -- i64 ) ;` beside `: - ( i64 i64 -- i64 ) ;` in one file
   compiles and links (the `qbe_name` fix, already merged; keep its regression
   tests green).

## Testing (convention, not a percentage)

Unit tests beside the stage code in `#[cfg(test)] mod tests`, named
`thing_condition_expected`. Diagnostics are behaviour: assert the exact message
text, not merely that it errors. Mutation-check each guard (prove the test fails
when the guarded logic is deleted).

- `builtin_table_has_a_row_per_printable_type_for_print`,
  `builtin_table_plus_has_a_row_per_numeric_type`.
- `overload_vec2_plus_dispatches_to_user_word` (golden: source → program output).
- `operator_i64_lowers_identically_after_table` and the R9 corpus baseline
  golden (emitted QBE byte-identical for gcd/factorial/strings/refs/resources).
- `plus_usize_and_int_literal_coerces` and
  `plus_usize_and_computed_i64_needs_conversion_is_error` (X10 wording, R2/Q-B).
- `mod_over_float_is_error`, `div_over_int_is_error`,
  `print_over_struct_is_error` (fallback diagnostics preserved, R3/R6).
- `overload_exact_input_match_is_error` (R1, incl. clash with a builtin row).
- `overload_arity_clash_is_error` and its import-site twin (R4).
- `overload_generic_and_concrete_overlap_is_error` (R5).
- `overload_missing_at_call_site_is_error` (R3).
- `extern_redeclaring_a_builtin_is_error` still passes (R8).
- REPL: existing goldens unchanged after `typed_env` adopts the new shape.

"Green" = `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

## Phases

### Phase 1: The table and the concrete-row dispatch (byte-for-byte refactor)

Introduce `BuiltinRow` / `BuiltinLower`; populate `builtin_table` with the
arithmetic tower, comparisons, `max`, `max-total`, and `.`'s 15 rows (Q-A).
Replace `check_operator`'s dispatch-selection with the R6 resolution step
(exact pass over builtin rows; `unify_pair` + operand-class diagnostics as the
numeric fallback, Q-B), and drop the retired arms from the `check_term` probe
chain. Adapt every `builtin_table()` caller to the new return type (`check`'s
env seed, `repl.rs` `typed_env`, the three test helpers). No user-overload
behaviour yet. Exit: the corpus is byte-for-byte unchanged (R9 baseline golden),
`check_operator`'s selection arms are gone, `builtin_table` is populated.

### Phase 2: User-overload dispatch, check and lowering

Merge user/imported words into R6 resolution so a `Vec2 +` site resolves to the
user word (check side), and record the resolved site per `Span` on the `Module`
so `lower_call` emits `Instr::Call` there instead of the builtin instruction
(R7). Exit: criterion 3 — the `Vec2 +` program compiles, links, dispatches, and
prints the right value; the corpus stays byte-for-byte (no records emitted for
it).

### Phase 3: Collision, arity, and overlap enforcement

Widen `check_duplicate_word_names`' key to `(module, name, input_types)` and add
its comparison against builtin rows (R1), an arity-agreement pass over each
name's candidate set (R4, local), and the concrete/generic overlap check (R5).
Mirror the arity/collision checks into `check_selective_imports` via the
`selective_collision_error` family (R4, import). Route an operator's no-match to
its existing diagnostic and add the user-name missing-overload message (R3).
Exit: rule 1 / 3 / 4 / 5 each a located error with the specified wording, tested
including mutation-checks; `drop`'s exemption still present.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Introduce BuiltinRow/BuiltinLower, populate builtin_table with the numeric-tower operators and `.`'s 15 concrete rows, replace check_operator's dispatch-selection with table-driven exact-match plus a unify_pair fallback, and adapt all callers; corpus byte-for-byte unchanged.",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "Merge user/imported words into resolution so a user overload of a builtin name dispatches (check side), and record resolved sites per Span so lower_call emits Instr::Call there; deliver the Vec2 `+` example compiling, linking, and dispatching.",
      "effort": "M",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "Enforce rules 1/3/4/5 at the existing definition and import sites: widen the duplicate-word key and compare against builtin rows, add arity agreement and concrete/generic overlap checks, and emit the missing-overload resolution error, each as a located diagnostic with specified wording.",
      "effort": "M",
      "difficulty": "standard"
    }
  ]
}
```
