# Phase 7 Slice 3e: user-declarable trait bounds (brief)

Open `Bound` (Phase 4 Slice 1) from a closed two-variant enum (`Copy`, `Ord`) satisfied by
a hardcoded predicate to a user-declarable one: `trait: Show 'T show ( 'T -- ) ;`, then
`: print-both ( 'T: Show 'T: Show -- ) | a b | a show b show ;`. No trait objects, no
runtime representation — a bound is satisfied at monomorphization by an ordinary word
resolving for the concrete type, and vanishes after checking.

## Recon (read against the built compiler; see "Resolved recon" and "Resolved: paper

dogfood" below for the two claims that have since been compiled and checked)

1. **`Bound` is a closed enum with the satisfaction predicate hardcoded at the call
   site**, not looked up polymorphically. `Bound::Copy | Bound::Ord`
   (`src/ast.rs:1032-1039`); the one place it is consulted, `check_poly_call`'s R6 loop
   (`src/check/poly.rs:1481-1497`), is a bare two-arm `match bound { Bound::Copy =>
   is_copy(...), Bound::Ord => is_ord(...) }`. Adding a third variant means this match
   grows a third arm; it does not need to become a lookup table, since there are only
   ever a handful of trait declarations to switch over — but see OQ2 for why "how many"
   matters.

2. **There are two independent checkers of a poly body, and they run at different
   times with different type information.** `check_poly_body`/`poly_term`
   (`src/check/poly.rs:271-`, `384-`) type-checks the body **once**, at declaration
   time, over abstract `PolyType::Var` slots with no concrete type in sight.
   `check_poly_call` (`poly.rs:1430-`) runs **per call site**, with a concrete `Subst`
   already unified, and is where R6's bound-satisfaction check lives today. A user
   bound needs a foothold in *both*: the body check needs to know `show` is a legal
   call on a bare `'T: Show` and what it returns, and the call-site check needs to
   confirm the concrete type instantiating `'T` actually has a matching `show`.

3. **The call-site half is nearly free — it is the same shape as R6 today, plus a
   lookup instead of a predicate.** `Bound::Copy`/`Bound::Ord` call `is_copy`/`is_ord`
   against the concrete type resolved by `subst`; a `Bound::User(trait_id)` arm calls
   something like `trait_satisfied_by(trait_id, concrete_ty, env)`, which walks the
   trait's required member list and confirms each name resolves in `env` for that
   concrete type with a matching `Sig`. `env: HashMap<String, Vec<Overload>>`
   (`Overload { sig: Sig, symbol: String }`, `src/check/builtins.rs:28-31`) already
   supports exactly this query — it's the same shape `resolve_overload`
   (`builtins.rs:37-`) already runs for ordinary static overloading (Phase 4 Slice 8).
   No new resolution mechanism, one new predicate function.

