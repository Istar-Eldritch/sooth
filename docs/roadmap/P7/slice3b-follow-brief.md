# Phase 7 Slice 3b-follow: row-typed quotation consumers in a polymorphic body (brief)

The four row-typed quotation consumers (`if`, `branch`, `times`, `tag`) are a located
rejection in a **non-inline** polymorphic body (`src/check/poly.rs:842-851`), naming this
slice in the message. P7.S3b shipped the eliminator consumer; P7.S3d ships the *rowless*
concrete consumer (a comparator). This slice is the remaining tier: a consumer whose
declared quotation parameter carries a row (`~[ ..a -- ..b ]`) and must be matched against
the poly walk's abstract stack.

Probe-verified at HEAD:

```sooth
: mymax ( 'T: Copy Ord 'T -- 'T ) over over gt ~[ drop ] ~[ swap drop ] if ;
```

```text
error: `if` on a quotation in the polymorphic body of `mymax` (line 1) is not yet supported
  only an enum eliminator consumes a quotation in a generic body today (P7.S3b-follow)
```

The same word with `inline` compiles today (`examples/poly_if.sth`), because an `inline`
poly word is spliced into a concrete caller and the row grounds against a real stack. So
the gap is specifically: **a generic word that wants to branch or loop without forcing
every call site to inline it.**

## Recon (verified against the source at HEAD, not inferred)

Three findings, and together they argue this slice is smaller than "the expensive tier"
implied when P7.S3d deferred it.

1. **The representation already exists.** `PolyType::Quotation(ins, outs, is_inline,
   row_in, row_out)` has carried both row fields since slice 10a (`poly.rs:3319`). They are
   rendered by `poly_type_str` and constructed in tests, and nothing ever unifies them.
   This slice adds unification, not representation.

2. **The arm-walk and N-arm join already exist, built for the eliminator.**
   `poly_eliminator_call` (`poly.rs:1139-1175+`) computes `let row: Vec<PolySlot> =
   stack[..base].to_vec()`, clones the enclosing scope per arm, recursively `poly_walk`s
   each arm body over `(row ++ narrowed variant)`, and joins the exits with a borrow-table
   union. That is structurally the whole shape this slice needs. Its own comment states
   the one difference precisely: for an eliminator "there is no declared `~[ ..a -- ..b ]`
   effect to match an arm against -- an arm is annotated by *variant*, and its input is the
   concrete narrowed variant this dispatch computes." Replace "narrowed variant" with
   "ground the declared row against `stack[..base]`" and the rest is the same machinery.

3. **The concrete path is a direct model to port.** `check/combinators.rs:671-680` grounds
   a row-bearing declared quotation parameter to `stack[..base]` — the same region the
   top-level row grounds to — and derives `shape_changing` from `row_in != row_out`. A
   shape-changing row gets no fixed exit-row check; instead the arms are cross-checked
   against a `shape_baseline` keyed by the output row id (`:713-730`), which is what makes
   `if`'s two arms agree on `..b` without a fixed expectation. The poly analogue is the
   same two-case split over `PolySlot` instead of `Slot`.

## The consumer

**Two candidate consumers were tried and both failed to motivate the slice. Recording that,
because it is the honest state and a spec should not re-argue them.**

1. *A recursive generic word has no inline escape.* False. `lib/arrays.sth`'s `bin_search`
   was rewritten (this session) from a fixed-32-halving `times` loop to a self-*tail*
   recursive `bin_search-helper`, still `inline`, still generic. It lowers to a loop
   back-edge exactly as `times-helper` does — verified in the disassembly: two backward
   jumps, one per `if` arm, both targeting the loop head, in a 98-instruction
   `sooth_main` with no 32-fold splice. Tail recursion is not blocked.
2. *`lib/binary_search.sth`'s sketch needs it.* Withdrawn. That untracked file is a
   "hypothetical grammar" sketch that is strictly worse than what `lib/arrays.sth` already
   ships, and its `if` usage is inside a word that could be `inline`.

**What is actually left, and it is a real cost rather than a capability gap:** every caller
of a generic row-consuming word splices its entire body. `sort`'s merge sort is spliced in
full at each call site, per instantiation. A non-inline generic word would be monomorphized
once per type instead. That is a code-size argument, not an "unwritable program" argument,
and it is weaker than the consumer stories that justified P7.S3b or P7.S3c.

