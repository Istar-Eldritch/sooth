# Phase 4 Slice 10: rows in quotation effects, and the loop primitive becomes library code (brief)

Sooth has exactly one loop construct left that user code cannot write: `times`. It is a
compiler intrinsic not because it needs compiler magic to *run*, but because user source
cannot *spell its signature*. Everything underneath it is already general: the
self-tail-call loop transform is a blanket compilation guarantee with no keyword (Slice 6,
"existing recursive words simply stop growing the stack in tail position"), quotation
parameters thread through a self-tail splice as compile-time constants (`while`,
`lib/combinators.sth`, proven constant-stack under a `ulimit -s` sweep in
`tests/phase0.rs`), and the loop-state bookkeeping is shared `Builder` state
(`save_loop_state`/`alloca_home`, `src/ir.rs:3016`/`:2718`), not `times`-private plumbing.

The charter is the language's own: as much of Sooth in Sooth as possible, the compiler's
surface being the type system plus the splice/TCO machinery, not a menagerie of blessed
words. `times ( ..s i64 [ ..s i64 -- ..s ] -- ..s )` is the language's single most
load-bearing signature, used by every combinator and every hand loop, and today it is
expressible only in Rust (`check_abstract_quotation_times`, `src/check.rs:6840`, synthesized
directly, never parsed). This slice makes that signature writable, in two parts:

- **10a, the mechanism**: row variables inside a declared quotation effect, plus the
  back-edge fix below, exiting on a *user-space* `my-times` golden. The intrinsic is not
  touched.
- **10b, the migration**: `times` moves to `lib/combinators.sth`, the intrinsic arms are
  deleted. Separate slice, 8c-shaped lightweight process (delete the special case, let the
  suite find the fallout), because migration risk and mechanism risk should not share a
  review.

## Recon: measured against the built compiler (probe programs run 2026-08-09, citations

re-verified 2026-08-10 against `main` after 7b and slice 9 merged — both landed real changes
to `check.rs`'s dispatch/self-tail spine and to `ast.rs`, so most line numbers below moved;
none of the findings themselves changed)

**1. The signature already parses everywhere except the nested effect.** A top-level
`..s` is recognized (`parse_poly_slots`, `src/parser.rs:1178`; `PolyBuilder.row_in/row_out`,
`src/parser.rs:662/669`) and represented (`PolySig.row_in/row_out/row_var_names`,
`src/ast.rs:629-640`). Inside a quotation type, `parse_poly_slot` (`src/parser.rs:1208`)
has no `..` branch at all: it falls through to `parse_type_expr`, which rejects. Verified:

```
: my-times ( ..s i64 i64 [ ..s i64 -- ..s ] -- ..s ) ... ;
=> error: unknown type `..s` at line 1, col 28
```

