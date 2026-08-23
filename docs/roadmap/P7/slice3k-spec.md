# P7.S3k spec — A non-inline generic word calling another generic word

Delivery plan. Read `docs/roadmap/P7/slice3k-brief.md` first: it holds the confirmed
root cause and the paper-traced design this spec commits to. All `path:line`
anchors below were re-verified against live `main`; `poly.rs`/`driver.rs`/`check.rs`
drift, so line numbers are anchors to re-confirm at implementation, not contracts.

**v2 revision.** The v1 phase split put transitive instantiation discovery in a new
`driver.rs` worklist. Investigation (see "Where discovery lives" below) found that
wrong: composition and interning must happen at **check time**, because (a) the ground
caller substitutions θ_g are exactly the concrete `CallInst`s the checker already
records, and (b) interning a composed θ_h (arrays/refs/generics, plus a `>=2`-output
bundle) must reuse the checker's `apply_subst` path, not be re-derived at drain time.
The routing consequence in `driver.rs`/`func_builder` is real but thin, and is
specified concretely below rather than left as an option list.

## Problem

`poly_call_term` (`src/check/poly.rs:942`) is threaded `poly_words: &HashSet<String>`
(the `poly_words` parameter) — callee *names* only, existing solely so the fall-through
can name the diagnostic — and never `poly_env: &PolyEnv`
(`type PolyEnv = HashMap<String, Vec<(PolySig, Option<u64>)>>`), the map holding each
generic word's signature. A non-inline generic body can learn *that* a name is a
generic word, but can never retrieve its signature to dispatch. Every call to a
*different* generic word — same-module, imported, user-defined, or a library word like
`gt`/`lt` — falls through to the located `poly_calls_poly_word_error` (defined
`poly.rs:1931`, thrown `poly.rs:1657`):

```text
error: `{caller}` cannot call the polymorphic word `{callee}` (line, col)
  a polymorphic word is not yet reachable from another polymorphic word across a module boundary
  inline the caller, make the callee concrete, or call the callee from a monomorphic word
```

The self-call case (P7.S3g, shipped) is not an instance of this gap: it special-cases
`ctx.mangled_name() == Some(name)` (`poly.rs:1615`) and reuses the walk's own `sig`,
with no registry lookup, no unification, no fresh variables. A *different* generic
word carries its own, differently-numbered rigid type variables, so its signature must
first be fetched (impossible today) and then related to the caller's.

**Dead code.** The six-name "comparisons need `Ord`" carve-out in `poly_call_term`
(`poly.rs:1202`, matching bare `eq`/`lt`/`gt`/`lte`/`gte`/`ne`, discharging via
`sig.has_bound(v, Bound::Ord)` at `poly.rs:1211`) is unreachable in any real build:
those six are `inline` library words (`lib/cmp.sth`) wrapping intrinsics, so a real call
arrives mangled (`lt__mN`) and the bare-name `matches!` never fires. It only "passes"
via `check_src`'s unmangled `parse_with_core` harness (`src/test_support.rs`). Deleting
it and re-expressing its one test is in scope.

## Requirements

- **R1** A non-inline generic word may call another generic word — same-module or
  imported, user-defined or a library word (`gt`/`lt`/…) — passing its own rigid type
  variables through. The call grounds at check time instead of hitting
  `poly_calls_poly_word_error`.
- **R2** The checker relates the callee's declared input `PolyType`s to the caller's
  rigid operand slots symbolically, producing a **variable-to-variable mapping**
  (callee variable ↦ caller rigid variable, or ↦ a concrete type), not a ground
  substitution — the caller's own variables are still abstract at check time.
  **Consistency requirement:** if a callee variable is matched in two positions against
  two *different* caller images, that is an inconsistent mapping and is a located error
  at the call site, mirroring the concrete path's `poly_var_conflict_error`
  (`poly.rs:5217`, thrown from the `Var` arm of `unify_poly_input` at `poly.rs:4154`
  when a second differing binding is seen). State it as a requirement, not an implicit
  consequence of "building the mapping".
- **R3** For each `(callee_var, bound)` in the callee's `bounds`, the mapped caller
  variable must satisfy that bound in the caller's own declared bound set
  (`caller_sig.has_bound(mapped_var, bound)`, `ast.rs:1384`). A mismatch is a **located
  error at the call site**, never a deferred monomorphization-time panic.
- **R4** The callee is monomorphized **once per distinct concrete instantiation the
  caller reaches**, the same way a concrete caller's generic callees already are.
- **R5** Indirect/mutual generic recursion that does **not** grow the type across the
  cycle (`g` → `h` → `g`, each hop a pure variable renaming) compiles and runs.
- **R6** A **growing** cross-call (one whose mapping would compose a structurally
  larger type at each hop) is a **located rejection at the call site**, not a hang or
  unbounded compilation. See "Resolved: growing-type detection".
- **R7** `poly_calls_poly_word_error` and the message it carries are deleted along with
  the gap. The six-name carve-out (`poly.rs:1202-1234`) is deleted. Its one test
  (`check_poly_ord_word_accepts_comparison_body`, `poly.rs:6915`) is retired or
  re-expressed against a real import/mangle fixture. The two narrowing tests that pin
  the gap — `tests/phase7_slice3g.rs::different_poly_word_call_still_names_the_narrowing`
  (`:190`) and
  `tests/phase8_slice2.rs::a_poly_word_calling_an_imported_poly_word_names_the_narrowing`
  (`:440`) — are retired and replaced by tests proving the call now grounds. The two
  prose references in `tests/phase7_slice3b_follow.rs` (`:267`, `:725`) that name the
  phase8 test are updated when it is retired.

## Non-functional requirements

- **N1** No monomorphization-time panic on any legal or illegal program in scope: every
  rejection in R2/R3/R6 is a located, at-check-time diagnostic. In particular no
  cross-call may reach the ordinary-user-call arm's `self.env.get(name).expect("checked
  user word exists")` (`calls.rs:716`/`725`) — that panic is the observable symptom of
  an unrouted generic-word call, and Phase 2's routing is precisely what keeps a
  cross-call from reaching it.
- **N2** The transitive-discovery machinery must **not** change the emitted IL for any
  existing concrete-caller-calls-generic-callee case: same symbols, same count, same
  order. A program with no generic-calls-generic records must produce byte-for-byte
  identical IL. Proven by regression coverage (see the named baseline below), not
  assertion.
- **N3** Compilation terminates on every program the checker admits — no depth cap, no
  time-out; termination is a consequence of R6 restricting the reachable
  `(word, θ)` set to a finite closure (see rationale).
- **N4** Craft-scope discipline (CLAUDE.md): no worklist abstraction beyond what these
  two phases need; no pre-staged plumbing for a future generic-generic feature.

