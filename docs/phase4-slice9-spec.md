# Phase 4 Slice 9: `Bool` as a library enum + `if`/`cond` as pure Sooth words (spec)

Retire `Bool`/`bool` as a primitive scalar and `if` as a keyword. `Bool` becomes a
two-variant zero-payload enum `type: Bool | False | True ;` with a **general**
zero-payload-enum scalar layout; `if` becomes an **ordinary clause-bodied Sooth
word** dispatching on that enum, which requires generalising three existing
clause-word restrictions first; `cond` follows as ordinary Sooth source. No
compiler-recognised `if` intrinsic is added: at the end of this slice `if` is a
definition a user could have written.

The ROADMAP frames the whole slice as a mechanical migration. That is false in two
places, and this spec settles both explicitly: `Bool`'s **representation** (which
the ROADMAP does not address at all) and `if`'s **inlining**, which turns out to be
load-bearing for a termination guarantee that predates this slice.

Every anchor below was verified against `main` **after 8a merged** (`e20c52f`).
Line numbers drift; each names a `grep`-able function or symbol.

## Correcting the brief, and the first draft of this spec

The brief said "retire `TermKind::If` and its two check/lower sites," and this
spec's first draft concluded `if` therefore had to become a name-recognised
intrinsic combinator beside `times`, because "`if` is the branch primitive; it
cannot be a pure-Sooth library word." **Both are wrong, and the second is wrong in
an instructive way.**

`if` *can* be a pure Sooth word. Once `Bool` is an enum, clause-style dispatch is
exactly the elimination form every user enum already has (`examples/shapes.sth`'s
`area`, `unwrap-or`), and clause dispatch is an independent compiler primitive
(`check_clause_word`, and its own discriminant-load lowering in `ir.rs`) that does
not depend on `if` existing. There is no circularity. What blocks a clause-bodied
`if` today is three specific, separable restrictions, none of them fundamental:

1. **A clause-bodied word may not take a quotation parameter.**
   `clause_bodied_quotation_word_error` (`check.rs:1848`, fired at `:1768`) rejects
   it. Its own comment states the intended lift: *"Slice 7's runtime quotation
   value lifts it (the word would then `call` a real value, no inlining needed)."*
   Slice 7a shipped that runtime `(code, env)` value; this rule was never
   revisited. It is stale, not load-bearing.
2. **Clause dispatch scrutinises the topmost input only.** `check_clause_word`
   reads `word.effect.inputs.last()` (`check.rs:4350`). Factor-order
   `cond [ then ] [ else ] if` needs the `Bool` to be the *deepest* of three
   inputs, so the order DESIGN.md documents is currently inexpressible (D-E).
3. **Splicing is `WordBody::Terms`-only.** `is_combinator` (`check.rs:5697`)
   requires a term body, and `combinator_of` (`:5678`) returns `None` for a clause
   body ("never a combinator"). This is the load-bearing one, and the reason the
   first draft's "cannot" was *directionally* right for the wrong reason: it is not
   that branching needs an intrinsic, it is that **an un-spliced `if` deletes the
   self-tail-call → loop transform.**

`body_tail_calls_self` (`ir.rs:2495`) recurses into `TermKind::If`'s branches to
find a tail self-call:

```rust
Some(TermKind::If { then_branch, else_branch, .. }) =>
    body_tail_calls_self(then_branch, name) || body_tail_calls_self(else_branch, name),
```

