# Phase 4 Slice 10c: `if` as an ordinary library word (as built)

Records the shipped state of slice 10c, not the plan that produced it. Supersedes the
delivery-plan spec's requirement/criterion numbering; requirement ids (R-P1-1, R-P3-3b …)
survive only where code comments still cite them.

## What the compiler knows now

Three machine-level primitives, and nothing else about control flow:

- **`branch ( ..a u32 ~[ ..a -- ..b ] ~[ ..a -- ..b ] -- ..b )`** — conditional jump on a
  32-bit flag (nonzero is true), each arm in its own block, join. Checker arm
  `check::terms::check_branch` / `check_branch_join`; lowering in
  `ir::func_builder::calls` (`"branch"`), the old `lower_if` body. It is the **single
  builtin exempt** from the R11 quotation-operand default-deny.
- **`tag ( E -- u32 )`** for a payload-free (`is_scalar`) enum. `is_scalar` is derived at
  check time from the AST (`check::word_families`), so `tag` on a payload-carrying enum, or
  on a non-enum, is a located *check* error. Lowering emits `Instr::Tag`, which exists only
  to give the discriminant an integer `IrType`; the backend emits a same-register `copy`
  that QBE coalesces. Reusing the operand's `Value` would leave it `IrType::Enum` (a later
  `.` prints `true`/`false`), and `Instr::Conv` would be the width conversion the no-op
  criterion forbids.
- **`u=` `u<` `u>` `u<=` `u>=` `u<>`** — each returns `u32` (0/1), one `BUILTIN_TABLE` row
  per numeric type, sign/float behaviour derived from the operand type exactly as `CmpOp`
  always did (no `s<`/`f<` split). These are **renames** of the old comparison rows, in both
  `check::builtins` and the lowering match; the old names left the table.
  `builtin_table_comparisons_have_a_row_per_numeric_type` was retargeted onto them, not
  deleted.

Everything typed lives in `lib/core.sth`: `=` `<` `>` `<=` `>=` `<>` (all
`'T: Copy Ord`-polymorphic and `inline`, bodies `u… [ true ] [ false ] branch`), plus `if`
and `unless` over `tag`+`branch`. `while` stayed in `lib/combinators.sth` with a postfix
body. `TermKind::If` and the `if`/`else`/`end` grammar are gone (a grep test pins it).

## Decisions that are still load-bearing

**The flag is `u32`, not `usize`.** `bool` is already a 32-bit 0/1, `Instr::Cmp` produces
that width, the conditional jump consumes it, and `tag` is a genuine no-op only at 32 bits.
A target-width condition would cost a widen after every comparison and a narrow at every
branch. Express widths as bit widths in IR-facing code; `backend/qbe.rs` is the only place a
register class is spelled.

**There is no flag→enum retype, deliberately.** `tag` is total; its apparent inverse is not,
so it could manufacture a `bool` no clause-dispatch chain matches. Comparison wrappers
construct their result by branching and naming a variant instead, which makes the invalid
value unconstructible. A future integer→enum conversion must be **branch-shaped**
(`untag ( ..i usize ~[ ..i 'E -- ..o ] ~[ ..i usize -- ..o ] -- ..o )`), never value-shaped;
its natural `[ Ok ] [ Err ]` wrapper additionally needs generic enums, which do not exist.

**Two inline gates, opposite fates.** The polymorphic-`inline` gate in
`check::word_entry::check_inline_declaration` was a *policy* rule and is deleted for all
three variable kinds; `check_inline_polymorphic_signature_is_accepted` is the retargeted
witness and is named `=` on purpose, since a neutral name like `cmpgt` slips past the second
gate and would pass either way. The builtin-name gate (`BUILTIN_TABLE.contains_key`) is a
*soundness* rule and stays: a spliced builtin-overload name would have lowering look up a
symbol in an `env` combinators are excluded from. The six library comparisons escape it only
because their rows left the table, which makes the ordering load-bearing: **rename the rows
before defining the words**.

