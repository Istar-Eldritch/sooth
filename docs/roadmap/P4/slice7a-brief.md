# Phase 4 Slice 7a: quotations as runtime values (brief)

A quotation has no runtime representation. It is a compile-time marker: the literal mints a
phantom `Value` with a placeholder `IrType` and *no* `Instr` (`src/ir.rs:2687`), its body is
stashed by `QuotId`, and every consumer splices that body textually. So a quotation cannot be
stored, returned, put in an array, or left on a branch join, and each of those is a located
rejection naming this slice. Verified by compiling, four of them verbatim:

```
type: Holder q [ i64 -- i64 ] ;
=> a quotation type `[ i64 -- i64 ]` cannot appear as the field `q` of struct `Holder`

: mk ( -- [ i64 -- i64 ] ) [ 1 + ] ;
=> a quotation type `[ i64 -- i64 ]` cannot appear as the output of `mk`

[ 1 + ] 4 fill
=> a quotation cannot be stored (escaping quotations are slice 7)

true if [ 1 + ] else [ 2 + ] end
=> a quotation's body must be known where it is used, but these two branches leave
   different quotations
```

The slice gives a quotation a `(code, env)` runtime value so those four become legal, and
`call` on one that the compiler cannot resolve to a literal becomes an indirect call rather
than an error.

## Recon: measured against the built compiler and read against the current checker

