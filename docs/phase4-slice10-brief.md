# Phase 4 Slice 10: rows in quotation effects, and the loop primitive becomes library code (brief)

Sooth has exactly one loop construct left that user code cannot write: `times`. It is a
compiler intrinsic not because it needs compiler magic to *run*, but because user source
cannot *spell its signature*. Everything underneath it is already general: the
self-tail-call loop transform is a blanket compilation guarantee with no keyword (Slice 6,
"existing recursive words simply stop growing the stack in tail position"), quotation
parameters thread through a self-tail splice as compile-time constants (`while`,
`lib/combinators.sth`, proven constant-stack under a `ulimit -s` sweep in
`tests/phase0.rs`), and the loop-state bookkeeping is shared `Builder` state
(`save_loop_state`/`alloca_home`, `src/ir.rs:2742/2477`), not `times`-private plumbing.

The charter is the language's own: as much of Sooth in Sooth as possible, the compiler's
surface being the type system plus the splice/TCO machinery, not a menagerie of blessed
words. `times ( ..s i64 [ ..s i64 -- ..s ] -- ..s )` is the language's single most
load-bearing signature, used by every combinator and every hand loop, and today it is
expressible only in Rust (`check_abstract_quotation_times`, `src/check.rs:5618`, synthesized
directly, never parsed). This slice makes that signature writable, in two parts:

- **10a, the mechanism**: row variables inside a declared quotation effect, plus the
  back-edge fix below, exiting on a *user-space* `my-times` golden. The intrinsic is not
  touched.
- **10b, the migration**: `times` moves to `lib/combinators.sth`, the intrinsic arms are
  deleted. Separate slice, 8c-shaped lightweight process (delete the special case, let the
  suite find the fallout), because migration risk and mechanism risk should not share a
  review.

## Recon: measured against the built compiler (probe programs run 2026-08-09)

**1. The signature already parses everywhere except the nested effect.** A top-level
`..s` is recognized (`parse_poly_slots`, `src/parser.rs:1178`; `PolyBuilder.row_in/row_out`,
`src/parser.rs:665`) and represented (`PolySig.row_in/row_out/row_var_names`,
`src/ast.rs:539-549`). Inside a quotation type, `parse_poly_slot` (`src/parser.rs:1208`)
has no `..` branch at all: it falls through to `parse_type_expr`, which rejects. Verified:

```
: my-times ( ..s i64 i64 [ ..s i64 -- ..s ] -- ..s ) ... ;
=> error: unknown type `..s` at line 1, col 28
```