## Success criteria (observable)

Phase attribution in brackets (see "Phase split").

- `[P1 check-clean; P2 IL]` `: id ( 'T -- 'T ) ;  : g ( 'T -- 'T ) id ;`
  (same-module generic-calls-generic) compiles; `g` instantiated at `i64` emits one
  `sooth_mono_id__…i64` alongside `sooth_mono_g__…i64`.
- `[P1 check-clean; P2 IL]` An imported generic callee behaves identically.
- `[P1]` A generic body using `gt`/`lt` on its own `'T` compiles iff the caller declares
  `'T: Ord`; without the bound it is a located call-site error.
- `[P2 run]` A mutual non-growing pair `g ↔ h` compiles, runs, and terminates
  compilation.
- `[P1]` A growing cross-call is a located call-site rejection with its own diagnostic.
- `[P1]` A cross-call whose mapping is inconsistent (one callee variable pinned to two
  different caller images) is a located call-site rejection (R2).

## Scope and boundaries

**In scope:** the two-phase design below (R1–R7). **Out of scope, rejected with a
located error (R6):** a cross-call composing an ever-larger type per hop.
**Untouched:** the self-call path (`poly.rs:1615`), the concrete-caller `check_poly_call`
path except where Phase 2 reuses its interning, and every diagnostic not named in R7.

## Resolved: growing-type detection (the brief's one open question)

The brief bounded but did not resolve whether a growing cross-call is caught
**structurally at check time** (mapping step, R2) or needs an
**instantiation-count/depth cap at drain time**. This spec resolves it in favour of the
**check-time structural rule**, and specifies it concretely.

**The structural signal at the mapping step.** `PolyType` (`src/ast.rs:1301`) is
`Concrete(Type) | Var(u32) | Array(Box<PolyType>, Len) | Quotation(..) | Ref(Box<PolyType>, bool) | Generic{..} | …`.
When step R2 structurally matches a callee input against the caller's supplied operand
slot, the *image* of each callee variable is exactly one `PolyType` drawn from the
caller's slot. That image is already in hand, fully structured, at the call site.

**The rule (R6).** After building the mapping, the image of every callee type variable
must be **either fully concrete (however deep) or a bare `Var(_)`** (a single caller
rigid variable, unwrapped). Reject only a **compound image that *mentions* a caller
variable** — `Array(.. Var(T) ..)`, `Ref(Var(T), _)`, `Generic{ args: [.. Var(T) ..] }`,
etc.: that is the caller *wrapping* its variable before passing it, the growing case.
Reject at the call site with a dedicated diagnostic (`poly_growing_cross_call_error`,
new).

Note the one-liner: it is **not** "`Concrete` or bare `Var`" read shallowly. A
fully-concrete image at any depth (an array of a concrete element, a `Box[i64]`) is
allowed; the reject predicate is exactly "the image is compound *and* some leaf is a
`Var` naming a caller variable".

**R6 vs. legitimate structural forwarding (the distinction an implementer must not
confuse).** These two look superficially similar and are opposite verdicts:

- *Accepted — forwarding.* The callee declares its own compound-shaped parameter and
  the caller's operand already has that shape. `h ( &'U -- )` called by
  `g ( &'T -- ) ... h`: `unify_poly_input`'s `Ref` arm decomposes both structurally,
  the image of `'U` is the *bare* `Var(T)`, no wrapping happened. Not growth.
  Likewise `h ( 'U -- )` called by `g ( 'T -- ) ... h`: image of `'U` is `Var(T)`.
  Not growth.
- *Rejected — growth.* The callee declares a **bare-variable** parameter and the
  caller builds a fresh compound over its own variable *at or before the call site* and
  hands that in. `h ( 'U -- )` called by `g ( 'T -- ) ... Box h`, where `Box`
  constructs a `Box['T]` enum value in `g`'s body (`type: Box 'T | Box 'T ;` — Sooth has
  no generic structs, only generic enums; see the R6 fixture note below): the callee
  sees a bare `'U`, but the image is `Generic{ Box, args:[Var(T)] }` — compound over a
  caller variable. Growth, reject.

The discriminator is precisely the *image* of a callee variable, not the callee's or
caller's declared shapes in isolation: growth is a compound-over-a-caller-variable
image, which arises only when the caller manufactures a larger type than the callee's
own signature demands.

**Why this terminates without a depth cap (N3).** Under the rule, when the caller is
instantiated at a concrete `θ_g`, the composed `θ_h = θ_g ∘ mapping` assigns each callee
variable either a fixed concrete type (from a concrete image) or `θ_g`'s image of a
single caller variable — never a constructor applied to `θ_g`'s images. So every
reachable callee substitution draws its types from the finite set of concrete types
introduced at the program's seed (concrete-caller) instantiations. **This argument is
about check-time discovery, not a drain:** the seed set is the concrete `CallInst`s the
checker already recorded (`module.instantiations`), and each discovery step composes a
θ_h whose type images are drawn from that same finite pool. The set of reachable
`(word, θ)` pairs is therefore finite; deduping newly-composed instantiations by
`instantiation_symbol` reaches a fixpoint. A mutual cycle `g ↔ h` revisits `(g, θ)` at
the *same* `θ` and stops. No K-bound is chosen or needed.

**Why check-time over a drain-time depth cap.**

1. **Locatable.** R6 demands a located error *at the call site*. The structural rule
   fires exactly there, at the mapping step, naming the offending call and operand. A
   depth cap fires far from source with a "recursion limit reached" message naming no
   call site.
2. **Total, on information already in hand.** The image `PolyType`s are produced by the
   same structural walk `unify_poly_input` (`poly.rs:4131`) already performs; no
   drain-time bookkeeping, no tuning constant.
3. **Termination as a theorem, not a bound.** N3 becomes "the reachable set is finite",
   provable, rather than "we stopped after K". A depth cap also *false-rejects* a
   legitimately deep-but-finite non-growing program, which the structural rule never
   does.
4. **Keeps discovery a pure fixpoint.** The composition step stays a plain
   symbol-dedup fixpoint, simpler to make N2-safe.

**Accepted cost (spec-level design choice, not "per the brief").** The rule *also*
rejects a single, non-recursive wrapping cross-call (`g` wraps `'T` and hands it to a
callee that just consumes it), which would in fact terminate. The brief locked out only
*recursive/growing* cross-calls; the over-rejection of the single-shot wrap is a
**deliberate simplification this spec chooses**, not something the brief mandated. It
buys a check-time structural rule with no cycle detection. The located diagnostic names
the restriction so a future slice can lift it (by tracking per-cycle growth rather than
per-call shape) if a real need appears. Lifting it now would require cycle detection the
two goldens do not motivate — deferred, not skipped.