Move the branches into quotation arguments of an un-spliced word and the last term
is `Call("if")`: the transform stops firing. Detection cannot be patched around it
either, because with a real call the recursive call genuinely is not in tail
position (it returns into the closure, which returns into `if`'s body). Only
inlining recovers it. `examples/countdown.sth` exists precisely to prove this
transform works (1,000,000 iterations, its own comment: *"overflows the host stack
under naive recursion, completes in constant stack under the self-tail-call ->
loop transform"*), and `examples/gcd.sth` — a Phase 0 golden — is self-tail
recursive through `if` in the same shape. **So a "naive, un-spliced pure-word `if`
shipped now, optimised later" ordering is not available: the inlining is not an
optimisation, it is a prerequisite.** Hence D-B's phase order.

Everything else in the brief held up. The 21 `TermKind::If` references
(`grep -rc TermKind::If src/`) are: the monomorphic checker arm (`check.rs` near
:8154), the poly-body arm (near :4879), the `ir.rs` lowering (`Jnz(cond, then_id,
else_id)` near :4611), the self-tail detector (`:2495`) and tail-lower (near
:3043), and eight pure-traversal helpers (`capture_names_into`, `nested_uses`,
`references`, `collect_tail_calls`, `collect_all_calls`, plus two REPL `if`-body
rewriters at `repl.rs` :276 and :2054). Every one loses its `If` arm in P4.

## Design decisions

### D-A — `Bool` gets a **general zero-payload-enum scalar layout**, not a per-`Bool` carve-out and not the tagged-aggregate cost

The crux (brief finding 5). Three options:

1. **`Bool`-specific scalar carve-out** in `EnumLayout`/`ir_type_of`: cheapest, but
   re-introduces a named `Bool` exception, the exact special case this slice
   removes. Rejected on principle.
2. **General rule: an enum all of whose variants carry zero payload lowers to a
   bare scalar discriminant** (register-resident, no memory aggregate). **Chosen.**
   `Bool` is its first client; `False`/`True` are ordinary variant constructors
   lowering to the `0`/`1` discriminant.
3. **Pay the general tagged-aggregate cost.** `EnumLayout` (`ir.rs:444`) is
   unconditionally "a fixed `i32` discriminant tag placed first, then a payload
   region"; elimination is clause-style only, loading the discriminant through a
   `Ptr`-offset field load. So every comparison result, every `and/or/xor/not`, and
   every *compiler-internal* condition (bounds-check `Jnz` near `ir.rs`:4163, loop
   back-edge tests) would construct a 4-byte aggregate and destructure it to feed
   `Jnz` — a per-branch regression on the hottest control-flow path in every
   program, including conditions carrying no surface-`Bool` semantics. Rejected.

DESIGN.md's own rule, "**Never charge a semantic price for a performance
property**" (`:1049`), points straight at (3). (2) honours "`Bool` is an ordinary
enum, not a compiler special case" at the *surface* (declaration, constructors,
exhaustiveness-checked elimination, library `.`) while the *representation* stays
scalar through a **general** layout rule. The C-style all-unit-variant-enum-is-a-tag
optimisation is general and reusable, not a `Bool` hack.

Consequences the spec leans on:

- **`Cmp`/`Jnz`/bitwise codegen stays byte-for-byte through P1–P2.**
  `ir_type_of(Bool)` still yields the scalar the backend emits as QBE `w`
  (`backend/qbe.rs` :262, :315). The change is at the **type/checker** layer
  (`Type::Bool` → `Type::Enum(Bool)`), not the IR-value layer; the scalar boolean
  `IrType` is *retained as the lowered form of the zero-payload enum*.
- **Brief finding 6 dissolves.** A returned `Bool` stays a scalar ABI value, so it
  never enters the aggregate-return path, and this slice is not that path's first
  two-variant client. `examples/bool_abi.sth` keeps its scalar ABI.

### D-B — Order: `Bool` first, then the clause-word mechanism, then `if`, then `cond`

Both halves ship in this spec, sequenced so nothing regresses mid-flight:

- **P1–P2 (`Bool`)** touch no branching syntax. Keyword `if` stays exactly as it
  is, so the whole corpus — `countdown.sth` included — stays byte-for-byte, and the
  representation crux lands with a byte-for-byte safety net.
- **P3 (mechanism)** generalises the three clause-word restrictions *while keyword
  `if` still exists*, so the new machinery is proven on a user-written clause word
  before anything depends on it. Its exit criterion is the countdown shape proven
  through a non-`if` word.
- **P4 (`if`)** flips over and deletes the keyword. **P5 (`cond`)** follows.

This is the project's established mechanism-then-migration shape (8a's phases,
7a/7b). The P2/P3 boundary is also a clean cut line: if P3–P5's load proves larger
than measured, they may spin out to `phase4-slice9b-spec.md` without reopening
P1–P2.

### D-C — `cond` ships fixed-arity as ordinary Sooth source; variadic `cond` stays blocked

