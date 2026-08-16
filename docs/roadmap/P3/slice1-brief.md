# Phase 3 Slice 1 — Linear analysis + move + `dup`-gate + explicit `drop` (brief)

The first cut of the linear spine, isolated from heap. Move-by-default, use-after-move
is an error, `dup`/`over` gated on `Copy`, `drop` runs a destructor, and **forgetting to
dispose a linear value is a compile error** (linear, not affine: use exactly once, nothing
auto-drops). Proven on a test-only builtin drop-spy. Aggregates are in scope via
destructure-whole. No heap, no refs, no RC, no fds.

This is the load-bearing novel analysis every later Phase 3 slice builds on. See
`DESIGN.md` "## The linear spine" for the target semantics (already rewritten from affine).

## Decisions (locked in the design conversation)

- **D1 — Linear, not affine.** A linear value is used *exactly once*. There is **no
  auto-drop**. Forgetting to dispose is a compile error, caught by the existing
  surplus-value / stack-effect machinery. Copy and linear are handled *symmetrically* for
  disposal: a surplus value at scope end is an error for both. The only Copy/linear
  difference is intrinsic: `drop` runs a destructor on a linear value (a no-op discard on
  Copy), and a linear value *must* be consumed while a Copy value may be discarded freely.

- **D2 — Move on mention (locals).** Mentioning a linear local moves its value out onto
  the stack; a second mention is a `use after move` error pointing at the move site. Stack
  values are linear by construction (consuming pops them). No mid-body rebinding exists
  (top-of-scope `| … |` only), so a moved local's name is simply dead for the rest of the
  scope — no new rule needed.

