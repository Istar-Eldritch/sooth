# Phase 4 Slice 13: `PolyType::Ref` — borrows inside a generic word (spec, as shipped)

A plain (non-combinator) `'T`-bounded word can now borrow: `&x`/`&!x` at its own top
level, plus signature slots that borrow a still-generic type (`&'T`, `&['T 4]`, `&!…`).
Before this slice the borrow family (`&x`, `&!x`, `&>`, `&^`, `&Struct>field`, `@`, `!`)
lived only on the monomorphic side; `PolyType` could not name "a reference whose referent
is still `'T`" and `poly_call_term` had no `&`-led branch.

The load-bearing asymmetry: a *combinator* is spliced and monomorphized per call site
before its body is checked, but a *plain* generic word is checked once, abstractly, and
lowered per instantiation with **no concrete re-check**. So representation, aliasing and
liveness must be modelled abstractly here, or they are modelled nowhere.

Invariants held: backend is QBE, no new IR opcode; `Type::Ref` stays the sole ground form
(`RefId` handle, never a `u64`); `core` stays `no_std`; a reference is neither
`Copy`-obligated nor linear, the drop obligation stays on the referent.

## OQ1 (settled): the poly body enforces borrow checks itself

Deferring aliasing/exclusivity to the monomorphized instantiation is unsound: nothing
re-checks a plain poly body, so a hazard the poly walk misses is caught by nothing.
The hazards are facts about *term order* (`&!a … &!a` is a double mutable borrow at every
instantiation), so they check fine abstractly.

**Locked minimum: soundness-equivalence.** Every program the monomorphic checker rejects
for a borrow reason must be rejected in a poly body with an equivalent located diagnostic.

**Shipped: the permitted conservative fallback, not `Provenance`/`Liveness` threading.**
`PolyScope` records its own `Vec<PolyBorrow>` (place, mutability, span). A borrow is
observable only through a *reference value*, and Sooth forbids storing one where it could
outlive the stack, so `prune_dead_borrows` clears **all** recorded borrows once neither the
`PolyType` stack nor any local holds a reference slot. `live_borrow_of` then answers the
mono mutability rule: a new `&!` conflicts with any live borrow of the place, a new `&`
only with a live mutable one.

This is coarser than mono's per-place `live_deriv` in one direction only (over-rejection,
never a missed hazard): one unrelated live `&b` keeps `a`'s already-dead borrow recorded.
Pinned as intentional by `poly_borrow_liveness_is_coarse_across_places`, and made legible
in the diagnostic by a mandatory note (E6 below). It is *not* "live until word end":
`settwo` writes two elements because `!` consumes each element reference.

## Design decisions

**D1 — `PolyType::Ref(Box<PolyType>, bool)`, referent then mutability.** Mirrors
`Type::Ref(RefId, bool, &'static str)` minus the interned handle and spelling. No `RefId`:
the referent may be a variable, which no registry entry can name; the `RefId` is minted
only at grounding. Mutability rides the variant because it is the classification bit
(`is_copy`, store-vs-fetch, exclusivity) asked at sites that hold no registry.

**D2 — a poly `&`-led slot folds like an array.** `RawTy::Ref(Box<RawTy>, bool)`;
`parse_poly_slot` intercepts a leading `&`/`&!` *before* its `parse_type_expr` fallthrough
(that path resolves the referent concretely, so `&'T` died on `'T` as an unknown type).
`raw_to_poly_type` folds `RawTy::Ref` to `Concrete(Type::Ref)` via `intern_ref_type` when
the referent folds fully concrete, else to `PolyType::Ref` — the same discipline `Array`
and `Quotation` follow. Two genuinely different parse cases, because the lexer's only word
delimiters are `; ( ) | [ ]`:

- **bare sigil** (`&['T 4]`): the sigil is its own token; recurse `parse_poly_slot` on the
  following token.
- **glued sigil+variable** (`&'T`, `&!'T`): one `Token::Word`; split the sigil off the
  *string* and intern the remainder as a type variable inline.

