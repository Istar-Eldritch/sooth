# Phase 4 Slice 6a — Quotation types, the inliner, and `each`/`map`/`fold` (brief)

Slice 4 shipped the quotation literal as a **compile-time-only marker** plus one
intrinsic, `times`, and deliberately deferred any runtime quotation value. Slice 5a gave
a library somewhere to live. This slice makes a combinator an **ordinary Sooth library
word**: a quotation becomes nameable in a signature, a word may take one as a parameter,
and every call to such a word is inlined at the call site. Native only — the REPL is 6c,
pinned here to a located rejection rather than left to whatever the session happens to do.

The phase-scale claim being tested is the one DESIGN.md makes: "combinators are library
words, not keywords". If `each` cannot be written in Sooth, the phase's headline exit is
compiler-known match arms wearing a library's clothes.

## Recon: measured against the built compiler, not read off ROADMAP

**1. The combinator library is not "writable but slow" today — it is not expressible at
all.** Two independent walls, both compiled rather than reasoned about:

- A quotation type cannot be spelled in a signature. `: apply ( i64 [ i64 -- i64 ] -- i64 )`
  dies in the *parser*: `array count must be a decimal literal, found '--'`. `[ ... ]` is
  already the array-type syntax.
- Passing a quotation literal to any user word, monomorphic or polymorphic, is a hard
  located rejection: `a quotation cannot be passed to 'twice'; only 'call' and 'times'
  accept one`.

So the inliner in this slice is the **enabling mechanism**, not an optimization laid over
a working library, and there is no un-inlined baseline to regress against.

**2. What the inliner must *produce* is fully expressible today.** Hand-inlined `each`,
`fold`, and `map` over a `[i64 4]` — a `times` loop, `>usize` on the index, `&a i &> @`
reads, `&!b i &!> v !` writes — compile and run correctly (`7 7 7 7`, `28`, `8`). There is
no IR, lowering, or loop-primitive gap. **The whole gap is front-end.**

**3. The polymorphism the library needs already works.** `: size ( ['T 'N] -- ['T 'N]
usize ) len ;` compiles once and runs at both `[i64 4]` and `[f64 7]`. Slice 1 delivered
the element and length variables `each`/`map`/`fold` need.

**4. Fusion today is check-time term splicing, not an IR pass.** `call` and `times`
(`src/check.rs:4763,4797`) clone the literal's AST body and re-run `check_terms` against
the live stack, bracketed like an `if` arm. `times` additionally **inspects the spliced
body** to prove three obligations: identity on the move state (no outer linear local
consumed, or it would be disposed N times), identity on the borrow state (`live_derivs`
before vs after, so no reference crosses the back-edge), and row-effect equality (D6 of
slice 4). All three are computed by *walking a body the checker can see*.

**5. No inlining exists, and a quotation-taking word can never be lowered standalone.**
Every user `:` word call emits a real `Instr::Call` (`src/ir.rs:2860`); nothing downstream
inlines either (QBE is per-function, `cc` runs with no `-O`). But the sharper point:
`each`'s body contains `f times` where `f` has no literal and no runtime representation, so
**no `IrFunc` for `each` can exist at all**. There is nothing at the IR level to inline.
This forces the attachment point rather than leaving it a preference (D2).

**6. `Type`, `PolyType`, and `IrType` have no quotation variant** (`src/ast.rs:670`,
`src/ir.rs:76`). Slice 4 kept the marker on a `Slot`/`Binding` side-channel (`quot`,
`QuotRef::Known`), which is exactly why nothing downstream had to change.

**7. The checker already checks every call against a signature alone.** `check_term`'s
user-word path takes `env: &HashMap<String, Sig>` and has never seen a callee's body at a
call site. So "check `f call` against a declared effect" is the discipline that already
governs every call in the language, not a new mechanism invented for this slice.

**8. Two graph-walk precedents exist for the recursion rejection D5 needs**:
`check_tail_call_cycles` (3-colour DFS over the tail-call graph, `src/check.rs:2434`) and
`drop_reachability_graph`/`reaches_start` (`src/check.rs:2587,2746`). This would be the
third instance of that shape; reuse it rather than invent a fourth.

**9. Nine shipped diagnostics misname the milestone.** `src/check.rs:2163`, `4395`-`4414`,
`5621`, `5647`, `5659`, `5670` all say higher-order values/user words are "Phase 6", which
reads as the stdlib phase. Under current numbering it is 6a (parameters) and 7 (runtime
values), and they become outright false the moment this slice ships.

## Decided (locked, one at a time)

**D1. The spelling stays `[ ... -- ... ]`, disambiguated on a top-depth `--`.** An array
type can never contain `--`; a quotation effect always does, including the nil effect
`[ -- ]`. The type parser scans to the closing `]` for a top-depth `--`: present means a
quotation effect, absent means an array. No new sigil, and the type mirrors the literal it
types. The malformed-array diagnostic must stay sharp (`[i64]` still says the count must be
a literal, not something quotation-flavoured). Arrays cannot hold quotations (slice 4), so
nesting stays unambiguous.

