# Phase 4 Slice 10c: `if` as an ordinary library word (brief)

`if` is the last compiler-known control-flow construct: a bespoke `TermKind::If`
node with its own grammar (`cond if T else E end`), its own checker arm, and its
own lowering arm. This slice retires it. Afterwards the compiler knows three
machine-level primitives (`branch` on a 32-bit flag, `tag` reading a discriminant,
and comparisons) and `if`, `unless` and `while` are ordinary words in `lib/`.
`cond` does **not** ship: it takes a variable number of `[ pred ] [ body ]` pairs,
so it is not a fixed-arity word and would need variadics.

The motivation is layering, not aesthetics. `if` today requires `Type::BOOL`
(`src/check/terms.rs:716`), so the primitive layer reaches up into a
**library-defined enum**, depending on `Bool` existing with those variants in
that order. That is the same category of mistake as a primitive that hardcodes
`pi`: a value, or a type, that belongs in a library baked into the compiler. A
primitive should know about machine words and about how it lays out sum types,
and nothing else.

The postfix surface syntax falls out for free rather than being a goal of its
own: once `if` is a word, `cond [ T ] [ E ] if` *is* how you call it, and
`if`/`else`/`end` is grammar to be deleted.

## Recon (measured against the built compiler, 2026-08-14, `main` at `c44d552`)

Three earlier designs for this slice died on findings 1, 2 and 5. Every claim
below was produced by running the compiler, not by reading it; the two spikes
live on `probe/tailsplice` (`de47e41`) and `probe/rowgate`, with full reports in
`docs/phase4-slice10c-tailsplice-probe.md` and
`docs/phase4-slice10c-rowgate-probe.md`. Line anchors were verified at `c44d552`;
re-anchor before editing.

1. **A user-written enum eliminator is impossible, which kills the previous
   design outright.** Enum dispatch is clause-only, and a clause body cannot
   receive the branch quotations: `clause_bodied_quotation_word_error`
   (`src/check/audits.rs:340`) rejects an ordinary `[ ... ]` parameter, and a `~`
   parameter is rejected earlier still because `effect_has_variable` returns
   `true` on `TildeLBracket` (`src/parser.rs:1234`), making the word polymorphic,
   which a clause body forbids (`src/check/poly.rs:158`, `:253`). There is no
   third route: the enum word table holds only `EnumWord::Construct`
   (`src/ir/layout.rs:264`, `:480`), and `=` refuses enum operands. Full working
   in `docs/phase4-slice10c-dogfood.md`. **Consequence: `if` cannot be built on
   enum dispatch, so it must be built on a machine-level branch.**

2. **Tail position does not survive a splice into a quotation argument.**
   `body_tail_calls_self` (`src/ir/func_builder/mod.rs:99`) inspects only
   `body.last()` and recurses only into `TermKind::If`;
   `drop_graph::collect_tail_calls` (`src/check/drop_graph.rs:87`) has the same
   `If`-only shape; and the combinator splice pins `tail = false`
   (`src/ir/func_builder/calls.rs:~519`, whose comment says so). So a word whose
   recursive call sits inside a quotation handed to a combinator is never
   recognised as self-recursive and no loop shape is built. Measured: `sum-to`
   through a hand-written combinator segfaults at 1e6 while the identical word
   through the primitive `if` runs at constant stack. **This is a latent
   correctness bug today**, independent of anything else in this slice.