**Correcting this spec's own earlier draft**, which claimed generalised clause
splicing would deliver variadic `cond` "for free." It does not. Clause dispatch is
N-way dispatch on **one scrutinee's variants**; `cond` is N **independent boolean
predicates** evaluated in order. They are different shapes, and the second is not
expressible as the first.

A truly variadic `cond` still needs quotations in a runtime collection: a quotation
cannot be stored in an array element (slice 4, D4 rejects it) and every `call`
needs a statically known literal. There is no varargs mechanism. So `cond` ships
**fixed-arity, written in Sooth as nested `if`** over statically known quotation
literals, predicates evaluated in order, first `True` arm running, with a default
body. What P3–P4 buy `cond` is only that it needs no intrinsic and no name
recognition. Its clause count is the implementer's call; minimal is fine (craft
ethos: do not build a varargs machine). The variadic form's blocker is recorded
here, not silently deferred.

### D-D — Surface spellings `true` / `false` / `bool` are retained; the *type* is the enum

`true`/`false` remain accepted literals that now construct the `True`/`False`
variants; `bool` remains an accepted spelling of the `Bool` type. Surface sugar,
not a retained primitive: the distinct `Type::Bool` scalar type is gone. Rationale:
REPL goldens type `true`/`false` (`tests/phase1.rs` :171, `tests/phase3_strings.rs`
:163) and corpus signatures spell `bool` (`bool_abi.sth`, `poly_if.sth`,
`lib/combinators.sth`), so the spellings keep them compiling and printing
identically with zero source churn — the exit is identical *output*, not identical
source. Migrating corpus signatures `bool`→`Bool` is optional and not an exit
criterion.

### D-E — Clause dispatch scrutinises the topmost **enum-typed** input, not merely the topmost input

