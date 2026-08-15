# Phase 4 Slice 13: `PolyType::Ref` — borrows inside a generic word (spec)

A plain (non-combinator) `'T`-bounded word gains the ability to borrow: `&x`/`&!x`
at its own top level, and a signature slot that borrows a still-generic type
(`&'T`, `&['T 4]`, `&!...`). Today the borrow family (`&x`, `&!x`, `&>`, `&^`,
`&Struct>field`, `@`, `!`) lives only on the monomorphic side of the checker;
`PolyType` (`src/ast.rs:623`) has no way to name "a reference whose referent is
still `'T`", and `poly_call_term` (`src/check/poly.rs:368`) has no branch that
recognises a leading `&`. Combinators dodge the gap because they are spliced and
monomorphized per call site before their body is ever checked; a *plain* generic
word is checked once, abstractly, and lowered per instantiation with **no concrete
re-check** — so anything the borrow needs (representation, aliasing, liveness) must
be modelled abstractly here or it is modelled nowhere.

All claims below are anchored to `main` at `f740f78` and verified against current
source. Backend is QBE; IR stays backend-neutral (`Ptr[T]` opaque, a `Type::Ref`
is a `RefId` handle, never a `u64`); `core` stays `no_std`; the linear spine is
untouched — a reference is neither `Copy`-obligated nor linear (the drop obligation
stays on the referent, exactly as `is_copy` already treats `Type::Ref`).

Two dependency-ordered parts, in the brief's lettering:

- **A** — representation and threading. Add `PolyType::Ref(Box<PolyType>, bool)`
  (referent + mutability), parse `&`-led poly slots, and thread the new variant
  through every exhaustive match and the two grounding steps. No new runtime
  capability: a signature can *declare* a poly borrow, but a body still cannot
  *produce* one.
- **B** — production and checking. Teach `poly_call_term` to recognise a leading
  `&`, produce a `PolyType::Ref`, and run the same borrow guarantees (exclusivity,
  use-after-move) the monomorphic checker enforces, plus the minimal consumer set
  (`&>`/`&!>`, `@`, `!`) that makes a borrow runnable end-to-end.

## Settled open question (OQ1 from the brief)

**OQ1 — does a poly body's borrow get the same aliasing/exclusivity/liveness
checks as the monomorphic checker, or is that deferred to the monomorphized
instantiation?**

**Answer: the poly body enforces them, at check time, in the poly body itself.
Deferral is unsound and is rejected.**

The deferral option assumes a plain poly word's body is re-checked concretely at
each instantiation. It is not. Only a *combinator* body is spliced and re-checked
per call site; a plain poly word is checked once, abstractly, by
`poly_walk`/`poly_term`/`poly_call_term` (`src/check/poly.rs:239`), then lowered
per instantiation by `subst_polytype` (`src/ir/driver.rs:591`), which grounds types
and never re-runs the checker. So a borrow hazard the poly body does not catch is
caught by nothing — a silent hole in the load-bearing linear spine (aliased mutable
borrow, use-after-move borrow). That is exactly the class of "Forth's silent
failure" this project exists to turn into a compile error.