3. **Fixing it is small, and the wall we expected is not there.** ~72 lines
   across `src/ir/driver.rs` and `src/ir/func_builder/calls.rs`, of which two are
   `false` → `tail`. The expected obstacle (a back-edge re-entering the loop
   header while a quotation's lexical captures stay live) never arises: branch
   quotations are **phantom literals spliced inline, not materialized closures**,
   so `acc`/`n` inside `[ acc n + n 1 - sum-to ]` resolve as ordinary scalar
   locals, which in a self-tail word *are* the header phis. The emitted SSA is
   shape-identical to the primitive-`if` version. Measured at 1e8 iterations
   under a 512 KB stack limit.

4. **The tail rule discriminates rather than blanket-accepting.** A combinator
   whose body is `... t call e drop ...` (discard *after* the call) correctly
   stays recursive, because its tail term is `drop`, not `call`, so the quotation
   does not inherit tail position. That negative is as important as the positive.

5. **The row gate has two guards, and the codebase already anticipates this
   slice lifting one.** `quotation_row_not_top_level_error`
   (`src/parser.rs:794`, 10a R4) rejects a row inside a quotation effect that is
   not already declared at the signature's top level, which excludes the output
   row `..o` because it is named later. `quotation_row_shape_change_error`
   (`src/parser.rs:813`, fired at `:1374`) rejects differing rows; its own doc
   comment reads *"only 10c, for a word without a back-edge, lifts this"*. The
   rejection is independent of the condition: it fires identically with a `usize`
   condition and with no condition at all.

6. **Lifting both needs no row unification.** ~36 lines across 6 files, and the
   only checker change was to bypass one fixed-point equality test. The checker
   never has to *solve* for `..o`: a quotation-taking word is always inlined, so
   `..o` is discovered by ordinary forward checking of the spliced terms. Row
   unification would only be needed if such a word were ever lowered as an opaque
   call, which R19/R20 guarantee never happens.

7. **The interaction the rowgate probe was built to catch is sound.** Guard 5's
   justification is that a quotation carrying a back-edge must be a fixed point,
   and recon 2's fix makes a *branch* quotation carry one. Measured on both spikes
   together: `sum-to` through a fully row-polymorphic library `if` runs at 1e8
   under a 512 KB stack and emits correct carried-region phis. The shape change is
   splice-local, resolved by the combinator body's own join, and never rides the
   back-edge; the self-tailing branch never materializes its declared `..o` at
   all, because control jumps to the header.

8. **Losing the per-branch row check is a diagnostics regression, not
   unsoundness. Corrected: the probe's own report called it "unsound in
   general".** A shape-changing combinator with no structural join
   (`: applyboth ( ..i ~[ ..i -- ..o ] ~[ ..i -- ..o ] -- ..o ) | b | | a | a
   call b call ;`) is still rejected, by forward checking at the splice site
   (`` `+` needs 2 values, but the stack holds 1 ``). The declared `..o` cannot
   mislead a caller because no caller believes it; they all see the splice. What
   is lost is an *early, local* error at the argument site.

9. **Nothing can produce a machine-word condition today.** `if` requires
   `Type::BOOL` (`src/check/terms.rs:716`), so `=`, `<`, `>` all return `Bool`.
   A `usize`-taking `branch` therefore has no possible operand until this slice
   adds machine-word comparisons.

10. **A discriminant read does not exist, and is nearly free for the enums that
    need it.** Only a clause body reads a tag, internally. For an `is_scalar`
    enum (every variant payload-free, `src/ir/layout.rs:641`) the value already
    *is* its discriminant, so `tag` is a no-op cast at lowering.

## Decisions (settled here, not reopened by the spec)

1. **The compiler keeps a branch, but stops knowing a library type.** Three
   primitives: `branch` on a **32-bit unsigned flag**, `tag` reading an `is_scalar`
   enum's discriminant, and comparisons returning that same flag. `Bool` becomes an
   ordinary library enum with no special status, and `if` a library word over it.
   `branch`'s lowering is today's `lower_if`
   (`src/ir/func_builder/control_flow.rs:99`) essentially unchanged; what is deleted
   is the *grammar* and the `TermKind::If` node, not the jump-and-join.
   (The condition is a *flag*, not a machine word: at 32 bits it matches `bool`'s
   width and the width `Instr::Cmp` already produces, so no conversion is needed
   anywhere. An earlier draft of this brief said `usize`, which would have cost a
   widening after every comparison and a narrowing at every branch. See spec
   R-P3-1.)

2. **No `bool` primitive.** It would reintroduce exactly the layering violation
   this slice removes, and leave two bool-ish types to reconcile. `Bool` stays a
   library enum (slice 9).

3. **Machine-word truthiness stops at the primitive layer.** `branch` takes a
   machine word and treats nonzero as true, because that is what `jnz` does. The
   typed discipline lives in `if`, which takes a `Bool`. No user code branches on
   an integer, and C's `if (x)` looseness is not imported.

4. **`tag` is in scope and is not contamination.** A typed `Bool` that can drive
   a branch needs both directions. `Bool` → flag is a **discriminant read**, which
   no word can express, hence `tag`: reading a discriminant is a fundamental
   operation on a sum type, expressible only by the compiler because only the
   compiler knows the representation. That is categorically unlike hardcoding a
   constant. Flag → `Bool` needs **no** primitive at all: a comparison wrapper
   branches and names a nullary variant
   (`: = inline ( 'T: Copy Ord 'T -- bool ) u= [ true ] [ false ] branch ;`, `'T: Copy
   Ord`-polymorphic over the numeric tower per spec R-P3-3). An earlier
   draft of this brief said that direction was 'ordinary enum construction and
   already exists', which was right in substance and wrong in mechanism — there is
   no value→enum reinterpret, and there must not be one, because it would be a
   partial operation able to manufacture an out-of-range discriminant. Spec OQ5
   has the worked form.

4a. **Slice 11's `inline` is a hard dependency, not just a shipped predecessor.**
   `if`/`unless` take quotation parameters and are therefore combinators, always
   spliced. The library comparison words are **not** combinators, so without an
   explicit `inline` they mint an `IrFunc` and every comparison in the language
   becomes a real call with a frame (measured). With `inline`, the emitted machine
   code is byte-identical to today's builtin.

1. **Enum eliminators, `match`, and tagged branch literals (`Variant[ ... ]`)
   are abandoned, not deferred.** Recon 1 shows the user-written form is
   impossible and the generated form is a much larger feature. Payload-carrying
   dispatch stays with clause bodies, which are irreducibly a *typing* construct
   (they tie "we are in this arm" to "the payload has this type") and cannot be
   derived from any control primitive. The previous 10c spec
   (`docs/phase4-slice10c-spec.md`, `2825c10`) is superseded wholesale, not
   patched: every requirement in it describes this abandoned mechanism.

2. **The parser relaxation must be narrower than the spike's.** The spike interns
   any fresh row name. The real rule admits only rows that are the signature's
   *own top-level* rows, which means deferring the R4 check until the whole
   signature is parsed and validating then. A genuinely fresh name stays an error.

3. **The per-branch check becomes a shape check, not a bypass.** Compare the
   region a branch actually leaves against the region the declaration and its
   sibling expect. This is a comparison, not unification (recon 6), and it exists
   to restore the early diagnostic recon 8 measured as lost, not to close a
   soundness hole.

4. **The tail-splice rule lives in the shared predicates.** The spike put it only
   in the lowering driver, so check and lowering now disagree about whether a
   splice is a loop, benign only because Phase 2 values are all `Copy`.
   `src/ir/func_builder/mod.rs:95-98` warns explicitly that these must agree or
   lowering panics on a missing header. There are **five** consumers, not the two
   the spike touched, and the spec must move all of them together:
   `func_builder::body_tail_calls_self` (`src/ir/func_builder/mod.rs:99`) and
   `drop_graph::collect_tail_calls` (`src/check/drop_graph.rs:87`), the two
   syntactic passes; and the three `has_self_tail_call` call sites, the per-word
   build gate (`src/ir/driver.rs:183`), the REPL path (`src/ir/driver.rs:643`),
   and the destructor path (`src/ir/destructors.rs:372`). The fifth is the one
   that matters most and the probe reports missed it:
   `src/check/combinators.rs:343` computes `splice_tail`, the **checker's** twin
   of the lowering `tail` flag, and is the exact site at which check and lowering
   currently disagree.

5. **Three phases, and the third is atomic by necessity.** P1 tail-splice, P2 row
   gate, P3 primitives plus library `if` plus keyword deletion plus corpus
   migration. P3 cannot be split at the `if` swap: the word and the keyword cannot
   coexist, and a transitional shim is exactly the backwards-compatibility hack
   the project bans. P1 and P2 each stand alone with their own visible value (P1
   fixes recon 2's latent segfault; P2 makes shape-changing quotations writable at
   all), so neither is plumbing staged ahead of its call site.

## Open questions for the spec

- **The tail-spliced parameter analysis, properly.** The spike is one level deep
  and positional: the caller's *last* term must be a combinator call, and the
  quotations must be a contiguous trailing run of literals. The real version
  computes, per combinator, which declared quotation parameters are `call`ed in
  tail position, then matches that set against the argument slots the caller
  actually passed, reusing the operand resolution `check_term`/`inline_combinator`
  already has. The spec must say what happens for: nested combinators, a
  combinator call that is the tail of an `if` arm in the caller, a quotation
  argument that is forwarded rather than a literal, and a name that resolves to
  more than one combinator.
- **Linear values across a spliced back-edge.** Both spikes only exercised `Copy`
  scalars, so no back-edge drop glue and no linear obligation was tested. The
  existing guards (`check_linear_value_across_self_tail_call_is_error`,
  `check_linear_value_forwarded_into_the_self_tail_call_is_ok`,
  `src/check.rs:~2491`) must extend to the spliced case, and the spec should say
  whether a linear value live across a spliced back-edge is rejected or given
  drop glue.
- **Where the inline-always invariant is written down.** Recon 6 and 8 both rest
  on "a quotation-taking word is always spliced and mints no `IrFunc`". That is
  load-bearing for soundness and currently only implied by R19/R20. Slice 7b
  (first-class runtime quotations) is precisely where it would break, so it
  should be stated as an invariant with a named owner rather than rediscovered.
- **`tag`'s domain.** `is_scalar` enums only (where it is a no-op cast), or
  payload-carrying enums too (where it is a real field read)? Only the former is
  needed by this slice.
- **Whether the `Bool`-returning comparisons survive as library words** wrapping
  the machine-word primitives, or whether the primitives are the only
  comparisons and `Bool` is constructed explicitly. The corpus reads much better
  under the first; the second is smaller.
- **Which library words move in P3.** `if` and `unless` must. Whether `while` and
  `cond` move in this slice or stay where they are is a scoping call, given
  `lib/combinators.sth` already defines `while` in terms of `if`.
- **DESIGN.md's amendment.** `:456` and `:487` name clause-bodied definition as
  the sole enum eliminator, which decision 5 keeps true. `:469` ("shrinks the core
  the honest way, by making `if` a word rather than by replacing it with a bigger
  feature") describes this slice and needs no change. What needs adding is the
  three primitives and the machine/library layer split. Current state only, no
  history of how the design got here.

## Out of scope

- Enum eliminators of any spelling, `match`, tagged branch literals, and the
  `Ident[` lexing question they raised. Decision 5.
- Payload-carrying enum dispatch, which stays with clause bodies. Decision 5.
- Dispatch-after-locals (`lib/binary_search.sth`'s sketch binds `| b idx |` then
  dispatches on `Ordering`, which a clause body cannot express). Unrelated
  capability, unaffected by this slice, still unbuildable after it.
- Early return. No `TermKind` for it exists and none is added; the two-way branch
  plus a join is sufficient, and early return would be a general control-flow
  feature with reach far beyond this slice.
- First-class runtime quotations and closures: 7b.
- Declared combinator recognition: 12, which follows this slice and must mark
  whatever `if`/`cond` become.

## Sequencing

Gates on 10a (`~` and rows inside a quotation effect), 10b and 11, all shipped on
`main`. Nothing new gates it. Touches `src/parser.rs` (the row guards, then the
`if` grammar), `src/check/drop_graph.rs` and `src/ir/func_builder/` (the shared
tail predicate and the splice `tail` threading), `src/check.rs` and
`src/check/combinators.rs` (the branch shape check), `src/ir/layout.rs` (`tag`),
and `lib/`.

Slice 12 follows and depends on the outcome. The two spikes are on
`probe/tailsplice` and `probe/rowgate` and are throwaway: they are evidence, not
a starting point, and P1/P2 should be implemented properly rather than by
promoting the hacks (recon 3 and 6 give the line counts a real implementation
should be measured against, not the patches themselves).

## Exit

- **P1**: a user-written combinator whose quotation argument self-tails lowers to
  a back-edge, asserted on lowered IR (a `jmp` back-edge and no self `Instr::Call`)
  and measured at 1e8 iterations under `ulimit -s 512`. The discard-after-call
  shape (recon 4) stays a real recursive call, as a negative golden. Check and
  lowering agree via one shared predicate, with a test that fails if they are
  allowed to diverge.
- **P2**: `~[ ..i -- ..o ]` parses when both rows are the signature's own
  top-level rows; a fresh name and a row named only later are still located
  errors (the four existing parser tests are *retargeted* to the new boundary, not
  deleted). A shape-changing quotation checks, runs, and prints correct results
  with a non-empty carried region below it. A branch whose actual output
  contradicts the declaration is rejected at the argument site, not merely at the
  splice site (decision 7).
- **P3**: `if` is defined in `lib/`, `TermKind::If` and the `if`/`else`/`end`
  grammar are deleted, and `grep -r "TermKind::If" src/` is empty. `nm` shows no
  symbol for `if` (it is a combinator). `gcd`, `countdown`/`sum-to`, and
  `filter_while` still run in constant stack, asserted on IR, not inferred from
  output. Corpus output is byte-identical across the migration.
- The whole-slice witness: a program using `if`, `unless` and `while` entirely
  from `lib/`, self-tailing through a branch, producing the same output and the
  same loop shape as its pre-slice equivalent.
- Every new test is mutation-tested: reverting the change it guards must fail it.
  This slice has three findings (recon 4, 7, 8) that a placebo test would have
  reported as passes.