Same rejection with a fresh name (`..t`), so it is structural, not a scoping accident.
`PolyType::Quotation(Vec<PolyType>, Vec<PolyType>)` (`src/ast.rs:622`) carries no row
fields. This is the boundary Slice 6a drew deliberately (its R2/R28: "a row variable `..s`
inside is out of scope").

**2. The top-level row already works, as emergent pass-through.** ~~Nothing in the checker
reads `PolySig.row_in`/`row_out` (grep: only `src/repl.rs:2305,2895` for signature
printing and the REPL multi-output gate).~~ **Corrected 2026-08-10: stale.**
`poly_sig_shape_eq` reads both (`src/check.rs:3188`/`:3191`) and `poly_sig_str`'s
`render_row` prints them (`:3231-3232`); the repl sites are `:2407` and `:3010`/`:3015`.
This helps rather than hurts — because `poly_sig_shape_eq` drives overload dedup, growing
`PolyType::Quotation` with row fields makes candidates differing only by row
distinguishable for free. Also note what the top-level row does **not** do: it is
untouched-pass-through only, modelled as size-zero during the word's own body check, so
`: shrinks-row ( ..a i64 -- ..b ) drop drop ;` fails with `` `drop` needs 1 values, but the
stack holds 0 ``. Differing `row_in`/`row_out` names parse but nothing verifies they differ
semantically. The pass-through falls out of unification only
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
loop back-edge (`SelfTailMarker`, `src/check.rs:648`, set at `:7178`, matched at `:8443`–`8446`),
gated on `is_combinator` (`:6921`) and `has_self_tail_call` (`:3943`); non-tail recursion
is rejected by `check_combinator_cycles` (`:6966`, "the inliner would splice it forever",
`:7063`). A user-space `times` is a quotation-taking combinator, so it rides the supported
path. The `spin` hole stays a known sharp edge, not this slice's problem.

**4. The back-edge arm models its result wrongly for a `times` shape, independent of
rows.** This is the paper pre-check's find. At the back-edge (`src/check.rs:8441-8484`)
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
the intrinsic's.** ~~`check_abstract_quotation_times` (`src/check.rs:6840`) already
implements "pop the declared fixed inputs above an opaque row, require the row restored"
for the one blessed signature.~~ **Corrected 2026-08-10, by review against the body: it
does not.** `check_abstract_quotation_times` (`:6855-6872`) requires the *declared effect*
be self-similar (`inputs == outputs ++ [i64]`) and then pointwise-matches the declared
outputs against the **top** `outputs.len()` slots; there is no fixed-inputs-above-a-row
decomposition and the row is never inspected — everything below is untouched by
construction. So there is no prototype to generalise, and the spec derives the grounding
instead (see `docs/phase4-slice10-spec.md`'s R8). The rest of this recon item stands. The general splice-site paths
(`check_literal_against_declared_effect` `:7301`, `check_poly_combinator_args` `:7226`)
ground a declared effect with no row concept. The architectural fact that bounds this
work: combinators mint no `IrFunc` and are term-spliced per call site (`inline_combinator`
`:7076`, 6a's R18/R20), so at every point where a row-bearing effect is checked against an
operand, the row is **concrete**: it is the caller's actual stack below the fixed inputs.
No abstract row unification, no `Subst` extension, no mangling impact (`Subst` stays
`ty`+`len`, combinators produce no symbols). "Row polymorphism" here is per-splice depth
arithmetic, the shape the intrinsic already implements.

**6. Lowering should need nothing new, and the goldens must prove it rather than assume
it.** Carried slots are computed from the concrete stack at the splice site; nested-loop
hoisting was generalized in 6d (`alloca_home`, `src/ir.rs:2718`); the intrinsic's own
lowering arm (`src/ir.rs:3441`) rides the same snapshot machinery a spliced self-tail loop
does. The two witnesses that keep this honest: constant stack at 1M iterations under
`ulimit -s 1024` (the `run_at_stack_limit` helper, `tests/phase4_combinators.rs:1403`, e.g.
`three_deep_times_nesting_runs_in_constant_stack` `:1127`), and an aggregate riding the row
across the back-edge with per-iteration data dependence, so the slice-3 aliasing class
(stale value from iteration k visible at k+1) shows up as a wrong number, not a crash.

**7. The back-edge arm's `outs` construction (recon 4) is the exact code 7b's own
post-implementation review flagged as an open, unfixed bug (`5f645f0`, merged 2026-08-10,
after this brief's recon).** Not the same bug, the same *lines*:

```rust
// src/check.rs:8472-8479, inside the back-edge arm (recon 4's target)
let outs: Vec<Slot> = stack[base..]
    .iter()
    .filter(|s| s.quot.is_none() && !matches!(s.ty, Type::Quotation(_)))
    .map(|s| Slot::computed(s.ty))
    .collect();
```

`Slot::computed(s.ty)` builds a bare slot carrying only a type — it drops `s.surviving`
(`7b/R19`: the surviving capture set of an erased capturing quotation, or of an *aggregate*
carrying one, e.g. a struct/array field holding a stored closure; `src/check.rs:213-218`).
The filter excludes a bare erased-quotation slot (`ty == Type::Quotation`) but **not** an
aggregate that carries one — exactly the gap 7b's review found and fixed in the
getter/array/cell paths (`d1b3f0a`, `bee407c`) but explicitly left here, undecided, as a
documented residual risk (`docs/phase4-slice7b-spec.md`'s "Known residual risk" section):
no live exploit found, masked because every self-tail combinator the stdlib actually uses
(`while`) exits through a conditional join and `union_surviving` reconstructs the dropped
set from the sibling arm; the one shape that would expose it — a self-tail combinator with
*no* conditional exit — hits an unrelated, pre-existing IR panic instead.

This matters here because **decision 3 below rewrites this exact block**, and rewriting
"which type populates `outs`" (non-quotation-input types → ground declared-output types)
does not by itself touch "how a slot gets built" — a rewritten `outs` can just as easily
reach for `Slot::computed(declared_ty)` and reintroduce the identical drop, now sourced
from a manufactured type instead of a live slot. See decision 7.

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
   (`check_reference_across_back_edge` `src/check.rs:6741`, `check_linear_across_back_edge`
   `:6762`) run unchanged at the back-edge; rows add no exemption.
5. **Decision 3's rewrite must not ship a second, freshly-sourced instance of 7b's
   residual surviving-set gap (recon 7).** The new `outs` construction is not exempt from
   `7b/R19` just because its types now come from the declared signature instead of the live
   stack: for any output position that positionally corresponds to a `stack[base..]` slot
   (the state-threading shape recon 4 and decision 3 both center on — `while`, and
   presumably `times`'s own row-adjacent fixed positions, if it has any), the rewritten
   code must forward that slot's `surviving`/`quot` fields, not manufacture a fresh
   `Slot::computed(declared_ty)`, following the exact pattern `d1b3f0a`/`bee407c` already
   established for the getter/array/cell paths. Where no such correspondence exists (a
   declared output with no positional source slot, if any signature shape needs one), the
   spec must say so explicitly rather than let it fall out of whatever the rewrite happens
   to do. This does not reopen 7b's residual-risk decision to leave it unfixed — it says
   decision 3 cannot silently inherit that decision's exemption while rewriting the exact
   code it was scoped against; 10a must either close the gap here or explicitly re-verify,
   against the rewritten code, that the same masking condition 7b documented (no self-tail
   combinator lacking a conditional exit exists in the corpus) still holds.
6. ~~**The index type stays `i64`.**~~ **Superseded 2026-08-10: the index becomes `usize`.**
   The reasoning below is unchanged and still holds — admitting *several* index types is 8a
   overload territory, not row machinery — but it argued about widening, not about which
   single type is right, and `usize` is. `len` already returns `usize` (`src/check.rs:5261`),
   so every library combinator converts it down with `>i64` purely to satisfy `times` and
   converts each index back up per iteration (twice in one body in
   `examples/array_totals_hand.sth:20` and `examples/inplace_fold.sth:33`); a count cannot be
   negative; literals already coerce into a `usize` parameter; and `IrType::Usize` is the same
   QBE register as 64-bit `Int` (`"l"`, `src/backend/qbe.rs:298`). Widening the loop counter to
   other integer types is overload territory (8a's table, one `times` candidate per index type
   if ever wanted), not row machinery. Out of scope here.
7. **10a does not touch the intrinsic; 10b deletes it.** 10a exits on a user-space
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
- **Whether `my-times`'s own declared-output shape has any position with no positional
  source slot on `stack[base..]`.** Decision 7 assumes state-threading positions carry a
  live slot to forward `surviving` from; if some declared-output position genuinely has no
  such source (nothing recon 4's or the brief's probes exercise), the spec needs to say
  what populates its `surviving` field and why that's sound, not default silently to `None`.
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
- decisions 1-3's rejections are located-error goldens;
- decision 5's surviving-set gap (recon 7) is resolved one way or the other, on the
  record: either the rewritten `outs` forwards `surviving`/`quot` where a positional
  source slot exists, or the spec documents, against the rewritten code (not by
  assumption), that 7b's masking condition still holds and the risk stays as documented
  residual risk — not silently reintroduced under a different rationale.

**10b:** `times` is Sooth source; `check_abstract_quotation_times` and the `"times"` arms
in `check.rs`/`ir.rs` do not exist; the corpus prints byte-identical output; the
diagnostics diff and binary-size delta are recorded in the spec.