4. **The body half is the real work, and it is a new branch, not an extension of an
   existing one.** `poly_call_term` (`poly.rs:499-`) falls through, in order: locals,
   `&`-led words, `@`/`!`/`+!`, the five shuffles, `len`, comparisons (`Ord`-gated),
   then an `env` lookup keyed by exact name (`poly.rs:~715-760`) that requires every
   input slot be a **concrete** type — a bare `PolyType::Var` hitting a concrete-typed
   input parameter is unconditionally `poly_var_to_concrete_error`
   (`poly.rs:~750-758`, comment: "a bare variable passed to a concrete-typed argument
   is a located error"). Calling `show` on `'T: Show` needs a new branch *before* that
   env lookup fails: if the top of stack is `PolyType::Var(v)` and `sig.has_bound(v,
   Bound::User(trait_id))` names a trait requiring `show`, push the trait's *declared*
   abstract output types (not a concrete `Overload`'s), and record nothing else — no
   symbol, no site table — because the concrete resolution genuinely cannot happen
   yet; `'T` is still abstract here.

5. **Lowering a poly instantiation plausibly needs no new machinery at all, on the
   theory that once `Subst` is concrete the "trait" concept has already done its job.**
   Once `check_poly_call` has a concrete `θ(T)`, lowering a call to `show` inside that
   instantiation is — on the reading of `instantiation_symbol`
   (`src/ast.rs:1160-`) and `lower_instantiation` (`src/ir/driver.rs:707-`) so far —
   just an ordinary concrete-body call against the substituted stack: `show` resolves
   in `env` against `Sprite` exactly as it would in a hand-written monomorphic word,
   with no residual "this came from a trait bound" bit needed downstream. **This is
   the load-bearing claim of the whole design and is not yet probe-verified** — it
   rests on reading, not on compiling a two-instantiation example and checking the
   emitted symbols, which the project's own convention (mutation-test the guards,
   verify claims of verification) says not to trust yet. See OQ1.

6. **`PolySig.bounds: Vec<(u32, Bound)>` already threads through parsing.**
   `'T: Copy Ord` parses today (`src/parser.rs:2160-2166`, appending both `Bound::Copy`
   and `Bound::Ord`); a third keyword (a trait name) needs the parser to distinguish a
   *known bound keyword* from an *arbitrary trait identifier*, which the current
   two-token match (`c == "Copy"` / `c == "Ord"`) does not generalize to without
   knowing what trait names exist yet — plausibly needs the trait table threaded into
   the parser, or a two-pass resolution (parse as a bare name, resolve against
   declared traits in a later pass, the same shape struct/enum names already use).

7. **No existing declaration form is a template for `trait:`.** `type:`, `extern:`,
   `static:` (P7.S2) all declare a single concrete thing. A trait declaration lists
   *required word signatures*, closer in shape to an `extern:` block's list of bindings
   than to a `type:`'s field list, but nothing in `src/parser.rs`'s declaration dispatch
   has been read yet for how cheaply a new top-level keyword slots in. Flagged, not
   measured.

## Resolved recon (worker-verified against the built compiler, post-brief)

**OQ1 is resolved, and it falsifies Recon 5.** Two independent, compiled findings
(probe source and exact output kept in the resolving session; not reproduced
here in full):

- **The body-check half rejects the call before lowering is ever reached.** A poly
  word whose body calls an overloaded name (`+`) on a *bare* `'T 'T` pair — the
  literal shape a trait-bounded method call would need — fails today with a stack
  effect mismatch (`` `+` needs 2 values, but the stack holds 0 ``), because
  `poly_delegate_op`'s maximal-concrete-suffix extraction sees a suffix of length
  zero when both operands are `PolyType::Var`. This is Recon 4's
  `poly_var_to_concrete_error` barrier confirmed from the other side: there is
  currently no path by which a bare-variable-typed operand's call ever reaches
  lowering at all, trait-bounded or not.
- **The lowering mechanism Recon 5 named is structurally instantiation-invariant,
  not instantiation-aware.** `Module::builtin_overloads: HashMap<Span, String>`
  records one concrete symbol per call-site *span*, shared across every
  monomorphization of that poly word. Probed directly: a poly body calling `+` on
  a *concrete* `Vec2` (today's only working shape), instantiated asymmetrically at
  `'T=i64` and `'T=usize`, produces two distinct monomorph bodies
  (`sooth_mono_pair_sum__m0__t0_i64`, `..._usize`) that both `call` the *same*
  mangled `+` symbol — because `+`'s operand type (`Vec2`) never varied across
  instantiations, only `'T` did, and `'T` wasn't the dispatch key. A `trait:`
  method dispatched on `'T` itself needs the recorded symbol to *differ per
  monomorphization* (`Sprite`'s `show` vs. `i64`'s `show`), which a `Span →
  String` map cannot express — and R7 (`resolve.rs`) is explicit that lowering
  never re-runs resolution.

**Consequence:** Recon 5 and the roadmap entry's "no IR, lowering,
monomorphization-walk, layout, or backend change" claim are **wrong**. The slice
needs one of: (a) a per-instantiation overload record (keyed by something like
`(mono-symbol-or-Subst, Span) → String`, mirroring how `instantiations` are
already tracked per-monomorph), or (b) lowering re-resolving the bound method
against the concrete `Subst` at each `lower_instantiation` site — which is itself
a change to the "lowering never re-runs resolution" invariant R7 states today, so
option (b) needs that invariant's own owner to weigh in, not just this slice.
Either way, the IR/lowering budget is real and non-zero; the spec's exit criteria
and effort sizing both need to reflect this, not the brief's original claim.

## Open questions

Renumbered into one sequence; strikethrough items are resolved (by the OQ1 probe or
the paper dogfood) and kept for their history, not left open.

1. ~~Does lowering really need zero new machinery, per Recon 5?~~ **Resolved: no.**
   Replace with: which of the two mechanisms (per-instantiation overload record, or
   lowering-time re-resolution against `Subst`) does the spec commit to, and does the
   second option require renegotiating R7's "lowering never re-runs resolution"
   invariant with whatever else depends on it?

2. ~~Nominal or structural satisfaction?~~ **Resolved: nominal.** `impl: Show for
   Sprite ; : show ( &Sprite -- ) ... ; ;` is an explicit opt-in, checked against
   the trait's required member list at `impl:` declaration time. Orphan rule: an
   `impl:` block must live in either the trait's own defining module or the
   implementing type's own defining module, never a third module — the same
   restriction a module-scoped structural design would have needed anyway, but
   caught as a located error at the `impl:` site (naming the trait, the type, and
   both legal modules) instead of surfacing later as a silent "doesn't satisfy"
   at an unrelated call site with no declaration-level evidence trail. Two worked
   examples (structural-module-scoped vs. nominal, including each one's orphan-
   violation case) are kept in the resolving session, not reproduced here; the
   nominal shape is what Recon 6/7 must now be written concretely against.

3. ~~What is the required-member list allowed to mention?~~ **Resolved by the
   dogfood: single-type-variable traits only, and it is enough** — `Eq`/`Hash`/
   `Order` all close over one variable, no consumer forced a `zip`-style
   two-variable trait. **New sub-question the dogfood raised instead:** required
   members must be allowed to take `&'T`, not just `'T` (every real method needs to
   inspect without consuming), and Recon 4's body-side checker branch ("top of
   stack is `PolyType::Var(v)`") must also fire on "top of stack is a reference to
   a bounded var" — confirmed `&'T` parses in a poly signature today (fails later,
   at `@`, not at the signature), so this is a checker-branch change, not a parser
   one.

4. **New (dogfood): multi-bound member-name collisions.** If two bounds on one
   variable (`'K: Eq Hash`) each declared a same-named required member, nothing
   disambiguates it today. **Live, not hypothetical, now that `Map` is in scope**
   (`'K: Eq Hash` is exactly this shape). **Ruling for the spec to state explicitly:**
   a located rejection — a member name must be unique across a variable's whole bound
   set, reported where the bound set is declared, naming both contributing traits and
   the colliding member. Not left free: an unruled open question ships permissive.

5. ~~Does a bound need to compose?~~ **Resolved by the dogfood: yes, and it already
   works** — `sort`'s `'T: Copy Order` is a real, working confirmation; compose is
   just the existing `'T: Copy Ord`-style capability list, greedy over N names, not
   a fixed two-slot special case. Confirm the Recon 6 parser change doesn't regress
   this by hardcoding an arity of one or two.

6. **Where does a trait declaration live across a module boundary?** `import:`/
   `export:` already answer this for `type:`/`extern:`/words (Phase 4 Slice 5).
   Confirm a `trait:` declaration is just one more exportable declaration kind rather
   than needing its own cross-module story — plausibly true, not yet checked against
   `assemble_module`.

7. **Diagnostic wording for a failed bound.** `poly_copy_bound_error`/
   `poly_ord_bound_error` name the variable and the bound. A `Bound::User` failure
   should name the trait *and* the specific missing member (`` `Sprite` does not
   satisfy `Show`: no `show ( Sprite -- )` found ``), not just "does not satisfy
   `Show`" — diagnostics are behaviour on this project, and "which method is
   missing" is the actionable half of the message.

8. ~~Can a user trait be named `Copy` or `Ord`?~~ **Resolved: split namespace from
   satisfaction.** Neither "reserve the names" nor "demote the builtins into
   ordinary `trait:` declarations" — the second is not expressible, because `Copy`
   and `Ord` are not method-set traits (`is_copy` is a structural property of a
   type's shape; `is_ord` is `is_numeric`), so they cannot carry a required-member
   list. Instead: pre-seed the trait table with `Copy` and `Ord` as **predicate-kind**
   entries whose satisfaction still runs `is_copy`/`is_ord` unchanged, and make
   `parse_capabilities` (`src/parser.rs:2158-2183`) do a single table lookup rather
   than two hardcoded string compares. A user `trait: Copy ... ;` then fails with the
   ordinary **duplicate-declaration** error against the builtin, not a bespoke
   reserved-word check. This also removes the hardcoded-name path OQ5 warns about
   regressing bound composition. **Spec must state:** `Copy`/`Ord` are prelude-global
   names, while every user `trait:` is module-scoped and exportable per OQ6.

## Resolved: paper dogfood (worker-verified, `docs/roadmap/P7/slice3-dogfood.md`)

Hand-wrote `Map['K 'V]` (open-addressing, `fixed`-layer-shaped) and an array `sort`
against the sketched `trait:`/`'T: TraitName` surface. Full detail and both paper
programs are in `slice3-dogfood.md`; the load-bearing findings:

- **A second, independent blocker, verified by compiling:** a polymorphic word
  cannot *name* a generic type applied to one of its own type variables in its
  signature today. `: unbox ( Box['T] -- 'T )` and `: or-default ( 'T Option['T]
  -- 'T )` both fail with `` error: unknown type 'T ``, while an *array* carrying a
  type variable in a poly signature (`( ['T: Copy 4] 'T -- ['T 4] )`) already
  builds green. This is a Phase-5-shaped gap, orthogonal to trait bounds, and it
  means `Map['K 'V]`, `Entry['K 'V]`, and the `Vec['T]` form of `sort` are all
  unparseable regardless of what P7.S3e ships — the gap is spun out as its own
prerequisite, **P7.S3a** (`docs/roadmap/P7-language-prereqs.md`), not fixed here.
**The bounds feature currently has
  no consumer that compiles.**
- Trait members must take `&'T`, not `'T` (every real method — `eq`/`hash`/`cmp`
  — needs to inspect a stored value without moving it out); Recon 4's "top of
  stack is `PolyType::Var(v)`" sketch is too narrow and must also fire on "top of
  stack is a reference to a bounded var."
  Confirmed `&'T` parses in a poly signature today (fails later, at `@`, not at the
  signature).
- A hashed `Map` needs *two* required methods on one variable (`eq` and `hash`),
  confirming bound composition (`'K: Eq Hash`) must work, which is already implied
  by today's `'T: Copy Ord` and requires the parser change (below) not special-case
  "exactly one trait name."
- A user trait cannot be named `Copy` or `Ord`: `parse_capabilities`
  (`src/parser.rs:2158-2183`) matches those two string literals before any trait
  lookup could run, so a user trait sharing either name is permanently shadowed by
  the builtin unless the builtins are demoted into the same lookup table.
- Multi-bound method-name collisions are unhandled: if two bounds on one variable
  each declare a same-named required method, nothing disambiguates it; needs a
  located "member name must be unique across a variable's bound set" rule.
- **OQ3 answered: yes, single-type-variable trait scope is enough** for both
  consumers. Every trait needed (`Eq`, `Hash`, `Order`) closes over one variable;
  the `'K`/`'V` relationship in `Map['K 'V]` lives in the *struct*, not in any
  trait, and multi-capability needs are covered by *composing* several
  single-variable bounds on one variable, not by a multi-variable trait.
- `sort` needs `'T: Copy Order` together (compare needs only `Order`'s ref-based
  `cmp`; the in-place swap needs `Copy` because there's no non-Copy array-element
  swap primitive today) — a real, working confirmation of OQ4 (bounds compose).

**Consequence:** this is why the roadmap now splits the slice — **P7.S3a** (generic
instantiation over a poly word's own type variable) is a hard dependency of this
slice's own `Map` consumer, tracked and specced separately. This slice (**P7.S3e**)
can proceed independently against the array form of `sort`, which types structurally
today with no dependency on S3a, while `Map` waits for S3a to land.

## Out of scope

- Trait objects / dynamic dispatch (`dyn Show`, `^Any`, erasure). Fully compile-time
  only; see the P7.S3e roadmap entry's own framing.
- Associated types, default method bodies, blanket impls, supertraits, generic
  constants. None of these have a named consumer yet (`Map`/`Vec`'s `Eq`/`Ord` need
  is the only forcing pressure); do not build them speculatively.
- Multi-type-variable traits (OQ3) unless the paper dogfood in "Ready to spec?" finds
  a real need.

## Ready to spec?

**Not yet — and for a stronger reason than "unverified." Both pre-spec checks ran,
and both came back with real findings that change the shape of the slice, not just
confirmations.**

- The paper dogfood found a **blocker outside this slice's own control**: generic
  types applied to a poly word's type variable don't parse today
  (`docs/roadmap/P7/slice3-dogfood.md`, finding #5), so `Map['K 'V]` — the slice's
  own stated forcing consumer — is not writable regardless of what P7.S3e ships.
  **Split out as P7.S3a**, its own roadmap entry — this slice now specs against the
  array form of `sort` only, with `Map` deferred until S3a lands.
- The OQ1 probe found that **Recon 5's central cost claim is false**: lowering does
  need new machinery (a per-instantiation-aware overload record, or a change to the
  "lowering never re-runs resolution" invariant), not none. The spec's effort sizing
  and exit criteria both need to price this in before implementation starts, not
  discover it mid-slice.

**No longer ready to spec — a spec was written (`slice3e-spec.md`) and then falsified by
probing, and finding 1 below was itself later falsified by a second probe.** Three
findings, in descending order of consequence:

1. **No consumer compiles — corrected.** The original claim here was that the array form
   of `sort` needs branching, so it must be `inline`, so it mints no monomorph symbol for a
   per-instantiation dispatch record to key on. That claim is false: **P7.S3b** shipped
   eliminator-arm branching in a non-inline poly word (`Ordering?`-style), probe-verified to
   compile and mint `sooth_mono_pick__m0__t0_i64`. The real remaining gap is narrower and is
   not branching at all: a **rowless quotation-consumer splice** — calling a fully concrete
   `~[ &'T &'T -- Ordering ]` (a comparator, no `..a`/`..b`) inside a poly body is still
   rejected (`` `call` on a quotation ... is not yet supported ``), distinct from the
   row-typed `if`/`branch`/`times`/`tag` family S3b deferred to S3b-follow. This is now its
   own slice, **P7.S3d**, inserted ahead of this one. `Map['K 'V]` is blocked separately: a
   generic struct whose field is an array of its own type variable (`keys ['K 8]`,
   `slots [Ent['K 'V] 8]`) fails with `` error: unknown type 'K `` — a third gap, distinct
   from S3a's. The `Map` scope widening recorded below is therefore withdrawn.
2. **The dispatch key is sound, but only for a leaf word.** Check-time and lowering-time
   monomorph symbols are byte-identical (probe: `sooth_mono_idc__m0__t0_i64`/`_bool` at both
   sites, and across modules), so the mechanism works. But a bounded poly word calling
   *another* poly word fails at check today (`` error: unknown word `inner__m0` ``), and
   `module.instantiations` is keyed by bare call-site span with nested poly calls explicitly
   out of scope, so a nested obligation has no coherent key. The slice must either restrict
   bounded bodies to leaf calls with a located rejection and a guarding test, or specify
   obligation propagation (which means re-keying `instantiations`, a much larger change).
3. **The illustrative bound syntax in the spec is wrong.** A bound rides on the variable's
   *first occurrence*, which is itself an input slot, so `( 'T: Show &'T &'T -- )` declares a
   spurious bare `'T` input. The intended two-reference form is `( &'T: Show &'T -- )`.
   Illustrative only, but it would propagate into golden signatures.

**Also to settle before a re-spec: a third trait kind.** The spec has predicate-kind
(`Copy`/`Ord`) and member-kind (user) traits. A *compiler-known, library-declared* trait is
neither, and is probably wanted: it would let intrinsic compiler logic be written against a
library implementation. `bool` is already this shape — a library-declared enum known by a
reserved registry position (`src/ast.rs:779`), with its `.` overload injected (`:816`). A
`Fallible`-style bound satisfied by `Result`/`Option` would give fallible slice indexing
(S3c), a failing allocator (S5), and S8's fallible push one shared desugaring. **Test it
first:** a trait is only justified with two or more carrier types, or if users can add their
own; with a single carrier this should be a lang *type* like `bool`, not a lang trait.
**Test before S3c locks its index-failure carrier, not after** — this decides whether
fallible slice indexing returns a plain `Option['T]` or rides a bound.

The decisions below are still good, and survive the falsification:

- **OQ2 — nominal satisfaction** via `impl: Trait for Type ; ... ;`, with an orphan
  rule confining an `impl:` block to the trait's or the type's own defining module.
- **OQ8 — predicate-kind trait-table entries** for `Copy`/`Ord`, one lookup path in
  `parse_capabilities`, duplicate-declaration error on collision.
- **OQ4 — located rejection** of a member name colliding across a variable's bound set.

**Consumer scope: withdrawn, pending P7.S3d (rowless quotation-consumer splice).** S3b
already supplies the branching `sort`'s comparator dispatch needs (`Ordering?` elimination);
the one remaining wall is calling the comparator quotation itself from inside the poly body.
`Map` stays separately blocked on the generic-struct array-of-own-type-variable field gap.
The multi-method-bound collision rule keeps a consumer regardless, since its rejection
golden needs only two hand-declared traits and no collection.

OQ1 is settled: a per-instantiation dispatch record, populated at check time and read at
lowering, so "lowering never re-runs resolution" stands. Probing confirmed the key matches
across that boundary. What is *not* settled is the nested case (see finding 2 above).