- **D3 — `dup`/`over` gated on `Copy`.** `dup` (and `over`, which copies) on a non-`Copy`
  value is a compile error with the `DESIGN.md`-style diagnostic ("cannot `dup` a value of
  type X; X is linear …"). `swap`/`rot`/`nip` and other pure reorderings are fine on linear
  values (they move, they don't copy).

- **D4 — `drop` is the universal disposal primitive.** On a linear value `drop` runs its
  destructor; on a Copy value it discards. `close`/`free` are library words layered on top
  (not in this slice). `drop` becomes *user-overridable* only when polymorphic words land
  (**Phase 4**); through Phase 3 destructors are compiler-known. "Define a `drop` for type
  T" is the eventual overridable-destructor mechanism.

- **D5 — The test vehicle (bootstrap 1a): a test-only builtin linear primitive (the
  drop-spy).** It carries an `i64` tag; its compiler-known destructor prints a deterministic
  line (e.g. `drop <tag>`) so drop **count / order / timing** are assertable in goldens.
  Affine-ness must originate in a builtin primitive (plain data is `Copy`; you cannot
  declare plain data linear), so the vehicle is builtin, not user-written. It is
  **convention-fenced**: an internal name (e.g. `__spy` / `Sentinel`), documented as a
  compiler test intrinsic, *not* part of the user-facing language surface; not flag-gated.
  It is permanent test infrastructure for now (real destructors — `free`, `close` — are not
  golden-observable), and it dissolves into an ordinary type once `drop` is overridable
  (Phase 4). Pick the exact internal name during spec/impl.

- **D6 — Aggregates in scope via destructure-whole (no partial moves).** A struct or enum
  is **linear iff any field / variant payload is linear** (transitive through nesting);
  plain-data aggregates stay `Copy`, unchanged. Rules:
  - `dup`/`over` on a linear aggregate: error (D3).
  - `S` (construct): moves fields in — works for linear fields. `S>` (destructure): consumes
    the aggregate and pushes *all* fields; each linear field then becomes a standalone value
    tracked by D2. This is the sanctioned way to open a linear aggregate; no live partial
    aggregate ever exists, so there is no partial-move tracking.
  - `S>fi` (get): stays `( S -- field )`, consuming, for **any** field. On a linear receiver
    it additionally runs drop glue on the *non-extracted* fields (so reading one field
    consumes the aggregate and disposes the rest). Uniform stack effect, existing Copy code
    unchanged (drop-the-rest is a no-op when the rest is Copy).
  - `S|>fi` (peek): **new word**, `( S -- S field )`, non-consuming, **Copy fields only**.
    Leaves the aggregate live, copies out a Copy field. Forbidden on a linear field (that is
    a borrow → refs, slice 4; workaround is `S>`). Sound without reference machinery (copies
    Copy bytes, transfers no ownership).
  - `S<fi` (set): functional update, allowed on linear aggregates; **drops the overwritten
    field if it is linear** (drop-on-overwrite, observable). The other linear fields transfer
    via the existing blit-move (old shell consumed, never dropped → no double free).
  - Enums: Slice 4 clause dispatch already consumes-the-scrutinee-and-pushes-fields
    (destructure-whole). The new part is a **synthesized tag-dispatched destructor** for
    dropping a linear enum that is never matched.
  - The compiler **synthesizes recursive drop glue** per linear aggregate: struct → drop its
    linear fields in a defined order; enum → dispatch on the tag, drop the active variant's
    linear payload.

- **D7 — Explicit disposal, symmetric, no auto-insert.**
  - Explicit `drop` runs the destructor (D4).
  - A linear value left on the stack beyond the declared outputs is a **stack-effect error**
    (same machinery as a surplus Copy value: `check_outputs`). A linear local never consumed
    by end of scope is an error (`linear value X is never consumed; drop it or return it`).
    Nothing is auto-dropped.
  - **Branch joins: no reconciliation.** Stack shapes already unify across `if`/clause arms
    (existing), so a linear stack value cannot diverge; only a *local's* move-state can. A
    local moved in one arm and live in another surfaces as use-after-move (if referenced
    after the join) or unconsumed-linear (if not). The programmer disposes explicitly on
    each path; the compiler errors rather than inserting compensating drops.

- **D8 — Deferred / out of scope this slice.**
  - **Loop-carried linear across the Slice 6 back-edge**: a linear value live across a
    back-edge is a located `not supported yet` error. Copy loops (`countdown`, `vm`, …) are
    unaffected. A later Phase 3 slice lifts this.
  - Partial / field-level moves (excluded by destructure-whole).
  - User-overridable `drop` (Phase 4).
  - Heap / owning pointers (slice 2), refs + `let`/`inout`/`sink`/`set` (slice 4), RC
    (slice 5), fds/resources (slice 6).

## Work by stage (rough)

- **check.rs**: Copy-vs-linear predicate per `Type` (spy = linear; aggregate linear iff any
  field linear, transitive). Move-tracking pass for linear locals (moved-state, forward flow
  through `if`/clause arms). `dup`/`over` gate. Surplus-linear and unconsumed-linear-local
  errors (extend `check_outputs` @check.rs:554 + a locals-linearity check). Typing for the
  new `S|>fi` word (Copy field, non-consuming; error on linear field).
- **ir.rs**: `drop` lowers to a destructor call on linear values; synthesize recursive /
  tag-dispatched drop glue for linear aggregates; `S>fi` drops the non-extracted linear
  fields; `S<fi` drops the overwritten linear field; `S|>fi` non-consuming projection.
  Reuse existing call lowering — expect **no new `Instr`/`Terminator` variant**.
- **backend/qbe.rs**: emit the spy's destructor (a runtime print of the tag); otherwise just
  calls.
- **lexer/parser + ast**: the drop-spy constructor word + internal name; `S|>fi` surface
  form (`Struct|>field`).
- **Diagnostics** (behavioural, tested for content): use-after-move (names move site),
  cannot-`dup`-linear, surplus-linear-value, linear-local-never-consumed,
  cannot-peek-linear-field.

## Success criteria (each a golden; native binary and/or REPL, not IL-string asserts)

1. `dup` on a spy (or a struct containing one) is a compile error with the linear diagnostic.
2. Use-after-move: mentioning a moved spy local twice is a compile error naming the move site.
3. A destructor runs **exactly once** at the explicit `drop`, observable in stdout (tag once).
4. Forgetting is an error: a spy left unconsumed (surplus on the stack, or a linear local
   never consumed) fails to compile — not a silent drop.
5. Destructure-whole: `S>` a struct-of-spies, drop the fields; each destructor runs; order
   asserted.
6. `S>fi` on a linear struct extracts one field and **drops the rest** (dropped spy prints).
7. `S|>fi` peeks a Copy field of a linear struct leaving it live; `S|>fi` on a linear field
   is a compile error.
8. `S<fi` on a linear struct **drops the overwritten spy** (prints) and keeps the rest.
9. Enum: an unmatched linear enum dropped via tag-dispatched glue drops the active variant's
   spy; a matched clause consumes/drops its payload.
10. Branch: a spy consumed in **both** arms compiles; consumed in one and left in the other is
    a compile error (no auto-reconcile).
11. **No regression**: every existing example and test (`gcd`, `factorial`, `vm`, `stack`,
    `shapes`, `vectors`, `rgb`, `countdown`, …) still builds and passes; green stays green.

## Risks / unknowns for the spec to resolve

- The move-tracking pass is new analysis in `check.rs`; getting the forward-flow moved-state
  through `if`/clause arms right (and interacting cleanly with existing branch-join
  unification) is the main correctness risk.
- Where drop glue lives: a synthesized IR function per linear aggregate type vs inline drop
  sequences. Keep minimal; one builtin spy destructor + compositional field drops.
- The surplus/unconsumed error must reuse the existing Copy surplus check without regressing
  it, and must not misfire on Copy values.
- Confirm the spy's internal name and that it never leaks into the tutorial / user surface.

## Current-state anchors

- `check_outputs` @check.rs:554 — surplus-value error today; extend to linear.
- `lower_struct_word` @ir.rs:1592 — `Construct`/`Get`/`Set`/`Destructure`; add drop-the-rest,
  drop-on-overwrite, and the `S|>fi` peek.
- Slice 4 clause dispatch (consumes scrutinee, pushes fields) = the enum destructure-whole;
  add the tag-dispatched drop glue.
- `WORD_WIDTH`/`usize` @ir.rs:24; bounds-trap `emit_oob_trap` @qbe.rs (Slice 5) — pattern for
  emitting a runtime effect (the spy print is analogous but benign).
- `DESIGN.md` "## The linear spine" — the target semantics, already rewritten.

## Not a zero-`src/` slice

Unlike Slice 7, this slice **changes the compiler** (`src/check.rs`, `src/ir.rs`,
`src/backend/qbe.rs`, `src/lexer.rs`/`parser.rs`/`ast.rs`) plus tests. Allowlist for the
diff: `src/**`, `tests/phase0.rs`, `tests/phase1.rs`, optionally a new `examples/*.sth`
demonstrating linear disposal, and `docs/phase3-slice1-spec.md`.