**D2. The inliner attaches at check time, by term splicing — forced by recon 5, not
chosen.** Retain the callee's AST body, substitute the caller's literal for the quotation
parameter, splice at the call site, and let the existing `call`/`times` fusion fire on the
substituted body. This generalizes machinery that already exists rather than adding a
second mechanism. A post-lowering IR pass is not a rejected alternative, it is an
impossible one: there is no `IrFunc` to inline.

**D3. A quotation literal passed to a user word may capture only `Copy` locals, by value;
no borrows of enclosing places.** This is what makes a declared effect a *complete* summary
of a literal, which is what D4 rests on. It matches the paper pre-check's finding (the
library needs no capture at all; the capture anyone wants is read-only ambient context,
i.e. `Copy`) and slice 4's own note that capture is free precisely when the quotation is
inlined, with no `Fn`/`FnMut`/`FnOnce` split if capture is `Copy`-restricted.

**D4. Standalone checking of a combinator body is compositional.** At `each`'s own
definition site, `f call` and `f times` check against `f`'s declared effect exactly as an
ordinary word call checks against its `Sig` (recon 7). The body is genuinely checkable
standalone — the signature is not documentation over a macro.

**D5. Inlining is total, not best-effort.** With a quotation type but no runtime value
there is no fallback, so every call to a quotation-taking word must inline, transitively
(`map` written over `each` inlines twice). Anything un-inlinable is a **located error**,
never a silent real call. Recursion among quotation-taking words is the first such case.

**D6. The new type variant must not bake in "a quotation is always statically known."**
Keep that a predicate on the *value*, not a property of the type, or slice 7 has to unpick
the assumption out of unification and the monomorphization walk to allow a runtime closure.

**D7. Native only. The REPL is a located rejection at every chokepoint, specified and
tested.** This is the 5a lesson, which slice 2's recon proved the hard way: slice 1 left
REPL polymorphic words unpinned and the gap turned out to be a silent miscompile, not a
clean deferral. 6c lifts the rejection.

**D8. The library lives at `lib/combinators.sth`, imported by relative path** like every
other file today. No stdlib layout is invented here; Phase 6 owns layering.

## Open questions the spec must answer

- **The hardest one, and the one a reviewer should attack first: does D3 actually discharge
  all three of `times`' body-inspecting obligations (recon 4)?** The spec must take each in
  turn — move-state identity, borrow-state identity, row-effect equality — and show it is
  either *implied* by the `Copy`-only capture restriction, *expressible* on the declared
  effect, or must **move to the inline site**. No hand-waving over the set. If any one of
  the three has to move to the inline site, say so plainly and state what that costs: the
  definition-site check becomes partial and the diagnostic lands at a call the author of
  `each` never wrote.
- **Directional checking of a literal against a declared parameter type.** Proposed: run
  the literal's body against the declared input row and compare exit rows, no inference,
  consistent with slice 4's D3 ("there is no standalone effect to infer for a bare body").
  The spec must state the rule and the diagnostic when the rows disagree, naming both.
- **What symbol, if any, a quotation-taking word mints.** Since the body is spliced at
  every call site, plausibly none is emitted at all — but slice 2's hazard is exactly this
  (`instantiation_symbol` colliding when two sites resolve to the same key), so state it
  rather than let it fall out. Two call sites passing *different* literals at the *same*
  concrete types must not collide.
- **Transitive inlining order and its termination rule** (`map` over `each`), and how it
  interacts with D5's recursion rejection: the cycle check and the splice must agree on
  what "un-inlinable" means, or one will accept what the other rejects.
- **A quotation parameter bound and used twice** (`| f | ... f call ... f call`): two
  splices of one body. Sound under D3, but it doubles code size silently. State the rule.
- **Exit-criterion witnesses.** Constant-stack run at 1M+ elements under a reduced
  `ulimit` is the primary witness (the Slice 6/`times` precedent). An `Instr::Call` count
  in the lowered caller (precedent `src/ir.rs:4396`) is a *regression guard*, not the
  primary evidence — with total inlining, "it compiled" already implies "it inlined", so
  the count test exists to catch a future fallback being added silently.
- **REPL rejection sites and wording**, including the interaction with slice 5b's imports:
  an imported closure exporting a quotation-taking word is a second chokepoint beside a
  session-typed definition. Pin against what 5b actually shipped, not against its spec.
- **Whether the nine stale "Phase 6" diagnostics (recon 9) are corrected here.**
  Recommended yes: they are wording, this is the slice that makes them false, and
  diagnostics are behaviour in this project.

## Out of scope

Everything slice 7 owns: the `IrType` variant, a calling convention, per-literal
environment struct synthesis, the `(code, env)` representation, escape rules for captures,
a quotation in an array element / struct field / branch join, dispatch tables, genuinely
non-inlined higher-order calls, and upward closures. The polymorphic-`if` and
polymorphic-self-tail gaps, and `filter`/`while` with them (6b). REPL support (6c).
`while` as a second floor intrinsic (weighed and declined in slice 4). Generic `type:`
declarations (Phase 6). The pre-existing native holes recorded on ROADMAP for slice 8:
duplicate `main` across modules, duplicate word names in one file, and
destructure-bypasses-`drop`. No new `Instr`/`Terminator`, and no `qbe.rs` change.