The hazards are checkable abstractly. Exclusivity ("two live borrows of one place,
at least one mutable") and use-after-move ("borrowing a local already consumed")
are facts about the *term order* of the body, independent of the concrete types a
variable is instantiated at. `&!a ... &!a` is a double mutable borrow at `[i64 4]`,
at `[Point 4]`, at every instantiation; the poly body sees the same term sequence
each concrete twin sees. So the checks not only *can* run abstractly, they lose
nothing by doing so.

**Locked minimum: soundness-equivalence.** Every program the monomorphic checker
rejects for a borrow reason (conflicting borrow, aliased place, use-after-move on a
borrow) must also be rejected in the poly body, with an equivalent located
diagnostic. **Target: acceptance parity** — reuse the existing `Provenance`
(`src/check/engine.rs:119`) and `Liveness` (`:621`) machinery threaded into
`PolyScope`, so an accepted poly program is exactly one every concrete twin accepts.

**Permitted fallback, if Liveness pre-computation for the poly body proves out of
scope for this slice:** a conservative rule held in `PolyScope` — a `&!`/`&` borrow
of a local is treated as live until that local is consumed or the word ends, and a
second borrow of the same local conflicts under the mono mutability rule (a new
mutable borrow conflicts with any live borrow; a new shared borrow conflicts with a
live mutable one). This over-approximates (it may reject a mono-accepted program
where the first borrow was provably dead), but it never *under*-rejects, so it
preserves the locked minimum. Any such over-rejection must be pinned by a test that
documents it as an intentional conservative bound, not left as an accidental
divergence. This is a fidelity choice, **not** a softer diagnostic tier: the rule
is a real compile error either way; the question is only how many valid programs it
additionally refuses.

Shared borrows alone (Part B phase P2, before `&!` exists) need neither `Provenance`
nor `Liveness`: with no mutable borrow reachable, two shared borrows of one place
never conflict, and use-after-move is already answerable from `PolyScope.moves`
(`scope.moves.moved_site`), which the poly checker already owns. The `Provenance`
threading is therefore isolated to phase P3, alongside `&!`.

## Design decisions

**D1 — `PolyType::Ref(Box<PolyType>, bool)`: referent then mutability.** Mirrors
`Type::Ref(RefId, bool, &'static str)` (`src/ast.rs:857`) minus the interned
handle and spelling. There is deliberately no `RefId`: the referent may be a
variable (`&'T`), which no registry entry can name; the `RefId` is minted only when
the referent grounds to a concrete `Type` (D5). Mutability rides the variant (not
just a wrapped inner type) because it is the *classification* bit — `is_copy`,
store-vs-fetch, exclusivity — asked at sites that hold no registry, exactly the
reason `Type::Ref` carries it too.

**D2 — a poly `&`-led slot folds like an array.** Add `RawTy::Ref(Box<RawTy>, bool)`
(`src/parser.rs:663`). `parse_poly_slot` (`:1388`) intercepts a leading-`&`/`&!`
word **before** its `parse_type_expr()` fallthrough (`:1436`), splits the sigil
(`&`/`&!`, one or two chars — reuse the logic in `parse_ref_type_expr:1811`), and
recurses into `parse_poly_slot` on the referent so `'T`/`['T 4]` resolve as poly
slots rather than concrete types. `raw_to_poly_type` (`:1622`) folds
`RawTy::Ref(inner, m)` to `PolyType::Concrete(Type::Ref(..))` via
`crate::ast::intern_ref_type` (`src/ast.rs:461`) when `inner` folds fully concrete,
else to `PolyType::Ref` — the same "fold to `Concrete` when nothing is variable"
discipline `Array` and `Quotation` already follow. This is the real fix to the
brief's recon 2: `parse_type_expr` *does* dispatch `&`-led words
(`parse_ref_type_expr`), but resolves the referent concretely, so `&'T` in a slot
dies on `'T` as an unknown type before the fold ever runs; intercepting in
`parse_poly_slot` routes the referent through poly resolution instead.

**D3 — a reference's `Copy`-ness tracks mutability, not the referent.**
CORRECTED after review: the monomorphic `is_copy` (`src/check/builtins.rs:233`,
*not* `src/ast.rs` — that anchor was wrong) answers `Type::Ref(_, mutable, _) =>
!mutable`: a **shared** reference is freely duplicated (the exclusivity rule has
nothing to protect), a **mutable** reference is not (duplicating it would let two
names observe/mutate through one exclusive borrow, exactly what `dup`'s Copy gate
exists to prevent). `poly_is_copy` (`src/check/poly.rs:14`) must answer
`PolyType::Ref(_, mutable) => !mutable`, mirroring this exactly — **not**
unconditionally `true`. The referent's own Copy-ness is still irrelevant (a
`&[Point 4]` is Copy even though `[Point 4]` is linear), so the arm ignores the
referent and inspects only `mutable`. Consequence: `poly_copy_gate` (`:590`) now
has a reachable error arm for a mutable ref — `dup`/`over` on a live `&!x` must
reject with a located diagnostic (R-A5/R-B5-adjacent; text pinned below), not
`unreachable!()`. Getting this wrong is not cosmetic: unconditional-Copy would let
a poly body freely `dup` an exclusive borrow, an acceptance a monomorphic
instantiation of the same source would reject — a genuine soundness regression,
not merely an inconsistent diagnostic.

**D4 — grounding interns.** Both `apply_subst` (`src/check/poly.rs:1184`, check
side, output-type grounding) and `subst_polytype` (`src/ir/driver.rs:591`, lowering
side) gain a `Ref` arm that grounds the referent recursively, then produces
`Type::Ref` via `intern_ref_type`. On the check side, `apply_subst` already threads
`arrays: &mut Vec<ArrayDecl>` (mutable, since a poly body can construct a shape
not yet interned); add the analogous `refs: &mut Vec<RefDecl>` so a `Type::Ref`
not yet seen at this instantiation can be interned here. **LOCKED (resolves a
review-flagged D4/R-A8 inconsistency): `subst_polytype`'s `refs` parameter is the
immutable `refs: &[RefDecl]`, matching R-A8 exactly, because by the time lowering
runs, check-side `apply_subst` has already interned every `Type::Ref` this word's
instantiations can produce** — lowering only looks one up by position (as the
existing array-shape arm already does for `arrays`), it never mints a new one.
If Phase 1's tests show a lowering-side lookup miss (a referent `RefId` not
already interned by check time), that is itself a bug in `apply_subst`'s
coverage, not a reason to widen `subst_polytype`'s parameter — flag and fix
the check side rather than loosening the lowering side's contract.