**D3 — a reference's `Copy`-ness tracks mutability, not the referent.** `poly_is_copy`
answers `Ref(_, mutable) => !mutable`, mirroring the monomorphic `is_copy`. A shared ref is
freely duplicated; a mutable one is not (two names observing one exclusive borrow is exactly
what `dup`'s Copy gate exists to prevent). Unconditional-Copy here would be a soundness
regression, not a cosmetic one. `poly_copy_gate`'s `Ref` arm is therefore a real located
error (E1), never `unreachable!()`.

**D4 — grounding interns.** `apply_subst` (check side) takes `refs: &mut Vec<RefDecl>` and
interns; `subst_polytype` (lowering) takes the immutable `refs: &[RefDecl]` and only looks
up by position, because check-side grounding has already interned every `Type::Ref` an
instantiation can produce. A lowering-side lookup miss would be a bug in `apply_subst`'s
coverage, not a reason to widen the lowering signature.

**D5 — only an aggregate local is borrowable.** As in the monomorphic `check_reference_word`
tail (`Struct | Enum | Array | OwnedCell`), `&x`/`&!x` requires `PolyType::Array(..)` or a
`Concrete(_)` in those four. A bare `PolyType::Var` local is rejected (E2): `'T` might
instantiate to a scalar, which has no address, and the rule refuses uniformly rather than
deferring the question to instantiation. A non-aggregate *concrete* local gets its own
wording.

**D6 — element access needs a concrete length.** `&>`/`&!>` statically bounds-check the
index, so a `['T 'N]` receiver is a located error (E3). Element access is available only on
a concrete-length, generic-element array (`['T 4]`); the prefix borrow `&a` still works for
`['T 'N]`. `'N`-length indexing is a dependent-bounds problem, deferred.

## Requirements

### Part A — representation and threading

**R-A1/R-A2.** `PolyType::Ref(Box<PolyType>, bool)` in `src/ast.rs`; `RawTy::Ref` in
`src/parser.rs`. No new `Type` variant.

**R-A3/R-A4.** Parser interception and fold per D2, both the bare-sigil and glued-token
cases. A bare `&`/`&!` with no referent reuses `parse_ref_type_expr`'s "no referent" error.

**R-A5.** `poly_is_copy`: `Ref(_, mutable) => !mutable`. `poly_copy_gate`: shared ref falls
through, mutable ref emits E1. Both change together, or the case D3 exists to reject becomes
an ICE.

**R-A6.** `unify_poly_input`: a declared `PolyType::Ref(rp, m)` unifies against a concrete
`Type::Ref(id, cm, _)` only when `m == cm`, recovering the concrete referent via
`ref_parts`/`refs[id]` and recursing. A mutability mismatch or a non-ref slot is a located
type mismatch, never a silent bind. This required threading `refs: &[RefDecl]` through
`unify_poly_input` and its ~11 call sites in `poly.rs` and `combinators.rs`.

**R-A7/R-A8.** Grounding arms in `apply_subst` and `subst_polytype` per D4.

**R-A9.** `Ref` arms in `poly_type_str` (`&`/`&!` + referent), the `poly_op_on_variable_error`
describer, `audit_poly_input_quotation` / `reject_poly_quotation_anywhere` (recurse the
referent so a quotation behind a `&` still cannot slip past the default-deny),
`collect_poly_concrete` (export privacy sees a private type behind a `&`), and
`remap_poly_type`. `poly_var_id` needs no arm; asserted by unit test.

**R-A10 (corrected in P2).** A poly word may declare a borrow in an **input** position:
`: pick ( &i64 'T: Copy -- 'T )` parses and round-trips through `poly_type_str`. An
*output* borrow (the earlier `: peek ( ['T 4] -- &['T 4] )` sketch) is rejected outright by
R-B9.

**Part A was not capability-free** (spec claim corrected during Phase 1 review): a
monomorphic caller can borrow a local into a generic word's ref slot
(`: firstref ( &['T 4] -- ) drop ;` called as `&a firstref`), so `unify_poly_input` and
`subst_polytype` ground a poly ref on the live path from Phase 1 on.

### Part B — production and checking

**R-B1.** `poly_call_term` fronts every `&`-led name with `poly_reference_word`, mirroring
`check_reference_word`'s position; a non-`&` name falls through unchanged.

**R-B2.** Prefix borrow `&x`/`&!x`: strip the sigil, require a local (else a located
non-local error; a bare sigil is a non-place error), require an aggregate (D5), reject a
use-after-move (E5). Borrowing is not a move — `x` stays live. Pushes
`PolyType::Ref(local_pt, mutable)` and records the borrow.

**R-B3.** `&>`/`&!>` on a reference-to-array receiver plus a literal index: bounds-check via
`check_poly_array_index` against a concrete `count`, require `recv_mut == mutable` (a
mismatch renders as a type mismatch off the normalized referent, matching the monomorphic
twin's ``` `&>` expected `&[i64 4]`, found `&![i64 4]` ```), push `Ref(elem, mutable)`. A
`Len::Var` receiver is E3. `poly_ref_array_parts` accepts both array representations
(variable-bearing `PolyType::Array` and registry-interned `Concrete(Type::Array)`).
A computed (non-literal) `i64` index needs the same explicit `>usize` conversion mono
requires.

**R-B4.** `@` on `Ref(rp, _)` gated by `poly_copy_gate(rp)`: consume the ref, push the
referent. `!` is `( &!T T -- )`: a shared receiver is a rendered mutability mismatch, the
referent must pass the Copy gate (storing overwrites, so a linear referent would lose its
drop obligation), and the value must equal the referent. `+!` is E4.

**R-B5.** Exclusivity per OQ1, enforced **symmetrically at both sites**, which is wider than
the original spec: checking only at the borrow catches `a … &!a` and misses `&!a … a`, the
same hazard with the terms swapped. So `poly_call_term`'s local-name read also prunes and
consults the borrow set — consuming a borrowed local is one error, merely *naming* a place a
live `&!` reaches is another (E6b/E6c).

**R-B6.** `&^`, `&Struct>field` and `+!` in a generic body are located errors (E4), never a
silent fallthrough to unknown-word.

**R-B7.** No new IR: a borrowing generic word lowers through the existing monomorphization
path, and the body's `&a`/`&>`/`@`/`!` reuse the monomorphic reference machinery once
instantiated.

**R-B9 (added in P2).** `check_reference_free_signature` runs on `word.effect`, which is
empty for a poly word, so no generic signature was audited at all — and once a body could
*produce* a `PolyType::Ref`, an escaping borrow reached lowering and panicked
(`checked: every reference value records its referent`). `audit_poly_reference_free_signature`
is the poly twin: any output transitively containing a reference is rejected; an input may
*be* a top-level borrow in **either** representation (`PolyType::Ref` for `&'T`,
`Concrete(Type::Ref)` for the fully concrete `&i64`) but may not carry one nested inside an
aggregate; skipped for a combinator, mirroring `check_word`'s own skip.

## Located errors (exact shipped text)

- **E1 — `dup`/`over` of a mutable poly reference** (`poly_copy_mutable_ref_error`):
  ``error: cannot `dup` a mutable reference in `dupmut` (line 1)`` /
  ``  `&!['T 4]` is not `Copy`: duplicating it would let two names observe or mutate through one exclusive borrow``
- **E2 — borrowing a bare type-variable local** (`poly_borrow_of_variable_local_error`):
  ``error: cannot borrow the local `t` of type `'T` in `badvar` (line 3, col 3)`` /
  ``  `'T` might instantiate to a scalar, which has no address; borrow an aggregate (a struct, enum, array, or owning cell) instead``.
  A non-aggregate concrete local gets the parallel `poly_borrow_of_non_aggregate_local_error`.
- **E3 — `&>`/`&!>` on a generic-length array** (`poly_generic_length_index_error`):
  ``error: cannot index a generic-length array in `badidx` (line 4, col 3)`` /
  ``  the array's length is the type variable `'N`, so its element cannot be statically bounds-checked; index a concrete-length array (`['T 4]`), or use a fixed length in this word's signature``
- **E4 — out-of-scope accessor** (`poly_unsupported_accessor_error`, parametrized on `&^`,
  `&Struct>field`, `+!`): ``error: `&^` is not yet supported in a generic body, in `badcell` (line 2)`` /
  ``  monomorphize this word (or write a concrete wrapper) to use `&^` today``
- **E5 — borrowing a consumed local**: `poly_use_after_move_error`, verbatim and unmodified.
- **E6 — the borrow-liveness family.** All three append
  `POLY_BORROW_LIVENESS_NOTE` = ``\n  note: this borrow's exact lifetime is not tracked in a generic body; it is conservatively treated as live while any reference value remains on the stack or in a local``,
  so a conservative false positive is legible as by-design rather than a checker bug:
  - **E6a conflicting borrow**: ``error: `&!a` conflicts with a live borrow of `a` in `twomut` (line 3, col 7)`` /
    ``  the mutable borrow taken at line 3, col 3 is still live`` /
    ``  at most one `&!` to a place, and never a `&` alongside a `&!`; consume the earlier borrow first``
  - **E6b consuming a borrowed local**: ``error: cannot consume the borrowed local `a` of type `['T 4]` in `consume` (line 3, col 6)`` /
    ``  the shared borrow taken at line 3, col 3 is still live`` /
    ``  a place stays borrowed until every reference derived from it is consumed``
  - **E6c naming a mutably borrowed local**: ``error: cannot name `a` in `alias` (line 3, col 7): a mutable borrow of it is still live (line 3, col 3)`` /
    ``  naming an aggregate does not copy it, so this name would denote the storage that borrow mutates`` /
    ``  finish with the borrow first, or `dup` for an independent copy``

The goldens assert these strings whole (`assert_eq`), with message *and* site, never
"rejected somewhere" — the project's standing placebo-test hazard.

## Witnesses

Runnable goldens in `tests/phase4_slice13_borrow.rs`, sources in `examples/`:

- **Read** — `examples/poly_borrow_first.sth`: `: first ( ['T: Copy 4] -- 'T ) | a | &a 0 &> @ ;`,
  asserting the printed value.
- **Write** — `examples/poly_borrow_setat.sth`: `: setat ( ['T: Copy 4] 'T -- ['T 4] ) | a v | &!a 2 &!> v ! a ;`,
  monomorphized at **two** types (`i64` and a two-field `Vec2`) and reading back both the
  written element and its neighbour, so a stride bug in generic element-ref lowering shows
  up as clobbered fields rather than only a wrong scalar.

Checker negatives and their positive controls (`src/check/poly.rs` tests): E1 with the
shared-`dup` control; E2 plus the concrete-scalar variant; E3; E4 for all three operators;
E5; E6a in both directions with `poly_reference_word_accepts_two_live_shared_borrows` as the
control; E6b/E6c with `poly_call_term_accepts_naming_a_local_beside_a_live_shared_borrow`;
liveness release (`settwo`), the reference-parked-in-a-local case (a stack-only scan would
admit two live `&!` to one place), and the pinned coarse-across-places over-rejection;
`!` mismatches (shared receiver, wrong value type, non-Copy referent); `@` on a non-Copy
referent; `&>` on a mutable receiver; `check_poly_array_index` directly; and
`subst_polytype_grounds_a_poly_ref_slot_from_a_monomorphic_caller` in `src/ir/driver.rs`.

Each positive is mutation-checked by deleting the arm it exercises and confirming the test
then fails — several tests above exist precisely because that exercise found reachable but
untested arms (`@`'s Copy gate, `&>`'s mutability guard, `subst_polytype`'s `Ref` arm).

Note the poly diagnostics restate monomorphic wording from `src/check/check.rs` and
`src/check/terms.rs` near-verbatim, and the drift is one-sided: the poly copies are pinned
whole, mono's tests only `contains` a leading fragment. Editing mono's wording silently
diverges the two; re-check the poly twins by hand.

## Deferred, with reasons

- **`'N`-length element access** (D6): needs a dependent bound the signature cannot express;
  its own slice, same class as `fill`'s dependent length.
- **`&^` and `&Struct>field` in a generic body** (R-B6): with no generic structs/enums, these
  never produce a variable-referent ref, so they can route through the monomorphic accessor
  once the poly body owns the aliasing machinery.
- **`+!`**: same shape as `!`; deferred to avoid widening the mutable path beyond one store.
- **A fully concrete `&[…]` parameter is unusable *inside* a generic body**:
  `raw_to_poly_type` folds it to `Concrete(Type::Ref)` and every accessor arm matches only
  `PolyType::Ref`, so `: setz ( &![i64 4] 'T -- 'T ) | r v | r 0 &!> v ! ;` rejects with
  ``` `&!>` is not permitted on `&![i64 4]` ```. Fixing it means folding a concrete `Ref`
  back to a `PolyType::Ref` at signature-binding time.
- **Exact acceptance parity with mono**: closing the coarse-across-places over-rejection
  means threading real `Provenance`/`Liveness` into `PolyScope`. A follow-up, not a
  soundness gap.
- **A shared renderer for the borrow-liveness diagnostics**: the poly and mono copies are
  not literal twins (mono renders location through `in_word(ctx)`, the poly ones always name
  a word and append the note, and they carry `Deriv` vs `PolyBorrow`), so unifying them
  means changing monomorphic diagnostics too.
- **REPL borrows in a poly line**: a REPL line has no polymorphic words, so
  `remap_poly_type`'s `Ref` arm is threaded for imported-word generation only and is
  unreachable from any test, exactly like its pre-existing `Array` sibling.
- **A `&`/`&!` sigil followed by a non-`--` delimiter** (`( 'T -- 'T & )`): both the poly
  guard and `parse_ref_type_expr` check only `--`/end-of-tokens, so a closing `)`, `|`, `;`
  or `]` yields a generic `expected a word, found RParen`. Inherited from the concrete path;
  fixing it means enumerating the delimiter set in both places.

## Growth structure

All edits landed in existing stage files (`src/ast.rs`, `src/parser.rs`, `src/check/poly.rs`
— the bulk, `src/check/audits.rs`, `declarations.rs`, `combinators.rs`, `word_families.rs`,
`src/ir/driver.rs`, `src/repl.rs`) plus `examples/` and `tests/` witnesses. `poly.rs` grew
most, but the borrow family still shares `PolySig`/`PolyScope` with unification and
substitution and changes together with them, so the split signals do not fire.
`PolyType::Ref` stays a single owned representation; `Type::Ref` stays the sole ground form.
