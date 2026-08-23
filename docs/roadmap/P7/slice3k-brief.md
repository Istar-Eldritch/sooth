# P7.S3k brief — A non-inline generic word calling another generic word

## Problem, confirmed live against current `main`

`poly_call_term` (`src/check/poly.rs:677`) is threaded `poly_words: &HashSet<String>` — bare
callee *names* only, existing solely so the fall-through can name the diagnostic
(`src/check.rs:670-674`) — and never `poly_env: &PolyEnv` (`HashMap<String, Vec<(PolySig,
Option<u64>)>>`, `poly.rs:119`), the map holding each generic word's actual signature. So a
non-inline generic body can find *that* a name is a generic word, but never *retrieve its
signature* to dispatch against. Every call to a different generic word — same-module,
cross-module import, user-defined, or a library word like `gt`/`lt` — falls through to the
located `poly_calls_poly_word_error` (`poly.rs:1637`, thrown at `poly.rs:1365`):

```
error: `{caller}` cannot call the polymorphic word `{callee}` (line, col)
  a polymorphic word is not yet reachable from another polymorphic word across a module boundary
  inline the caller, make the callee concrete, or call the callee from a monomorphic word
```

Live-probed and confirmed identical for:

- a same-module generic-calls-generic call,
- a cross-module (imported) generic-calls-generic call,
- a bare comparison op (`gt`/`lt`/etc.) used directly on the poly word's own `'T`.

The self-call case (P7.S3g, shipped) is not an instance of this gap and needs no fix: it
special-cases `ctx.mangled_name() == Some(name)` (`poly.rs:1323`) and reuses the walk's own
`sig` directly — no registry lookup, no unification, no fresh type variables. Calling a
*different* generic word cannot reuse that trick because the callee's `PolySig` carries its
own, differently-numbered rigid type variables; you must first fetch that signature (today
impossible) and then relate it to the caller's.

**Dead code, confirmed by a real build.** The six-name "comparisons need `Ord`" carve-out
inside `poly_call_term` (`poly.rs:918-950`, matching bare `eq`/`lt`/`gt`/`lte`/`gte`/`ne`) is
unreachable in any real build: those six names are `inline` library words (`lib/cmp.sth:22-27`)
wrapping intrinsics, so a real call arrives mangled (`lt__mN`) and the bare-name `matches!`
never fires — confirmed by a live probe hitting `poly_calls_poly_word_error` instead. It only
"passes" today via `check_src`'s unmangled `parse_with_core` test harness
(`src/test_support.rs:67`), which is a test-harness artifact, not a shipping capability. Per
the roadmap's own exit criteria, deleting this carve-out and re-expressing its one test
(`check_poly_ord_word_accepts_comparison_body`, `poly.rs:5719`) against a real import/mangle
fixture is in scope for this slice.

## Existing precedent (what's already there to build on)

**Concrete-caller-calls-generic-callee, the mechanism this slice extends.**
`check_poly_call` (`poly.rs:3521`, dispatched from `terms.rs:709-714`) unifies each declared
input `PolyType` against the caller's *concrete* stack slot (`unify_poly_input`,
`poly.rs:3554-3596`), producing a ground substitution θ. It checks each declared bound against
θ's concrete type (`is_copy`/`is_ord`, `poly.rs:3596-3609`) as a **located, at-check-time**
rejection (`poly_copy_bound_error`/`poly_ord_bound_error`, `poly.rs:4631`/`4650`) — never a
monomorphization-time panic. It then records a `CallInst` (`poly.rs:3627-3641`) keyed by call
site `Span`, carrying θ and a mangled `symbol = instantiation_symbol(callee, subst, generation)`
(a pure function of `(callee, θ, generation)`).

**Correction to the roadmap's own framing:** actual lowering does **not** walk a pre-existing
worklist. `check()` collects `insts: HashMap<Span, CallInst>` into `module.instantiations`
(`check.rs:904`); `driver.rs:221-291` drains it **once**, deduping by `instantiation_symbol`
into a `HashSet` (`driver.rs:242-250`), and never appends new entries while lowering a body.
This is a flat collect-then-drain pass, not a worklist, because it has never needed to be
recursive — a generic body couldn't call another generic word before this slice. **The
recursive/transitive worklist this slice needs is net-new machinery in `driver.rs`, not a
reuse.**

**The bound-check primitive already exists in the right shape.** `PolySig::has_bound(var,
bound)` (`ast.rs:1384`, over `PolySig.bounds: Vec<(u32, Bound)>`, `ast.rs:1376`) is already used
*symbolically* — with no concrete type in hand, purely against the caller's own declared bounds
— in the body-side Ord-comparison gate (`poly.rs:927`, `sig.has_bound(v, Bound::Ord)`). What's
missing is threading that same pattern *across a call boundary*: today only
concrete-type-satisfies-`Bound` checks (`is_copy`/`is_ord`) cross a call boundary; no code
compares one signature's declared bound set against a different signature's required bound.

## Paper-traced design (validated, not yet spec'd)

For `g['T]` calling `h['U: Ord]` where neither is yet instantiated:

1. **Find `h`'s signature.** Thread `poly_env: &PolyEnv` (not just `poly_words`'s name set)
   through the poly-body walk chain: `check_poly_body`, `check_poly_combinator_standalone`
   (`check.rs:776,830`), `poly_walk` (`poly.rs:540`), `poly_term` (`poly.rs:557`), the recursive
   arm re-entry points (`poly_eliminator_call`, `poly.rs:1673`, and the `if`/combinator arm
   handlers) — about six functions plus the REPL mirrors, which already hold `poly_env` and
   would pass it unchanged.
2. **Relate `h`'s variables to `g`'s, symbolically.** Structurally match `h`'s declared input
   `PolyType`s against `g`'s rigid operand types, producing a *variable-to-variable* mapping
   (`h`'s `'U` ↦ `g`'s `'T`) — not a ground substitution, since `g`'s own `'T` is still abstract
   at this point.
3. **Discharge `h`'s bounds against `g`'s declared bounds, at the call site.** For each
   `('U, bound)` in `h.bounds`, look up the mapped `g` variable and check
   `g_sig.has_bound(mapped_var, bound)` — reusing the exact primitive already proven at
   `poly.rs:927`. Failure is a located error at the call site, not a deferred one.
4. **Record the symbolic mapping**, distinct from the concrete `CallInst` used by
   monomorphic callers.
5. **At lowering, compose and enqueue recursively.** When `g` is eventually instantiated at
   some concrete θ_g (from any concrete caller, or transitively from another generic caller),
   compose `θ_h = θ_g ∘ mapping` and enqueue `h`'s instantiation at θ_h if not already in the
   emitted set — recursing into `h`'s own generic callees the same way. Dedup against the
   emitted set **before** recursing into a newly discovered instantiation's own callees (the
   standard worklist/fixpoint pattern), so a mutual cycle `g ↔ h` simply stops the second time
   `(word, θ)` repeats.

