# Phase 4 Slice 9: `if` as an ordinary combinator + `Bool` as a library enum (brief)

ROADMAP frames this as cleanup: `if` stops being a keyword, `cond [ then ] [ else ] if`
(Factor-style) replaces it, and `type: Bool | False | True ;` replaces the primitive.
"Mechanically it is a large migration rather than a design problem" (ROADMAP, slice 9). That
claim is right for the `if`-as-word half and wrong for the `Bool`-as-enum half, measured
against the built compiler's actual enum layout. This brief keeps them separate because they
turn out to have different risk profiles, not because the ROADMAP entry already splits them.

## Recon: measured against the built compiler

**1. `if` is a real keyword AST node today, not sugar over a call.** `TermKind::If {
then_branch, else_branch, else_span, end_span }` (`src/ast.rs`), parsed as a dedicated form
in `parser.rs` (`Token::Word(w) if w == "if"`), checked at two sites (`check.rs` ~4237,
~7094 — the second is the poly-body path) and lowered at `ir.rs` ~4069 via
`Terminator::Jnz(cond, then_block, else_block)`. 21 references to `TermKind::If` across the
tree.

**2. The combinator-recognition machinery already accepts `if`'s shape for free.**
`is_combinator` (`src/check.rs`) is `WordBody::Terms && word_declares_quotation_parameter`
— any word with a quotation-typed input, arity-agnostic. A hand-written
`: if ( Bool [ -- ] [ -- ] -- ) ;` taking *two* quotation parameters already satisfies it,
and `poly_if.sth` (existing dogfood) proves `if` is used inside polymorphic bodies today
(`mymax`, nested in `mymax3`) — exactly the shape `each`/`map`/`fold`'s poly-combinator path
(6c/6e) already splices. **The mechanism half of this slice is genuinely the cleanup the
ROADMAP claims**: retire `TermKind::If` and its two check/lower sites, ship `if` (and the new
multi-way `cond`) as ordinary library combinators over the existing splicing machinery, and
rewrite `lib/combinators.sth`'s own two keyword-`if` users (`while`'s
`p call if p while else end`, `filter`'s predicate branch) to the new call form.

**3. `Bool` is a genuine primitive scalar today, not a degenerate struct.** `Type::Bool` /
`IrType::Bool` is a 1-byte value in a QBE word-class register (`qbe.rs`: `"w"` class,
`loadub`/`storeb`). It is the *sole* result type of every comparison (`Cmp`), every bitwise
op (`and|or|xor|not`), `BoolLit`, and the operand every `Jnz`/branch terminator needs — 69
references to `Type::Bool`, 38 to `IrType::Bool`, 9 to `is_bool`, spread through the numeric
tower, `check_operator`, and `MirBuilder`'s own internal condition values (bounds checks,
loop back-edges) that have nothing to do with surface `if` at all.

**4. Every existing enum lowers to a tagged aggregate, never a scalar, and nothing branches
on one directly.** `EnumLayout` (`src/ir.rs`) is unconditionally "a fixed `i32` discriminant
tag placed first, then a payload region" — there is no zero-payload/unit-variant special
case that collapses to a scalar register. Elimination is clause-style only (`D2`: "Enums
have no getter/setter/destructure... elimination is clause-style"), which loads the
discriminant via a `Ptr`-offset `FieldLoad` into a value used to pick a clause body — not fed
to `Terminator::Jnz` directly anywhere in the tree. `Jnz`'s `cond` operand is always a bare
scalar `Value` produced by a comparison or `BoolLit`, never a discriminant load.

**5. So "`Bool` replaces the primitive" is the slice's actual design question, not a detail
that falls out of `if` becoming a word.** Taken literally, every comparison result, every
`and/or/xor/not`, and every internal condition (bounds-check guard, `while`'s back-edge test)
would construct a 4-byte tagged aggregate, immediately destructure it via clause dispatch,
and feed *that* into `Jnz` — a real per-branch cost (aggregate construction + a memory load
in place of a register test) on the single hottest control-flow path in the compiler, and a
representational change nothing in the existing enum machinery anticipates. Either (a) `Bool`
gets a special-cased layout distinct from an ordinary user enum (a discriminant with no
payload region, kept in a register) — new design and layout-code work, not cleanup — or (b)
internal boolean-producing/consuming sites keep working in a scalar condition type distinct
from the surface `Bool` enum, and only genuine `if`/`match`-visible boolean values pay the
aggregate cost. The ROADMAP's "delete the special cases, let the exhaustiveness checker find
the arms" line answers the elimination side; it does not answer the representation side, and
the representation side is where the cost lives.

**6. A returned/passed `Bool` today is a scalar ABI value; `bool_abi.sth` is the existing
golden for it (`pos ( i64 -- bool )` returned across a call).** Once `Bool` is
`IrType::Enum`, a returned `Bool` routes through the aggregate-return path — the same
aggregate-return machinery Phase 4 Slice 3 fixed a real aliasing bug in (stable slot + staged
back-edge blit). That is a known-fragile area; this slice would be its first new caller with
a two-variant, zero-field payload, worth a targeted regression rather than assuming the
existing fix generalizes silently.

**7. `.`'s bool row is already committed, and it is 8a's example of exactly this pattern.**
8a's spec ships `bool` as one of `.`'s 15 concrete builtin rows (`BuiltinLower::Print`,
generated off `printable_types()`), by design so a *user's* `.` overload becomes reachable
"for free" through the same table. Once `bool` stops being a primitive `IrType`, that row has
no operand type to match; slice 9 must delete it and ship a library `: . ( Bool -- ) ;`
(pattern-matching `False`/`True`, printing the literal strings) as an ordinary overload. **As
of this writing 8a's impl worktree (`impl/phase4_slice8a_spec-2608091623`, 20 commits ahead of
`main`) has all three phases implemented, not just Phase 1** — but the specific mechanism
slice 9 leans on here needed its own fix cycle to actually work: Phase 2 recorded a
builtin-named user overload's resolution on `Module::builtin_overloads` but `lower_call` was
never wired to read it, so such an overload compiled, linked, and silently mis-lowered through
the name-directed builtin arm — a segfault for `+`/`-`, a wrong answer for `<` — until a later
cycle wired the read; `poly_delegate_op`'s own copy of the same gap (a poly body's operator
calls never reaching a user overload at all) was fixed in the same cycle. That the mechanism
needed a real bug fix, not just a phase, is the useful signal: it is now built and exercised
by 8a's own tests, but slice 9 should not assume it's merge-ready without checking 8a's final
review round didn't touch `builtin_table`'s row set or that wiring again.