**1. The rejection inventory is closed and small.** Eleven sites name slice 7:
`check.rs:1239`/`:1259` (R7a, a quotation type in any position but a direct word parameter),
`:1251` (a quotation-taking word with a clause body), `:2481` (a quotation left on a REPL
line's stack), `:5108`/`:5112` and `:5743`/`:5747` (`call`/`times` given a non-quotation),
`:7013` (a quotation as an operator operand), `:7026` (stored), `:7041` (passed to a word
other than `call`/`times`), `:7053`/`:7064` (the two branch-join cases), plus the
`unreachable!` in `ir_type_of` (`src/ir.rs:189`). This slice lifts the storage, output,
field/element, join, and indirect-`call` cases; it does *not* lift the clause-body one
(`:1251`), which is about splicing a callee, not about representing a value.

**2. The seam is already cut.** `Type::Quotation(&'static QuotEffect)` and
`PolyType::Quotation` exist with unification and `apply_subst` following (6a). What is missing
is downstream: no `IrType` variant, no layout, no backend. `ir_type_of`'s arm is an
`unreachable!` whose comment already names the replacement ("slice 7 lifts this with a
`(code, env)` runtime value"), so the change is additive at a known point rather than a
refactor.

**3. The env struct has a working precedent.** `intern_bundle_struct` (`src/ast.rs:433`,
slice 1) synthesizes a positional `__ret_N` struct for a word's multi-output tuple and dedups
it structurally, interned into `Module::structs` before the layout pass. An env struct is the
same construction with a different dedup key (the capture tuple), and inherits layout,
destructor synthesis, and backend aggregate emission unchanged.

**4. The allocator for escaping closures exists.** `emit_alloc_shim` (`src/backend/qbe.rs`)
is `malloc` plus an OOM trap, with `^T`'s full Phase 3 disposal story already wired to it, so
an upward closure's `^Env` needs no new runtime.

**5. Nothing in the compiler has ever seen an unresolved quotation reach lowering.** The
checker *does* accept an abstract quotation at `call`/`times` (`check_abstract_quotation_call`,
`src/check.rs:6096`), but only while checking a quotation-taking word's own definition in
isolation. Every real call site inlines a `Known` literal instead (D2, 6a), so the abstract
path never reaches `ir.rs`. The indirect-call path is genuinely new code, not a relaxed guard.

**6. Provenance is already tracked, and is already the right bit.** `Slot.quot:
Option<QuotRef>` is `Some(Known(id))` exactly when the checker can name the literal, and
`QuotRef` is a one-variant enum (`src/check.rs:89`) — the shape it was left in for a second
variant to arrive here. The join rejection compares ids, not shapes: the *same* quotation in
both arms already compiles (verified: `[ 1 + ] | q | true if q else q end 5 swap call` prints
`6`), so `:7053` fires only on genuinely differing identity.

**7. There is no capture-set analysis anywhere.** The D3 check
(`check_literal_against_declared_effect`, `src/check.rs:5599`) only *rejects*: it flags a
literal that consumes a linear enclosing local, or that leaves a borrow of an enclosing place
on its exit row. It never computes which names a body reads. Materialization needs that set,
including through nested quotations and through the alpha-renaming at `src/ast.rs:1054`. This
is new machinery the roadmap's "synthesize an env struct per quotation literal" understates.

**8. Two capture regimes exist today, and only one is restricted.** A literal checked against
a *declared parameter* gets D3. A literal spliced at a direct `call` gets nothing — "capture
is free, recon 9" (`src/check.rs:6106`). Any capture rule this slice introduces has to say
which regime it extends.

**9. The headline: splice and materialize are not the same semantics, and the difference is a
silent wrong value.** Splicing is textual, so a captured aggregate is re-read *at the call
site*. A materialized env snapshots *at the literal*. Measured:

```
0 4 fill | arr |
&!arr 0 >usize &!> 7 !
[ &arr 0 >usize &> @ ] | q |
&!arr 0 >usize &!> 99 !
q call .
=> prints 99 today (late read). A snapshot env would print 7.
```

This is unobservable for scalars — a scalar local cannot be borrowed at all ("a scalar has no
address; borrow a field or an aggregate instead") — and unobservable for a capture never
mutated between literal and call. It is fully observable for the aggregates that are actually
captured in practice.

**10. The shipped combinator library depends on the late read.** `map`
(`lib/combinators.sth`) writes `arr` through `&!` on every iteration of its `times` body and
reads it back on the next:

```
count [ | i | &arr i >usize &> @ f call | v | &!arr i >usize &!> v ! ] times
```

Under snapshot-at-literal each iteration would read a stale copy and write into a discarded
one. A minimal replica of the pattern (a `times` body accumulating a running sum through the
array it captures) prints `6` today and would print `3` under snapshot semantics. So "make
splicing snapshot too, and the two agree" is **off the table**: it silently breaks a shipped
library word.

**11. Shadowing cannot confuse the two.** Re-binding a live name is already rejected outright
("a name may not be re-bound while it is in scope"), so a literal and its call site cannot
disagree about which binding a name denotes. The divergence in recon 9 is purely *when* the
value is read, never *which* name is read.

## Decisions

- **D1. Provenance decides, never a size heuristic or a budget.** `call`/`times` on a `Known`
  quotation splices, exactly as today; `call`/`times` on one whose identity has erased emits
  an indirect call. The fast/slow boundary is then a source-visible property — you pay an
  indirect call exactly where you wrote data-driven dispatch — which is DESIGN.md's
  "never charge a semantic price for a performance property" pointed at codegen. A budget was
  already called "actively harmful" pre-7a (ROADMAP) and nothing here improves that argument.

- **D2. Quotation-taking words stay force-inlined; 6a's D2 survives intact.** `each` still
  mints no `IrFunc` and every call site splices it. The unknown-quotation case then composes
  for free: `table @ each` splices `each`'s loop skeleton as always, its abstract parameter
  binds a runtime value instead of a literal, and the `call` inside the spliced body sees
  erased provenance and goes indirect. Inlined loop, one indirect call per element, no
  `IrFunc`-for-`each` variant, no new rejection.

- **D3. 7a materializes only quotations that capture nothing.** Recon 9/10 make snapshot
  semantics wrong for a capturing quotation and recon 10 makes changing splice semantics
  wrong for the library. Preserving today's meaning under materialization requires the env to
  hold a *reference* to the captured aggregate, which is 7b's subject and is exactly why 7b
  needs 6f's settled liveness rule. So the 7a/7b line is **no captures / captures**, not
  "non-reference captures / reference captures" as the ROADMAP entry currently says; that
  entry is corrected by this brief. A capturing literal keeps working exactly as today
  wherever it is spliced, and a *capturing* literal reaching a materialization boundary is a
  located rejection naming 7b (D4).

- **D4. The materialization boundary is a checked event with its own diagnostic.** Identity
  erases at: a store into a struct field or array element, a word output, a branch join with
  differing ids, and capture into another quotation. At that boundary a non-capturing literal
  mints its `IrFunc` once and becomes a `(code, env)` value; a capturing one is rejected
  naming 7b, reusing `:7026`'s wording shape rather than inventing a second vocabulary.

- **D5. One uniform representation, with an unused env in 7a.** `(code, env)` as the roadmap
  specifies, env always empty this slice. Building the pair now rather than a bare code
  pointer is what keeps 7b additive (it fills the env in) instead of a representation change
  in a later slice.

- **D6. `times` with an erased quotation is allowed, not rejected.** An indirect call per
  iteration is still constant stack, and `times` is the primitive everything else splices
  onto, so it needs its own golden rather than inheriting `call`'s.

## Open questions for the spec

- **Q1. Where does the capture-set analysis live, given recon 7 says it does not exist?** D3
  needs only a *predicate* ("does this body read any enclosing name?"), not a full set, which
  is strictly less work than 7b will need. Decide whether to build the cheap predicate now
  and the set in 7b, or build the set now and use it for the predicate. The predicate must
  see through nested quotations and through `rename_terms` (`src/ast.rs:1054`).

- **Q2. What is a quotation value's `IrType` and layout?** A two-field aggregate is the
  obvious answer and reuses struct machinery wholesale, but it makes every quotation an
  aggregate (pointer-to-storage at runtime, `:S` in ABI positions), which is heavier than the
  bare code pointer a non-capturing 7a quotation actually needs. Decide whether 7a's empty env
  is a real zero-field struct field or elided, and whether eliding it now costs a
  representation change when 7b arrives — D5 says do not elide, but the spec should price it.

- **Q3. Does the backend have indirect calls at all?** The IR's `Instr::Call` is
  symbol-keyed. QBE spells an indirect call `call %fnptr(...)`, so this is a new `Instr`
  variant plus a backend arm, not a QBE limitation. Confirm no ABI wrinkle for aggregate
  arguments through an indirect callee.

- **Q4. Is a materialized quotation `Copy` or linear?** With an empty env in 7a it is a bare
  code pointer and `Copy` is the honest answer. But 7b's `^Env` closure is linear (single
  owner), so the same surface type would change linearity between slices. Decide whether to
  make it linear from the start (conservative, costs a `drop` on every dispatch-table entry)
  or `Copy` in 7a and split the type in 7b.

- **Q5. The dogfood's dispatch table needs one uniform effect, but `Op`'s variants carry
  different payloads** (`Push`/`Load`/`Store`/`Jz`/`Jmp` carry `i64`/`usize`, `Add`/`Sub`/
  `Mul`/`Halt` carry none). The workable shape is one effect `[ Vm Op -- Vm ]` where each
  entry extracts its own known variant's payload, safe because the table already routed by
  tag. Confirm at the spec that this does not just reintroduce the clause match inside every
  entry, which would make the dogfood prove nothing.

## Out of scope

- **Capturing closures**, both downward and upward (7b, gated on 6f). Recon 9/10 are the
  argument; D3 is the rule.
- **Upward/escaping closures and `^Env`.** With no captures there is nothing to escape *with*:
  a non-capturing quotation is a bare code pointer with no lifetime story at all. The `^Env`
  machinery lands with the captures that need it.
- **Inline budgets, `inline`/`noinline` annotations, and sinking a `call` into branch arms to
  avoid materializing at a join.** All three are optimizations against a semantics that is not
  settled until 7b.
- **The clause-body rejection** (`check.rs:1251`). It is about splicing a callee, not
  representing a value.
- **Any change to what splicing means.** Recon 10 makes that a library-breaking change; the
  splice path must come out of this slice bit-identical, which the existing 6a-6f goldens
  already assert.

## Exit

A quotation stored in a struct field and in an array, returned from a word, and left by two
differing branches of an `if`, all compile; `call` on each of those emits an indirect call and
runs correctly; `times` driving an erased quotation runs in constant stack; every existing
6a-6f golden still lowers to the same spliced tight loop with no per-element `Instr::Call`;
and a capturing literal at a materialization boundary is a located error naming 7b. The
dogfood is `examples/vm.sth` rewritten around a table of quotations, compared against the
enum-plus-clause version it replaces.