## Locked decision: indirect/mutual generic recursion is allowed, with a scope cap

`g` calling `h` calling `g` (two distinct signatures, not a self-call) is in scope and must
work, *provided the type doesn't grow across the cycle* — i.e. the variable-to-variable mapping
at each hop is a positional identity (`h`'s `'U` maps to exactly one of `g`'s existing rigid
variables, unmodified), so revisiting `(g, θ)` at the same θ is guaranteed and the emitted-set
dedup terminates the walk.

**Out of scope for this slice, rejected with a located error:** a cross-call that would
*compose a new, structurally larger type* at each hop (e.g. wrapping the operand in a struct or
array before passing it on, so each recursive round targets a distinct, ever-deepening
concrete type and the emitted-set dedup never fires). This is the same failure mode Rust caps
with a recursion-limit error rather than solving in general. The check for "did this cross-call
grow the type" needs to be part of the design (comparing the mapped operand `PolyType`'s shape
against the plain rigid-variable case), and the located rejection needs its own diagnostic and
test. Whether this can be caught structurally at check time (the mapping step, #2 above, isn't
a pure identity) or needs an instantiation-count/depth cap at drain time (#5) is an open
question for the spec to resolve — cite the exact structural signal available at each step
before choosing.

## Exit criteria (from the roadmap, unchanged)

- A non-inline generic word may call another generic word — same-module or imported,
  user-defined or a library word like `gt`/`lt` — passing its own rigid type variables through.
- The callee is monomorphized once per concrete instantiation the caller reaches, the same way
  a concrete caller's generic callees already are.
- A bound mismatch (the callee needs a bound the caller's type variable doesn't declare) is a
  located error at the call site, not a hang or a monomorphization-time panic.
- Indirect/mutual generic recursion (non-growing) works; a growing cross-call is a located
  rejection, not a hang or unbounded compilation.
- `poly_calls_poly_word_error`'s message is deleted along with the gap it named; the dead
  six-comparison carve-out in `poly_call_term` is deleted; its one test
  (`check_poly_ord_word_accepts_comparison_body`) is retired or re-expressed against a real
  import/mangle fixture, not the unmangled `parse_with_core` harness. The same-module control
  test (`tests/phase7_slice3g.rs::different_poly_word_call_still_names_the_narrowing`) and the
  cross-module narrowing test (`tests/phase8_slice2.rs::a_poly_word_calling_an_imported_poly_word_names_the_narrowing`)
  are retired with the gap they pinned, replaced by tests proving the call now grounds.

## Sizing

This is **not** a one-phase slice like S3j. Distinct pieces of real design/implementation:

1. Plumb `poly_env` through the poly-body walk chain so a different generic word's signature is
   reachable (mechanical, low risk — the "can it find the callee" half only).
2. Symbolic variable-to-variable relation + cross-signature bound discharge at the call site
   (new mechanism; the primitive it reuses is proven, but the cross-signature composition isn't
   built anywhere today).
3. The recursive/transitive instantiation worklist in `driver.rs` (net-new; today's mechanism
   is a flat single pass and doesn't need to become one until this slice).
4. The type-growth detection and located rejection for the out-of-scope indirect-recursion case
   (needs its own design: is it a check-time structural signal or a drain-time depth cap).
5. Deleting the dead comparison carve-out and retiring/re-expressing its test and the two
   narrowing tests this gap currently protects.

Recommend spec-writer treats 1-2 as one phase, 3 as its own phase (distinct file, distinct risk
profile — `driver.rs` correctness affects every existing concrete-caller instantiation too, so
needs its own regression coverage), and 4-5 folded into whichever phase naturally produces the
diagnostic. Given the design surface, expect at least one spec-review round to probe the
worklist's interaction with existing concrete-instantiation dedup before implementation.

## Ready to spec: yes, with one instruction for spec-writer

Verify every citation above against live `main` before writing (do not trust this brief's line
numbers without re-checking — poly.rs and driver.rs are active files other in-flight slices
also touch). Treat the growing-vs-non-growing recursion boundary (see "Locked decision" above)
as an open design question to resolve with a concrete mechanism, not a restated requirement —
it hasn't been designed yet, only bounded.