`check_clause_word` hard-codes `word.effect.inputs.last()` (`check.rs:4350`), so
`if ( Bool [ ..a -- ..b ] [ ..a -- ..b ] -- ..b )` — `Bool` deepest, per DESIGN.md's
documented `cond [ then ] [ else ] if` order — cannot dispatch today. The
alternatives were condition-last order (`[ then ] [ else ] cond if`, which buries
the condition and breaks the documented idiom) or a `rot` at every branch site
(unacceptable for the language's most common construct).

**Decision: relax scrutinee selection to the topmost enum-typed input**, in the
checker and in the mirroring clause lowering. `if`'s quotation parameters are not
enums, so `Bool` is selected unambiguously; for every existing clause word the
topmost enum-typed input *is* the topmost input, so behaviour is unchanged (a
regression test pins `area`/`unwrap-or`). If two inputs are enum-typed the topmost
still wins, deterministically, exactly as today.

## Requirements

**R0 — Scope.** (a) Migrate `Bool` from a primitive scalar to a zero-payload enum
with a general scalar layout (D-A); (b) turn `.`'s `bool` row into a library `Bool`
overload through 8a's dispatch; (c) generalise the three clause-word restrictions
(D-E, quotation parameters, splice-eligibility); (d) ship `if` and `cond` as
ordinary Sooth words and delete the `if` keyword. Out: ordinary user-enum
declaration/matching mechanics beyond what (a)/(c) need, `cond`'s variadic form
(D-C), output/return-type overloading, and any 8a rule beyond deleting the one `.`
row it named.

### P1 — `Bool` as a zero-payload enum

**R1 — General zero-payload-enum scalar layout (D-A).** `EnumLayout` and the
registry builder (`ir.rs` `EnumLayout` :444, `EnumWord::Construct` :504) gain a
rule: an enum every variant of which has an empty payload lowers to a **bare scalar
discriminant** — no payload region, no memory aggregate, the tag is the value,
register-resident. `ir_type_of` of such an enum yields the scalar `IrType` the
backend already emits as QBE `w`. Payload-bearing enums are untouched
(`shapes.sth`, `vm.sth` byte-for-byte).

**R2 — `Bool` is that enum; `True`/`False` are its constructors.** `Bool` is
declared as `type: Bool | False | True ;`, `False`→`0` and `True`→`1` by
declaration order (the existing discriminant convention), preserving both `Jnz`
truth polarity and the `$boolstrs` print table's `{ l $false_str, l $true_str }`
order (`backend/qbe.rs` :56–62). `True`/`False` replace `TermKind::BoolLit`;
`true`/`false` remain surface spellings (D-D). The primitive `Type::Bool` is
removed from the type layer; `bool` parses as a spelling of `Type::Enum(Bool)`.

**R3 — `Cmp`/bitwise/internal-condition codegen byte-for-byte.** Because `Bool`
lowers to the same scalar (R1), every `Cmp`, every `and/or/xor/not`, `Jnz`, the
bounds-check guard (`ir.rs` near :4163), and the loop back-edge test keep their
current QBE verbatim. First-class exit: the `tests/qbe_baseline*` goldens are
**unchanged** after P1, `countdown.sth` included. R3 scopes byte-for-byte to
*internal-boolean codegen* and to every baseline whose source prints no `Bool`;
P2's R6 necessarily reroutes the *print call sites* of the bool-printing baseline
(`leap.ssa`), recorded as an accepted deviation under R6. The condition-computing
code (`Cmp`/`Jnz`/bitwise, and `$leap`'s own body) stays byte-for-byte through
P1–P2.

**Accepted deviation (P1, cycle-1 review).** Two baselines moved: `list.ssa` and
`refs.ssa` renumber `sooth_enum_drop_0` → `sooth_enum_drop_1` everywhere. This is a
behaviourally-inert symbol rename, not a codegen change: no instruction, block, or
shape differs; `countdown.ssa`, `gcd.ssa`, `bool_abi.ssa`, `shapes.ssa`, `vm.ssa`
are byte-identical; `corpus_qbe_stays_byte_identical_to_baseline` is re-anchored and
passes; runtime output is identical. Cause: reserving `BOOL_ENUM_ID = EnumId(0)`
(fixed so `Type::from_name("bool")` resolves with no registry access) occupies
registry slot 0, shifting every user enum's id up by one, so `List`'s destructor
symbol renumbers. The alternative — numbering *enum* drops to skip the no-drop
`bool` slot so `List` stays `drop_0` — was rejected: it breaks the documented
"one uniform naming scheme" shared by the `struct`/`enum`/`cell` drop symbols
(`ir.rs` `enum_drop_symbol`), carries blast radius into the epoch/REPL/cross-module
drop paths, and re-introduces a `bool`-shaped carve-out D-A rejects on principle.
R3's byte-for-byte requirement therefore holds for every instruction and for every
payload-bearing baseline; the sole delta is this enum-drop symbol ordinal, forced
by a fixed `BOOL_ENUM_ID`.

**R4 — `and`/`or`/`xor`/`not` keep their `Bool` rows through 8a's table.** The four
explicit `Type::Bool` rows in `builtin_table` (`check.rs` :346–431) continue to
resolve, now keyed on `Type::Enum(Bool)`, behaving identically (bitwise coincides
with logical on a strict two-valued scalar). No output change.

**R5 — Exhaustiveness over `{False, True}`.** Clause-style elimination on `Bool`
is exhaustiveness-checked over its two variants by the existing enum machinery; a
missing arm is the existing located error. No new elimination mechanism.

### P2 — `.` as a library overload

**R6 — Delete `.`'s `bool` row; ship `: . ( Bool -- ) ;` via 8a dispatch.** With
`bool` no longer a primitive, `.`'s row generated from `printable_types`
(`check.rs` :346–351, row at :424) has no primitive operand and is removed. A
library `: . ( Bool -- ) ;` clause-matching `False`/`True` and printing
`false`/`true` (including the trailing newline the `$boolstrs` path emits) is
shipped and reached through 8a's overload dispatch: the checker records the site on
`Module::builtin_overloads` (`check.rs` :2124) and `lower_call` consumes it
(`ir.rs` :3139). Output for `true .` / `false .` is unchanged
(`tests/phase3_strings.rs` :163 expects `true`).

**Accepted deviation (P2, cycle-1 review).** One baseline moved: `leap.ssa`
(`leap.sth` prints three `Bool`s). Its three print sites change from the inline
`$boolstrs`-index sequence to `call $.2e.(w %vN)`, and the file gains the injected
`$.2e.` word plus its two `$strb`/`$strd` literals. This is R6's mandate, not a
codegen regression: `$leap`'s own function body is byte-for-byte identical, no
back-edge or condition codegen changed, and runtime output (`false`/`true` lines)
is unchanged. Every bool-print-free baseline (`gcd.ssa`, `countdown.ssa`,
`shapes.ssa`, `vm.ssa`, `list.ssa`, `refs.ssa`) is byte-identical to its P1 state.
The reroute is exactly what "ship `.` as a library call" means; a baseline that
prints a `Bool` cannot both route through the library word and stay byte-for-byte.

**R7 — Verify 8a's dispatch wiring before relying on it.** Confirm on merged `main`
that `Module::builtin_overloads` is read by `lower_call` on **every** path:
`check_term`'s chain, `poly_delegate_op` (records at `check.rs` :7973/:8082), and
the REPL's `lower_line`/`lower_word` (`empty_builtin_overloads`, `ir.rs` :2453). A
library `.` overload must not fall back to the deleted builtin arm and segfault —
the exact failure 8a's own fix cycle closed. Verification requirement with a test,
not an assumption.

**R8 — Returned/carried `Bool` regression.** `examples/bool_abi.sth` (returned
`Bool` across a call) and a REPL carried-`Bool` line keep identical output; the
returned `Bool` stays a scalar ABI value (D-A), so this guards against an
accidental aggregate-return regression rather than adding machinery.

**Follow-up, not this phase's scope:** R6 makes the `IrType::Bool` `Print` codegen
arm and the unconditionally-emitted `$boolstrs`/`$true_str`/`$false_str` QBE header
(`qbe.rs`) dead from surface code; nothing in `.sth` source can reach them anymore
since the primitive row is gone. Retaining them here is correct (removing either
now would break R3's byte-for-byte guarantee across every baseline). Deleting the
dead arm and header, and retiring the codegen test that exercises them
(`emit_print_on_bool_indexes_boolstrs_via_sfmt`), is a later-phase cleanup once
something depends on the header's absence rather than its presence.

### P3 — The clause-word generalisations (keyword `if` still in place)

**R9 — A clause-bodied word may take quotation parameters.** Delete
`clause_bodied_quotation_word_error` and its guard (`check.rs` :1848/:1768). Its
premise (a quotation parameter is only supportable on a splice-able term body) is
retired by R11: a clause body becomes splice-able. The `ir_type_of` `unreachable!`
arm that guard was protecting must be shown unreachable by R11's splicing, not by
the rejection.

**R10 — Scrutinee selection is the topmost enum-typed input (D-E).**
`check_clause_word` (`check.rs:4350`) and the mirroring clause lowering select the
topmost **enum-typed** input as the scrutinee instead of assuming
`inputs.last()`. Existing clause words are unaffected (regression test on
`area`/`unwrap-or`); the selection is deterministic when several inputs are
enum-typed (topmost wins).

**R11 — Splicing covers clause bodies.** `is_combinator` / `collect_combinators` /
`combinator_of` (`check.rs` :5697/:5659/:5678) accept a `WordBody::Clauses` word
that declares a quotation parameter; `alpha_rename_locals` (`ast.rs:1009`) renames
a clause body's locals per splice with the same `uid` discipline; the splice emits
the scrutinee's discriminant test and inlines the matching clause's body into each
target block, so a two-variant scrutinee produces exactly today's
`Jnz` + inline-branch shape and no `Instr::Call` to the spliced word.

**R12 — Self-tail detection sees through a spliced clause body.**
`body_tail_calls_self` (`ir.rs:2495`) and the checker's self-tail machinery
(`SelfTailMarker`, `self_tail_combinator`) recognise a tail self-call inside a
clause body's arm and inside a quotation literal passed to a spliced
quotation-taking word, so the self-tail → loop back-edge transform still fires. **This
is P3's load-bearing exit, not a follow-on**: without it P4 stack-overflows
`countdown.sth`.

**R13 — P3 is proven on a non-`if` word.** A user-written clause-bodied
quotation-taking word over a user enum (e.g.
`type: Choice | Left | Right ;` with `: pick ( [ ..a -- ..b ] [ ..a -- ..b ] Choice -- ..b )`)
compiles, splices with no `Instr::Call`, and a self-tail call inside one of its
branch quotations lowers to a back-edge and runs in constant stack at 1M
iterations. Keyword `if` is untouched in this phase, so the whole corpus stays
byte-for-byte.

### P4 — `if` as a pure Sooth word

**R14 — `if` is an ordinary clause-bodied word, no name recognition.** Shipped as
Sooth source:

```sooth
: if ( [ ..a -- ..b ] [ ..a -- ..b ] Bool -- ..b )
| False   swap drop call
| True    drop call
;
```

with the call-site order DESIGN.md documents, `cond [ then ] [ else ] if`, reached
through D-E/R10 (the `Bool` is the deepest input; the quotations sit above it).
Nothing in the checker or lowering may match on the name `if`. Its two quotation
arguments must be statically known literals, the same located rejection any spliced
combinator gives.

**R15 — The keyword and its AST node die.** The parser's `if`/`else`/`end` arm
(`parser.rs` near :1986) is removed; `if` lexes as an ordinary `Call`.
`TermKind::If` is deleted from `ast.rs` along with all 21 references: both checker
arms, the lowering, the self-tail detector's `If` arm (now general per R12), the
eight traversal helpers, and both REPL `if`-body rewriters (`repl.rs` :276, :2054).

**R16 — `lib/combinators.sth` is rewritten, not grandfathered.** `while` (`:57`,
`p call if p while else end`) and `filter` (`:45`) move to the call form, e.g.
`: while ( 'a [ 'a -- 'a bool ] -- 'a ) | p | p call [ p while ] [ ] if ;`.
`while`'s self-tail `p while` now sits inside a branch quotation and **must** still
lower to a loop back-edge (R12).

**R17 — The termination guarantee is preserved, and tested at the shape that
proves it.** `examples/countdown.sth` (1,000,000 iterations) completes in constant
stack; `examples/gcd.sth` and `examples/poly_if.sth` (`mymax`/`choose`/nested
`mymax3`, `if` inside polymorphic bodies) compile and print identically. A branch
whose two arms disagree on effect is the **same located diagnostic** the keyword
`if` produces today, message text asserted.

**R18 — P4 preserves observable behaviour; the QBE baseline may shift once.**
Every golden and `examples/*.sth` produces identical program output and
diagnostics. Byte-for-byte QBE is *not* asserted across P4 (a spliced clause body
may renumber blocks/temporaries against the keyword path); where
`tests/qbe_baseline*` shifts, it is updated deliberately with a one-line
justification in the commit, and the shift must be renumbering only — no new
`Instr::Call`, no closure materialisation, no lost back-edge. R3's P1–P2
byte-for-byte requirement is unaffected.

### P5 — `cond`

**R19 — `cond` is ordinary Sooth source, fixed arity (D-C).** A fixed-arity `cond`
is written in Sooth as nested `if` over statically known quotation literals,
predicates evaluated in order, first `True` arm running, default body last. No
intrinsic, no name recognition. One golden dogfoods it. The variadic
`[ pred ] [ body ]`-list form is recorded as blocked on first-class
quotations-in-collections and stays out of scope.

## Out of scope (hard boundary)

- Ordinary user-enum declaration/matching mechanics beyond R1's layout rule, R5's
  two-variant elimination, and R10's scrutinee relaxation.
- `cond`'s variadic form (D-C).
- Output/return-type overloading, traits/type-classes, non-static dispatch.
- Re-deriving or re-opening any 8a rule; this slice deletes the one `.` `bool` row
  8a shipped and reuses its dispatch unchanged.
- Byte-for-byte QBE across P4 (R18); behaviour-identical there, byte-for-byte
  through P1–P2 (R3).
- A native hand-backend, LLVM, JIT, or comptime interpreter (standing invariants).

## Exit criteria (testable)

1. **Zero-payload enums lay out as a bare scalar discriminant** (unit test on a
   non-`Bool` enum, so the general rule is tested, not just `Bool`);
   payload-bearing enums (`shapes.sth`, `vm.sth`) byte-for-byte unchanged.
2. **`Bool` is that enum**: `type: Bool | False | True ;` declared, `True`/`False`
   constructors at discriminants 0/1, `true`/`false`/`bool` still accepted
   spellings, primitive `Type::Bool` gone from the type layer.
3. **Internal-boolean codegen byte-for-byte after P1–P2** (R3): every
   `tests/qbe_baseline*` golden unchanged, `countdown.sth` included, save two
   accepted deviations — the behaviourally-inert enum-drop symbol renumber recorded
   under R3 (`list.ssa`/`refs.ssa`: `sooth_enum_drop_0`→`_1`, P1) and the
   bool-print call-site reroute recorded under R6 (`leap.ssa`: inline `$boolstrs`
   index → `call $.2e.`, P2). Both leave condition codegen and runtime output
   unchanged.
4. **`.` is a library overload**: the primitive `bool` row is gone,
   `: . ( Bool -- ) ;` dispatches through 8a's `builtin_overloads` on the native
   **and** REPL paths (no segfault, R7), `true .`/`false .` print `true`/`false`.
5. **A user-written clause-bodied quotation-taking word splices** with no
   `Instr::Call`, and a self-tail call inside one of its branch quotations runs in
   constant stack at 1M iterations, proven on a non-`if` word before `if` depends
   on it (R13). Existing clause words unaffected by the scrutinee relaxation (R10).
6. **`if` is a pure Sooth word**: no `if`/`else`/`end` in the parser, no
   `TermKind::If` in `ast.rs`, no checker or lowering arm matching the name `if`;
   `cond [ then ] [ else ] if` compiles and runs; a mismatched two-arm effect is
   the same located diagnostic, message text asserted.
7. **The termination guarantee survives** (R17): `countdown.sth` completes in
   constant stack at 1,000,000 iterations after the migration; `gcd.sth`,
   `poly_if.sth`, and `lib/combinators.sth`'s `while`/`filter` behave identically.
8. **`cond` ships** as ordinary Sooth source with a golden selecting the correct
   arm, and no intrinsic.
9. **Corpus identical output**: full goldens + `examples/*.sth`, including
   `bool_abi.sth` (returned `Bool` still a scalar ABI value) and the REPL
   `true`/`false` sessions.
10. **Green.** `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

## Testing

Unit tests beside stage code, `thing_condition_expected` naming, exact message text
asserted, each guard mutation-checked (delete what the test guards, prove the test
fails). Key tests:

- `zero_payload_enum_lowers_to_scalar_discriminant` (non-`Bool` enum, R1),
  `payload_bearing_enum_layout_unchanged`.
- `bool_enum_true_false_construct_0_and_1`,
  `cmp_result_is_byte_for_byte_after_bool_migration`,
  `qbe_baseline_unchanged_after_bool_migration` (R3, all baselines).
- `bool_print_dispatches_to_library_overload` plus a REPL twin proving no
  fall-through to the deleted builtin arm (R6/R7),
  `bool_abi_returned_bool_stays_scalar` (R8).
- `clause_bodied_word_may_take_a_quotation_parameter` (R9),
  `clause_scrutinee_is_topmost_enum_typed_input` and
  `existing_clause_words_unaffected_by_scrutinee_relaxation` (R10),
  `clause_bodied_combinator_splices_without_a_call` (R11),
  `self_tail_inside_a_spliced_clause_arm_lowers_to_back_edge` (R12),
  `pick_word_runs_constant_stack_at_1m_iterations` (R13).
- `if_is_not_recognised_by_name` (grep-style guard that no checker/lowering arm
  matches `"if"`, R14), `if_branch_effect_mismatch_is_error` (message text
  re-homed), `if_non_literal_branch_is_rejected`, `poly_if_still_compiles` (R17).
- `countdown_completes_in_constant_stack_after_migration` (R17, the load-bearing
  one), `while_self_tail_still_lowers_to_back_edge` (R16).
- `cond_selects_first_true_arm`, `cond_falls_through_to_default` (R19).

Green = `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

## Phases

Ordered per D-B. P1, P3, and P4 are the high-blast-radius phases (layout code, the
splice/self-tail machinery, the hottest control-flow path) and route to the
stronger implementer.

1. **`Bool` as a zero-payload enum with scalar layout.** The general layout rule,
   the declaration, constructors replacing `BoolLit`, retained spellings, the
   `and/or/xor/not` rows, exhaustiveness. Keyword `if` untouched; every QBE
   baseline unchanged.
2. **`.` as a library overload.** Delete the primitive `bool` printable row, ship
   `: . ( Bool -- ) ;` through 8a's dispatch on native and REPL paths after
   verifying the wiring; returned/carried-`Bool` regression.
3. **The clause-word generalisations.** Quotation parameters on clause bodies,
   topmost-enum-typed scrutinee selection, splice-eligibility for clause bodies,
   and self-tail detection through a spliced clause arm — proven on a non-`if`
   user word at 1M iterations, with keyword `if` still in place and the corpus
   byte-for-byte.
4. **`if` as a pure Sooth word.** Ship the clause-bodied `if`, delete the keyword,
   `TermKind::If`, and all 21 references including both REPL rewriters; rewrite
   `lib/combinators.sth`'s `while`/`filter`; `countdown.sth` still constant-stack.
5. **`cond` as ordinary Sooth source.** Fixed-arity nested-`if` `cond` with a
   dogfood golden; record the variadic blocker.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Migrate Bool to a zero-payload enum with a general scalar layout. Add the rule that an enum whose every variant has an empty payload lowers to a bare scalar discriminant (register-resident, no aggregate), unit-tested on a non-Bool enum too. Declare type: Bool | False | True ; make True/False constructors at discriminants 0/1 replacing BoolLit while keeping true/false/bool as surface spellings; keep the scalar IrType so Cmp/Jnz/bitwise/internal-condition codegen stays byte-for-byte (every QBE baseline unchanged, countdown.sth included); keep and/or/xor/not's Bool rows and {False,True} exhaustiveness. Keyword if untouched.",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "Delete `.`'s primitive bool printable row and ship a library `: . ( Bool -- ) ;` (clause-match False/True, print false/true with newline) reached through 8a's builtin_overloads dispatch on both the native and REPL paths, after verifying on merged main that lower_call still reads builtin_overloads on check_term/poly_delegate_op/lower_line (no fall-through to the deleted builtin arm, which segfaulted once already). Returned/carried-Bool regression: bool_abi.sth stays a scalar ABI value.",
      "difficulty": "standard"
    },
    {
      "phase": 3,
      "focus": "Generalise three clause-word restrictions while keyword if is still in place. Delete clause_bodied_quotation_word_error so a clause body may take quotation parameters; relax clause scrutinee selection from inputs.last() to the topmost enum-typed input in both the checker and the clause lowering (existing clause words unaffected); make is_combinator/collect_combinators/combinator_of and alpha_rename_locals accept a WordBody::Clauses word with a quotation parameter and splice its matching arm inline (discriminant test plus inlined arm, no Instr::Call); extend body_tail_calls_self and the checker self-tail machinery to see a tail self-call inside a spliced clause arm and inside a quotation literal passed to a spliced word. Prove it on a non-if user word (Choice/Left/Right pick) that splices and runs constant-stack at 1M iterations; corpus stays byte-for-byte.",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "Ship if as an ordinary clause-bodied Sooth word (( [ ..a -- ..b ] [ ..a -- ..b ] Bool -- ..b ), False/True arms calling the right quotation) with DESIGN.md's documented cond [ then ] [ else ] if call order, and delete the keyword: remove the parser if/else/end arm, TermKind::If from ast.rs, and all 21 references including both checker arms, the lowering, the self-tail detector's If arm, the eight traversal helpers, and both REPL if-body rewriters. No checker or lowering arm may match the name `if`. Rewrite lib/combinators.sth's while and filter to the call form. countdown.sth must still complete in constant stack at 1,000,000 iterations and gcd.sth/poly_if.sth behave identically; QBE baseline may shift for renumbering only, never a new Instr::Call or a lost back-edge.",
      "difficulty": "hard"
    },
    {
      "phase": 5,
      "focus": "Ship a fixed-arity multi-way cond as ordinary Sooth source: nested if over statically known quotation literals, predicates evaluated in order, first True arm running, default body last, no intrinsic and no name recognition. One dogfood golden selecting the correct arm. Record that the variadic [pred][body]-list form stays blocked on first-class quotations-in-collections.",
      "difficulty": "standard"
    }
  ]
}
```