Same rejection with a fresh name (`..t`), so it is structural, not a scoping accident.
`PolyType::Quotation(Vec<PolyType>, Vec<PolyType>)` (`src/ast.rs:526`) carries no row
fields. This is the boundary Slice 6a drew deliberately (its R2/R28: "a row variable `..s`
inside is out of scope").

**2. The top-level row already works, as emergent pass-through.** Nothing in the checker
reads `PolySig.row_in`/`row_out` (grep: only `src/repl.rs:2305,2895` for signature
printing and the REPL multi-output gate). The pass-through falls out of unification only
touching `sig.inputs.len()` slots. Verified:

```
: passthru ( ..s i64 -- ..s ) drop ;
: main ( -- ) 1 2 3 passthru . . ;
=> prints 2 1, exit 0
```

So 10a adds no top-level row machinery; the combinator's own `..s` costs nothing new.

**3. Poly self-recursion exists only for combinators, only at splice time.** A poly
non-combinator cannot name itself at all:

```
: spin ( 'a i64 -- 'a ) | n | n 0 > if n 1 - spin end ;
=> error: unknown word `spin` in `spin` (line 1)
```

A quotation-taking combinator's self-call is instead intercepted inside the splice as the
loop back-edge (`SelfTailMarker`, `src/check.rs:392`, set at `:5942`, matched at `:6968`),
gated on `is_combinator` (`:5697`) and `has_self_tail_call` (`:3169`); non-tail recursion
is rejected by `check_combinator_cycles` (`:5742`, "the inliner would splice it forever",
`:5829`). A user-space `times` is a quotation-taking combinator, so it rides the supported
path. The `spin` hole stays a known sharp edge, not this slice's problem.

**4. The back-edge arm models its result wrongly for a `times` shape, independent of
rows.** This is the paper pre-check's find. At the back-edge (`src/check.rs:6956-6998`)
the arm's result stack is set to the combinator's *non-quotation inputs*, with the
assumption stated in its own comment: "which for a self-tail combinator are exactly its
declared outputs". True for `while` (`'a` in, `'a` out, state-threading shape); false for
a loop that *consumes* its counters. Verified with no row anywhere in sight:

```
: my-times ( 'a i64 i64 [ 'a i64 -- 'a ] -- 'a )
  | from to f |
  from to < if
    from f call
    from 1 + to f my-times
  end ;
: main ( -- ) 0 0 5 [ bump ] my-times . ;
=> error: stack effect mismatch in `my-times` (line 4)
     `if` branches leave different stack depths (then: 3, else: 1)
```

The then-arm "leaves" `from+1 to` (the carried counters, minus the filtered quotation),
the else-arm leaves the declared output. So even a fully concrete `times`-shaped self-tail
combinator is unwritable today. 10a must fix this to exit, row or no row.

**5. Row-effect reasoning exists, intrinsic-only, and the general case is *easier* than
the intrinsic's.** `check_abstract_quotation_times` (`src/check.rs:5618`) already
implements "pop the declared fixed inputs above an opaque row, require the row restored"
for the one blessed signature. The general splice-site paths
(`check_literal_against_declared_effect` `:6067`, `check_poly_combinator_args` `:5993`)
ground a declared effect with no row concept. The architectural fact that bounds this
work: combinators mint no `IrFunc` and are term-spliced per call site (`inline_combinator`
`:5842`, 6a's R18/R20), so at every point where a row-bearing effect is checked against an
operand, the row is **concrete**: it is the caller's actual stack below the fixed inputs.
No abstract row unification, no `Subst` extension, no mangling impact (`Subst` stays
`ty`+`len`, combinators produce no symbols). "Row polymorphism" here is per-splice depth
arithmetic, the shape the intrinsic already implements.

**6. Lowering should need nothing new, and the goldens must prove it rather than assume
it.** Carried slots are computed from the concrete stack at the splice site; nested-loop
hoisting was generalized in 6d (`alloca_home`); the intrinsic's own lowering arm
(`src/ir.rs:3138`) rides the same snapshot machinery a spliced self-tail loop does. The
two witnesses that keep this honest: constant stack at 1M iterations under `ulimit -s
1024` (the `run_at_stack_limit` pattern, `tests/phase4_combinators.rs:1354`), and an
aggregate riding the row across the back-edge with per-iteration data dependence, so the
slice-3 aliasing class (stale value from iteration k visible at k+1) shows up as a wrong
number, not a crash.

## Decisions (settled here, not reopened by the spec)

1. **One row per signature.** A row name inside a quotation effect must be the signature's
   own top-level row (`..s` bound at the deepest input slot). A fresh name is a located
   error: there is nothing else it could denote, since the literal reaches into exactly
   the caller region below the combinator's fixed inputs. Same discipline as bounds
   (declared at the binding occurrence, X3 rejects a bound on a use).
2. **Both sides or neither.** A row in a quotation effect appears in the inputs and the
   outputs or not at all (`[ ..s i64 -- ..s ]`), mirroring the intrinsic. A one-sided row
   is a located error; a quotation that consumes an unknown region and does not restore it
   cannot be spliced into anything meaningfully.
3. **The back-edge arm produces the ground declared outputs, and the self-call's arguments
   are checked against the ground declared inputs explicitly.** Today the arm's fiction
   (non-quotation inputs) lets the `if`-join transitively type-check the carried values;
   replacing the fiction with the declared outputs loses that, so the edge check gains an
   explicit unify of `stack[base..]` against the ground declared inputs. Sound because the
   marker only matches in tail position (recon 3), so the join this fiction feeds is the
   body-final join. `while` checks identically before and after (its non-quotation inputs
   equal its outputs; that equality is exactly why the bug was invisible until now).
4. **R8/R9 stay.** Linear-across-the-edge and reference-across-the-edge checks
   (`src/check.rs:5474-5536`) run unchanged at the back-edge; rows add no exemption.
5. **The index type stays `i64`.** Widening the loop counter to other integer types is
   overload territory (8a's table, one `times` candidate per index type if ever wanted),
   not row machinery. Out of scope here.
6. **10a does not touch the intrinsic; 10b deletes it.** 10a exits on a user-space
   `my-times` living beside the intrinsic. 10b then: `times` written in
   `lib/combinators.sth`, the `check_term` interception arm, `check_abstract_quotation_times`,
   and the `src/ir.rs` `"times"` arm deleted, diagnostics goldens re-pointed deliberately
   (the intrinsic's bespoke messages become the general combinator ones; each re-point is
   reviewed, none silently). Corpus *outputs* stay byte-identical; splice depth of
   `each`/`map`/`fold`/`filter` call sites grows by one, and the binary-size delta across
   `examples/` is measured and recorded in the 10b spec, not waved through.

## Open questions for the spec

- **Where the row check runs when the operand is not a `Known` literal.** Inside a future
  `each`-built-on-`times`, the `f` passed down to `my-times` is the outer combinator's own
  parameter, not a literal. The intrinsic handles this via the abstract path
  (`check_abstract_quotation_times`, composing declared effects); the general mechanism
  must serve both the Known-literal splice and the abstract-parameter pass-down, and 6a's
  spec claims its obligations discharge at the def site while the depth-mismatch in recon 4
  surfaced with ground types at the call site. Map which check runs where before writing
  requirements; the intrinsic's two paths are the prototype.
- **Diagnostic wording and numbering** for the three new rejections: fresh row name in a
  quotation effect (decision 1), one-sided row (decision 2), back-edge argument mismatch
  (decision 3). Located, naming the row/argument and the declared signature.
- **Whether `PolyType::Quotation` grows row fields or a new wrapper carries them.** The
  representation should mirror `PolySig` (`row_in`/`row_out` as options sharing the
  signature's row id space), but the spec should check what `unify_poly_input`'s pointwise
  row walk (6a R6) needs to skip.

## Out of scope

- A numeric/arithmetic bound (`Bound::Int`); nothing here needs arithmetic on a type
  variable. The counter is concrete `i64` (decision 5).
- Fixing non-combinator poly self-recursion (`spin`, recon 3). `times` does not need it.
- Rewriting `while`/`each`/`map`/`fold`/`filter` onto rows. 10b moves `times` only; the
  Slice 6a finding that the others do not need a row (fold's accumulator via a captured
  cell) stands.
- Anything runtime-quotation (7b): a capturing literal passed to a row-bearing combinator
  follows 7b's rules unchanged.
- Mutual recursion between combinators; non-tail self-calls. Already rejected, stay
  rejected.

## Sequencing, and the phase question

This is Phase 4 work: it completes the phase's own arc (slice 4 introduced the intrinsic
and its signature; slice 6 made loops fall out of recursion; slice 9 deletes the `if`
keyword; this deletes the last blessed loop word). It depends on nothing in 8a's table or
7b's captures, but all three edit `check_term`'s dispatch spine and the literal checks, so
**10a starts after 7b and 8a land**, avoiding a three-way merge on `src/check.rs`. 10b
follows 10a alone.

## Exit

**10a:** the recon-4 `my-times`, with the row restored to its signature
(`( ..s i64 i64 [ ..s i64 -- ..s ] -- ..s )`), compiles from user source beside the
intrinsic and:

- sums correctly through a literal (`0 0 5 [ bump ] my-times .` prints `10`);
- runs 1M iterations to completion at `ulimit -s 1024` (constant stack, exit 0);
- carries a struct through the row with per-iteration dependence and prints the
  arithmetically correct fields (aliasing witness);
- nests inside itself with correct output (6d parity);
- `while` and the full existing corpus are unchanged;
- decisions 1-3's rejections are located-error goldens.

**10b:** `times` is Sooth source; `check_abstract_quotation_times` and the `"times"` arms
in `check.rs`/`ir.rs` do not exist; the corpus prints byte-identical output; the
diagnostics diff and binary-size delta are recorded in the spec.