## Where discovery lives (the v2 architectural fix)

**The symbolic cross-call record: type and home.** Phase 1's poly-body walk (the new
generic-callee arm in `poly_call_term`) must record, for each grounded generic-to-generic
call it admits, a `PolyCrossCall { callee: String, span: Span, mapping: Vec<(u32, Image)> }`
where `Image` is `enum Image { Concrete(Type), CallerVar(u32) }` (R6's own accept set --
an image is exactly one of these two shapes by construction, since a compound image is
already rejected at this point). This is an **`ast.rs` addition, added in Phase 1**
(Phase 1's Scope below is corrected to include it) alongside the new diagnostics, since it
is Phase 1's walk that produces it. It is stored on `Module` as
`Module.poly_cross_calls: HashMap<String, Vec<PolyCrossCall>>`, keyed by the **generic
word whose body contains the calls** (not by caller instantiation -- at Phase 1's walk
time there is no instantiation yet, only the word's rigid signature). Phase 1 populates
it and consumes nothing from it (no IL, no `driver.rs` touch). Phase 2's fixpoint (below)
is the sole reader: for a taken `(w, theta_w)`, it looks up `poly_cross_calls[w]` to find
what `w`'s body calls, and composes each entry's `mapping` against `theta_w`. This is
distinct from the two Phase-2-only fields below (`CallInst.poly_calls`,
`Module.transitive_instantiations`), which hold *composed, grounded* `CallInst`s, not the
symbolic mapping -- `poly_cross_calls` is never grounded and never touched after Phase 2's
fixpoint reads it.

**Discovery + interning run inside `check()`, not `driver.rs`.** Right after `check()`
builds its concrete `CallInst` set — concretely, at the point where the multi-output
bundle-interning loop runs today (`check.rs:963`, `for inst in insts.values_mut() { if
inst.out_arity >= 2 { … } }`, immediately after `intern_output_bundles(module)` at
`check.rs:957` and before `module.instantiations = insts` at `check.rs:971`; re-locate
live) — run a fixpoint:

1. Seed a worklist with the recorded concrete instantiations (`insts.values()`).
2. For each taken `(w, θ_w)` and each entry in `Module.poly_cross_calls[w]` (a
   `PolyCrossCall { callee: h, span: S, mapping }`): compose `θ_h = θ_w ∘ mapping`. Ground every callee
   input/output through the **same** `apply_subst`-based interning the concrete path
   uses (`poly.rs:4389`; confirmed general-purpose — it takes `(sig, pty, subst, …,
   arrays, refs)` and interns `Array`/`Ref`/`Generic` shapes into the module registries
   via the live instantiator, not tied to `check_poly_call`). Build a `CallInst` for
   `(h, θ_h)`, and run the **same `out_arity >= 2` bundle-interning step** the concrete
   loop at `check.rs:963` runs (`intern_bundle_struct` into `module.structs`), so a
   composed callee that returns a bundle is laid out like any other.
3. Record the composed `CallInst` two ways (see "Two records" below): into `(w, θ_w)`'s
   per-instantiation routing map (for the emitted call site in `w`'s body), and into a
   flat transitive set (so the callee itself gets one `IrFunc`).
4. If `(h, θ_h)` (by `instantiation_symbol`) is new, enqueue it — so `h`'s own cross-call
   records are then discovered against `θ_h`. Dedup **before** recursing. Finiteness is
   R6's theorem (above): the reachable `(word, θ)` closure is finite.

**Two records, and why `module.instantiations` alone cannot hold routing.** The call
site in `w`'s body has *one* span `S`, but `w` is instantiated at N distinct `θ_w`,
each yielding a distinct `θ_h`. Lowering resolves a call site's target through a
**span-keyed** lookup — `func_builder`'s `self.instantiations.get(&span)`
(`calls.rs:332`) over the single global `module.instantiations: HashMap<Span, CallInst>`
(`ast.rs:64`). A `HashMap<Span, _>` structurally *cannot* hold N distinct targets for
one span. So routing needs a **per-caller-instantiation** map, exactly as the P7.S3e
trait-dispatch mechanism already does with `CallInst::trait_calls`
(`ast.rs:1575`, `HashMap<Span, String>`, threaded per-instantiation onto the
`FuncBuilder` at `driver.rs`'s per-instantiation lowering and consulted at
`calls.rs:278`).

Therefore:

- **New field `CallInst.poly_calls: HashMap<Span, CallInst>`** (mirrors `trait_calls`,
  but maps a body cross-call span to the *fully composed callee `CallInst`* rather than
  to a bare symbol string, because lowering the cross-call needs the callee's `subst`/
  `symbol`/`out_arity`/`bundle` to emit through `lower_poly_call`, not just a name).
  Populated in step 3 for the caller instantiation; empty (default) on every
  monomorphic word and on any instantiation with no generic-word calls, so N2 holds
  trivially for the existing corpus. The `poly_calls`-nested copies do **not** need
  their own `poly_calls` populated (they route a single `Instr::Call`, they do not
  lower `h`'s body).
- **New module field `Module.transitive_instantiations: Vec<CallInst>`** holding the
  flat, symbol-deduped set of composed callee monomorphs. `module.instantiations` is
  `Span`-keyed and cannot hold N callee monomorphs sharing one body span, so a flat Vec
  is the minimal container that lets the driver emit one `IrFunc` per distinct composed
  `(callee, θ_h)`. Each entry here **does** carry its own populated `poly_calls` (it is
  the authoritative record from which `h`'s body is lowered, so `h`'s own cross-calls
  must route). `generation` (`ast.rs:1560`) is inherited from the caller instantiation
  so `instantiation_symbol` stays collision-free in REPL builds.

**Consequent `driver.rs`/`func_builder` change (thin, but real — do not claim
"none").** Two edits:

1. **Emit set.** The flat drain (`driver.rs`, `for inst in module.instantiations.values()`
   → dedup by `instantiation_symbol` into `emitted`/`distinct` → sort → lower; re-locate,
   ~`:256`) additionally iterates `module.transitive_instantiations`, deduping by the
   same `instantiation_symbol` into the same `emitted` set. Because a concrete-only
   program has an empty `transitive_instantiations`, the drain produces an identical
   `distinct` list and identical IL (N2).
2. **Routing.** The per-instantiation lowering call already threads `&inst.trait_calls`
   into `lower_word_parts` → `FuncBuilder`; thread `&inst.poly_calls` the same way (the
   monomorphic-word loop passes an empty map, as it does for `trait_calls` via
   `empty_trait_calls()`). In `func_builder`'s call dispatch (`calls.rs`), add one arm
   **before** the global `self.instantiations.get(&span)` lookup at `calls.rs:332`:
   `if let Some(inst) = self.poly_calls.get(&span) { self.lower_poly_call(inst); return; }`.
   This is what keeps a cross-call from falling to the ordinary-user-call arm's
   `self.env.get(name).expect("checked user word exists")` (`calls.rs:716`/`725`), which
   would otherwise panic on the callee's mangled name (a poly word has no monomorphic
   `env` entry). No θ composition and no interning happen here — both were done at check
   time; the driver only walks a finished graph and dedups symbols.

## Codebase map

- `src/check/poly.rs:942` `poly_call_term` — dispatch site; today takes
  `poly_words: &HashSet<String>`, needs `poly_env: &PolyEnv`. Fall-through at `:1657`
  throws `poly_calls_poly_word_error` (`:1931`). Self-call arm at `:1615` (untouched).
  Dead carve-out `:1202-1234` (delete, R7).
- `src/check/poly.rs` `poly_walk`/`poly_term`/`poly_eliminator_call`/`poly_walk_arms` —
  the walk chain that threads `poly_words`; each gains `poly_env` (mechanical).
- `src/check.rs:119` `PolyEnv` typedef; construction near `:627`. The poly-body/
  combinator checkers around `check.rs:696-710` already hold `poly_env`.
- `src/check/poly.rs:4131` `unify_poly_input` — the structural matcher; its `Var` arm
  (`:4154`) raises `poly_var_conflict_error` (`:5217`) on a second differing binding
  (the R2 consistency precedent). `apply_subst` (`:4389`) is the general-purpose
  interner Phase 2 reuses for θ_h.
- `src/check/poly.rs:3708` — the concrete precedent's Ord bound check against a ground
  slot; Phase 1's cross-signature discharge mirrors it *symbolically* via `has_bound`.
- `src/ast.rs:1301` `PolyType`, `:1283` `Bound`, `:1376` `PolySig.bounds`,
  `:1384` `has_bound`, `:1549` `CallInst` (`callee`/`subst`/`symbol`/`out_arity`/
  `output_types`/`bundle`/`generation`/`trait_calls`), `:1584` `instantiation_symbol`,
  and `Subst` — for composing `θ_h = θ_g ∘ mapping`.
- `src/check.rs:957` `intern_output_bundles`, `:963` the `out_arity >= 2` bundle loop,
  `:971` `module.instantiations = insts`. The transitive fixpoint slots in here.
- `src/ast.rs:64` `Module.instantiations: HashMap<Span, CallInst>` (Span-keyed, cannot
  hold cross-call multiplicity — the reason for the two new fields).
- `src/ir/driver.rs` the flat drain (`for inst in module.instantiations.values()`,
  re-locate ~`:256`) and the per-instantiation lowering loop threading
  `&inst.trait_calls` — both get the thin change above.
- `src/ir/func_builder/calls.rs:278` `trait_calls` routing arm (the exact template for
  the new `poly_calls` arm), `:332` the global instantiation lookup (new arm goes
  before it), `:716`/`:725` the `expect("checked user word exists")` panic N1 forbids
  reaching.

## Tests to retire / re-express (R7)

- `src/check/poly.rs:6915` `check_poly_ord_word_accepts_comparison_body` — re-express
  against a real `import:`/mangle fixture (the capability it names now ships), or retire
  if the new grounding goldens cover it.
- `tests/phase7_slice3g.rs:190` `different_poly_word_call_still_names_the_narrowing` and
  `tests/phase8_slice2.rs:440` `a_poly_word_calling_an_imported_poly_word_names_the_narrowing`
  — retired with the gap; replaced by grounding goldens.
- `tests/phase7_slice3b_follow.rs:267,725` reference the phase8 test by name in prose;
  update those references when the test is retired.

## Open risks

- **Transitive set ↔ existing dedup (highest).** The drain shares `emitted`/`distinct`
  with every current concrete-caller instantiation. A change to iteration or dedup can
  reorder or drop existing IL (N2). Mitigated by the named regression baseline below and
  by an expected spec-review round probing the drain/dedup interaction.
- **REPL mirrors.** The walk chain has REPL mirrors that already hold `poly_env`; Phase 1
  must thread it there too or the REPL path silently keeps the old `poly_words`-only
  behaviour.
- **`generation` on composed instantiations.** A composed `θ_h` must inherit the
  caller's `generation` so `instantiation_symbol` stays collision-free in REPL builds.
- **Fixture constructibility (R6).** The R6 witness relies on a generic **enum**
  (Sooth has no generic structs) being constructible inside a poly body; probed clean
  live (see the R6 fixture note below), re-probe at implementation.

## Phase split

Discovery+interning are now check-time, so the old "checker vs driver.rs worklist"
split collapses. The split that survives is by **risk surface**, and both phases are
predominantly check-time:

- **Phase 1** — the checker mechanism that produces every call-site diagnostic: thread
  `poly_env`, the symbolic variable-to-variable relation (R2, incl. the consistency
  rejection), cross-signature bound discharge (R3), the growth rejection (R6), the
  inconsistent-mapping rejection, and the dead-code/test retirement (R7). Phase 1
  records a **symbolic cross-call record** per grounded generic-generic call and emits
  no IL.
- **Phase 2** — the check-time transitive fixpoint (composition + `apply_subst`
  interning + bundle interning, populating `poly_calls` and `transitive_instantiations`)
  **plus** the thin driver/func_builder routing consequence (emit-set drain extension +
  per-instantiation `poly_calls` threading + the `calls.rs` dispatch arm) **plus** the
  full run/IL/regression suite. Split from Phase 1 for review-round manageability and
  because it is the only phase that touches the IL-emitting path every existing
  instantiation shares (N2 is load-bearing here).

## Phase 1 — Reachability, symbolic relation, bound discharge, growth rejection, dead-code deletion

**Scope (modify):** `src/check/poly.rs` (thread `poly_env` through the walk chain; add
the generic-callee arm: fetch callee `PolySig` from `poly_env`, build the var-to-var
mapping with the R2 consistency check, discharge bounds via `has_bound`, apply the
growth rule, record a symbolic cross-call; delete the six-name carve-out `:1202-1234`
and `poly_calls_poly_word_error` `:1931`; add `poly_growing_cross_call_error`, an
inconsistent-mapping error, and a bound-mismatch call-site error);
`src/check.rs` (thread `poly_env` at the walk entry; populate `Module.poly_cross_calls`
from the walk's recorded `PolyCrossCall`s); `src/ast.rs` (add `PolyCrossCall`, `Image`,
and `Module.poly_cross_calls: HashMap<String, Vec<PolyCrossCall>>` -- see "The symbolic
cross-call record: type and home" above); REPL mirrors of the walk chain.
**Out of bounds:** `driver.rs`/`func_builder` (Phase 2). No lowering change; a grounded
call records a symbolic mapping but is *not yet* monomorphized.

**Entry conditions:** green `main`.

**Exit criteria:**

- `poly_call_term` receives `poly_env` and a call to a different generic word no longer
  reaches `poly_calls_poly_word_error` (the function and its message are deleted; a
  build that still references it fails to compile — that is the proof it is gone).
- A same-module and an imported generic-calls-generic call each **check clean**.
- A bound-requiring callee (`gt` on `'T`) checks clean iff the caller declares the bound;
  otherwise a located call-site error.
- A growing cross-call is a located call-site error (`poly_growing_cross_call_error`).
- An inconsistent mapping is a located call-site error (R2 consistency).
- The six-name carve-out is deleted; `check_poly_ord_word_accepts_comparison_body` and
  the two narrowing tests are retired/re-expressed; no dangling references remain.

**Golden test plan (check-level — no run yet):**

- `check_generic_word_calls_same_module_generic_grounds` — source
  `: id ( 'T -- 'T ) ;  : g ( 'T -- 'T ) id ;` → checks clean (no diagnostic).
- `check_generic_word_calls_imported_generic_grounds` — a two-module fixture importing
  the callee → checks clean.
- `check_generic_comparison_body_with_ord_checks_clean` — `: g ( 'T: Ord 'T -- Bool ) gt ;`
  via a real `import:`/mangle fixture → checks clean (re-expresses the retired test).
  (Renamed from the v1 draft's `…requires_ord_declared` per CLAUDE.md's
  `thing_condition_expected` convention: this is the clean-check case.)
- `check_generic_comparison_body_without_ord_is_error` — same without `: Ord` → expected
  located call-site diagnostic naming the missing `Ord` bound.
- `check_generic_cross_call_bound_mismatch_is_error` — caller `'T` (no bound) passed to a
  callee needing `Ord` → located call-site error.
- `check_growing_cross_call_is_error` — **generic-enum-wrapper fixture** (see below), the
  caller wrapping `'T` in a `Box['T]` enum value before passing it → expected
  `poly_growing_cross_call_error`.
- `check_inconsistent_cross_call_mapping_is_error` — a callee variable matched against two
  different caller variables → located call-site error (R2 consistency).
- `check_poly_calls_poly_word_error_is_gone` — a compile-time guarantee via deletion; the
  retired narrowing tests' replacements assert grounding, not the old message.

**R6 fixture must not use an array wrapper.** Array *construction* inside any polymorphic
body is unconditionally rejected today by a pre-existing guard (`src/check/poly.rs:770`,
"an array constructor in the polymorphic body of `{}` … is not yet supported";
re-locate live). An array-based growth fixture would therefore be a **placebo**: it is
rejected by that pre-existing guard before the new growth rule is ever consulted,
regardless of whether the growth rule is implemented. Use a **generic enum wrapper**
instead — Sooth has no generic structs (every generic type in the codebase, e.g.
`lib/option.sth`/`lib/result.sth`, is a generic enum); a `type: Box['T] val 'T ;`
struct-style declaration does not parse (`expected a word, found LBracket`) and a
struct-style `type: Box val 'T ;` is rejected (`unknown type 'T`). The correct,
check-clean fixture is a single-variant generic **enum**: `type: Box 'T | Box 'T ;`,
constructed in the caller's body (`Box`) and passed to a callee declaring a bare `'U`
input. Confirmed live: this construction checks clean, and the resulting cross-call
reaches the new growth-rule site (today it reaches `poly_calls_poly_word_error`,
confirming the operand arrives at the gap this slice closes) — so the growth rule is
the site that actually fires, not some earlier guard. `tests/phase7_slice3a.rs`'s
`poly_word_constructs_a_monomorph…` already builds a comparable generic-enum
construction in a poly body as further precedent. Record in the fixture's comment *why*
the array form is not used, so nobody re-adds an array-based "second witness" thinking
it strengthens coverage — it would silently test the pre-existing guard, not R6.

**Difficulty:** hard (new cross-signature mechanism + diagnostic surface).

### Delivered, with six deviations from the plan above

1. **A fifth diagnostic, `poly_cross_call_unsupported_error`.** The plan named
   three new diagnostics; a fourth is needed for the callee-signature shapes a
   `Vec<(u32, Image)>` mapping structurally cannot carry, each of which N1
   requires be a located rejection rather than an admitted shape mis-lowered
   later: a row (`..s`), a quotation parameter, a length variable (a second id
   space `Image` does not model), a **user trait bound** (see phase 2 finding 3),
   and a compound *output* (`( 'U -- Box['U] )`). It is not the deleted
   whole-feature narrowing under a new name -- it fires for five named declared
   shapes, and each names itself. Those five are the *signature*-side reasons;
   deviations 5 and 6 add two more that are properties of a call's operand
   rather than of the callee's declaration, so the diagnostic carries seven
   reason strings in all. All five are reachable from source; **review
   fix:** the row shape was reachable but pinned by nothing (this section's
   original "all five ... pinned" was false for it) -- fixed by
   `check_cross_call_unsupported_callee_shapes_name_themselves`'s fourth
   table entry, mutation-tested by deleting the row gate.

2. **Compound outputs are rejected, symmetrically with R6.** A declared
   compound always mentions a variable (a fully concrete one folds to
   `Concrete` at parse), so substituting the mapping into a compound output
   either grows a type over a caller variable -- R6 in the return direction --
   or needs the registry interning `apply_subst` performs for a *ground* θ and
   nothing symbolic can do. So the rule across a cross-call is symmetric: a
   type mentioning a caller variable must be a bare variable, in both
   directions.

3. **`check_generic_word_calls_imported_generic_grounds` is split.** A
   genuine two-module fixture needs the driver's closure assembly, and a
   `tests/` build golden cannot be written for a *non-inline* callee until
   phase 2 routes it. The unit half is
   `check_generic_word_calls_mangled_generic_grounds`, which runs the real
   `resolve_modules` mangling -- the only thing that distinguishes an imported
   callee at this level, since the arm dispatches on `poly_env`'s post-mangle
   keys and never on a spelling. The end-to-end half is phase 2's.

4. **An `inline` generic callee lands end-to-end already**, so phase 1 ships
   more than "checks clean": lowering *splices* such a callee, so it needs no
   monomorph and no routing. `lib/cmp.sth`'s comparisons on a body's own `'T`
   therefore build and run (`tests/phase7_slice3k.rs`), and `clampsum` -- the
   exit-criterion program of P7.S3b-follow, whose per-iteration `gt` cannot be
   hoisted to a monomorphic caller -- is un-`#[ignore]`d with its two goldens.

5. **Review fix: R6's wildcard over-rejected a fully concrete compound
   image as growth.** `poly_cross_match`'s `Var` arm sent every `Ref`/`Array`/
   `Generic` supplied type to `poly_growing_cross_call_error`, including one
   that mentions no caller variable at all (`&n` on a scalar `static n`,
   which is `&i64`, fully concrete) -- contradicting R6's own rule ("reject
   only a compound image that *mentions* a caller variable"). Root cause:
   folding such an image into `Image::Concrete(Type)` would have to mint a
   fresh `RefId`/`ArrayId`, and the poly-body walk holds no mutable
   array/ref registry to do that with (only `structs`/`enums` get a mutable
   path, via `ctx.generics()`'s `RefCell`). Fixed narrowly: a new predicate
   (`poly_type_mentions_caller_var`) distinguishes the two cases, so a fully
   concrete image now gets the honest `poly_cross_call_unsupported_error`
   ("passing the concrete compound value `&i64`") instead of the false
   growth claim; the growing case is unchanged. This does **not** make the
   call succeed -- R6's accept case ("a fully-concrete image at any depth
   ... is allowed") is still not actually implemented, only honestly
   rejected. See phase 2 finding 5 for the real fix.

6. **Review fix: the R3 `Copy` discharge panicked on a body-local generic
   instantiation, and four structural guards were reachable but untested.**
   `is_copy` resolves a struct or enum image by *indexing* `structs`/`enums`,
   and an instantiation this body's own walk minted is not in those slices
   yet: `check_poly_body` rebases the instantiator at entry but `check::check`
   appends the batch only after it returns, so the id sits past the end and
   indexing it panics. Five lines of source reach it
   (`type: Box 'T | Box 'T ;  : h ( 'U: Copy -- ) drop ;
   : g ( 'T -- 'T ) 1 Box h ;`), in both the generic-enum and generic-struct
   forms -- a check-time panic at the exact arm N1 forbids one at. Fixed with
   a `type_is_registered` guard that rejects the undecidable case honestly
   (`poly_cross_call_unsupported_error`), the same posture and the same
   underlying cause as deviation 5: deciding it needs a registry the walk does
   not hold. `Bound::Ord` needs no guard (`is_ord` reads no registry) and a
   user bound never reaches the loop (`poly_cross_signature_supported` rejects
   one first). The root cause is wider than this slice and is **not** fixed
   here -- see phase 2 finding 7. Also pinned this round: the four structural
   guards in `poly_cross_match`/`poly_cross_relate` (operand count,
   `Concrete`/`Concrete` equality, reference mutability, array length), each
   reachable from source with a correct diagnostic yet deletable with the whole
   suite green, and each failing *open* -- one into a subtract-overflow panic,
   the other three into an accepted call (a `Bool` filling an `i64` parameter,
   a shared borrow filling a mutable one, a `['T 4]` filling a `['U 3]`). Now
   covered by `check_cross_call_operands_the_callee_cannot_accept_are_errors`
   and `check_cross_call_copy_bound_on_a_body_local_instantiation_is_unsupported`,
   both mutation-tested.

The REPL deliberately keeps today's behaviour: it passes an empty registry, so
a session line calling another polymorphic word still gets `unknown word`.
REPL lowering resolves an instantiation through its own per-generation store
and nothing composes a cross-call's substitution into it, so grounding the call
there would check clean and then mis-lower -- worse than a clean rejection.
Lifting it needs the REPL's own composition step, not just the thread-through.
**Review fix:** this was undocumented as deliberate-but-untested; pinned by
`repl_poly_word_calling_another_poly_word_is_unknown_word_not_grounded`
(mutation-tested by swapping in `self.poly_env()`, which the test now catches
before it becomes the `calls.rs:725` panic a real session would otherwise hit).

## Phase 2 — Check-time transitive fixpoint + thin routing consequence + regression

**Scope (modify):**

- `src/check.rs` (near `:963`, after the concrete `out_arity >= 2` bundle loop and
  before `module.instantiations = insts` at `:971`): the transitive fixpoint of "Where
  discovery lives" — seed from `insts.values()`, compose `θ_h = θ_w ∘ mapping` per
  symbolic record, intern via `apply_subst`, run the same bundle-interning step, populate
  `CallInst.poly_calls` per caller instantiation, collect into
  `module.transitive_instantiations`, dedup by `instantiation_symbol`, iterate to
  fixpoint.
- `src/ast.rs`: add `CallInst.poly_calls: HashMap<Span, CallInst>` and
  `Module.transitive_instantiations: Vec<CallInst>`.
- `src/ir/driver.rs`: the drain also iterates `module.transitive_instantiations` into the
  same `emitted`/`distinct` dedup; the per-instantiation lowering call threads
  `&inst.poly_calls` (monomorphic loop passes empty, mirroring `empty_trait_calls()`).
- `src/ir/func_builder/calls.rs`: `FuncBuilder` gains a `poly_calls` field threaded like
  `trait_calls`; a new dispatch arm before `:332` consults it and calls
  `lower_poly_call`.

**Out of bounds:** any change to `check_poly_call`'s concrete recording; the self-call
routing; the growth/mapping diagnostics (Phase 1).

**Entry conditions:** Phase 1 merged and green.

**Exit criteria:**

- A generic caller instantiated at a concrete type emits exactly one monomorphized
  `IrFunc` for each generic callee it transitively reaches, once per distinct
  `(callee, θ)`; a cross-call in a lowered body routes through `poly_calls` and never
  reaches the `expect("checked user word exists")` panic (N1).
- A mutual non-growing pair compiles, links, runs, and **compilation terminates**.
- **N2 regression:** the emitted IL for every pre-existing concrete-caller-calls-generic
  case is unchanged (same symbols, count, order).

**Golden test plan (run-level + IL + regression):**

- `phase7_slice3k::generic_calls_generic_monomorphizes_callee_once` (new `tests/` file):
  compile `: id ( 'T -- 'T ) ;  : g ( 'T -- 'T ) id ;` with a concrete `main`; assert the
  emitted IL contains one `sooth_mono_id__…` per distinct θ `g` reaches (assert callee
  identity in the IR, not runtime output — per "poly instantiation is unobservable at
  runtime").
- `phase7_slice3k::imported_generic_callee_monomorphizes` — cross-module variant, run to a
  known result.
- `phase7_slice3k::mutual_non_growing_recursion_terminates_and_runs` — `g ↔ h` non-growing
  pair; assert compilation completes and the program's output matches. This is the
  termination witness (N3).
- `driver`/`check` unit test `transitive_discovery_dedups_repeated_instantiation_symbol`
  — a `(callee, θ)` reached twice yields one entry (fixpoint dedup).
- **Regression baseline (named).**
  `tests/phase7_slice3a.rs::two_asymmetric_instantiations_mint_distinct_symbols_nm`
  (`:101`, re-verified live: it asserts, via `nm` over the built object, the two exact
  monomorph symbols `sooth_mono_reorder__m0__t0_i64_t1_str` and
  `…__t0_str_t1_i64` for a concrete caller reaching a generic callee at two asymmetric
  instantiations) is the pre-change snapshot the N2 requirement diffs against: it must
  pass **unchanged** through the new drain. Mutation-test it — a change that reorders or
  drops an existing instantiation must fail it.

**Accepted-case goldens must not construct-and-lower a generic aggregate in a poly
body.** R6 *allows* a concrete image (a caller passing a concrete `Box[i64]` built in a
poly body). If an accepted-case golden both constructs a generic aggregate inside a poly
body **and** actually lowers it, it risks a **pre-existing, out-of-scope backend gap**:
the ordinary-user-call arm's `expect("checked user word exists")` (`calls.rs:716`/`725`)
panics for a lowered body call that resolves to no monomorphic `env` entry. (Generic
*enum* construction alone is known-good — `tests/phase7_slice3a.rs`'s
`poly_word_constructs_a_monomorph…` builds one — but do not let a golden depend on the
untested construct-in-a-poly-body-and-lower path, which is what actually panics.) Keep the S3k accepted goldens on the simple
value-forwarding shapes (`id`/`g`, `gt` on `'T`, mutual `g ↔ h`), which construct no
aggregate in a poly body and so cannot reach that arm. This slice does **not** fix that
pre-existing backend gap; it is documented here so a golden is not silently authored onto
a crash.

**Difficulty:** hard (net-new discovery machinery + a routing change on the path shared
by all existing instantiation lowering; N2 is the load-bearing risk).

### Findings from phase 1 that phase 2 must act on

1. **The fixpoint must skip an `inline` callee.** Lowering splices one and an
   `inline` word mints no `IrFunc`, so composing a `(callee, θ_h)` for it and
   routing the body span through `poly_calls` would emit a call to a symbol
   that never exists -- turning a case that *already works* (deviation 4 above)
   into a link failure. Phase 1 records the cross-call regardless, because
   "this body calls `h` with this mapping" is true either way and how `h` is
   lowered is not the record's business; the fixpoint runs where `module.words`
   is in hand, so it can filter on `is_combinator` precisely, by candidate,
   rather than by name.

2. **`Module::poly_cross_calls` is name-keyed, and a polymorphic overload set
   merges into one entry.** Two poly words may share a name with different
   signatures (only *identical* signatures are rejected,
   `check_duplicate_poly_signatures`), and each record's `mapping` indexes its
   own signature's variables. The fixpoint looks the map up by
   `CallInst::callee`, a bare name, so it would compose one candidate's mapping
   against the other's θ -- silently, producing a wrong monomorph. `PolySig` is
   what disambiguates (`WordObligations` carries it beside the name for exactly
   this reason). Phase 2 should key on `(name, PolySig)` or reject a cross-call
   inside an overloaded generic word; it must not iterate the merged list.

3. **A user trait bound is a located rejection in phase 1, not R3's
   `has_bound` discharge.** Satisfaction itself would transfer soundly (the
   caller declares the same bound), but the *callee's* recorded obligations are
   resolved against a ground θ (`resolve_user_bound`, into
   `CallInst::trait_calls`), and nothing composes them for a cross-call -- an
   admitted one would lower a body whose member call resolves to nothing,
   exactly the monomorphization-time failure N1 forbids. Phase 2's fixpoint
   runs where `TraitResolveCtx`'s tables are already in scope, so lifting this
   is a candidate there; it is a scope decision, not an oversight.

4. **`docs/roadmap/P7-language-prereqs.md` still states the gap as open**
   (`:128`, `:381`, `:420`, `:533`). Left untouched in phase 1 on purpose: the
   slice is not delivered until a non-inline generic callee is monomorphized.
   Phase 2 updates it, and states the residual narrowings (deviation 1) rather
   than declaring the whole gap closed.

5. **R6's accept case (a fully concrete compound image) is still only
   honestly rejected, not implemented (deviation 5 above).** The real fix
   needs `Image` to carry a ground-but-uninterned shape through to a point
   that holds a mutable array/ref registry -- which phase 2's fixpoint
   already does, at `check.rs`, via the same `apply_subst` it composes θ_h
   with. This is **not** in phase 2's scope as specified above (which reads
   `Image` as exactly `Concrete(Type) | CallerVar(u32)` and never revisits a
   rejected mapping), so lifting it means either widening `Image` with a
   third, ground-but-uninterned variant that phase 2's fixpoint interns
   lazily, or -- equivalently -- threading a `refs`/`arrays` `RefCell` into
   `Ctx` the way `generics()` already is, so phase 1 itself can fold the
   image and never reject it. Recorded here as a candidate for its own
   follow-up slice, not phase 2 by default: phase 2's own scope list does not
   mention `Ctx` or `Image`'s shape, and widening either is a design decision
   a reviewer should make explicitly, not inherit silently.

6. **Phase 1 turned a clean rejection of the slice's headline program into a
   lowering panic, and nothing pins the interval.**
   `: id ( 'T -- 'T ) ;  : g ( 'T -- 'T ) id ;  : main ( -- ) 1 g drop ;` --
   the first success criterion above -- now checks clean and then panics at
   `calls.rs:725` (`checked user word exists`). At `ac4eb2a` the same program
   got the located `poly_calls_poly_word_error`. This is the phase split
   working as designed (`[P1 check-clean; P2 IL]`), and deviation 3 gestures at
   it, but the handover is *not* diagnostic-neutral: for a non-`inline` callee
   there is no located error anywhere between the two phases, and no test says
   so. Recorded explicitly so phase 2 does not read that panic as pre-existing
   -- phase 1 caused this instance of it, and phase 2's routing arm is what
   closes it. An `inline` callee is unaffected (deviation 4).

7. **The registries a polymorphic body walks are stale for the instantiations
   that body itself mints, and this slice only worked around it.**
   `check_poly_body` rebases the instantiator at entry, but the minted batch is
   flushed into `structs`/`enums` only after it returns, so for the duration of
   the walk any body-local generic instantiation carries an id past the end of
   the slices the walk holds. Every registry-indexing predicate reached from
   the walk is exposed, not just deviation 6's cross-call arm: `dup` on the
   same value panics identically through `poly_is_copy`
   (`: g ( 'T -- 'T ) 1 Box dup drop drop ;`), on a program with no cross-call
   at all, and it panics the same way at `ac4eb2a` -- so it is pre-existing and
   was left alone. Phase 2 gets the flushed registries for free (its fixpoint
   runs at `check.rs`, after every body), so it does **not** inherit the fix:
   closing this needs the *walk* to see pending mints, either as a merged
   read-only view or by restructuring the flush, and that is its own slice.
   Deviation 6's honest rejection is what keeps phase 1 inside N1 until then.

### Delivered, with five deviations from the plan above

1. **Finding 2 is answered by rejecting, not by keying on `(name, PolySig)`.** A
   cross-call into *or out of* a polymorphic overload set is a located
   rejection, and the second direction is why: `PolySig` would disambiguate the
   record map, but `CallInst::callee` is a bare name on both sides and nothing
   on it says which candidate it resolved to. The rejection also does not
   pretend to fix the overload set itself -- an overloaded non-inline generic
   word already mis-lowers today with no cross-call anywhere in the program
   (`poly_arities` and `driver`'s `poly_words` are name-keyed with last-wins, so
   `lower_poly_call` pops the wrong arity and panics on a subtract overflow).
   What this slice adds is that a cross-call inside or into one is refused at
   check time instead of reaching that panic.

2. **Finding 3 stands: a callee's user trait bound stays phase 1's located
   rejection.** The fixpoint does run where the resolution tables are in scope,
   but composing a callee's recorded obligations against a composed θ is a
   second mechanism, not a line -- `resolve_user_bound` writes into
   `CallInst::trait_calls` keyed by the *callee body's* spans, and a cross-call
   would have to compose those per caller instantiation the same way the
   instantiation itself is composed. Left for its own slice.

3. **The callee's declared *inputs* are not ground.** "Where discovery lives"
   step 2 says to ground inputs and outputs alike, for `apply_subst`'s
   interning. On the input side there is nothing to intern: `poly_cross_match`
   decomposes a compound input structurally and R6 rejects a compound the
   caller built itself, so a cross-call's input shapes mirror the caller's
   operand slots, which the caller's own instantiation already interned.
   Shipped and then deleted after mutation testing found it unkillable. The
   output grounding stays: it is where `out_arity`/`output_types` come from --
   but a code-review mutation test then showed the *`generics_cell`
   rebase/flush bracket* around it is also dead, for a stronger reason than
   the input side ever was: every declared output that reaches `compose`
   already passed phase 1's `poly_cross_output`, which rejects every compound
   shape (`Array`/`Ref`/`Generic`) at record time, so `apply_subst`'s only arm
   that would mint through the live instantiator can never run here. The
   bracket, `CrossGround::generics`, and the `Some(self.generics)` threaded
   into `word_ctx` are removed; `compose` grounds a `Var`/`Concrete` output
   with `ctx.generics() == None`, same as it always effectively did.

4. **`Module::transitive_instantiations` is sorted by symbol.** Discovery seeds
   from a `HashMap`, so without it the field's *order* would be randomized even
   though its content is not. Lowering sorts anyway; this is so a test reading
   the field does not have to.

5. **An `inline` callee whose own body calls a polymorphic word is a located
   rejection, found by code review.** The skip in finding 1 above is correct
   as far as it goes, but `h`'s own body is never walked by `check_poly_body`
   at all -- `check.rs`'s own-body loop excludes every combinator, checking
   one standalone instead (`check_poly_combinator_standalone`, every type
   variable stood in for a concrete dummy) -- so a call `h`'s body makes to
   another polymorphic word never reaches `poly_cross_calls`, and the
   fixpoint has nothing to compose even though lowering really does splice
   `h`'s generic body at the outer call site. Unfixed, this reached the exact
   panic N1 forbids (`f` non-inline, `g inline` calling `id`, `f` calling `g`)
   and, worse, a silent wrong-symbol splice when two callers reached the
   spliced body at different θ. `cross_calls_of` now runs a syntactic,
   one-level scan of the combinator's body terms (`body_calls_a_poly_word`,
   recursing only into `Quotation`) and rejects the outer call site if it
   names any polymorphic word -- conservative (an untaken branch still
   counts) rather than under-detecting. A call to a *further* `inline`
   combinator is itself a call to a polymorphic word, so this needs no
   explicit recursion through a chain of `inline` hops: each hop trips the
   same check the next time it is reached as a callee.

**One claim no test reaches.** A composed `CallInst` inherits the caller's
`generation`, and nothing can exercise it: the REPL hands the walk an empty
cross-call registry on purpose (phase 1's own note), so `poly_cross_calls` is
always empty there and the fixpoint returns before composing anything. Writing
`None` instead would be wrong the day the REPL grows its own composition step,
so the inheritance stays, unkilled.

**Findings 5 and 7 are untouched and still open**, as their own entries say:
R6's accept case for a fully concrete compound image is still an honest
rejection, and a polymorphic body's walk still sees registries that are stale
for the instantiations that body itself mints.

**Pre-existing, confirmed and left alone (found probing deviation 5's fix).**
An `inline` generic caller spliced at two different θ -- `: g inline ( 'T --
'T ) id ;` called at `i64` and at `str` from a concrete `main` -- mints one
`id` monomorph and segfaults at run time. Identical at `ac4eb2a`: the span-
keyed global instantiation map is last-write-wins across a splice's own call
sites, the same root family as finding 7, and out of this slice's scope.

**Split-signals re-check (CLAUDE.md, phase exit).** `poly.rs` grew again this
phase (9204 -> 10590 lines across the slice). Import divergence and
high/low-level mixing are still absent -- everything added shares
`word_ctx`/`apply_subst`/`intern_bundle_struct`/`is_combinator` with the rest
of the file -- but a third signal now fires that did not at phase 1: the
discovery/worklist code (`discover_transitive_instantiations`,
`CrossGround`, `cross_calls_of`, `compose`, `fixpoint`) is called only from
`check.rs`'s driver, never from any other walk function in `poly.rs` --
"functions in a file that never call each other." Two of the four signals is
below the split threshold; the prior "split deferred" call
([[project_poly_rs_split_deferred]]) still holds, revisit alongside it rather
than splitting phase 2's code out on its own.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "Checker: thread poly_env, symbolic variable-to-variable relation with consistency check, cross-signature bound discharge, growing-type and inconsistent-mapping call-site rejection, dead-code/test retirement; records a symbolic cross-call, emits no IL", "effort": "L", "difficulty": "hard" },
    { "phase": 2, "focus": "Check-time transitive instantiation fixpoint (compose + apply_subst interning + bundle interning) populating new CallInst.poly_calls and Module.transitive_instantiations, plus the thin driver.rs drain extension and func_builder poly_calls routing arm, plus run/IL and the named N2 regression baseline", "effort": "M", "difficulty": "hard" }
  ]
}
```