The secondary consequence is a capability one, and it may be the stronger half: a non-inline
generic word cannot take a `~[ ]` parameter at all (probe-verified: "declares an
inline-quotation parameter `~[ 'T -- 'T ]` but is not `inline`"), so *every* generic word
wanting a quotation argument is forced to be spliced. This slice does not by itself lift
that, but nothing can lift it while row-typed consumers are rejected in a non-inline body.

**A sequencing consequence follows: this slice should be scheduled on the code-size
argument or not at all, and the spec should say which.** If the answer is "not yet", the
right outcome is to keep the located rejection and re-point its message at a later slice,
not to build the machinery for a consumer that does not exist.

## Shape of the work

Ground a declared row against the abstract stack, in the one place the family dispatches
(`poly.rs:842-851`, currently the rejection), reusing `poly_eliminator_call`'s arm-walk and
join rather than growing a second copy of it. Two cases, mirroring the concrete path:
a non-shape-changing row (`row_in == row_out`, e.g. `times`'s body) checks each arm's exit
against the entry row; a shape-changing one (`if`'s `..a -- ..b`) checks the arms against
each other via a poly `shape_baseline`.

## Locked decisions

**Type variables stay rigid; no mid-body `Subst`.** Carried forward from P7.S3b's locked
decision, unchanged and for the same reason: nothing in a polymorphic body binds a type
variable, and admitting arm A `Var(0)` against arm B `Concrete(i64)` would mean a genuinely
new mid-body unifier with ripples into mangling. An arm disagreement is a located error.

**Row *variables* stay rigid too, and this is the row-level analogue of the rule above.**
`..a` grounds once, to `stack[..base]` at the dispatch site, and is not solved for. Nothing
in this slice infers a row.

**The arm merge unions the borrow table.** Non-negotiable, and for the reason P7.S3b
recorded: `PolyScope`'s borrow table is name-keyed and a *missing* record reads as "no
conflict", so a merge that intersects or picks one arm produces a **false accept**, not a
false reject. `poly_eliminator_call` already unions; this slice must not introduce a second
join that doesn't.

**Splice-consumed quotations only.** Also carried forward. The arms here are `~[ ]`
inline-only parameters by declaration (`lib/core.sth:42`), so an ordinary `[ ]` arm is the
wrong bracket and must produce the same diagnostic the eliminator path already emits
(`ordinary_literal_at_inline_param_error`), not a new one.

## Open questions

1. **Is `poly_eliminator_call`'s arm-walk extractable, or does sharing it distort it?**
   The recon argues the machinery is common. If factoring the arm-walk/join out of the
   eliminator path makes that path worse to read, the spec should say so and duplicate
   deliberately. This is the main sizing question.
2. **Do all four consumers land together, or does `tag` separate?** `if`/`branch` are
   two-arm and shape-changing; `times` is single-arm and non-shape-changing; `tag` is the
   one I have not traced. Any consumer left out needs to keep a located rejection rather
   than fall through to `unknown word`.
3. **What happens to `poly.rs`'s deferred split?** P7.S3d was already named as the trigger
   to re-run `poly.rs`'s split signals (3/5 fired, both available splits judged wrong). This
   slice adds a second row-consuming path to the same file. Re-run the signals at exit, per
   CLAUDE.md, rather than deciding now.
4. **Does an erased (non-literal) quotation reach this path?** A row-typed consumer driving
   a quotation that came from a word return, rather than a literal, is one of the three
   pre-existing ICE shapes (`while` over an erased quotation panics in
   `ir/func_builder/control_flow.rs`). Confirm whether the poly path can reach it, and if
   so reject it located rather than inheriting the panic.

## Out of scope

- The rowless concrete consumer (P7.S3d) — this slice assumes it, and should not re-do it.
- Slices (P7.S3c) and trait bounds (P7.S3e). Recon found no interaction: the row machinery
  is orthogonal to both. Verify once, do not design around them.
- The three pre-existing row-combinator ICEs, except as open question 4 requires a located
  rejection rather than a fix.
- A generic word calling another generic word (`unknown word g__m0`), still a standing limit
  from P7.S3b, which bounds what can be written against this slice.
- Materialised/escaping quotations in a poly body.

## The golden

A generic word whose body consumes a row-typed combinator while **not** being spliced. It
cannot be today's comparator-taking `bin_search` with `inline` removed (a non-inline word
cannot declare a `~[ ]` parameter), so the fixture takes its ordering from the `Ord` bound
instead of a comparator quotation:

```sooth
: bin_search ( ['T: Copy Ord 'N] 'T -- ['T 'N] usize bool )
```

with a non-inline recursive helper whose body uses `if`. **A test fixture, not a library
word**: an `Ord`-bounded search is strictly less capable than the comparator version
`lib/arrays.sth` already ships (`is_ord` is `is_numeric` and nothing else, so it cannot
reach a user struct), and two `bin_search`es in `lib/` would be a downgrade shipped beside
the real one. Revisit only if P7.S3e makes the bound mean something.

Two halves, because "it compiles and runs" does not test the claim:

- **Behavioural.** Search results over a duplicate-bearing array: leftmost index for a
   duplicate, both miss directions (below all, above all) reporting the insertion point,
   exact hits at first and last index, and a single-element array. This exact matrix was run
   against the recursive `inline` rewrite and is known to discriminate.
- **Structural.** That the word is monomorphized once per instantiation rather than spliced
   per call site. **`nm` for the minted symbol is a known placebo here**: `poly_indices`
   already excludes poly words from symbol minting, so the check can pass for reasons
   unrelated to this slice. Whatever discriminator the spec picks must be mutation-tested —
   delete the thing it guards and watch it fail — before it counts.

## Ready to spec?

**Technically yes, and cheaper than advertised. But the consumer is weak, and that is the
decision to make before speccing, not during.**

Sizing revises **down**: P7.S3d's text calls this "the expensive tier ... machinery this
slice does not add", written before anyone checked what P7.S3b's eliminator dispatch had
already built. The representation exists, the arm-walk and union-join exist, and the
concrete path has a two-case model to port. That reads **M**, not XL — open question 1 is
the only thing that could move it back up.

But the consumer story is the weakest of any P7 slice so far. Two candidate motivating
programs were tried and both turned out writable today with `inline` (see The consumer).
What is left is code size, plus the downstream fact that no generic word can take a
quotation parameter without being spliced. Compare P7.S3b, which had a compiling witness
that *no polymorphic word could eliminate an enum at all*. This has no such witness.

So the ruling this brief asks for is a scheduling one: **does the code-size argument justify
an M-sized slice now?** If not, the honest action is to keep the located rejection, re-point
its message, and spend the M elsewhere in P7 — not to build the machinery and find the
dogfood afterwards.

No dependency on P7.S3c, P7.S3e, or Phase 8 (probe- and source-verified). Depends on P7.S3b
(`[ done ]`) and assumes P7.S3d.
