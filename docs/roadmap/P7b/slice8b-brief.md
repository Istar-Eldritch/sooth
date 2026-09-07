# P7b.S8b brief — the construction-wall fix and traitful `List` members (`map`, `append`)

- Date: 260906. Base: `main` `445a74e` (S6/S6b/S6c, S7, S8, S8c, S9, and S10 all
  landed and merged; suite green at each slice's recorded gate). The `p7b-s8b`
  worktree was stale-based at `a9eca84` (pre-S6-merge) and was fast-forwarded to
  `445a74e` before any work — the first S8b action, recorded here.
- Source: the S8 carve-out, decided 260905 by probe-round P8 rulings R3/R4 and
  recorded in the roadmap S8 entry and [slice8-spec](./slice8-spec.md)
  ("Deliberate limitations"); the wall's exact shape comes from the P8 round's
  P8-2a..P8-2d2 verbatim evidence in [slice8-probes](./slice8-probes.md).
- Carve-out record, verbatim (roadmap S8 entry): "**P7b.S8b** (carved out, same
  date): the S6 construction-wall fix (`poly_bind_construction_arg`'s
  bare-`Generic` `^Self['T]` self-reference-field arm, refined by the P8 round)
  plus per-impl traitful `List` members (`map`, `append`) over the Iterator
  protocol."

## Problem

Two blockers, one shared root.

1. **The S6 construction wall is a missing checker arm, and it is
   field-shape-specific.** Constructing a generic ctor whose field list contains
   the self-referential `^Self['T]` field panics — in ANY poly body, trait-member
   or plain generic word alike:

   ```text
   thread 'main' panicked at src/check/poly.rs:6117:18:
   internal error: entered unreachable code: a generic `type:` field is never
   Generic { is_enum: true, idx: 0, module: 1, args: [Var(0)], len_args: [], name: "List" }
   ```

   (p7b-s8-era line numbers; on `main` `445a74e` the function
   `poly_bind_construction_arg` starts at `src/check/poly.rs:6074` and the
   catch-all sits at `:6193` — pin by function name and message text, not by
   line; the probe round re-measures.) The `^List['T]` self-reference field is
   stored as a bare `PolyType::Generic` — S6's phase-3 `OwnedCell` arm unwraps
   the `^` wrap and recurses, then the inner `Generic{List, [Var(0)]}` field hits
   the catch-all, which covers `Var`/`Concrete`/`App`/`OwnedCell` but not
   `Generic`. The P8 round refined S6's recorded framing in both directions:

   - it fires at **impl-declaration check time** (member bodies are checked
     eagerly), not at dispatch — a main-less twin panics identically (P8-2b);
   - it is **not** trait-member-specific: a plain generic word constructing
     `Cons` walls identically (P8-2d2). What spares mono bodies is
     *concreteness*, not member-ness (the shipped `mklist` golden constructs
     fine); plain-field ctors (`Box2`, `Some`, `Nil`, `Range`) construct fine in
     poly member bodies too (P8-2a/2c/2d).

2. **The blocked members are exactly the ones the roadmap wants next.** S6's
   "Recorded walls" dropped `Monoid for List` / `Functor for List` from its
   goldens because a real `map` or list append reconstructs a `Cons` — the wall
   shape. S8's R4 then pinned that a per-impl member is the **only** kind of
   `map`: a bound-generic body can neither produce `'It['U]` (P8-5b, located
   error) nor `dup` the abstract iterator (P8-5c, conservative linearity fence).
   So `map`/`append` for `List` are per-impl traitful members, and both are
   wall-blocked today. S8b is the slice that unblocks them.

## Already decided (settled 260905 — do not reopen)

- **The carve-out itself (S8 R3/R4):** the wall fix plus per-impl traitful
  `List` members (`map`, `append`) land in S8b. Nothing in S8's Step-row `next`
  shapes needed the wall (plain-field constructions only).
- **The wall's exact statement** (P8-2a..2d2, above): field-shape-specific,
  poly-body-wide, fires at declaration check time; mono constructions and
  plain-field poly constructions are unaffected.
- **`List` trait members are non-inline** (S6 phase 3): they mint a real
  `IrFunc` and lower as ordinary calls, so non-tail self-recursion is fine —
  only `inline` combinators face the splice budget, and non-tail self-recursion
  in those is rejected at check time. `map`'s recursion needs no new mechanism.
- **The destructure twin already works:** `substitute_generic_variant_field`'s
  `OwnedCell` arm (S6 phase 3) handles the self-reference field when
  *destructuring* — S8's own `next` impl deconstructs `Cons` in a poly body
  today. Only the construction-bind walk lacks the arm.
- **`empty` grounds via explicit instantiation only** (S6 R4): a nullary member
  needs `empty[i64]`-style instantiation; consuming-context grounding was ruled
  a future slice and is not S8b.
- **`List`'s destructor** drops linear payloads per instantiation, per
  instantiation, with constant stack (S6 phase-3 goldens) — S8b must not regress
  it, and needs no new drop machinery for constructed nodes.

## The compiler delta (one arm, checker-only)

`poly_bind_construction_arg`'s catch-all (`src/check/poly.rs:6193` on `main`)
gains a `PolyType::Generic` field arm. Design sketch, to be confirmed by the
probe round:

- the field is `Generic{gid, field_args}` — the self-reference
  (`^List['T]` unwrapped);
- the operand must be a `Generic` carrying the **same ctor identity**
  (`GenericId`: `is_enum`, `idx`, `module`); anything else is a located
  `poly_rendered_type_mismatch_error`, mirroring the `App` arm's treatment of
  non-`Generic` operands — never a panic, never a silent bind;