**D5 — the borrowable poly local is an aggregate.** As in the monomorphic
`check_reference_word` (`src/check/word_families.rs:12`, the aggregate gate at its
tail: `Struct | Enum | Array | OwnedCell`), a prefix borrow `&x`/`&!x` is legal only
on a local whose `PolyType` is an aggregate: `PolyType::Array(..)` or a
`PolyType::Concrete(_)` that is one of those four. A **bare-variable** local
(`PolyType::Var`) is rejected with a located error — a `'T` might instantiate to a
scalar, which is not borrowable, so the conservative rule refuses it uniformly
rather than deferring an "is it an aggregate?" question to instantiation. This keeps
Part B's blast radius bounded to the array-shaped case, which is the only poly
aggregate with a *variable* referent (there are no generic structs/enums this slice).

**D6 — element access needs a concrete length.** `&>`/`&!>`
(`check_reference_word`'s `">"` arm) statically bounds-checks the index against the
array's known `count` (`check_array_index`). A fully-generic-length array `['T 'N]`
has no known count, so its element cannot be statically bounds-checked and `&>` on
it is a located error. Element access is available only on a concrete-length,
generic-element array (`['T 4]`). This is the honest capability this slice unlocks:
the prefix borrow `&a` works for `['T 'N]`, but indexing into it does not. `'N`-length
element access is a documented gap (a dependent-bounds problem, its own slice).

## Codebase map

The variant is matched exhaustively across the checker and IR. Each site below is a
real "what does a poly reference mean at this stage" decision; the compiler will
flag any additional non-exhaustive match, and those are mechanical (recurse the
referent or answer `false`/`None`). Review-verified: every anchor below resolves to
the right function; a few (`poly_op_on_variable_error`'s match vs. its `fn` line,
`audit_poly_input_quotation`, `reject_poly_quotation_anywhere`,
`collect_poly_concrete`) are off by single-digit lines against the exact arm cited.
Treat every line below as ±15 and grep the named symbol rather than trusting the
exact digit.

- `src/ast.rs:623` — `PolyType` (add `Ref`); `:857` — `Type::Ref` (unchanged, the
  monomorphic target); `:461` — `intern_ref_type` (reused for grounding).
- `src/parser.rs:663` — `RawTy` (add `Ref`); `:1388` — `parse_poly_slot`
  (interception); `:1622` — `raw_to_poly_type` (fold); `:1811` —
  `parse_ref_type_expr` (sigil-split logic to reuse).
- `src/check/poly.rs:14` — `poly_is_copy` (D3); `:368` — `poly_call_term` (Part B
  dispatch); `:561` — `poly_var_id` (covered by its `_ => None`; a ref is not a bare
  variable — assert, no code change); `:590` — `poly_copy_gate` (unreachable arm);
  `:1049` — `unify_poly_input` (a declared `&`-slot unifies against a concrete
  `Type::Ref`); `:1184` — `apply_subst` (grounding); `:1296` — the describer inside
  `poly_op_on_variable_error` (`"a reference"`); `:1463` — `poly_type_str`
  (`&`/`&!` + referent rendering).
- `src/check/audits.rs:284` — `audit_poly_input_quotation` and `:322`
  `reject_poly_quotation_anywhere` (recurse the referent, so a quotation buried in a
  ref's referent still cannot slip past the default-deny).
- `src/check/declarations.rs:294` — `collect_poly_concrete` (recurse the referent so
  export-privacy still sees a private type named behind a `&`).
- `src/check/combinators.rs:197` — the `is_combinator` parameter test (a `&`-slot is
  not a combinator parameter → `false`; confirm the match arm / wildcard).
- `src/ir/driver.rs:591` — `subst_polytype` (grounding, +`refs` param, D4).
- `src/repl.rs:220` — `remap_poly_type` (recurse the referent across REPL
  generations; the poly ref carries no `RefId`, so `ref_base` is unused for it).
- `src/check/engine.rs:119`/`:330`/`:621` — `Provenance`/`Provenance::borrow`/
  `Liveness` (reused by Part B phase P3 for exclusivity).

## Requirements

Requirement ids are stable handles for code comments and tests.

### Part A — representation and threading

**R-A1 (AST).** Add `PolyType::Ref(Box<PolyType>, bool)` to
`src/ast.rs:623` with a doc-comment stating D1 (referent + mutability, no `RefId`
because the referent may be a variable). No new `Type` variant — the monomorphic
`Type::Ref` already exists and is the ground form (invariant S1 from slice 6a: the
variable forms live only in `PolyType`).

**R-A2 (parser, raw form).** Add `RawTy::Ref(Box<RawTy>, bool)` to
`src/parser.rs:663`.

**R-A3 (parser, interception).** In `parse_poly_slot` (`:1388`), before the
`parse_type_expr()` fallthrough at `:1436`, add an arm for a leading-`&` `Token::Word`,
split into two genuinely different cases (CORRECTED after review — the lexer's only
word delimiters are `; ( ) | [ ]`, so `&`/`&!`/`'` are never delimiters and "recurse
on the referent remainder" is not literally possible in both cases):

- **Bare sigil** (`&['T 4]`, `&![T 4]`): the sigil word (`"&"`/`"&!"`) is followed
    by a genuine next token (`[`, or a plain word). Recurse `parse_poly_slot` on that
    *following token*, exactly as written.
- **Glued sigil+variable** (`&'T`, `&!'T`): `'T` is not a separate token — it lexes
    as one glued `Token::Word("&'T")`/`Token::Word("&!'T")`, a substring, not a
    token to recurse on. Split the sigil off the *string* (as `parse_ref_type_expr`
    already does), then intern the remainder as a type variable inline the same way
    the existing bare-`'T` arm does (`builder.intern_ty_var`, `:1406`) — this is new
    code, not a reuse of `parse_ref_type_expr` (which resolves its remainder via
    `resolve_type`, concrete types only, and has no poly-slot equivalent to borrow).

  Both cases return `RawTy::Ref(Box::new(inner), mutable)`. A bare `&`/`&!` with no
referent (`--` follows) is the same located "no referent" error `parse_ref_type_expr`
already emits. R-A10's `&'T` witness exercises the glued case specifically — it is not
covered by the bare-sigil path.

**R-A4 (parser, fold).** In `raw_to_poly_type` (`:1622`), fold `RawTy::Ref(inner, m)`:
fold `inner` first; if it is `PolyType::Concrete(t)`, return
`PolyType::Concrete(intern_ref_type(self.refs, t, m))`; else return
`PolyType::Ref(Box::new(inner), m)`. Mirrors the array/quotation concrete-fold.

**R-A5 (Copy).** `poly_is_copy` (`:14`): `PolyType::Ref(_, mutable) => !mutable`
(D3, corrected — **not** unconditionally `true`). `poly_copy_gate` (fn at `:573`,
the target arm ~`:604`): a shared ref falls through the existing `true` path
unchanged; a mutable ref emits a new located error rather than `unreachable!()`
(pinned text below, R-B8 adds the mutation-checked negative: `dup`/`over` on a
live `&!x` rejects, `&x` accepts). Both arms must change together — fixing only
`poly_is_copy` while `poly_copy_gate` keeps `unreachable!` turns a clean
diagnostic into an ICE on the very case D3 exists to reject.

**R-A6 (unification).** `unify_poly_input` (`:1049`): a declared `PolyType::Ref(rp, m)`
parameter unifies against a concrete slot `Type::Ref(id, cm, _)` only when `m == cm`;
recover the concrete referent (`ref_parts`/`refs[id]`) and recurse
`unify_poly_input` on `(rp, referent)`. A mutability mismatch, or a non-ref slot, is
a located type-mismatch — never a silent bind — reusing the existing mismatch
diagnostic shape.

**R-A6a (unification, plumbing — CORRECTED, added after review).** Unlike
`apply_subst`/`subst_polytype` (R-A7/R-A8, which already call out their own
signature change), `unify_poly_input` needs a `refs: &[RefDecl]` parameter added
too — `ref_parts` requires it and `unify_poly_input` currently only takes
`arrays`. This is **mandatory in Phase 1**, not implicit: grep every call site of
`unify_poly_input` (at last count, ~11 across `src/check/combinators.rs` and
`src/check/poly.rs` itself) and thread `refs` through all of them. Left
unstated, an implementer writing R-A6's arm hits a compile error with no `refs`
in scope and has to discover and thread the parameter unassisted; that discovery
work is what this sub-requirement makes explicit.

**R-A7 (grounding, check side).** `apply_subst` (`:1184`): ground `Ref(rp, m)` by
recursing to a `Type` referent, then `intern_ref_type(refs, referent, m)`. Add the
`refs` handle to `apply_subst`'s signature alongside `arrays`.

**R-A8 (grounding, lowering side).** `subst_polytype` (`src/ir/driver.rs:591`):
add a `refs: &[RefDecl]` parameter; the `Ref(rp, m)` arm grounds the referent, then
finds the interned `(referent, mutable)` `RefId` (by position, as the array arm does)
and returns `Type::Ref`. Update the (few) callers to pass `refs`.

**R-A9 (diagnostics + audits + collectors).** Add the `Ref` arm to: `poly_type_str`
(`:1463`, render `&`/`&!` then the referent's `poly_type_str`); the `what` describer
in `poly_op_on_variable_error` (`:1296`, `"a reference"`);
`audit_poly_input_quotation` (`audits.rs:284`) and `reject_poly_quotation_anywhere`
(`:322`) — recurse the referent so a quotation nested behind a `&` is still rejected;
`collect_poly_concrete` (`declarations.rs:294`) — recurse the referent;
`remap_poly_type` (`repl.rs:220`) — recurse the referent, mutability verbatim.
`poly_var_id` (`:561`) needs no arm (its `_ => None` already answers "not a bare
variable"); assert this with a unit test rather than editing it.

**R-A10 (exit, Part A).** A poly word may *declare* a borrow: `: peek ( ['T 4] -- &['T 4] )`
parses, and its signature round-trips through `poly_type_str` as `&['T 4]`. Producing
the borrow (a body `&a`) is still an unknown-word error at this point — Part A adds no
`poly_call_term` dispatch. Unit tests beside the parser (fold of `&'T`, `&['T 4]`,
`&!'T`; the concrete-referent fold to `Concrete(Type::Ref)`) and beside `poly_type_str`.

### Part B — production and checking

**R-B1 (dispatch).** In `poly_call_term` (`src/check/poly.rs:368`), after the plain
local-name lookup block and before the shuffle `match`, intercept
`name.starts_with('&')` and route to a new poly-side borrow function
(`poly_reference_word`), mirroring how `check_reference_word` fronts the monomorphic
family. A non-`&` name falls through unchanged.

**R-B2 (prefix borrow).** `poly_reference_word` handles a bare prefix borrow
`&x`/`&!x`: strip the sigil, require `x` be a local (else the existing
"not a local" located error), reject a quotation local
(`reject_quotation_operand`-equivalent) and a bare-variable local (D5, a new located
"cannot borrow a type-variable local; only an aggregate is borrowable" error).
Borrowing is not a move: `x` stays live. Push `PolyType::Ref(Box::new(local_pt), mutable)`.
Use-after-move: if `x` was already consumed (`scope.moves.moved_site`), reject with
the poly use-after-move diagnostic (`poly_use_after_move_error`).

**R-B3 (array element ref).** `&>`/`&!>` on a `PolyType::Ref(Array(elem, Len::Concrete(n)), m)`
receiver and a literal index: bounds-check the index against `n` (reuse
`check_array_index`), require receiver mutability `== m`, push
`PolyType::Ref(elem, m)`. A `Len::Var` array is the D6 located error ("cannot index a
generic-length array"). This is the sole accessor in scope; `&^` (owning-cell) and
`&Struct>field` in a poly body are **out of scope** and emit a located
"not yet supported in a generic body" error (R-B6), never a silent fallthrough.

**R-B4 (fetch/store).** `@` on a `PolyType::Ref(rp, _)` gated on `poly_is_copy(rp)`
(a Copy referent — a bare variable needs its `Copy` bound, reusing the X7 wording):
consume the ref, push the referent `PolyType`. `!` on `PolyType::Ref(rp, true)` plus a
top value unifying with `rp`: consume both, push nothing (`( &!T T -- )`). `@` lands in
phase P2, `!` in P3 with the mutable path. `+!` is out of scope (R-B6).

**R-B5 (exclusivity + liveness).** Per OQ1: a `&!x` conflicts with any live borrow of
`x`; a `&x` conflicts with a live `&!x`; a borrow of a consumed local is
use-after-move. Phase P2 (shared only) needs no `Provenance` — no mutable borrow is
reachable, so no two borrows conflict, and use-after-move is `scope.moves`. Phase P3
adds `&!` and the exclusivity check, reusing `Provenance`/`Liveness` threaded into
`PolyScope` (target) or the conservative `PolyScope`-local "borrow live until consumed
or word-end" rule (permitted fallback, soundness-equivalent per OQ1). Every
mono-rejected borrow program must be rejected here with an equivalent located message;
any conservative over-rejection is pinned as intentional.

**R-B6 (out-of-scope, located not silent).** `&^`, `&Struct>field`, and `+!` in a
generic body each emit a located "not yet supported in a generic body" error, matching
the eager-rejection style `poly_term` already uses for a quotation/array-constructor in
a poly body (`src/check/poly.rs`, the `TermKind::Quotation`/`ArrayCtor` arms). No
silent fallthrough to an unknown-word error.

**R-B7 (lowering + run).** A plain generic word that borrows lowers through the
existing monomorphization path: `subst_polytype` (R-A8) grounds the signature refs,
and the body's `&a`/`&>`/`@`/`!` lower via the already-existing monomorphic
`Instr`/`func_builder` reference machinery once instantiated at a concrete type
(the concrete twin already compiles and runs). No new IR opcode.

**R-B8 (exit witnesses, Part B).** Runnable goldens (source in → stdout out), each
also compiled at its concrete twin to prove the borrow lowers, not just checks:

- *Read* (phase P2): `: first ( ['T: Copy 4] -- 'T ) | a | &a 0 &> @ ;` with a `main`
  that builds `[ 10 20 30 40 ] first` and prints `10`. Asserts the computed value, not
  "exit 0".
- *Write* (phase P3): a generic in-place element set through `&!a ... &!> ... !`,
  returning the array; `main` reads the mutated element back and prints it.
- *Copy-gate negative* (P1, E1): `dup`/`over` on a live `&!x` rejects with E1's text;
  `dup`/`over` on a live `&x` (shared) still accepts, as a positive control.
- *Exclusivity negative* (P3, E6): two live `&!a` reject **at the second borrow site**
  with E6's text (asserted on the message and the site, not merely "rejected
  somewhere"; if P3 ships the conservative fallback, assert its exact
  fallback-suffixed text instead).
- *Use-after-move negative* (P2, E5): borrowing a local consumed earlier rejects with
  E5's text (`poly_use_after_move_error`, unmodified).
- *D5 negative* (P2, E2): `&t` where `t` is a bare-`'T` local rejects with E2's text.
- *D6 negative* (P2, E3): `&>` on a `['T 'N]` receiver rejects with E3's text.
- *R-B6 negative* (P2, E4): `&Struct>field` (or `&^`) in a generic body rejects with
  E4's text, parametrized on the operator.
- *Parser negatives* carried from Part A: `&q` (quotation local) rejected; a mutability
  mismatch at a `&`-slot unification rejected.

Assertion shapes are load-bearing (a placebo hazard this project has shipped before):
the value goldens assert the *number*, the negatives assert the *message and site*, and
each positive is mutation-checked by deleting the arm it exercises and confirming the
test then fails.

## Located error definitions

ADDED after review (CLAUDE.md: "diagnostics are behaviour ... the errors are part
of the spec"; slice 12's spec pinned exact E-code text for the same reason, and
this section was originally missing here). Each entry below is load-bearing: the
R-B8 goldens assert against this exact wording, not "rejected somewhere," so an
implementer cannot write the goldens to whatever string they happen to emit.

- **E1 (D3/R-A5) — dup/over of a mutable poly reference.** Reuses the existing
  `poly_copy_body_error` shape (`src/check/poly.rs:1265`) verbatim, since a
  mutable `&!x` failing `Copy` is the same class of fact as a bare non-`Copy`
  type variable failing it: `cannot`{op}` ... {var} has no `Copy`bound` — with
  `{var}` rendered as the reference's `poly_type_str` (e.g. `&!T`) rather than a
  bare variable name, and the note adjusted to name exclusivity instead of a
  missing bound: `error: cannot`{op}` a mutable reference in `{word}`(line {N})`
  / ` a mutable reference is not `Copy`: duplicating it would let two names
  observe or mutate through one exclusive borrow`.
- **E2 (D5/R-B2) — borrowing a bare type-variable local.** Mirrors
  `borrow_of_scalar_local_error`'s shape (`src/check/word_families.rs:1074`):
  `error: cannot borrow the local`{name}` of type `{var}` in `{word}`(line {N},
  col {N})` / `  `{var}`might instantiate to a scalar, which has no address;
  borrow an aggregate (a struct, enum, array, or owning cell) instead`.
- **E3 (D6/R-B3) — `&>`/`&!>` on a generic-length array.** New wording (no
  existing analogue — a concrete array is never generic-length):
  `error: cannot index a generic-length array in`{word}`(line {N}, col {N})` /
  ` the array's length is the type variable `{N_var}`, so its element cannot be
  statically bounds-checked; index a concrete-length array (`['T 4]`), or use a
  fixed length in this word's signature`.
- **E4 (R-B6) — an out-of-scope accessor (`&^`, `&Struct>field`, `+!`) in a
  generic body.** One shared wording, parametrized by the operator, matching the
  eager-rejection style `poly_term` already uses for a quotation/array-constructor
  in a poly body: `error:`{op}` is not yet supported in a generic body, in
  `{word}`(line {N})` / ` monomorphize this word (or write a concrete wrapper)
  to use `{op}`today`.
- **E5 (R-B2, use-after-move) — borrowing a consumed local.** Reuses
  `poly_use_after_move_error` (`src/check/poly.rs:1257`) verbatim, unmodified —
  a borrow of a moved-from local is exactly the existing use-after-move fact, not
  a new one.
- **E6 (R-B5, exclusivity negative) — two live mutable borrows, or a shared
  borrow live against a mutable one.** Target design reuses the monomorphic
  `conflicting_borrow_error` shape (`src/check.rs:1874`) verbatim: `error:
  conflicting borrow of`{place}` in `{word}`(line {N})` / ` a {new} borrow
  conflicts with a live {held} borrow of `{place}``, with the same projected-path
  note when applicable. If Phase 3 takes the conservative `PolyScope`-local
  fallback instead of threading `Provenance`/`Liveness` (R-B5's permitted
  fallback), the message must say so explicitly — e.g. append `note: this
  borrow's exact lifetime is not tracked in a generic body; it is conservatively
  treated as live until consumed or the word ends` — so a false-positive rejection
  is legible as conservative-by-design rather than a checker bug, and the R-B8
  negative asserts whichever exact text the implementation ships.

Any wording change during implementation is fine; the point is that the goldens
are written *against* one of these two pinned texts (E1-E5 fixed, E6
fallback-conditional), never invented ad hoc per the placebo-test concern CLAUDE.md
and this project's history (repeatedly shipped placebo tests) both flag.

## Phased delivery plan

Sequenced so each phase is independently green (`cargo fmt --check && cargo clippy -- -D
warnings && cargo test`) and, from P2 on, runnable. Sized for a less-capable
implementer: P1 is wide but mechanical; P2 introduces production with the simplest
(shared, no-`Provenance`) checking; P3 isolates the aliasing machinery.

- **Phase 1 — Part A (representation + threading).** R-A1..R-A10,
  R-A6a. Add the variant and raw form, the parser interception and fold (both the
  bare-sigil and glued-`&'T` cases, R-A3), every exhaustive-match arm, all three
  `refs`-taking signature changes (`apply_subst`, `subst_polytype`, and
  `unify_poly_input` per R-A6a — the last one threads through ~11 call sites, not
  just its own arm), and the diagnostic renderers. A signature can declare
  `&'T`/`&['T 4]`/`&!...`; a body `&a` is still an unknown-word error. Standalone
  value: the type is nameable and round-trips — and, **corrected during Phase 1
  review**, it is also already *runnable*. This phase was scoped as "no new
  capability", which is false: a monomorphic caller can borrow a local into a
  generic word's ref slot (`: firstref ( &['T 4] -- ) drop ;` called as `&a
  firstref`), so `unify_poly_input` and `subst_polytype` ground a poly ref on the
  live path and the program compiles, links, and runs. Phase 2 must not assume no
  borrow reaches lowering yet. The placebo hazard is that an arm
  added but never reached is untestable — pin the reachable ones (fold,
  `poly_type_str`, `unify_poly_input` against a concrete `&`-arg) with unit tests,
  and mutation-check them. **Difficulty: standard-to-hard, revised up after
  review** (the exhaustive-match arms alone are mechanical breadth across ~7
  files, but R-A6a's ~11-call-site `refs` threading plus R-A3's glued-token case
  are real plumbing/parsing work, not boilerplate; route this phase to the
  stronger model if `/implement`'s standard-tier capacity looks tight).

- **Phase 2 — Part B, shared read path.** R-B1, R-B2, R-B3 (read `&>` only), R-B4 (`@`
  only), R-B5 (use-after-move via `scope.moves`, no `Provenance`), R-B6, R-B7, and the
  P2 witnesses in R-B8. `&x`, `&>`, `@` on a concrete-length generic-element array;
  `first` runs. No mutable borrow, so no exclusivity machinery. Standalone value: a
  generic word can read through a borrow. **Difficulty: standard.**

- **Phase 3 — Part B, mutable path + exclusivity.** `&!x`, `&!>`, `!`, and the
  exclusivity check (R-B5, mutable half) — reusing `Provenance`/`Liveness` in
  `PolyScope` (target) or the conservative soundness-equivalent rule (fallback). The P3
  witnesses: the in-place-write golden and the conflicting-borrow negative. This is
  where OQ1's guarantee is actually exercised (before `&!` exists, exclusivity is
  vacuous), so it must not be deferred out of the slice. **Difficulty: hard** (the
  `Provenance` threading / conservative-rule decision and its mutation-checked
  negative).

## Growth structure

Edits land in existing stage files: `src/ast.rs`, `src/parser.rs`, `src/check/poly.rs`
(the bulk — representation, dispatch, checking), `src/check/audits.rs`,
`src/check/declarations.rs`, `src/check/combinators.rs`, `src/ir/driver.rs`,
`src/repl.rs`, plus `lib/`/`examples/`/`tests/` witnesses. Re-run the CLAUDE.md split
signals at phase exit against `src/check/poly.rs`, which Part B grows most: today it
already holds representation, unification, substitution, the poly-body walk, and the
diagnostics together (they change together and share `PolySig`/`PolyScope`), so no
split is anticipated — but if Part B's `poly_reference_word` plus the aliasing threading
makes the borrow family a self-contained cluster that never calls the unification code,
that is one of the two signals; check for a second (import divergence) before splitting.
`PolyType::Ref` stays a single owned representation; `Type::Ref` stays the sole ground
form.

## Deferred, with reasons

- **`'N`-length element access** (D6): indexing a fully-generic-length array needs a
  dependent bound the signature cannot express; its own slice (same class as
  `fill`'s dependent length, `[[project_fill_dependent_type_unimplementable_as_word]]`).
- **`&^` (owning-cell) and `&Struct>field` accessors in a generic body** (R-B6): a
  cell payload / struct field is always a concrete `Type` (no generic structs/enums this
  slice), so these never produce a *variable*-referent ref; they operate entirely on
  concrete types and can route through the monomorphic accessor once the poly body owns
  the aliasing machinery. Deferred to keep Part B's scope on the array case.
- **`+!`** (R-B4/R-B6): the add-in-place store, alongside `!` in shape; deferred to
  avoid widening the mutable path beyond one store operator.
- **Full `Provenance`/`Liveness` acceptance parity**, if phase P3 ships the conservative
  fallback: any over-rejection is pinned and documented; closing it to exact mono parity
  is a follow-up, not a soundness gap.
- **REPL borrows in a poly line**: a REPL line has no polymorphic words (slice 6a D2), so
  `remap_poly_type`'s `Ref` arm (R-A9) is threaded for imported-word generation only, not
  for a poly borrow typed at the REPL. The arm is consequently unreachable from any
  test, exactly like its pre-existing `Array` sibling.
- **A `&`/`&!` sigil followed by a non-`--` delimiter** (`( 'T -- 'T & )`): the
  R-A3 guard and `parse_ref_type_expr` both check only `--`/end-of-tokens, so a
  closing `)`, `|`, `;` or `]` yields a located but generic `expected a word, found
  RParen` instead of `ref_no_referent_error`. The poly path inherits this from the
  concrete path rather than adding a gap; fixing it means enumerating the delimiter
  set in both places, so it is a standalone cleanup, not Phase 1 or Part B work.

## Phases (JSON)

```json
{
  "phases": [
    { "phase": 1, "focus": "Add PolyType::Ref and RawTy::Ref. Intercept and fold a &-led poly slot in the parser, handling both the bare-sigil case (&['T 4], recurse on the next token) and the glued-token case (&'T, a single Token::Word with no separate referent token -- intern the variable from the string remainder inline). poly_is_copy answers Ref(_, mutable) => !mutable (NOT unconditionally true -- a mutable ref is not Copy) and poly_copy_gate gets a real diagnostic arm, not unreachable!(). Thread the variant through every exhaustive match (poly_copy_gate, unify_poly_input, apply_subst, poly_type_str, the op-on-variable describer, audits, collect_poly_concrete, remap_poly_type; poly_var_id needs no arm, assert via test) and all three refs-taking signature changes: apply_subst (&mut Vec<RefDecl>), subst_polytype (&[RefDecl]), and unify_poly_input (&[RefDecl], threaded through its ~11 call sites in poly.rs and combinators.rs). A signature can declare &'T/&['T 4]/&!...; a body borrow is still unknown. Unit-test and mutation-check the reachable arms, including a dup/over-of-mutable-ref rejection test.", "difficulty": "hard" },
    { "phase": 2, "focus": "Teach poly_call_term to recognise a leading & and produce a PolyType::Ref: prefix borrow &x on an aggregate local (reject bare-variable and quotation locals), the shared &> array-element ref on a concrete-length array, @ fetch on a Copy referent, and use-after-move via scope.moves (no Provenance needed for shared-only). The `first` read witness runs; the D5, D6, and use-after-move negatives assert message and site.", "difficulty": "standard" },
    { "phase": 3, "focus": "Add the mutable path (&!x, &!>, !) and the exclusivity check per OQ1, reusing Provenance/Liveness threaded into PolyScope (target) or a conservative soundness-equivalent PolyScope-local borrow-liveness rule (permitted fallback). Land the in-place-write golden and the conflicting-borrow negative (asserted at the second borrow site). This exercises OQ1's guarantee and must ship in-slice.", "difficulty": "hard" }
  ]
}
```