Without `inline`, every comparison in the language becomes a real call with a frame; with it,
QBE folds the branch-and-construct diamond to a `cmov`. The emitted **IR** is deliberately not
what it was (a library `=` is `Cmp` plus a diamond, and there is no folding pass); equivalence
is asserted on **machine code**. `tests/qbe_baseline/` was regenerated for that reason;
`tests/corpus_stdout/` (captured before the swap) is what pins behaviour across the migration.

**INV-INLINE-COMBINATOR.** A quotation-taking word is always inlined at each call site and
mints no `IrFunc`; it has no opaque call form, and its output row is discovered by forward
checking the spliced terms, never solved by row unification. Written in DESIGN.md and as
doc-comments on `check::drop_graph`'s tail walk and the combinator splice in
`ir::func_builder::calls`. Slice 7b (first-class runtime quotations) is where it breaks.

**Tail-splice recognition is one rule with one owner.** `drop_graph::tail_position_calls` /
`tail_called_param_slots` compute a per-combinator tail-called-parameter set; `branch` has no
walkable body, so it is **seeded** with "both quotation operands are in tail position", taking
over the role the deleted `TermKind::If` descent played. Omit that seed and `if`'s set
computes empty and `gcd`/`sum-to` silently lose their loops. The walk carries a visited set and
declines on a cycle, declines on an opaque (non-literal, non-own-parameter) quotation slot, and
declines on an ambiguous name; declining costs a loop transform, never correctness.
`body_tail_calls_self` carries the same builtin-name refusal `has_self_tail_call` has, so
neither pass can treat a builtin as a self-call while the other refuses.

**A `~` branch arm is exempt from three argument-site rules**, keyed on being a *tail-called
parameter slot*, not on being a `~` and not on `is_inline` (which was tried and wrongly disarms
the D3 rules for `times`). `check_literal_against_declared_effect`'s premise, that the callee may
run the quotation any number of times, is wrong for an arm that runs at most once in place, so
for a tail-called slot it walks with `back_edge = false`, keeps the caller's real row slots, skips
the D3 linear-capture and borrow-place rejections (the splice enforces them properly through
`check_branch_join`'s cloned scopes and `MaybeMoved` join), and restores `scope.moves` afterwards
because sibling arms are alternatives from the same move state.

**Interop the migration forced.** Keeping a user overload of a comparison name working took three
changes: `check_generic_concrete_overlap` asks whether the concrete candidate could actually
instantiate the generic signature (an `Ord` bound admits only the numeric tower);
`poly_sig_could_match` honours the same bound; and `check_term` falls through to the concrete
lookup when no polymorphic candidate admits the operands, recording the symbol in
`builtin_overloads` so lowering emits a call instead of splicing the library body.
`check_poly_combinator_args` also re-teaches D8's literal coercion to the poly path (a fresh
integer literal filling a bare type variable is deferred and unified last, restricted to
`usize`/`isize`), so `5 3 >usize <` still checks while `1 >i32 2 <>` stays rejected.

**`>=` versus `>T`.** With `>=` out of `BUILTIN_TABLE`, `check_operator`'s conversion-prefix test
claimed it and rejected with `` unknown type `=` ``. Not a lexer bug (`>=` lexes as one word):
`check::operators::conversion_target_name` carves it out by name, pinned by
`parser::tests::ge_is_not_read_as_a_type_conversion`.