- with identity matched, bind **positionally by recursion** over
  `field_args` vs the operand's args — the `Generic` analogue of the `App` arm
  (which binds a head *variable* to a `CtorImage`; here the header is already
  concrete, so only the arguments bind), and of `unify_poly_input`'s
  decomposition one level up;
- grounding to the concrete instantiation remains `apply_subst`'s job — no new
  grounding path, `src/ir/` expected diff-empty (verify at phase exit).

Design questions the probe round must answer:

- **PB-1 — the fix itself.** Patch the arm minimally; both P8 shapes must flip
  from panic (exit 101) to grounding: P8-2b (an impl member body constructing
  `Cons`) and P8-2d2 (a plain generic word constructing `Cons`). A ctor-mismatch
  operand (constructing `Cons` against a differently-headed operand) must
  produce the located mismatch error, byte-exact pinned. Full suite green —
  mono constructions (`mklist`), `Range`'s plain-field `next`, and S8's 24
  slice tests must be byte-unchanged.
- **PB-2 — does `len_args` need handling in the new arm?** The stored
  self-reference field carries `len_args: []` today, but the arm's contract
  should say what it does if a future field carries a length variable (reject?
  bind?). One probe fixture or an explicit recorded fence.
- **PB-3 — which operand shapes reach the arm.** Only `Generic` (the recursive
  tail) is known today; can a `Concrete` operand reach it (a mono-typed tail
  inside a poly body, e.g. a `List[i64]` tail in a poly `map`)? The `App` arm
  rejects non-`Generic` operands with a mismatch; mirror whatever is measured.
- **PB-4 — `map` end-to-end.** `impl: Functor for List` with the trait's
  `map ( 'F['T] [ 'T -- 'U ] -- 'F['U] )` member: does the recursive body
  ground, dispatch through a shared bound (the S4/S6 precedent
  `shared_bound_poly_word_dispatches_over_the_real_core_option`), lower as one
  non-inline real frame, and drop the constructed `Cons` chain per
  instantiation when the mapped list is dropped?
- **PB-5 — `append`'s host, and how much of S6's wall closes.** The carve-out
  names `append`; S6's recorded wall named `Monoid for List` (combine = append,
  empty = `Nil`) and `Functor for List`. Probe both host spellings —
  `Monoid for List` (then `empty[i64]`/`mconcat` per S6's goldens fall in
  scope-by-consequence) vs a `List`-specific trait member — and record which
  grounds and reads better. **User ruling needed:** the host trait, and
  whether S8b closes S6's *whole* recorded wall (both dropped goldens) or only
  the named members. Note the roadmap's "over the Iterator protocol" phrase is
  loose; the S6 recorded-wall shapes are the concrete reading this brief
  probes.
  Outcome (260907, user ruling): Monoid for List; whole wall closes — both goldens land
  in S8b. See slice8b-spec.md.
- **PB-6 — linearity teeth on the new constructions.** An undropped mapped or
  appended list is a compile error (`drop` is the explicit destructor; nothing
  auto-drops); `dup` of a `List['T]` operand stays fenced (conservative
  linearity). Pin both.

## Scope

Per the carve-out record, refined by the P8 round and the questions above:

1. The wall fix: one arm in `poly_bind_construction_arg` (`src/check/poly.rs`),
   located-error fencing, byte-exact diagnostic pins, unit tests beside the
   function (CLAUDE.md stage convention).
2. Per-impl traitful `List` members: `map` and `append`, host trait per the
   PB-5 ruling (S6's recorded-wall shapes are `Functor for List` /
   `Monoid for List`).
3. The S6 recorded-wall witness (`tests/phase7b_slice6.rs:410`,
   `monoid_for_list_append_construction_wall_is_recorded`) flips from recording
   the wall to a positive golden, or retires in favor of the new goldens —
   spec's call.
4. Ship surface (PB-5 outcome): S6's convention landed the container-trait
   surface as golden fixtures, not `lib` modules (only `list.sth` ships); S8
   shipped `core::iterator`. S8b follows one of the two, ruled by the spec.

**Out of scope:** Iterator adaptors (`zip`/`take`/`rev`/...) and lazy/streaming
iteration; a bound-generic `map` producing `'It['U]` (recorded impossible,
P8-5b/5c); consuming-context grounding for nullary members (S6 R4's future
slice); the D5 borrow gate; numeric traits; phantom parameters; `src/ir/`
changes (expected diff-empty — verify at phase exit); the QBE backend; new
trait-declaration syntax; the array-as-`'F` kind story (S6b territory); S8's
residual per-`next`-call frame question (S8 only measured it).

## Goldens (sketch — finalized by the spec)

- the P8-2b shape grounds and runs (impl member constructing `Cons`);
- the P8-2d2 shape grounds and runs (plain generic word);
- `map` through a `Functor` bound over the real core `List`;
- `append` golden (and `empty[i64]`/`mconcat` if `Monoid` is ruled in);
- linearity-teeth pins (PB-6);
- byte-exact located-error pin for the ctor-mismatch operand;
- the S6 wall witness flipped or retired;
- unit tests beside the new arm covering happy path + the mismatch error.

## Sequencing

- Base `445a74e`; the worktree fast-forward is done. Re-baseline (fmt, clippy,
  full `cargo test`) belongs to the probe round, one time, before patching.
- No known parallel-edit conflicts: the wall site is checker-only
  (`src/check/poly.rs`); S8c's fence (member-signature free vars) is a
  different gate; S10's export-origin walk lives in resolve/builtins. Re-check
  at spec time per the S10∥S6 lesson (a landing rule if a merge lands first).
- Probe round **P8b** runs before the spec (fragile-slice convention: the fix
  touches the checker's construction core). Verbatim log to
  `slice8b-probes.md`; spec to `slice8b-spec.md`.