## What this brief treats as settled (ROADMAP / DESIGN.md, not reopened)

- `if` stops being a keyword; a multi-way `cond` combinator lands alongside it (DESIGN.md,
  *Conditionals and dispatch*: "a library word taking `[ pred ] [ body ]` pairs").
- `Bool` becomes a user-visible two-variant enum, `False | True` (naming per ROADMAP; DESIGN.md's
  nullability example already establishes the "ordinary enum, not a compiler special case"
  precedent for exactly this shape of type).
- Ordering: after quotations (slice 4, done) and after 8a specifically, because `.`'s
  type-directed printing needs to become an ordinary overload rather than a re-added special
  case (finding 7).

## Open questions for the spec

- **Does `Bool` get a special-cased scalar layout, or does it pay the general enum cost?**
  Finding 5 is the crux. If the answer is "special-cased," that is new work in
  `EnumLayout`/`ir_type_of` (a zero-payload discriminant that stays a register value), and the
  spec needs to say so rather than inherit "mechanical." If the answer is "pays the general
  cost," that is an accepted, measured performance regression on every comparison and branch
  in every existing program, and the spec should say that explicitly rather than let it fall
  out silently.
- **Should this split 9a/9b the way 7 and 8 did** — 9a: `if`/`cond` as ordinary combinators,
  left running over the *existing* `Type::Bool` primitive (finding 2's mechanism, genuinely
  ROADMAP-shaped cleanup, no representation question); 9b: `Bool`'s actual migration to an
  enum, gated on answering the layout question above? The two halves have unrelated risk
  (9a is machinery reuse; 9b is a representation decision touching the hottest path in the
  compiler) and 9a has no dependency on 9b landing first. This is a recommendation for the
  spec to accept or reject, not a decision made here.
- **8a's status, not its phase.** Finding 7 is now about *merge*, not *phase*: all three
  phases are implemented and the exact mechanism slice 9 needs (a builtin-named user overload
  actually dispatching, not just being accepted) already had a real bug — a silent
  mis-lowering, segfault included — caught and fixed on this worktree. What is still open is
  whether that state is what lands on `main`: check at spec time that the merged 8a still
  lists `bool` among `.`'s rows and that `Module::builtin_overloads` is still read by
  `lower_call` on every path (`check_term`, `poly_delegate_op`, the REPL's `lower_line`).
- **`lib/combinators.sth`'s two keyword-`if` users** (`while`, `filter`) need rewriting to
  the call form as part of this slice, not left on the retired syntax; confirm they're in
  scope rather than assumed grandfathered.

## Out of scope

- Anything `match`/clause-dispatch mechanics beyond what `Bool`'s own elimination needs — no
  change to how ordinary user enums are declared or matched.
- `cond`'s exact arity/short-circuit semantics beyond "multi-way `if`" — a design question for
  the spec, not re-derived here.
- Re-deriving 8a's rules; this brief only cites the one row (finding 7) slice 9 touches.

## Exit (sketch, spec settles the real one)

- `TermKind::If` and its two check/lower sites are gone; `if` and `cond` are library words
  compiled through the existing combinator-splicing machinery, including inside polymorphic
  bodies (`poly_if.sth`-shaped programs still compile).
- `type: Bool | False | True ;` is a real, ordinary (or deliberately special-cased —
  spec's call) enum declaration; `Type::Bool`/`IrType::Bool` as a distinct primitive is gone.
- `. ( Bool -- )` is a library overload reached through 8a's dispatch, not a builtin row.
- The full existing corpus (goldens + examples, including `bool_abi.sth`, `poly_if.sth`,
  `lib/combinators.sth`'s `while`/`filter`) compiles and produces identical output.