**Prelude mechanics.** `lib/core.sth` is `include_str!`'d and parsed once by
`parser::prelude_words`, which `parser::parse` and `driver::assemble_module` **append** (not
prepend, so a file's own words keep their indices) and `Session::new` seeds through `eval_def`.
`resolve::mangle` never mangles a prelude name, for the reason it never mangles `main`. The REPL
import path excludes them from `body_rename`. Prelude bodies are checked against the whole
program's environment, so their locals are spelled `if--cond` / `if--then-arm` / `if--else-arm`
(and the `unless--` twins): a local may not shadow a callable name, and even plain `cond` would
collide with DESIGN.md's documented future multi-way branch word. Prelude-word locals have no
real hygiene; that is a separate slice. Branch-join diagnostics are located at the first arm
literal, which is always the caller's own code, never at `branch` inside library source.

## Tests and the placebos they exist to exclude

`tests/phase4_slice10c_tail_splice.rs`, `_row_gate.rs`, `_primitives.rs`, `_corpus_stdout.rs`,
plus unit tests beside `check_branch`, `tag`, the comparison lowering and the tail walk.

The assertion shapes are load-bearing, not stylistic:

- Loop shape is asserted on **lowered IR** (`jmp` back-edge, zero self `Instr::Call`), and the
  constant-stack runs use **1e6 iterations under `ulimit -s 512`** and assert the computed value.
  A small N or a bare "exit 0" passes under real recursion.
- The recon-4 negative (`t call e drop`) asserts a self `Instr::Call` is **present**; "it builds"
  passes either way.
- A contradicting branch output is asserted rejected **at the argument site** with its message,
  not merely "rejected somewhere" (the splice-site forward check would satisfy that).
- `if`-is-a-library-word asserts **resolution to a `lib/` `WordDef`**. `nm`-silence and
  "jump-and-join, no call" pass identically for a primitive `if`.
- The poly-`inline` witness is named `=`, not a neutral name.
- The comparison cost check asserts the **instruction sequence**, since IR byte-identity is
  impossible by construction, and one golden runs a comparison library word on `u32` and `i8` so
  a silent narrowing to `i64` cannot hide.
- The parser's three existing quotation-row negatives keep their assertions (fresh name, non-top-
  level row, output-only row named from the input side), and the output-side differing-row test
  stays an error: the lift keys on an **input-side quotation parameter of an always-inlined word**.

## Known gaps, recorded rather than closed

- **`branch`'s def-site check is partial in the mixed case.** With one literal arm and one
  forwarded parameter, the `Forwarded` arm wins and the literal is never walked, so
  `: w ( u32 ~[ -- i64 ] -- i64 ) | t | t [ 999 888 ] branch ;` passes its own definition. Not
  unsound (no reachable use skips the splice; a real caller reports the disagreement). Closing it
  needs `check_literal_against_declared_effect`, which re-splices without bound on exactly this
  shape. Pinned by `check_branch_leaves_a_literal_arm_unchecked_beside_a_forwarded_one`.
- **Combinator-argument splice termination (pre-existing).** A word that is itself a combinator and
  self-tails through a splice cannot be compiled: the argument-site check re-enters
  `inline_combinator` without bound and the compiler overflows its stack with no diagnostic.
  Reproduces at P1's base commit. Because of it, three of the six shared-predicate sites
  (`check_combinator_cycles`, `splice_tail`, the lowering splice gate) survive the empty-index
  mutation and are witnessed only by `tail_splice_both_predicate_wrappers_agree_for_a_combinator`,
  which asks the two wrappers directly. "Agree by construction" is tested for the `tail` threading
  and the two reachable `self_tail` sites, and inspected for the other three. `collect_all_calls`
  now descends into quotation literals, so a *non-tail* self-call moved into an arm is still
  rejected rather than hanging the inliner.
- **A polymorphic non-`inline` word can no longer branch.** `if` takes quotation literals and
  `poly_walk` rejects a quotation in a polymorphic body outright, so such a word must be declared
  `inline`. `examples/poly_if.sth` and the poly-branch tests took the one-word edit with unchanged
  output, but it is a user-visible narrowing this slice introduced. Lifting `poly_walk`'s
  quotation rejection is a separate slice.
- **`cond` is not shipped** and was never in scope: a variadic `[ pred ] [ body ]` word is not
  fixed-arity. DESIGN.md keeps it as a documented future word.
- **ROADMAP item 12 needs an edit that was reported, not made.** It claims the polymorphic-`inline`
  lift (10c did it) and says it migrates "10c's *injected* `if`/`cond`"; `if` is an ordinary
  `lib/` word and `cond` does not exist. Item 12 narrows to retiring the
  `word_declares_quotation_parameter` leg of `is_combinator`, requiring the `~` literal at call
  sites, and migrating `lib/combinators.sth`.
- Still out of scope: enum eliminators of any spelling and payload-carrying enum dispatch
  (clause bodies keep it), early return, generic enums, first-class runtime quotations,
  dispatch-after-locals (`lib/binary_search.sth`'s sketch remains unbuildable).
