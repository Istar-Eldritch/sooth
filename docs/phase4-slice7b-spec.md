# Phase 4 Slice 7b: capturing closures

7a gave a quotation a `(code, env)` runtime value but only when it captures **nothing**: a
capturing literal reaching any of 7a's four materialization boundaries (a struct field, an array
element, a word output, a differing-arm branch join) is rejected outright, naming this slice.
7b makes those legal: an `env` that actually holds the captured references (D4), a checker that
proves the referents outlive the closure's calls, and a located rejection when they do not.
Splicing is untouched (recon 1 / D2): the entire new surface is those four boundaries plus their
new indirect-call obligations. Force-inlining of quotation-taking words (6a's D2) is untouched.

Base `main` @ `c1b1e0a`. Discovery: [`phase4-slice7b-brief.md`](./phase4-slice7b-brief.md).
Q1/Q2 empirical resolution: [`phase4-slice7b-q1-probe.md`](./phase4-slice7b-q1-probe.md) (cited,
not re-derived).

## Problem statement

7a shipped the representation (`IrType::Quotation` = `{ code: Code@0, env: Ptr@WORD_WIDTH }`,
`ir.rs:82`/`:200`) with the `env` slot **hardcoded null and never read**: `materialize_quot_value`
writes `Const(env, 0)` unconditionally (`ir.rs:4188`) and `lower_indirect_call` loads only the
`code` slot (`ir.rs:4245`). A capturing literal at a boundary is rejected before it can reach that
null env: `materialize_quotation_at_boundary` runs `body_captures_enclosing` and returns
`capturing_quotation_error` on any capture (`check.rs:6221`/`:6238`/`:6204`), with a copy of the
same guard inline at the join (`check.rs:7204`).

The probe lifted those guards in a throwaway copy and measured (not argued) the two facts this
slice is built around:

1. **The checker cannot today tell a sound escaping capture from an unsound one.** `make-a` (a
   closure over a `[i64 4]` local of its *own* frame, returned upward) and `make-b` (a closure over
   a `&[i64 4]` that arrived as a *parameter*) are **accepted identically** with the guard lifted;
   `make-a` only fails later in lowering with no diagnostic (`panicked at ir.rs:3577`). Probe P3.
2. **6f's existing liveness gives opposite answers depending on where it is asked**, so the brief's
   original "just rely on 6f" direction (b) is unbuildable: evaluated at the boundary it admits the
   unsound `make-a` (its captured local is still read on the return line, so it reads *alive*);
   evaluated at the call site it rejects both motivating programs (a dispatch table, a struct-stored
   closure observing a later mutation) because erasure sets `quot = None`, so `capture_alive_names`'s
   `if let Some(QuotRef::Known(id))` guard (`check.rs:1066`) has already dropped their captured names
   from the walk. Probe P4.

The one piece of existing machinery that *does* separate `make-a` from `make-b` is
`Deriv.owned_root` compared against the current frame's locals: the R12 exit-row check in
`check_literal_against_declared_effect` (`check.rs:6108`) and `check_reference_across_back_edge`
(`check.rs:5519`/`:5501`) both already make exactly this distinction (root in the current frame vs.
a parameter/global). 7b points that same test at the capture set.

## Q1 sequencing decision (explicit, per the brief's open fork)

The probe splits Q1 into an ordered pair, not a binary. **This spec ships them as two sequential
phases, the floor first**, because the floor is the soundness boundary and is independently
testable, while the env-plumbing capability is strictly additive on top and carries the harder
checker machinery. Concretely:

- **Phase 1 (floor + escaping single-capture).** Retire 7a's blanket boundary rejection (R12) and
  replace it with the `owned_root`-vs-current-frame admission rule (R15). A capture rooted in a
  **parameter or global** is admissible at any boundary; a capture rooted in a **current-frame
  local** is rejected at a **word-output (escaping) boundary** with a located past-owning-frame
  error (this is the `make-a` hole, closed with a real diagnostic). Build the minimum env plumbing
  to actually run the admitted escaping case: a **single, word-sized reference capture stored inline
  in the `env` slot** (R16/R17). `make-b` compiles and runs; `make-a` is rejected. This phase alone
  is a shippable soundness increment and forces the env-build / env-read path that Phase 2 extends.
- **Phase 2 (surviving capture set + in-frame same-frame captures).** The two motivating programs
  capture a local of the *same* frame that later calls the closure (never escaping upward), so the
  floor's escape guard does not reject them, but admitting them safely needs a capture set that
  **survives erasure** and feeds `capture_alive_names` past the call site (R19/R20/R21). Add the
  multi-capture **stack-allocated env bundle** (R16), the past-last-use rejection (R24), and the
  escape guard over the surviving set so a same-frame-rooted-capture closure cannot leave its frame
  through a carrier (R22).

Rationale for floor-first rather than one bundled phase: Phase 1 is green and sound on its own
(reject `make-a`, run `make-b`), so the make-a class stops being silently mis-accepted before any
capability work adds risk; and the inline-single-reference env de-risks the harder bundle +
surviving-set work by proving the env plumbing end to end first. The cost is one extra phase
boundary; the payoff is that the soundness fix is not entangled with the capability feature.

## Where the change lands

Every anchor below was re-grepped against `main` @ `c1b1e0a`; the brief's line numbers had not
drifted materially.

- **The blanket capture rejection to retire.** `materialize_quotation_at_boundary` (`check.rs:6221`)
  runs `body_captures_enclosing` (`check.rs:6150`) and returns `capturing_quotation_error`
  (`check.rs:6204`) on any capture. A second, inline copy of the same guard sits in the join
  (`check.rs:7204`, inside the `(Some(Known(a)), Some(Known(b)))` arm at `:7195`). Both are the R12
  the brief names; both are replaced by the admission rule.
- **The four boundaries (unchanged locations, D1).** Word output: the materialize loop calls
  `materialize_quotation_at_boundary` at `check.rs:3675`. `!`/`+!` store through a `&!Quotation`
  (array element / struct field via reference): `check.rs:6912`. Quotation parameter / struct
  constructor / setter: `check.rs:7038`. Differing-arm `if` join: `check.rs:7190`–`:7245`.
- **The capture set source (D3, reuse, no new analysis).** `capture_names`/`capture_names_into`
  (`check.rs:777`/`:783`) compute a literal's free-name set at intern time, recursing into nested
  quotations and both `if` arms; `Provenance::quotation_captures(id)` (`check.rs:544`) caches it by
  `QuotId`, populated by `prov.quotation_captures.push(capture_names(body))` (`check.rs:7321`).
- **The admission test (the make-a / make-b separator).** `Deriv.owned_root` compared against the
  current frame's locals. Two existing precedents: the R12 exit-row borrow check
  (`check.rs:6108`, `if outer_locals.contains(place)`), and `check_reference_across_back_edge`
  (`check.rs:5519`), whose diagnostic `reference_across_back_edge_error` (`check.rs:5501`) is the
  wording model for past-owning-frame. A word's declared inputs start as `Slot::computed` with
  `deriv: None` (so a `&T` **parameter** carries no `owned_root` in this frame), while a borrow of a
  frame-local carries `owned_root: Some(place)` — the distinction is already load-bearing.
- **The liveness walk to extend (recon 3).** `capture_alive_names` (`check.rs:1054`) fixed-point
  unions `quotation_captures(id)` for every slot/binding whose `quot` is `Some(QuotRef::Known(id))`
  and is itself alive; `live_derivs` (`check.rs:1097`) folds it into the derivation-liveness scan.
  Its whole reach is the `if let Some(QuotRef::Known(id))` guard (`check.rs:1066`/`:1077`): the
  moment `quot` becomes `None` at erasure, the value drops out. Phase 2 teaches it to also read an
  erased slot's surviving capture set.
- **The single-variant marker (recon 4).** `enum QuotRef { Known(QuotId) }` (`check.rs:88`, dated
  note at `:85`) and `Slot.quot: Option<QuotRef>` (`check.rs:121`). A join of two *different*
  literals is necessarily erased (`quot: None`); there is no variant to carry "one of a or b", so
  the join's unioned capture set (Q4) rides on a new `Slot` field, not on `QuotRef`.
- **The env, hardcoded null (recon 5).** `materialize_quot_value` (`ir.rs:4170`) writes
  `Const(env, 0)` at `ir.rs:4188` and stores it into the `env` slot; `lower_indirect_call`
  (`ir.rs:4245`) loads only `code`. `lower_materialized` (`ir.rs:2370`) mints the callee `IrFunc`
  with signature = the quotation's declared effect and **no env parameter**, which is why a captured
  name falls through to user-word resolution and panics (`ir.rs:3488`/`:3577`, probe P2).
- **The structural escape check is blind to the env (recon 6).** `contains_reference`
  (`check.rs:285`) recurses `Struct`/`Enum`/`Array` hunting `Type::Ref` with a `_ => false`
  wildcard, so `Type::Quotation` is invisible to it. The word-output escape guard (R22) therefore
  cannot come from this predicate; it must read the quotation's surviving capture set directly.
- **The `^` owned-cell construction rejects a reference payload (recon 7).** `check_owned_cell_word`
  (`check.rs:8482`), `"^"` arm, calls `contains_reference(payload, ...)` and returns
  `constructed_reference_error` (`check.rs:8035`). A heap `^Env` carrying reference fields would hit
  this; this slice does not build a heap env (Q2a), so no carve-out here is needed or added.
- **Copy-ness (Q4/7a).** `is_copy` (`check.rs:239`) treats `Type::Quotation` under the `_ => true`
  arm: a quotation value is Copy. With D4 (env holds references, which are non-owning), every closure
  this slice admits stays Copy — see the Exit note on the linear-capture criterion.
- **The by-value `call` (recon 9).** `call` handling at `check.rs:6715`: a `Known` literal splices,
  an abstract `Type::Quotation` (erased) checks via `check_abstract_quotation_call` and lowers
  indirect. There is no `&q`/`&!q` reference-mode call anywhere (Q3 keeps it that way).
- **IR / backend (7a, reused unchanged except the env param).** `IrType::Code`/`IrType::Quotation`
  (`ir.rs:82`), `quotation_layout` (`ir.rs:200`), `Instr::FuncAddr`/`CallIndirect`
  (`ir.rs:1007`/`:1013`) with QBE emission (`qbe.rs:979`/`:985`), `is_call_instr`
  (`ir.rs:4855`). The materialized-body signature (`lower_materialized`, `ir.rs:2370`) gains one
  env parameter; the indirect call (`ir.rs:4245`) gains one argument.

## Locked decisions (from the brief, binding)

- **D1.** The four materialization boundaries are unchanged; only the admission rule at them changes.
  No fifth boundary, no moving the existing four.
- **D2.** Direct-splice capture needs no new checking (recon 1, exhaustive): every
  quotation-parameter position inlines. Nothing about `call`, `times`, or a combinator's own
  quotation argument changes. Splicing comes out bit-identical, pinned by the 7a regression units
  (`ir.rs:5214`/`:5228`, widened per 7a's R13a) — carried forward here as R26.
- **D3.** Reuse `quotation_captures`/`capture_names` as the capture-set source; add no new capture
  analysis. The env's fields are enumerated from the existing cached set, keyed by `QuotId`.
- **D4.** The env holds a **reference** for a captured **aggregate** value (a `Type::Struct`,
  `Type::Enum`, `Type::Array`, or `Type::OwnedCell` local) and for an explicit **borrow**
  (`&x`/`&!x`, a `Type::Ref`), never a snapshot — including an escaping one. (`map` in
  `lib/combinators.sth` depends on the late read of a captured array.) A materialized closure over
  an aggregate or borrow means what the spliced one means precisely because it reads through a live
  reference.

  **D4 amendment (scalar snapshot).** A captured **scalar** value — any local whose type is NOT
  `Type::Struct | Type::Enum | Type::Array | Type::OwnedCell` and is not itself a `Type::Ref`, i.e.
  precisely the class `check_borrow_word` (`check.rs:8210`–`:8213`) already uses to reject `&`/`&!`
  on a local (`i64`/`u64`/`f32`/`f64`/`bool`/`usize` etc.) — is **snapshotted into the env (copied),
  not referenced**. This is sound, not a hedge, and loses nothing D4 was protecting: a scalar local
  can never be mutated after capture through any path Sooth admits, because (a) a scalar has no
  address and so can never be borrowed at all (`borrow_of_scalar_local_error`, `check.rs:7845`), and
  (b) R4 forbids rebinding a name already in scope, for Copy values too (`check.rs:5563`). So a
  reference to a captured scalar and a snapshot of it are observationally identical in every program
  the checker admits. An aggregate value or an explicit borrow *can* be mutated in place after
  capture (D4's original motivating case, `map`'s captured-array mutation), so those still need a
  live reference. Consequence: a scalar snapshot occupies the same one-word `env` slot a reference
  would (Q2a's null/inline/bundle ladder is unchanged), but it never needs a surviving-capture-set
  entry, never participates in R20's liveness extension, and never participates in R23's join union
  — a snapshot has no referent that can go dead. Only an aggregate-value or borrow capture needs
  that machinery (R19/R20/R23).

## Resolved questions

- **Q1 — capture-set survival / sequencing.** Resolved above: floor first (Phase 1), env-plumbing
  capability second (Phase 2). The floor closes the `make-a` hole with the `owned_root` test; the
  surviving capture set (a new `Slot` field, Phase 2) is what lets `capture_alive_names` keep a
  frame-local capture live past erasure so the motivating programs are admitted soundly. Per the
  probe's verdict, there is no single existing evaluation point that admits the sound programs and
  rejects the unsound one, so both pieces are genuinely needed; neither is the deferred "no new
  machinery" option the brief originally imagined.

- **Q2a — where the env lives (representation).** Decided: **no heap `^Env` in this slice.**
  - 0 captures → `env` = null (7a, unchanged).
  - exactly **one** word-sized reference capture → the reference is stored **inline** in the `env`
    slot itself (no bundle, no allocation). Works for both an escaping (Phase 1, `make-b`) and an
    in-frame closure.
  - **2+** captures → a synthesized positional **bundle** of the captured references (built like
    `intern_bundle_struct`, `ast.rs:433`), whose storage is **stack-allocated** (`Instr::Alloc`) in
    the materializing frame; the `env` slot holds the pointer. A stack bundle dies at return, so a
    2+-capture closure is admissible only at an **in-frame** boundary, never at a word-output one
    (R16/R22).

  Reasoning: nothing forces heap for anything this slice must ship. `make-b` (the only escaping
  example) captures one reference. References are non-owning (`is_copy` → Copy), so the quotation
  value stays Copy in every admitted case: no `drop` synthesis, no `^`-site carve-out (recon 7), no
  owned-cell linearity. This is the least-apparatus choice consistent with D4 and the project's
  craft ethos. A heap `^Env` (needed only for a 2+-capture *escaping* closure, which no motivating
  program and no exit criterion requires) is explicitly out of scope, along with its `^`-site
  carve-out and drop plumbing.

- **Q2b — what an escaping capture may be rooted in (admission).** The `owned_root`-vs-current-frame
  precedent (`check.rs:6108`/`:5519`), reused verbatim in spirit. A captured reference rooted in a
  **parameter or global** may cross any boundary (its referent outlives the frame). A captured
  reference rooted in a **current-frame local** may not cross a **word-output / escaping** boundary
  (its storage dies at return, `make-a`); it may cross an **in-frame** boundary only under Phase 2's
  surviving-set liveness (R21). This copies an existing rule rather than inventing a lifetime system
  — the non-goal Slice 6 explicitly bought (NF2).

- **Q3 — `call` through `&q`/`&!q`.** Decided: **no `&q`/`&!q` surface syntax, and no FnMut shape,
  in this slice.** By-value `call` keeps its 7a semantics exactly (splice a `Known` literal, indirect
  for an erased value; `check.rs:6715`, `ir.rs:4245`). Because every admitted closure is Copy (Q2a),
  calling a stored/erased closure **repeatedly** is already expressible as `dup call`: `dup` copies
  the two-slot value (both slots non-owning), so a dispatch-table entry or an `each` body can call the
  same closure per element with no consume-and-lose problem. There is therefore no closure that `&q`
  would make "callable again" that `dup` does not already. And D4 forbids closure-local mutable state
  (mutation is routed entirely through what the captured reference points *at*, never through
  separate state owned by the closure), so there is no FnMut/`&!q` "call with internal state change"
  semantics to define. Inventing `&q`/`&!q` here would be new grammar + new checking from nothing
  (recon 9) that no exit criterion needs. `&q`/`&!q`, the Fn/FnMut/FnOnce split, and an owning-`^Env`
  closure are deferred together with the heap-env case.

- **Q4 — join capture-set unioning.** A join of two *different* capturing/erased literals produces
  one erased value that might be either (`quot: None`, `check.rs:7195`). Its surviving capture set is
  the **union** of both arms' sets. Since `QuotRef` is single-variant (`Known` only) and cannot
  represent "one of a or b", the set rides on the new erased-slot field (R19), not on `QuotRef`.
  **No cap** on compounding across further joins/stores: the union is monotone and bounded by the
  count of names live in the enclosing scope, and a cap ("at most one join before the set is frozen")
  is unnecessary apparatus for a bound that is already small — the craft ethos says do not add it.

- **Q5 — retire the old R12 wording.** `capturing_quotation_error`'s
  "capturing closures are slice 7b" (`check.rs:6204`) has no correct audience once 7b ships (recon
  12) and is **retired**. It is replaced by two new located diagnostics (R24): **past-owning-frame**
  (an escaping closure captures a local of this frame whose storage does not survive the return —
  modeled on `reference_across_back_edge_error`, `check.rs:5501`) and **past-last-use** (a captured
  reference is read after its referent's last use / after the referent is consumed or exclusively
  re-borrowed). The four 7a goldens that asserted the exact old string
  (`capturing_literal_stored_is_error_naming_7b`, its array-element variant
  `capturing_literal_stored_in_array_element_is_error_naming_7b`, the nested variant
  `capturing_through_nested_quotation_is_error`, and
  `capturing_literal_at_join_is_error_naming_7b`; `tests/phase4_quotations.rs:123`/`:143`/`:160`/
  `:231`) all capture a bare **scalar** local (`10 | x | [ x + ]`), which the D4 amendment
  snapshots, so under R15 rule 1 they now **compile** and are re-pointed to *positive* tests (R25),
  not to past-owning-frame rejects. Only a genuinely **frame-rooted aggregate-value or borrow**
  escape triggers past-owning-frame; the four re-pointed goldens are no longer examples of it. The
  actual Phase-1 past-owning-frame golden is **T-makea** (`make-a`, an escaping borrow of a
  frame-local aggregate `[i64 4]`) — it already covers that case, so no additional past-owning-frame
  golden is needed from the old 7a set.

## Requirements

Continues 7a's R-numbering (7a defined R1–R13/R13a). **R12 (7a) is retired** (Q5). New 7b
requirements are R14+.

**Retire the blanket rejection; admission rule (Phase 1)**

- **R14.** Retire the two blanket-capture rejection sites — `materialize_quotation_at_boundary`'s
  `if body_captures_enclosing { return Err(capturing_quotation_error(...)) }` (`check.rs:6238`) and
  the inline join copy (`check.rs:7204`). A capturing literal at a boundary is no longer rejected for
  *being* capturing; it is admitted or rejected by the admission rule (R15). `body_captures_enclosing`
  (`check.rs:6150`) stays as the cheap "does it capture at all" gate deciding whether the admission
  rule and env build run; `capturing_quotation_error` is deleted.
- **R15.** Admission rule (Q2b), a **three-way classification on capture kind**, branched *before*
  the frame-rooted/outer-rooted test even applies. For each captured name (the set is
  `quotation_captures(id)`, D3; no new analysis):

  1. **Scalar value capture** — the name binds a scalar local (D4's snapshot class: not
     `Struct`/`Enum`/`Array`/`OwnedCell`, and not a `Type::Ref`). **Always admissible, at every one
     of the four boundaries including word-output**: the env snapshots it (D4 amendment), so it can
     never dangle. No `owned_root` test, no surviving-set entry. (This is the new first branch, and
     it is what admits the four re-pointed 7a goldens, R25.)
  2. **Aggregate value capture, no derivation** — the name binds an aggregate local
     (`Struct`/`Enum`/`Array`/`OwnedCell`) read *directly*, not via `&`/`&!`, so it carries no
     `Deriv`/`owned_root` (the general `[ arr ... ]` shape where `arr` the aggregate itself, not a
     borrow of it, is read inside the quotation). Classify by whether the *local's binding* belongs
     to the current frame — the same current-frame membership test the R12 exit-row check
     (`check.rs:6108`) and `check_reference_across_back_edge` (`check.rs:5519`) already apply to a
     `place` name, here applied to the captured name directly rather than to a `Deriv.owned_root`.
     **Frame-rooted** (the local is a binding of the current frame) → the original rule: rejected at
     a **word-output** boundary with past-owning-frame (R24), admitted at an **in-frame** boundary
     per R21 (Phase 2). **Outer-rooted** (an aggregate parameter/global) → admitted at any boundary.
  3. **Explicit borrow capture** — the name binds a `&x`/`&!x`, carrying a `Deriv`/`owned_root`.
     Unchanged from the original rule: read `Deriv.owned_root` and compare against the current
     frame's locals (the machinery `check.rs:6108`/`:5519` already uses). `owned_root` in the
     **current frame** → frame-rooted (rejected at word-output, admitted in-frame per R21);
     `owned_root` in a **parameter/global** → outer-rooted, admitted at any boundary. This is
     `make-a` (frame-rooted, rejected) vs `make-b` (parameter, admitted).

  Boundary summary: a **scalar** capture (kind 1) admits at all four boundaries unconditionally; an
  **aggregate value or borrow** capture (kinds 2/3) admits an *outer-rooted* capture at any
  boundary, admits a *frame-rooted* capture only at an **in-frame** boundary (Phase 2, R21), and
  rejects a frame-rooted capture at a **word-output** boundary with past-owning-frame (R24).

**Env representation and lowering (Phase 1: inline single; Phase 2: bundle)**

- **R16.** Env layout per Q2a: 0 captures → `env` null (7a, unchanged); exactly one word-sized
  reference capture → the reference stored inline in the `env` slot; 2+ captures → a synthesized
  positional bundle of references (like `intern_bundle_struct`, `ast.rs:433`) stack-allocated in the
  materializing frame, `env` holding the pointer. `env` stays `IrType::Ptr` (opaque; NF1). The
  inline single case lands in Phase 1; the bundle in Phase 2 (it is only reachable by an in-frame
  multi-capture closure, R21).
- **R17.** Lowering. `lower_materialized` (`ir.rs:2370`) mints the callee `IrFunc` with **one extra
  env parameter** (`IrType::Ptr`) appended after the declared inputs; inside the body a captured name
  resolves to a read from `env` (inline: the `env` value itself; bundle: a field load at the
  capture's bundle offset). `materialize_quot_value` (`ir.rs:4170`) builds the env from the live
  borrow values instead of `Const(env, 0)`: inline → store the reference into the `env` slot; bundle
  → `Alloc` the bundle, `FieldStore` each reference, store the pointer. `lower_indirect_call`
  (`ir.rs:4245`) loads the `env` slot and passes it as the trailing argument. The 0-capture path is
  byte-identical to 7a.
- **R18.** Phase 1 escaping single-capture. A **word-output** boundary admits exactly one
  word-sized capture stored inline in `env`: either one **outer-rooted reference** (`make-b`,
  compiles and runs, R15 rule 3) or one **scalar snapshot** (R15 rule 1 / D4 amendment — e.g.
  `pick`'s returned `[ x + ]` over a scalar `x`, T-repoint-join compiles and runs). `make-a` (a
  frame-rooted **aggregate** borrow at a word-output boundary, R15 rule 3) is rejected by R15/R24. A
  2+-capture escaping closure is rejected with a located "an escaping closure may capture at most
  one reference (a heap env is deferred)" error until the deferred heap-env work lands.

**Surviving capture set; same-frame captures (Phase 2)**

- **R19.** Extend the erased `Slot` with an optional **`Copy` interned handle** to a surviving
  capture set — a `SurvivingCaptureSetId` (a `Copy` id into a side table), **not** an inline
  `HashSet<String>`-shaped field, which would force `Slot` (derived `Copy` at `check.rs:102`) to
  drop `Copy` and ripple through every shuffle/join/stack move that assumes `Slot: Copy` today. This
  follows the exact pattern `QuotId` / `QuotRef::Known(QuotId)` already uses to keep
  `Slot.quot: Option<QuotRef>` `Copy` (`check.rs:89`/`:121`). The actual sets live in a
  `Vec<HashSet<...>>`-style table keyed by that id, mirroring how `Provenance::quotation_captures`
  (`Vec<HashSet<String>>`, field `check.rs:437`, accessor `:544`) already stores capture sets by
  `QuotId`. Each entry holds the surviving names with their root classification and the
  `DerivId`/`owned_root` they resolve to. **A scalar-snapshot capture is never a member of a
  surviving set** (D4 amendment): only aggregate-value and borrow captures are tracked, so a closure
  capturing only scalars carries no `SurvivingCaptureSetId` at all — there is nothing that can go
  dead. Set at every materialization boundary (R14 path) for an aggregate/borrow capture and
  forwarded by shuffles (`Slot` stays `Copy`). Because `QuotRef` is single-variant, this rides on a
  new `Slot` field, not on `QuotRef`.
- **R20.** Extend `capture_alive_names` (`check.rs:1054`) and thereby `live_derivs`
  (`check.rs:1097`) to also union an erased slot's/binding's surviving capture set (R19), not only
  captures reachable through `Some(QuotRef::Known(id))` (`check.rs:1066`/`:1077`). This keeps a
  frame-local capture's borrow live from the store past the call site, so a consume/exclusive-reborrow
  of the referent before the call is rejected (past-last-use, R24) exactly as it is for a still-`Known`
  closure today (probe P4 contrast: `lateread_known` rejected, `lateread` wrongly accepted). Only
  aggregate-value and borrow captures are members of a surviving set (R19 / D4 amendment); a
  scalar-snapshot capture never enters this liveness extension, because a snapshot has no referent
  to go dead.
- **R21.** Admit **frame-rooted** captures at **in-frame** boundaries (struct field, array element,
  join): the two motivating programs — a dispatch table (array of capturing closures) and a
  struct-stored closure observing a later mutation — compile and run, observing the same values the
  spliced form does, with R20 enforcing the referent stays live to each call.
- **R22.** Word-output escape guard over the surviving set. At a **word-output** boundary, reject a
  returned quotation — directly, or transitively through a returned struct field / array element
  carrier — whose surviving capture set (R19) includes a **frame-rooted** capture, with
  past-owning-frame (R24). This is a targeted walk over the surviving capture set of a returned
  quotation-typed (or quotation-containing) slot; it cannot come from `contains_reference`
  (`check.rs:285`), which is structurally blind to the env (recon 6). The walk reuses `owned_root`
  classification (R15). Scalar-snapshot captures are absent from the surviving set by construction
  (R19), so they never trigger this guard — only a frame-rooted aggregate-value or borrow capture
  can.
- **R23.** Join union (Q4). At a differing-literal join (`check.rs:7195`), the merged erased slot's
  surviving capture set (R19) is the union of both arms'. The join **interns a new set** (a fresh
  `SurvivingCaptureSetId` for the union); it does **not** mutate either arm's existing set in place
  — this is what keeps the field `Copy`-compatible, and it is a real (bounded) allocation per join,
  not free. A scalar-captured arm contributes nothing to the union (no surviving-set membership,
  R19 / D4 amendment). No cap.

**Diagnostics (Phases 1–2)**

- **R24.** Two new located diagnostics replacing `capturing_quotation_error` (retired, Q5):
  - **past-owning-frame** — `error: an escaping closure captures`{name}`, a local of this frame,
    whose storage does not survive the return (line {n})` (wording modeled on
    `reference_across_back_edge_error`, `check.rs:5501`). Fires for `make-a` and R22.
  - **past-last-use** — `error: a captured reference to`{name}`is read after its last use (line
    {n})` (or after the referent is consumed / exclusively re-borrowed). Fires from R20.

  Each names the capture. Each has an exact-message unit/golden test (diagnostics are behavior).
- **R25.** Re-point the four 7a goldens that asserted the retired string
  (`capturing_literal_stored_is_error_naming_7b` `tests/phase4_quotations.rs:123`,
  `capturing_literal_stored_in_array_element_is_error_naming_7b` `:143`,
  `capturing_through_nested_quotation_is_error` `:160`,
  `capturing_literal_at_join_is_error_naming_7b` `:231`). Every one captures a bare **scalar** local
  `x: i64` (`10 | x | [ x + ]`, or the nested variant), which under the D4 amendment / R15 rule 1 is
  a snapshot and is **always admissible**. So all four now **compile**, not reject; each is
  re-pointed to a **positive** test with a `call` **added** so it observes the captured scalar's
  value (not a vacuous store-then-drop). Do not delete them.
  - `:123`, `:143`, `:160` → positive tests of R15 rule 1 at an **in-frame** boundary, landing in
    **Phase 1** (rule 1 has no Phase 1/2 distinction — it is unconditional admission — and the
    admission-pass change lands in Phase 1). Each adds a `call` and asserts the observed value.
    `:160` keeps its **nested-quotation** shape (it uniquely exercises transitive capture through
    `capture_names` recursion), adds a `call`, and asserts the printed value.
  - `:231` (`pick`, returns the join) → a positive **run** test under its own golden name
    **T-repoint-join**: the returned closure, called, produces the correct value regardless of which
    branch was taken — a scalar capture surviving a **word-output** boundary via snapshot (inline
    env, R18). This is a *different* kind of test from **T-makeb** (which is a *reference* capture of
    a parameter, not a snapshotted scalar), so it gets its own name and rationale.
  - **Unchanged by this amendment:** `make-a`/**T-makea** (`0 4 fill | arr | [ &arr ... ]`) captures
    via an explicit **borrow** of an **aggregate** (`[i64 4]`) — R15 rule 3, `owned_root` in the
    current frame — and is still correctly rejected past-owning-frame; do not weaken it. **T-makeb**
    (capture of a `&[i64 4]` **parameter**) is also unaffected — rule 3, `owned_root` in a
    parameter, still admissible.

**Regression protection (Phase 3, verifies 7a's pins still hold)**

- **R26.** Every 6a–6f combinator golden and the two 7a splice-vs-indirect pins
  (`call_of_literal_emits_no_call_instr` `ir.rs:5214`, `times_lowers_...` `:5228`, and the
  `is_call_instr` sites `ir.rs:4855`) stay green and bit-identical: splicing is unchanged (D2). The
  added env parameter must not turn any spliced literal into a `CallIndirect`.

## Non-functional requirements

- **NF1 — backend-neutral IR.** `env` stays `IrType::Ptr` and `code` stays `IrType::Code` (opaque
  handles); no code assumes either is a `u64` or does pointer arithmetic on them (CLAUDE.md
  invariant; `ir.rs:146` documents `Code`'s opacity). A future WASM lowering must be free to realize
  `env` as a linear-memory offset and `code` as a table index. The bundle offsets are word-width
  derived (`WORD_WIDTH`), never hardcoded.
- **NF2 — no lifetime apparatus.** Admission reuses the existing `owned_root`-vs-current-frame test
  (`check.rs:6108`/`:5519`); no lifetime variables, regions, or generic escape solver are
  introduced. Slice 6 explicitly bought "no lifetime apparatus" and this slice does not spend it.
- **NF3 — no premature heap/drop/`^` machinery.** No heap `^Env`, no `drop` synthesis for a
  quotation value, no carve-out at `check_owned_cell_word`'s `^` arm (`check.rs:8482`). Every
  admitted closure is Copy (D4 → references-only env). These are deferred with the heap-env case.
- **NF4 — splice is untouched (D2).** `call`/`times` on a `Known` literal lower byte-identically;
  the regression pins (R26) are the guarantee.

## Scope

**In scope:** widening the admission rule at 7a's four boundaries (D1); building and reading the
`env` (inline single reference in Phase 1, stack bundle in Phase 2); the surviving capture set and
its liveness extension; the join union; the two new diagnostics; a dogfood program.

**Out of scope** (reuse the brief's list; not re-litigated):

- Opt-in RC (`Rc`/`Arc`-equivalent) — deferred to Phase 6.
- Any change to non-capturing quotation behaviour — 7a's splice/materialize/indirect-call story is
  unchanged; this slice only widens the admission rule at the same four boundaries (D1).
- Any change to combinator inlining — `each`/`map`/`fold`/`while`/`times` stay force-inlined (6a's
  D2); captures are already free there (recon 1).
- The clause-body rejection (`clause_bodied_quotation_word_error`, `check.rs:1641`) — unrelated (splicing a callee, not representing a
  captured value).

**Additionally out of scope for this slice** (deferred, decided above):

- Heap `^Env` and any owning/linear closure, its `^`-site carve-out, and its `drop` plumbing (Q2a,
  NF3).
- A 2+-capture **escaping** closure (needs the heap env) — rejected with a located error in Phase 1
  (R18).
- `&q`/`&!q` reference-mode `call` and the Fn/FnMut/FnOnce split (Q3, recon 9).
- Polymorphic quotation *values* — as in 7a, only concrete-effect quotations materialize.

## Exit criteria (golden tests)

| ID | Test | Kind | Phase | Source in → expected out |
|----|------|------|-------|--------------------------|
| T-makeb | `escaping_closure_over_param_ref_compiles_and_runs` | run | 1 | `make-b` (captures a `&[i64 4]` parameter), returned and called → same value as the spliced form; path emits `CallIndirect` and a non-null `env` |
| T-makea | `escaping_closure_over_frame_local_is_past_owning_frame` | reject | 1 | `make-a` (captures a `[i64 4]` local of its own frame), returned → exact past-owning-frame message via `assert_eq!` |
| T-env-inline | `materialized_single_capture_builds_inline_env` | unit+IR | 1 | one-capture materialization → `env` slot holds the reference, no `Alloc` bundle, `IrFunc` has one extra `Ptr` param |
| T-multi-esc | `multi_capture_escaping_closure_is_rejected_deferred` | reject | 1 | a returned closure capturing two param refs → exact "at most one reference (heap env deferred)" message |
| T-dispatch | `dispatch_table_of_capturing_closures_runs` | run | 2 | an array of same-frame capturing closures, indexed and `call`ed in-frame → same values as spliced; `CallIndirect` present |
| T-lateread | `struct_stored_closure_observes_later_mutation` | run | 2 | a closure stored in a struct field, its captured array mutated, then `call`ed → observes the mutation (D4 late read); `CallIndirect` present |
| T-lastuse | `captured_reference_read_past_last_use_is_error` | reject | 2 | a same-frame capture whose referent is consumed/exclusively re-borrowed before the `call` → exact past-last-use message via `assert_eq!` (contrast: `lateread_known`-shaped program stays rejected as today) |
| T-bundle | `materialized_multi_capture_builds_stack_bundle` | unit+IR | 2 | two-capture in-frame materialization → `Alloc` bundle, two `FieldStore`s, `env` = bundle pointer; offsets word-width derived |
| T-carrier | `frame_capture_escaping_via_struct_is_past_owning_frame` | reject | 2 | a same-frame-capturing closure stored in a struct that is then returned → exact past-owning-frame message (R22; `contains_reference` cannot catch this) |
| T-join | `join_of_two_capturing_arms_unions_capture_sets` | run+IR | 2 | two differing capturing literals joined, the merged closure `call`ed while both arms' captures live → correct value. A pure run test cannot prove the set is the *union* (it can pass with only one arm tracked); pair with T-join-union (R23) |
| T-join-union | `join_capture_union_kills_one_arm_referent_is_past_last_use` | reject | 2 | companion to T-join: one arm captures an **aggregate/borrow** referent (R15 rule 2/3); consume or exclusively re-borrow that referent before the `call` → exact past-last-use via `assert_eq!`, which fires *only* if that arm's capture actually entered the unioned surviving set (R23). Applies only to an aggregate/borrow-captured arm — a scalar-captured arm has no surviving-set membership (R19) |
| T-repoint | three 7a goldens (`:123`/`:143`/`:160`) re-pointed to positive **in-frame** tests | run | 1 | R15 rule 1 (scalar snapshot) admits; each adds a `call` and asserts the observed value (`:160` keeps its nested-capture shape, R25) |
| T-repoint-join | `capturing_literal_at_join_is_error_naming_7b` (`:231`) re-pointed | run | 1 | a scalar capture surviving a **word-output** boundary via inline-env snapshot (R18); returned closure `call`ed → correct value regardless of branch; distinct from T-makeb (reference, not snapshot) |
| T-reg | 7a's splice pins (`ir.rs:5214`/`:5228`, `is_call_instr` sites) stay green | IR | 3 | no spliced literal regresses into `CallIndirect` when the env param is added (R26) |
| T-dogfood | `capturing_dispatch_matches_spliced_version` | run+IR | 4 | a new `examples/*.sth` capturing-closure program == its hand-spliced twin's stdout; path emits `CallIndirect` with a non-null `env` |
| T-roadmap | ROADMAP 7b marked implemented | doc | 4 | prose exit line; no test |

**Exit note on the brief's linear-capture line.** The brief's Exit says "dropping a
linear-capturing closure disposes its captures." Under this slice's D4 + Q2a (the `env` holds only
**references**, which are non-owning / `is_copy` → Copy), **no closure this slice admits is ever
linear** — there is no owned capture to dispose. That criterion is therefore vacuously satisfied
here and its substance belongs to the deferred heap-`^Env` / owning-closure work (NF3). This is a
deliberate, documented narrowing of the brief's Exit, flagged rather than silently dropped.

## Load-bearing / mutation-test-required criteria

Prove each guard can fail by reverting it in a **throwaway copy** of the compiler (not a shared
worktree), per the project's mutation-testing convention.

- **M1 (R15 admission — T-makea/T-makeb).** Force the admission classifier to treat every root as
  outer-rooted: T-makea must stop rejecting (the make-a hole reopens). Force it to treat every root
  as frame-rooted: T-makeb goes red (a sound param capture wrongly rejected). Proves the
  `owned_root`-vs-current-frame test is the switch, not a blanket rule.
- **M2 (R20 surviving-set liveness — T-lastuse).** Drop the erased-slot union added to
  `capture_alive_names`: T-lastuse stops rejecting (the referent reads dead before the call, exactly
  the probe-P4 erasure gap). Proves the surviving set actually feeds liveness.
- **M3 (R22 escape guard — T-carrier).** Delete the surviving-set walk at the word-output boundary:
  T-carrier stops rejecting (a frame capture escapes through a struct, undetectable by
  `contains_reference`). Proves the guard is real and not delegated to the blind structural check.
  *Verify at mutation-test time (implementer note):* before trusting M3, confirm in the throwaway
  copy that the pre-existing R12 exit-row borrow check (`check.rs:6108` area) does not already
  incidentally reject T-carrier's returned struct for an unrelated reason — if it does, M3 is a
  placebo and T-carrier proves nothing about R22.
- **M4 (R17 env is read — T-makeb/T-dispatch).** Force the materialized body to ignore its env
  parameter (resolve captures as before): T-makeb/T-dispatch go red or panic. Proves the env is
  built *and* read, closing the probe-P2 "env null and never read" gap.
- **M5 (R26 splice unchanged — 7a pins).** Mutate a `Known`-literal `call` lowering to emit
  `CallIndirect`: the 7a pins (`ir.rs:5214`/`:5228`) must flip red. Proves adding the env parameter
  did not silently reroute splicing.

## Sanctioned files

- `src/check.rs` — retire the blanket rejection (R14); admission rule (R15); surviving capture set
  on `Slot` (R19); `capture_alive_names`/`live_derivs` extension (R20); in-frame admission (R21);
  word-output escape guard (R22); join union (R23); the two new diagnostics (R24); unit tests.
- `src/ir.rs` — env param on the materialized body (R17, `lower_materialized`); env build in
  `materialize_quot_value` (R16/R17); env pass in `lower_indirect_call` (R17); bundle synthesis
  (R16); unit tests.
- `src/backend/qbe.rs` — only if the env parameter needs an ABI adjustment (it is one `Ptr` arg,
  `l`, so likely none); unit test if touched.
- `src/repl.rs` — touched only if the new `Slot` field or IR shape requires it.
- `tests/phase4_quotations.rs` — re-pointed 7a goldens (R25) plus the new goldens above (per-file
  harness convention).
- `examples/*.sth` — the dogfood (T-dogfood); a hand-spliced twin as the parity oracle.
- `ROADMAP.md` — mark 7b implemented.

No other files.

## Phased delivery

Each phase must be green under `cargo fmt --check && cargo clippy --all-targets -- -D warnings &&
cargo test`.

- **Phase 1 — soundness floor + escaping single-capture.** R14, R15, R16 (inline single only), R17
  (env param + inline build/read), R18, R24 (past-owning-frame), R25. Goldens T-makeb, T-makea,
  T-env-inline, T-multi-esc, T-repoint, T-repoint-join. Drive M1, M4 (partial), M5.
- **Phase 2 — surviving capture set + in-frame same-frame captures.** R16 (bundle), R19, R20, R21,
  R22, R23, R24 (past-last-use). Goldens T-dispatch, T-lateread, T-lastuse, T-bundle, T-carrier,
  T-join, T-join-union. Drive M2, M3, M4.
- **Phase 3 — regression pins.** R26; confirm the 7a splice pins stay green with the env parameter
  in place (T-reg). No new capability.
- **Phase 4 — dogfood + ROADMAP.** The capturing-closure example and its spliced-twin parity golden
  (T-dogfood); mark ROADMAP 7b implemented (T-roadmap).

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Soundness floor + escaping single-capture: retire 7a's blanket capturing-quotation boundary rejection (R14, delete capturing_quotation_error at check.rs:6238 and the inline join copy at :7204); add the admission rule (R15) as a three-way classification on capture kind: (1) a scalar value capture is always admissible at every boundary via an env snapshot (D4 amendment, the scalar class check_borrow_word rejects &/&! on at check.rs:8210-8213), (2) an aggregate value capture (no deriv) is classified by whether the local's binding belongs to the current frame, and (3) an explicit borrow capture is classified by owned_root-vs-current-frame reusing the test at check.rs:6108/:5519; build and read an inline single-reference env (R16/R17) by giving the materialized IrFunc one extra Ptr param (lower_materialized, ir.rs:2370), building env in materialize_quot_value (ir.rs:4170, replacing Const(env,0) at ir.rs:4188) and passing it in lower_indirect_call (ir.rs:4245); reject a 2+-capture escaping closure as deferred (R18); add the past-owning-frame diagnostic (R24, modeled on reference_across_back_edge_error at check.rs:5501); re-point the four 7a goldens (tests/phase4_quotations.rs:123/:143/:160/:231) to positive tests since all four capture a bare scalar and now compile (R25): the three in-frame goldens (:123/:143/:160) admit via R15 rule 1 with an added call, and the join golden (:231, T-repoint-join) runs as a scalar snapshot surviving a word-output boundary via inline env (R18). Goldens T-makeb, T-makea, T-env-inline, T-multi-esc, T-repoint, T-repoint-join; drive mutation tests M1, M5.",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "Surviving capture set + same-frame in-frame captures: add an optional surviving capture set field to Slot (R19) as a Copy interned SurvivingCaptureSetId into a side table (not an inline HashSet, which would break Slot: Copy at check.rs:102; mirrors the QuotRef::Known(QuotId) and Provenance::quotation_captures patterns at check.rs:437/:544), set at every boundary for aggregate/borrow captures only (a scalar snapshot is never a member) and forwarded by shuffles; extend capture_alive_names (check.rs:1054) and live_derivs (:1097) to union an erased slot's surviving set, not only Some(QuotRef::Known) reachable captures (R20); admit frame-rooted aggregate/borrow captures at in-frame boundaries so the dispatch-table and struct-stored-closure programs run (R21); add the stack-allocated multi-capture bundle env built like intern_bundle_struct at ast.rs:433 (R16 bundle case); add the word-output escape guard walking the surviving set (R22, since contains_reference at check.rs:285 is blind to the env); union both arms' surviving sets at a differing-literal join by interning a new set (R23, check.rs:7195); add the past-last-use diagnostic (R24). Goldens T-dispatch, T-lateread, T-lastuse, T-bundle, T-carrier, T-join, T-join-union; drive mutation tests M2, M3, M4.",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "Regression pins: confirm every 6a-6f combinator golden and the two 7a splice-vs-indirect pins (call_of_literal_emits_no_call_instr at ir.rs:5214, times_lowers_... at :5228, and the is_call_instr sites at ir.rs:4855) stay green and bit-identical now that the materialized body carries an env parameter, so no spliced Known literal regresses into a CallIndirect (R26, NF4). Golden T-reg.",
      "difficulty": "standard"
    },
    {
      "phase": 4,
      "focus": "Dogfood and ROADMAP: add a capturing-closure example program under examples/ with a hand-spliced twin as the parity oracle, and a parity golden asserting identical stdout plus a CallIndirect with a non-null env on the closure path (T-dogfood); mark Phase 4 Slice 7b implemented in ROADMAP.md (T-roadmap).",
      "difficulty": "standard"
    }
  ]
}
```
