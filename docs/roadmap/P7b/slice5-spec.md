# P7b.S5 spec — member-word routing and env-dispatch residuals

Scoped against worktree `p7b-s5`, HEAD `4cfb887`, baseline `cargo test
--no-fail-fast` green. Discovery input: [slice5-brief](./slice5-brief.md) (recon
Round 1 **and** the appended Round 2 correction) and its verbatim
[probe log](./slice5-probes.md) (Round 1 + Round 2), plus **four** converged
reviewer rounds. Round 4's three fresh-context reviewers found zero P0s and verified
every round-3 fix genuinely landed, but BLOCKed on one central self-contradiction
(Tier 3's two mandated deliverables were mutually exclusive: a panic *and* a returned
variant to assert) plus two further design gaps (tiers applied to raw `candidates`
rather than input-matching survivors; the tier-2 golden's cited precedent never
reaches the selector). This **fifth and final** version closes all of them as
self-consistent rulings. This spec **rules** on the brief's questions; the
review-round cap is reached, so no ruling below is left open.

> **Correction notice.** Earlier versions ruled item (b) "diagnostic-only", then
> split the fix into "Phase 2a (mangle the generic registry names) + Phase 2b (widen
> the match)". **Both framings are dead.** Round 2's `pb2` proved a live silent
> cross-pick (a mono caller in one module silently executing a different module's
> same-named, same-shaped generic ctor's impl, wrong output, exit 0), so (b) is a
> real dispatch-correctness bug. The reviewer round then proved the registry-mangle
> mechanism false: the module identity the fix needs **already exists** on the decls,
> and the mangle spike is provably a no-op or breaking (F8). That deleted Phase 2a.
> This version's Phase 2a/2b split is a **different** split — a sizing split of the
> single dispatch fix, driven by the real fan-out (R3), not the dead mangle spike.

The roadmap's original S5 framing
([P7b-higher-kinded-types.md](../P7b-higher-kinded-types.md) line 141) carves three
items out of S2's golden #10:

- **(c)** matches the roadmap exactly: a pure diagnostic-text fix (R1/R2).
- **(b)** is a genuine dispatch-correctness bug: overload-candidate selection at the
  call site is blind to output type and calling module, so two same-shaped
  cross-module generic ctors both register under one bare env key and the first
  `Vec` entry silently wins regardless of the calling module (R3/R4). This spec fixes
  the dispatch by disambiguating on caller module, the identity the decls already
  carry — not by mangling the registry name.
- **(a)**'s `mono_member_unroutable_error` guard could not be triggered in either
  round (F4/F5/F10). This spec does not budget speculative routing work against an
  unwitnessed path; it requires a per-call-site reachability verdict (R5).

## Rulings

### R1 — (c): interpolate the trait header variable. (Phase 1)

`nested_receiver_member_error` (`src/check/declarations.rs:444`) formats the literal
`'F` regardless of the trait header's actual variable spelling. The function already
holds `decl: &TraitDecl` and `member: &TraitMember`.

**Where the header var actually lives (confirmed).** `TraitDecl` (`src/ast.rs:1952`)
has **no** header-variable-name field — its fields are `name`, `kind`, `var_kind`,
`var_span`, `members`, `module`, `span`. The header variable's *spelling* is reachable
only via `member.sig.ty_var_names` — the field on **`PolySig`** (`src/ast.rs:2732`),
which is what `member.sig` is; **not** `src/ast.rs:576`, which is
`GenericStructDecl::ty_var_names`, a different struct that happens to share the field
name. Index 0 is the header var. That index-0 invariant is not a field-declaration accident: the parser seeds it
in `parse_trait_member_effect`, whose first act is
`builder.intern_ty_var(ty_var, ty_var_span)` (`src/parser.rs:3926`, pushing the
header var at `PolyBuilder::intern_ty_var`, `src/parser.rs:1934`), guarded by
`debug_assert_eq!(id, 0, "the trait header var is each member sig's var 0")`
(`src/parser.rs:3931`). Read it there, not off `decl`.

Prefer `member.sig.ty_var_names.first()` over a bare `[0]` index, with a one-line
comment naming the `parser.rs:3931` invariant, so the read is robust to a
degenerate empty-sig member rather than panicking. (A trait member is guaranteed
var 0 by that assert; `.first()` costs nothing and documents the dependency.)

Fix: the *head* of the expected-shape example is that first type-variable name. The
*application argument* in the `H[A]` example is a placeholder distinct from the head:

- pinned placeholder rule: the example argument is `'T`, unless the head variable is
  literally `'T`, in which case it is `'U`. (This is the only collision the goldens
  below can produce; a general "first fresh single-letter var" is out of scope —
  three lines beats a premature abstraction.)

**Resulting text (full live format string, prefix included so an implementer does not
drop it and break the `` `pick` of `Functor` ``-style goldens):**

```text
error: trait member `{member}` of `{trait}` (line {L}, col {C}) has no input for a
call to dispatch on (expected the trait's variable `<HEAD>` bare or heading an
application like `<HEAD>[<ARG>]`)
```

Only the two backticked variable fragments change (`'F` → `<HEAD>`, `'F['T]` →
`<HEAD>[<ARG>]`); the `trait member … of … (line …)` prefix and the rest of the
sentence are byte-identical. The nested-composite `note:` line rides along unchanged
(still conditional on `member_inputs_nest_trait_var`).

### R2 — (c)'s pinned assertions: four sites, two flip. (Phase 1)

Sites that assert the literal `'F`/`'F['T]` fragment must be reconciled under R1. The
Phase-1 checklist grep **must span `src/` and `tests/` both**, and must key on the
`"heading an application like"` substring, **not** `"expected the trait's variable"`:
the latter misses the inline `src/` test whose assertion only quotes the second
fragment. This is the regression an earlier round hit — its grep on
`"expected the trait's variable"` found only three sites and wrongly concluded a
fourth "does not exist". Run `grep -rn "heading an application like" src/ tests/`.

Verified this pass — that grep returns **exactly four** hits:

1. `src/check/declarations.rs:447` — the live **format string** (R1's edit site).
2. `src/check/declarations.rs:3875` — an **inline `src/` `#[test]`**,
   `check_trait_decls_rejects_a_receiver_nested_in_an_array_input` (`:3866`), fixture
   `trait: Show['T] : sum ( array[ 'T 4 ] -- i64 ) ; ;`, asserting the literal
   `` `'F['T]` `` fragment. Header var is `'T`, so under R1 this **flips** to
   `` `'T['U]` ``. **This is the site the prior round wrongly declared nonexistent.**
3. `tests/phase7b_slice2.rs:158` —
   `hkt_member_without_dispatchable_input_is_located_error`, trait header
   `Functor['F: * -> *]`. Head var **is** `'F`, so the text is **unchanged** by R1.
   Regression witness that R1 did not perturb the common case; keep passing verbatim.
4. `tests/phase7_slice3t.rs:505` — `a_nested_receiver_member_is_still_rejected`,
   trait header `Show['T]`. Head var is `'T`, so R1 **flips** the pinned text to
   `` `'T` bare or heading an application like `'T['U]` ``; its `note:` assertion is
   unchanged.

**Two sites flip, both `Show['T]`:** the inline `src/` test (`declarations.rs:3875`)
and the `tests/` site (`phase7_slice3t.rs:505`). The Phase-1 checklist must re-pin
**both**, not one. The two `'F`-headed sites (the format string's own default is now
computed, and `phase7b_slice2.rs:158`) stay verbatim. Reconcile against the grep,
but the expected set is now known: four hits, two flip.

### R3 — (b): disambiguate on the caller's module the decls already carry. (Phase 2)

Round 2's `pb2` falsifies the "diagnostic-only" ruling: `a::run` executes `b`'s impl
(prints `2` where it should print `1`), silently, exit 0. The reviewer round then
falsified the "mangle the generic registry name" fix. The corrected mechanism:

1. **The two `StructId`s are already distinct (F7).** `instantiate_struct`'s dedup
   key is `(idx, module, args, lens)` (`src/ast.rs:1334`), and each module's `Widget`
   header is its own `GenericStructDecl` entry with its own `idx`, so the two
   `Widget[i64]` instantiations mint distinct `StructId`s. Identity is **not** the
   bug, and does not need fixing.
2. **The module identity already lives on the decls.** `StructDecl` carries
   `pub module: u32`, set on every minted monomorph by `instantiate_struct`
   (`src/ast.rs:1338`). `GenericStructDecl` / `GenericEnumDecl` also carry
   `module: u32` (`src/ast.rs:587`/`601`). But `struct_generated_sigs`
   (`src/check/declarations.rs:1815`) returns only `Vec<(String, String, Sig)>` —
   name / symbol / sig, **no module** — so its callers, which build each `Overload`,
   never receive the module. That is the missing plumbing (see R3.6).
3. **The bug is overload-candidate selection, blind to module.** Both modules' ctors
   register under `env[generic_surface_name(name)]` = `"Widget"`, appended to avoid
   clobbering, so `env["Widget"]` legitimately holds two `Overload`s (one per module,
   both input `[i64]`, distinct `StructId` outputs). The multi-candidate arm at
   `src/check/terms.rs:956`:

   ```rust
   let hit = candidates.iter().find(|o| {
       operands.len() >= o.sig.inputs.len()
           && operands[operands.len() - o.sig.inputs.len()..] == o.sig.inputs[..]
   });
   ```

   matches on **input signature only** — `o.sig.outputs` never enters, and neither
   does the caller's module. Both `Widget` overloads have input `[i64]`, so `.find`
   returns the **first `Vec` entry** (module-assembly order), regardless of the
   calling module.

**Why the mangle mechanism is dead (F8).** `resolve::resolve_modules` runs *after*
`parse_bodies` has already minted monomorphs, so mangling
`generic_structs`/`generic_enums`' names is a no-op for anything already minted, and
for anything minted later it changes the env-dispatch **key**, breaking bare call
sites. F8 measured `pb2` unchanged at `2 2` after the spike and 3 collateral test
failures. There is no registry-mangling step in this spec, so there is no
collateral-test reconciliation to budget for it.

**Ruling.** Fix the dispatch by widening `Overload` to carry its origin module, then
disambiguating multi-candidate matches on the caller's module (R3.5), with the
plumbing budgeted honestly (R3.6).

### R3.5 — the disambiguation policy: three tiers, module-only (output type dropped)

"Own module and/or output type" is underspecified, and **output type is
unimplementable** as a discriminator here: the multi-candidate arm at
`src/check/terms.rs:956` has no expected-output-type parameter in scope, and Sooth
has no bidirectional inference at a ctor call site. **Drop output type. The policy is
module-only.**

**Two-step structure (Fix B — this is the selector's actual algorithm).** Tiers apply
*after* input-matching, not to the raw `candidates` vector. There is no separate
input-narrowing pass in today's code — the input match **is** what
`candidates.iter().find(...)` does at `terms.rs:956`, in the same step as picking. The
selector splits that single step in two:

- **Step 1 — filter by input signature.** Reduce `candidates` to those whose
  `sig.inputs` match `operands` (the exact predicate today's `.find` uses:
  `operands.len() >= o.sig.inputs.len() && operands[operands.len() - o.sig.inputs.len()..] == o.sig.inputs[..]`).
  Call the survivors `matching`. This preserves today's behaviour byte-for-byte; it is
  a refactor, not a change.
- **Step 2 — apply the tiers to `matching`, never to `candidates`.** The tiers below
  disambiguate *within* `matching`. This is what makes tier 2's "exactly one remaining
  candidate" satisfiable: the raw multi-candidate arm always holds 2+ total, but after
  input-filtering, `matching` may have exactly one, zero, or 2+.

The tiers, applied to `matching`:

1. **Own-module wins.** A candidate in `matching` whose `Overload.module` equals the
   caller's **lexically-declaring** module is selected. **This must be the module that
   owns the source text making the call, not `ctx.module()`** — see the splice ruling
   below. Precedent for own-module-first ordering: `poly_construction_header`
   (`src/check/poly.rs:5609`) does own-module-first-else-foreign over
   `generics.structs`/`enums`. **Note the precedent does *not* read `ctx.module()`:**
   it takes `module: u32` as an explicit caller-supplied parameter. So it supports
   "compare candidate module against a caller module value", not "obtain that value
   from `ctx.module()`".
2. **Single visible candidate wins.** If no candidate in `matching` is own-module but
   exactly **one** is visible to the caller (per the `caller_visible` predicate,
   Fix E), that one wins (the common case — ctors are usually used from a module that
   *imports* the type). Visibility is read from `Ctx::modules()`; see R3.7 for the
   `None` path.
3. **2+ visible, or zero visible, from a `matching` set of 2+ ⇒ return
   `OverloadPick::Ambiguous`.** See R3.8 (the 2+-visible ctor shape is vacuous) and
   R3.7/Fix C (the zero-visible case, an accepted scoped regression — Ruling A).
   `Ambiguous` maps at the call site onto the existing `no_overload_matches_error`, a
   real compile error; no panic, no *newly-minted* diagnostic.

**The `caller_visible` predicate (Fix E).** Follow the existing precedent exactly:
`is_name_visible_to_module` (`src/check/word_families.rs:1155`), which the drop-import
gate at `src/check.rs:3269` (`if let (Type::Struct(id, _), Some(m)) = (top.ty, ctx.modules())`)
already routes through. Its body is
`defining == caller || modules[caller].selective.get(name) == Some(&defining)` —
own-module, **or** the name is one the caller selectively imports from the candidate's
module. This is **direct-only**: no transitive/re-export closure. Define
`visible(candidate_module) := is_name_visible_to_module(m, caller_module, candidate_module, name)`.
The `caller_module` in this predicate is **`span.module`** (the term's source module),
**not `ctx.module()`** (see the splice ruling below) — the call site closes over `name`,
`m`, and `span.module` to feed `select_overload` a `caller_visible: impl Fn(u32) -> bool`.

**Re-export transitivity is OUT OF SCOPE for S5.** A caller that sees a name only
through a module which itself re-exports it either sees zero direct-visible candidates
(Fix C's accepted zero-visible error, Ruling A) or is unaffected.
`ModuleInfo.imports`/`.selective` carry no transitive closure; widening them is
**P8.S5**'s job (`docs/roadmap/P8-packages-modules.md:240`), not S5's.

**The `ctx.module()` splice hazard (reviewer-flagged, resolved).** Tier 1 must **not**
read `ctx.module()` (`src/check/engine.rs:1284`). `ctx.module()` is re-scoped to the
**callee's** module inside an inline combinator splice: `inline_combinator`
(`src/check/combinators.rs:572`) builds `let spliced_ctx = ctx.with_module(comb.word.module)`
before re-checking the spliced body's terms, and the comment there (`:567-571`)
states this is deliberate so a library combinator's *own* module-scoped gates resolve
against the module that declares it. The multi-candidate arm at `terms.rs:956` **is
reachable under such a splice** (the splice re-enters `check_terms_relaxed` →
`check_term`, whose `Call` arm reaches this exact arm), so reading `ctx.module()`
there would make tier 1 prefer the splice *target's* module inside a spliced body —
reintroducing the silent cross-pick class this slice exists to kill.

Use the **lexically-declaring module of the call term** instead: `span.module`
(`Span::module`, `src/ast.rs:14`). `span` is already bound at the top of `check_term`
(`let span = term.span;`, `src/check/terms.rs:122`) and already used for exactly this
"which module owns this source term" purpose at `src/check/terms.rs:157`
(`crate::resolve::mangle(name, span.module)`). It is in scope at the `terms.rs:956`
arm with no new plumbing. **Tier 1 keys on `span.module`, not `ctx.module()`.**

**Goldens must discriminate the policy, not merely satisfy it.** The `pb2` fixture
(both modules *declare* their own `Widget`) is satisfied by several possible fixes and
proves little about which policy shipped. The behavioral change and its discriminating
goldens all land in **Phase 2b** (Fix G): the tier-1 `pb2` golden, the corrected
tier-2 fixture (R3.5/Fix F), and the tier-3 selector unit test (R3.8). Phase 2a is a
pure, behavior-preserving refactor with no new goldens (R3.6.D, phase split below).

### R3.6 — the plumbing fan-out, budgeted honestly; extract a pure selector

Populating `Overload.module` is **not** "widen `Overload`, 11 sites". It is a
3-tuple→4-tuple widening of three helpers plus every consumer, ~two-and-a-half times
the earlier estimate, and is exactly why Phase 2 splits (R-phase). Concretely:

**(A) Widen the three `*_generated_sigs` helpers** from
`Vec<(String, String, Sig)>` to a 4-tuple carrying the module
(`(name, symbol, module, sig)` or a small named struct — implementer's call):
`struct_generated_sigs` (`src/check/declarations.rs:1815`), `enum_generated_sigs`
(`:1852`), `variant_generated_sigs` (`:1885`). Each already iterates `StructDecl` /
`EnumDecl` values that carry `.module`, so the module is in hand at the build point.

**(B) Update every tuple-consumer destructure** of those helpers (~25 sites across 6
files): `src/check.rs:586,589,592`; `src/check/terms.rs:1409,1410,1415,1416,1421,1422,1458,1464`;
`src/check/word_families.rs:2029,2051`; `src/check/poly.rs:798,799,808,809,818,819`;
plus the existing unit tests that destructure the triple at
`src/check/declarations.rs:4243,4255,4281,4282,4285,4286`. (Grep the three helper
names to confirm the live set before editing; line numbers drift.)

**(C) Widen `Overload`** (`src/check/builtins.rs:37`, fields `{sig, symbol}`) with
`module: u32`, and update **all ~13 construction sites** (grep `Overload {`):
`src/check.rs:587,590,593` (the three `*_generated_sigs` consumers — now supply the
new tuple field), `src/check.rs:612` (**extern registration** — `ExternDecl` carries
`module`, `src/ast.rs:2948`, supply it), `src/check.rs:695` (the per-word overload —
supply the word's module), `src/check/terms.rs:1412,1418,1424`,
`src/check/word_families.rs:2030`, `src/check/poly.rs:806,816,826`, and
`src/check/audits.rs:944` (an **in-`src` test harness** building `Overload`s — it will
fail to compile once the field is added; update it).

**(D) Extract a pure selector, then tier it.** `terms.rs:956` is an inline `match` arm
inside `check_term`, not an extractable function as written. This lands in two steps
across the phase split (Fix G):

- **Phase 2a (behavior-preserving):** extract a free function `select_overload` in
  `src/check/builtins.rs` (the natural lowest-common-ancestor: it already hosts
  `Overload`/`resolve_overload`) that reproduces today's `.find`-by-input semantics
  **exactly** — no tiering yet. **It takes only the two arguments it actually uses in
  this phase:**

  ```rust
  pub(super) fn select_overload(candidates: &[Overload], operands: &[Type]) -> OverloadPick
  ```

  It returns the input-matched pick or the no-match disposition, nothing more. **No
  `caller_module`/`caller_visible` parameters yet** — an unconsulted parameter is an
  unused-argument clippy warning, and CLAUDE.md's green bar requires
  `cargo clippy -- -D warnings` to pass at *every* phase exit, so 2a must not carry the
  wider signature dead. Full suite stays green; Phase 2a's one behavior-preserving unit
  test (below) is its unit artifact.
- **Phase 2b (behavioral):** **widen the signature to its final 4-argument form** and
  add the tier 1/2/3 policy (Fixes A–F) inside `select_overload`:

  ```rust
  pub(super) fn select_overload(
      candidates: &[Overload],
      operands: &[Type],
      caller_module: u32,
      caller_visible: impl Fn(u32) -> bool,
  ) -> OverloadPick
  ```

  so that (i) `terms.rs:956` and `poly.rs:3260` (if the R4 audit confirms it needs the
  same fix) can **share** it, and (ii) the mandated tier unit test can call it directly
  with hand-built inputs rather than only through `.sth` goldens. `caller_module` is fed
  `span.module` at the call site (R3.5); `caller_visible` closes over
  `name`/`m`/`span.module`. The two new arguments arrive in the same phase that first
  consults them, so there is no dead-parameter clippy window.

**`OverloadPick` (Fix A) — a concrete enum, homed in `src/check/builtins.rs`** next to
`Overload`/`resolve_overload`:

```rust
pub(super) enum OverloadPick<'a> {
    Pick(&'a Overload),   // the selected candidate; call site reads .sig / .symbol
    Ambiguous,            // no single winner; maps to the existing no-match path
}
```

`Pick` borrows a candidate out of the `candidates` slice (lifetime `'a` tied to that
slice) because the call site needs `&Overload` to read `.sig`/`.symbol` — an index into
`candidates` is an acceptable alternative if the implementer prefers, but the
borrow/lifetime shape must be stated either way. There is **no** separate `NoMatch`
variant: the empty-`matching` case (wrong argument types) and the ambiguous case both
map onto the **existing** `no_overload_matches_error` (`src/check.rs:1465`) at the call
site (see Fix A/C below), which is exactly the diagnostic `terms.rs:956` already raises
today via `hit.ok_or_else(|| no_overload_matches_error(...))`. No new diagnostic is
minted; the call site maps `OverloadPick::Ambiguous` onto that same `ok_or_else` path.

### R3.7 — the empty cases and the `Ctx::modules() == None` path

`Ctx::modules` is `Option<&[ModuleInfo]>` (`fn modules`, `src/check/engine.rs:1291`)
and is `None` on unit-test / retained-poly-word paths (`src/check/engine.rs:1701`'s
own comment: "only unit-test harnesses build a `Ctx` this way"). The mandated
table-driven unit test (R3.6.D) will hit this immediately, so rule it now, do not
leave it for mid-phase discovery.

**Two empty cases under Fix B's two-step structure (Fix C).**

- **`matching` is empty (zero of `candidates` match input).** This is today's
  existing "wrong argument types" behaviour, unchanged: `select_overload` returns
  `OverloadPick::Ambiguous`, which the call site maps onto the pre-existing
  `no_overload_matches_error` (`src/check.rs:1465`) via the same `ok_or_else` it uses
  today. No new error, no behaviour change for the always-existing wrong-types case.
- **`matching` has 2+, but zero are *visible* to the caller after the tier-2 filter
  (own-module misses, and none of the 2+ appear in the caller's direct
  imports/selectives).** This IS a new, accepted, scoped regression (Ruling A).
  `select_overload` returns `OverloadPick::Ambiguous`, mapped at the call site onto
  `no_overload_matches_error` (`src/check.rs:1465`) via the same `ok_or_else` path
  every other `Ambiguous` disposition uses — a real compile error, not a permissive
  fall-through. **State plainly: a program that resolves this shape today via
  unfiltered first-match will no longer compile after this fix ships.** That is
  intentional and scoped, not a bug. Why the error is *accepted* rather than avoided:
  `ModuleInfo.imports`/`.selective` are **direct-only** (no transitive/re-export
  closure), so a caller reaching a name only through a hub's re-export sees zero
  direct-visible candidates even though the program compiles today. S5 does not have
  the data to tell that caller's legitimate re-export path apart from a genuinely
  ambiguous one, so it fails closed on both rather than silently first-picking. **Scope
  fence, named:** S5 tightens dispatch for the cases it can prove (own-module and
  single-direct-import) and fails closed (a real error) on the transitive-import case
  it cannot yet prove.

  **Forward reference — the tracked follow-up is `P8.S5`.** This exact gap
  (`is_name_visible_to_module` is direct-only; a module reaching a name only through a
  hub's re-export sees zero visible candidates) is carved out as **P8.S5,
  "Transitive re-export visibility for overload disambiguation"**
  (`docs/roadmap/P8-packages-modules.md:240`). Once transitive re-export visibility
  exists, a caller whose sole route to the name is a legitimate re-export will resolve
  to exactly one visible candidate and this error case will resolve correctly. A reader
  hitting this error is hitting a **known, tracked limitation with a home**, not a
  permanent wall.

**The `modules() == None` path.**

- **Tier 1 still applies when `modules()` is `None`.** Own-module selection needs only
  `span.module` and each candidate's `Overload.module` — no visibility table. A
  candidate in `matching` on the caller's own module still wins.
- **Tier 2 runs unfiltered when `modules()` is `None`.** With no import-closure data,
  "exactly one visible candidate" is unknowable, and the unit-test harnesses that build
  a `Ctx` without modules must still resolve their single-candidate cases. So in that
  mode, after tier 1 fails to pick, tier 2 selects the single remaining candidate if
  there is exactly one **in `matching`** (visibility unfiltered). The `caller_visible`
  closure for the `None` path is simply `|_| true`, which makes "exactly one visible"
  degenerate to "exactly one in `matching`". **2+ in `matching` with no own-module pick
  returns `OverloadPick::Ambiguous` — the same real error as the `Some` path (Ruling A,
  R3.8), not a permissive first-pick.** Whole-program builds (`modules() == Some`)
  apply the real `caller_visible` filter. State this in the selector's doc comment.

### R3.8 — tier 3 (2+ visible candidates) is vacuous for the ctor shape; demote it

An implementability reviewer tried every surface route to construct "2+ same-shaped
ctor candidates visible **bare** to one caller" and all are blocked **before**
`check::check` runs (`src/driver.rs:888`):

- Two selective imports of the colliding name: rejected by `check_selective_imports`
  (`src/check/declarations.rs:819`, dispatched at `src/driver.rs:824`).
- Path-wildcard import (`import: "a.sth" a * ;`): does not parse.
- Two `self::` wildcard imports of the same name: rejected —
  "wildcard import of `Widget` collides with the wildcard import of `Widget`"
  (`declarations.rs:966`/`1000`).
- The only surviving 2-visible shape is a local declaration + one wildcard import.
  Both are genuinely visible to the predicate: the local decl via `defining == caller`,
  and the wildcard import via `.selective` (a wildcard desugars to a selective import
  of every exported name and populates `selective_map`, `driver.rs:595-601` — verified
  this pass; it is **not** invisible). So this shape really is 2-visible, but it never
  reaches the tier-3 branch: **tier 1** selects the own-module candidate first, so it is
  resolved, not left ambiguous.

**Ruling: tier 3 returns `OverloadPick::Ambiguous`, a real non-panicking variant — no
`debug_assert!`/`unreachable!` anywhere in `select_overload` (Fix A).** The two
mandated deliverables in the prior version were mutually exclusive (`unreachable!`
panics, so there is no returned variant to assert, and a value-assertion unit test
would have to become `#[should_panic]`; worse, an assert added in one phase would turn
an earlier phase's own green test red). Resolved as follows:

- **(a)** At the "2+ remaining after tiers 1–2" branch, `select_overload` returns
  `OverloadPick::Ambiguous` (a real value, defined in R3.6.D). The **doc comment** on
  `select_overload` records the vacuity evidence — the three blocked import routes
  above — as a **NOTE** explaining why `Ambiguous` is *believed* unreachable **for the
  ctor-collision shape specifically**, not as a runtime assertion. Nothing panics.
- **(b)** A unit test at the **selector-function level** (not an `.sth` golden)
  hand-builds the 2+-candidate state directly, bypassing the import gates the surface
  language cannot get past, and asserts
  `select_overload(...) == OverloadPick::Ambiguous` (a value assertion, not
  `#[should_panic]`). This test is green from the phase it lands in and stays green.
- **Call-site mapping.** `terms.rs:956` maps `OverloadPick::Ambiguous` onto the
  **existing** `no_overload_matches_error` (`src/check.rs:1465`) via the same
  `hit.ok_or_else(|| no_overload_matches_error(...))` path it uses today. No new
  located diagnostic is minted; if a future implementer decides this branch warrants
  its own small located error, that is a minimal add, not a fully-designed new
  diagnostic, and only then would it need R1-style pinning.

**Do not require a compiled `.sth` golden for tier 3.** The vacuity is proven, with
the three blocked routes as evidence. This mirrors the escape hatch the spec already
grants the legitimate-overloading guard (R4) — tier 3 does not get a harder bar than
that guard. Because tier 3 emits no new user-facing text (it reuses the existing
no-match message), there is nothing new to pin R1-style.

### R4 — per-site audit of the input-only candidate-selection sites (Phase 2b, first task)

Up to **six verdicts across five call sites** were found across two independent greps.
Phase 2b's first task produces a **per-site verdict** — reachable-for-this-shape
(route through `select_overload`), not-reachable (why), or doc-comment-only fix. Do
not narrow the enumeration.

1. **`src/check/terms.rs:956`** — the named primary fix site (R3). Route through
   `select_overload`.
2. **`src/check/poly.rs:3202`** — binds `single_candidate` (`Some([only])`), **not** a
   multi-candidate `find`; its own comment (~`:3196`) states an overloaded name "never
   matches `single_candidate` below and keeps the rejection". Confirm this stays a
   non-issue (an overloaded ctor here *rejects*, does not cross-pick); do **not**
   widen it unless the audit overturns that.
3. **`src/check/poly.rs:3260`** — a genuine input-only `candidates.iter().find(…)`
   (the `find` inside the `let chosen = env.get(name)...` statement that begins at
   `poly.rs:3258`), structurally identical to `terms.rs:956` but matching on
   `PolyType::Concrete` rather than `Type`. Audit whether the ctor-collision shape
   reaches it; if confirmed reachable, route it through the shared `select_overload`
   — and state the `PolyType::Concrete` → `Type` conversion invariant (non-concrete
   means no match) if the shared selector is used here.
4. **`src/check/builtins.rs:46` `resolve_overload`** — same exact-input-match shape.
   Its doc comment (`builtins.rs:42-45`) explicitly claims the invariant "two
   candidates registered under one name in scope never share input types", which
   generic ctors now **violate**. Live caller `src/check/operators.rs:196`. Audit
   whether ctors reach this path; if not, still **correct the now-false doc comment**
   (P2 priority, not skip).
5. **`src/check/poly.rs:6557 resolve_poly_overload`** and
   **`src/check/poly.rs:6771 resolve_combinator_overload`** — two separate functions,
   two separate verdicts, same "first declared wins" shape. Lower priority (ctors
   likely do not route through these); each carries its own explicit verdict.

**Scope of the tier policy (Fix D).** The tiers apply to whatever is in the
`candidates` slice at `terms.rs:956` — but that slice has **three** provenances, not
one, and the earlier "whole-program `env` only" fence was false at this call site. Read
directly from `terms.rs:886-928`, `candidates` is bound from (i) `scoped_ops`
(operator-scoped overloads built at `word_families.rs:2029-2051`) when `Some`, else
(ii) `env.get(name)` — the whole-program env `check.rs` builds via the
`struct_generated_sigs`/`enum_generated_sigs`/`variant_generated_sigs` path
(`src/check.rs:587,590,593`), else (iii) `mint_fallback_candidates`
(`terms.rs:1406-1428`) on an env miss — check-time monomorph mints. `select_overload`
receives whichever of the three fed `candidates`, so the tiered policy applies
**uniformly to `candidates` regardless of provenance** at this one site. Resolve the
three cases:

- **Env path (ii)** — the fenced-in, reliable path. `pb2`'s bug and fix both live here,
  and `Overload.module` is reliable: `StructDecl.module` is set per-monomorph by
  `instantiate_struct` under a real per-module dedup key (`(idx, module, args, lens)`,
  `src/ast.rs:1334`). Tiers 1–3 apply as specified.
- **`scoped_ops` path (i)** — operator names only (never a bare ctor name like
  `Widget`), so the ctor-collision shape does not arise here; tier 1 ("prefer own
  module") is still safe to apply and cannot make an operator resolution worse.
- **`mint_fallback` path (iii)** — this **is** a genuine source of 2+ colliding
  same-input candidates: the existing test
  `mint_fallback_candidates_at_a_colliding_name_returns_both` (`terms.rs:2527`) proves
  two module-blind variant ctors named `Same` both mint at `[i64]`. Its own doc comment
  (`terms.rs:1401-1405`) requires it "must not invent a stricter rule than a present
  `env` entry would have had" — applying the **same** tiers the env path now gets
  *satisfies* that contract (parity, not a stricter rule), so tiers do not violate it in
  principle. **But `Overload.module` reliability on this path is not verified:** these
  mints come from the live `generics_cell`, and `poly_construction_fallback`
  (`poly.rs:5642`) can mint under `ctx.module()` rather than the type's declaring
  module. Tier 1 (own-module) is safe regardless. **Ruling for tiers 2/3 on
  fallback-sourced candidates: a probe is Phase 2b's first task** — establish whether a
  mint_fallback collision's `.module` is the reliable declaring module or the
  `ctx.module()` fallback. If reliable, apply tiers 2/3 uniformly. If not, **exclude
  fallback-sourced candidates from tiers 2/3** (tier 1 still applies) and document why.
  Do not assume the fence holds.

**Separate call sites are still out of scope.** `poly.rs`'s **own local envs**
(`poly.rs:806/816/826`, distinct from `check.rs`'s whole-program env) and the
`poly.rs:3258-3260` `find` (which matches on `PolyType::Concrete`, not `Type`) are
different selection sites, not the `terms.rs:956` `candidates` slice. Their
`Overload.module` provenance was not verified reliable, so they are **OUT of Phase
2a/2b's tier-application scope by assumption** and remain part of the R4 per-site
audit, where each gets an **independent** verdict rather than inheriting Phase 2b's fix
uniformly. An implementer must not assume `Overload.module` means the same thing at
every site it is constructed.

**Legitimate-overloading guard.** The widening changes *general* overload resolution.
Phase 2b must test that a legitimate same-input / different-output overload set (both
in scope, resolved from one module) still resolves as before — the fix must break the
cross-module ctor cross-pick **without** regressing intra-module overloading. If no
such construct is expressible today, record that the risk is vacuous (with the probe
that establishes it) rather than leaving it untested.

### R5 — (a): per-call-site reachability verdict; no speculative routing fix. (Phase 3)

`mono_member_unroutable_error` has **two** call sites, not one:

- **`src/check/poly.rs:2390`** (mono generic-target branch of
  `resolve_mono_member_call`, non-inline path). **Dead-guard reading reconfirmed**
  (F4/F5/F10): every cross-module attempt is intercepted upstream by
  `mono_member_no_dispatch_error` (`find_bound_impl` never matches the cross-module
  operand, blocked by `project_generic_instantiation_cannot_cross_modules`), and
  `poly_env` is built once, whole-program (`src/check.rs:684-698`,
  `src/driver.rs:512`), so the guard's premise of a per-module `poly_env` does not
  exist. No test exercises it.
- **`src/check/poly.rs:1915`** (inline re-entry path). **Inconclusive** (F10). Direct
  self-recursion in an inline generic-target member is intercepted by a *different,
  dedicated* guard ("an always-spliced word cannot be recursive") *upstream* of
  `:1915`. The next candidate shape — mutual recursion between two inline
  generic-target members — was not completed in recon (trait-member declaration
  syntax friction, not a design question).

**Ruling (a THREE-branch verdict, one per call site; do not collapse to one).**

- **`:2390`** — favoured outcome is dead-guard: replace with a documented
  `unreachable!`/`debug_assert` (comment: whole-program-flattened `poly_env` means a
  found impl's member word is always present) **and** add a regression test asserting
  the reachable cross-module colliding mono call fails with
  `mono_member_no_dispatch_error`, not this guard. If Phase 3 instead finds a live
  trigger, route mono member lookup through the whole-program registry and pin the
  triggering fixture as the golden.
- **`:1915`** — see the static route and ceiling below. If a genuine attempt triggers
  the guard, that is the live-trigger outcome with a golden. **If `:1915` remains
  inconclusive after a genuine attempt, the phase's exit criterion must say so
  explicitly** — one site settled, one inconclusive-pending-X. An inconclusive
  `:1915` does not block the slice.

**Cheaper static route, try this FIRST — but its logic is the opposite of the prior
spec's.** `check_combinator_cycles` (`src/check/combinators.rs:173`) is a **rejection**
pass: it stops a program whose combinator call graph has a cycle. Its `idx`/`adj`
build keys on bare `c.word.name` (`:186-189`), while a trait-member combinator is
registered under a synthesized `member;Trait;Type` spelling — so a bare dispatched
member call may produce **no edge** in that graph. The prior spec concluded "no edge ⇒
`:1915` dead". **That is backwards.** The file's own comment (`:175-184`) is explicit:
a missed edge here "is not a missed diagnostic, it is the inliner splicing a real
cycle forever", which is why the pass over-approximates (ambiguous name ⇒ edge to
*every* candidate, never none). So a **missing** edge means the mutual-recursion cycle
is **not rejected** upstream, the program proceeds further, and `:1915` becomes
**more** likely to be reached, not less.

Corrected task for Phase 3: the keying check is still worth doing first (cheap,
static). But read its result correctly — if it shows the dispatched member call
produces no edge, that means the fixture-based mutual-recursion attempt (the three
known-failing shapes plus at most one novel) is **more** likely to reach `:1915`, so
you must then *try the fixtures*. It does **not** let you declare `:1915` dead by that
argument. Remove any "settle `:1915` dead by no-edge" framing.

**Prior art for the fixture route (do not re-derive).** Trait member syntax is
`: name inline ( … ) ;` per member in a loop (`src/parser.rs:3792-3866`). Three
distinct upstream guards intercept before `:1915` fires: (1) direct cross-member call
— members aren't visible to each other in an impl body (`unknown word`); (2) via a
bounded helper — the generic receiver can't satisfy the helper's bound; (3)
forwarding the receiver bare — a poly call site may pass a type variable bare, not
partially applied. (A concrete-target variant built clean, but concrete targets are
what `:1915`'s `!target.is_concrete()` excludes.)

**Ceiling.** Attempt the three known-failing shapes plus at most **one** genuinely
novel shape. If none succeed, declare `:1915` inconclusive with every attempt and its
exact error text recorded, and move on.

Either way, the roadmap's exit clause "a mono caller … dispatches a colliding member
word per its operand's constructor" is **not** achievable in S5: it is blocked behind
`project_generic_instantiation_cannot_cross_modules` (F4). S5 delivers the *ruling* on
the guards, not cross-module member routing.

### R6 — golden #10 retained; roadmap deviation edit assigned. (Phase 3)

**Golden #10.** With the dispatch fix, same-payload same-named cross-module ctors now
resolve to the *correct* impl per calling module. Golden #10
(`same_named_ctors_in_two_modules_dispatch_distinct_impls`,
`tests/phase7b_slice2.rs:644`) **keeps its `i64`/`str` payload split unchanged as a
regression witness** — its inputs never collide, so it never exercises the
`terms.rs:956` site, and its payload-disambiguation split is *retained, not removed*.
The **new** headline golden is R3.5's tier-1 `pb2` same-payload (`i64`/`i64`) case,
pinned to each module's `run` printing its own impl's result (`1` then `2`).
Deviation from the roadmap's original framing, on the record per
[[workflow_slices_break_design_invariants_silently]].

**Roadmap edit.** No phase currently edits
[`docs/roadmap/P7b-higher-kinded-types.md`](../P7b-higher-kinded-types.md), whose S5
exit-clause text after this spec ships holds a now-false claim (the cross-module
member-dispatch text). The "#10's workaround becomes unnecessary" half is now actually
**false** in the sense the edit must state precisely: same-payload cross-module ctors
now dispatch correctly per the caller's module/visibility (the 3-tier policy), but
golden #10 **keeps** its `i64`/`str` split as a regression witness — the workaround is
not removed. Phase 3 (last) rewrites the S5 exit-clause text to current-state-only per
[[feedback_roadmap_design_no_history]]: describe what holds now, no "used to say X"
narrative, and do not claim the #10 workaround was deleted.

## Phased delivery

Each phase is green (`cargo fmt --check && cargo clippy -- -D warnings && cargo test`)
at its exit, goldens pinned as diagnostic-text / stdout assertions per CLAUDE.md
(`thing_condition_expected` naming, happy path + edge case). Per CLAUDE.md, **every
phase adds unit tests beside the mechanism it edits**, not only end-to-end `.sth`
goldens.

### Phase 1 — (c) diagnostic-text fix (R1, R2)

- Edit `nested_receiver_member_error` (`src/check/declarations.rs:444`) to read the
  head var off `member.sig.ty_var_names.first()` (invariant comment citing
  `src/parser.rs:3931`) and apply the `'T`/`'U` placeholder rule.
- Unit test beside it: `nested_receiver_error_uses_header_var_G_expected` — header var
  `'G` produces `` `'G` bare or heading an application like `'G['T]` ``;
  `nested_receiver_error_header_var_T_uses_U_placeholder` — header var `'T` produces
  `'T['U]`. Happy path + placeholder-collision edge.
- Repo-wide grep `grep -rn "heading an application like" src/ tests/`; reconcile the
  **four** hits (R2). Re-pin the **two** `'T`-headed sites to `'T['U]`:
  `check_trait_decls_rejects_a_receiver_nested_in_an_array_input`
  (`src/check/declarations.rs:3875`, inline `src/` test) **and**
  `a_nested_receiver_member_is_still_rejected` (`tests/phase7_slice3t.rs:505`).
  Confirm the two `'F`-headed sites unchanged (the format string default;
  `hkt_member_without_dispatchable_input_is_located_error`,
  `tests/phase7b_slice2.rs:158`).

Exit: the diagnostic names the trait header's real variable; all four asserting sites
are reconciled (both `'T`-headed sites re-pinned); the `'F` common case unchanged.

### Phase 2a — mechanical widening + behavior-preserving selector extraction (R3.6; Fix G)

**Pure refactor. No behavioral change and no new `.sth` goldens; behaviour at
`terms.rs:956` stays byte-identical.** Per the phased-delivery preamble (every phase
adds a unit test beside the mechanism it edits), 2a still lands **one** unit test — one
that is genuinely behavior-preserving to write.

- Widen the three `*_generated_sigs` helpers to carry module (R3.6.A) and update all
  ~25 tuple-consumer destructures (R3.6.B).
- Widen `Overload` with `module: u32` and update all ~13 construction sites, incl.
  extern (`check.rs:612`), per-word (`check.rs:695`), and the in-`src` test harness
  (`audits.rs:944`) (R3.6.C).
- Extract `select_overload` into `src/check/builtins.rs` (R3.6.D) reproducing today's
  `.find`-by-input semantics **exactly** — the two-step split's Step 1 only, **no
  tiering**. **Its signature is the 2-argument form `select_overload(candidates,
  operands) -> OverloadPick`** (no `caller_module`/`caller_visible` yet — an unused
  parameter would fail `clippy -D warnings`; the 4-arg widening lands in 2b when the
  tiers first consult it). Wire `terms.rs:956` to call it. Behaviour is byte-identical
  to today.
- **Unit test beside `select_overload`** (behavior-preserving, so it costs nothing
  behaviorally): `select_overload_single_input_match_returns_pick` — a `matching` set
  reduced to one input-matching candidate returns `OverloadPick::Pick` on it; and
  `select_overload_no_input_match_returns_ambiguous` — an empty `matching` set (no
  candidate's inputs match the operands) returns `OverloadPick::Ambiguous` (which the
  call site maps onto today's `no_overload_matches_error`). Both assert only Step-1
  semantics that already hold today.

Exit: the plumbing fan-out is complete and compiles; `select_overload` exists as a
pure, behavior-preserving 2-argument function with a unit test beside it; `clippy -D
warnings` passes (no dead parameter); **full suite green with no behaviour change** and
no new `.sth` goldens.

### Phase 2b — the tier policy + goldens + audit + guard + tier-3 (R3.5–R3.8, R4, R6; Fix G)

The risky behavioral change and its regression guard land together here.

- **Widen `select_overload` to its 4-argument final form** (`candidates`, `operands`,
  `caller_module`, `caller_visible`) and add the tier 1/2/3 policy (Fixes A–F) inside
  it: Step 2's tiers over `matching`, `OverloadPick` (R3.6.D / Fix A), the
  `caller_visible` predicate (`is_name_visible_to_module`, Fix E), the
  `modules() == None` path and both empty cases (R3.7 / Fix C). Wire `terms.rs:956` to
  feed `span.module` as `caller_module` and a `caller_visible` closure over
  `name`/`m`/`span.module`. **Tier 1 keys on `span.module`, not `ctx.module()`** (R3.5
  splice ruling). The two new parameters arrive in the same phase that consults them,
  so `clippy -D warnings` stays green (no dead parameter).
- **Table-driven unit test** of `select_overload` (pure, hand-built inputs): tier 1
  (own-module among 2 in `matching`), tier 2 (single visible candidate), tier 2 under
  `modules()` None (single remaining in `matching`), the zero-visible accepted error
  (2+ in `matching`, none visible → `Ambiguous`, Ruling A / Fix C), and the tier-3 2+-visible
  case asserting `select_overload(...) == OverloadPick::Ambiguous` (R3.8.b / Fix A — a
  value assertion, not `#[should_panic]`).
- Goldens (discriminating — R3.5):
  - `cross_module_same_shaped_ctor_dispatches_callers_own_impl` — tier-1 `pb2`:
    `a::run` prints `1`, `b::run` prints `2`. Headline soundness golden. (Verified
    constructible; `pb2` reproduces `2 2` today, 2/2 deterministically.)
  - `imported_generic_ctor_resolves_single_visible_candidate` — **tier-2, and it must
    actually reach `select_overload` with 2 candidates in `matching`** (Fix F). The
    old adaptation of `selectively_imported_generic_name_applies_bare`
    (`src/driver.rs:1433`) is **rejected**: that fixture has only ONE module declaring
    the ctor, so `env[name]` holds one candidate, the call takes the `[only]` fast
    path, and never reaches `select_overload` — it proves nothing about tier 2.
    Replacement fixture: **TWO** modules each declaring a same-shaped ctor (as in
    `pb2`/tier-1), plus a **THIRD** module that imports **only ONE** of the two and
    calls the ctor bare. **The import must be a form that populates `.selective`, which
    is what `is_name_visible_to_module` reads** (`modules[caller].selective.get(name)`,
    `word_families.rs:1161`). Verified against `assemble_module` (`driver.rs`): a
    **selective import** (`import: b | Widget | ;`) inserts `name -> target` into
    `selective_map`, and a **wildcard import** (`import: b * ;`) desugars to a selective
    import of every exported name and populates `selective_map` identically
    (`driver.rs:595-601`). A **qualified-only import** (`import: "b.sth" b ;`) does
    **not** — `is_name_visible_to_module`'s own doc comment (`word_families.rs:1149`)
    states a qualified-only import "makes nothing visible by bare name", so it is not a
    valid spelling here and is dropped from the options. **Mandate the selective form
    `import: b | Widget | ;`** in the fixture (unambiguous, no dependence on wildcard
    desugaring). Assert that third module's own imported impl runs, not the other. This
    reaches `select_overload` with 2 candidates in `matching` and exercises the
    `caller_visible` filter for real.
- **The six-verdict / five-site audit (R4)** and the false-doc-comment fix at
  `builtins.rs:42-45` (both moved here per Fix G): per-site verdict for `terms.rs:956`,
  `poly.rs:3202`, `poly.rs:3260`, `builtins.rs:46`, `poly.rs:6557`, `poly.rs:6771`.
  If `poly.rs:3260` is confirmed reachable, route it through the shared selector. Per
  Fix D, `poly.rs`'s local envs (806/816/826) and the `poly.rs:3258-3260` `find` are
  out of Phase 2a's tier scope by assumption and get their own audit verdict here.
- **`mint_fallback` module-provenance probe (R4/Fix D, Phase 2b's first task):**
  establish whether a `mint_fallback_candidates` collision's `Overload.module` is the
  reliable declaring module or the `ctx.module()` fallback. Verdict decides whether
  tiers 2/3 apply to fallback-sourced candidates or those are excluded (tier 1 applies
  regardless). Record the verdict.
- **Legitimate-overloading guard test** (R4): a same-input/different-output overload
  set resolved intra-module still resolves as before, or the recorded-vacuous note if
  inexpressible.
- **Tier 3 disposition (R3.8 / Fix A):** `select_overload` returns
  `OverloadPick::Ambiguous` at the 2+-remaining branch (no `debug_assert!`/`unreachable!`),
  the doc-comment NOTE cites the three blocked routes, and the selector-level unit test
  above asserts the variant. No `.sth` golden; no new user-facing text.
- Golden #10 unchanged (R6).

Exit: `pb2`'s cross-pick is gone (each module runs its own impl); the selector unit
test covers tiers 1/2, the `None` path, and both empty cases; the tier-2 golden
actually reaches `select_overload` with 2 `matching` candidates; all six audit
verdicts recorded; the false doc comment corrected; legitimate intra-module
overloading unregressed; tier 3's vacuity documented and unit-witnessed; full suite
green.

### Phase 3 — (a) guard reachability ruling + roadmap edit (R5, R6)

- **`:1915` first, statically — but read the result per R5's corrected logic:** check
  the `check_combinator_cycles` (`combinators.rs:173`) keying question
  (`member;Trait;Type` vs bare `c.word.name`). A no-edge result means `:1915` is
  **more** likely reachable, so it does **not** settle the site dead — proceed to the
  fixtures.
- Attempt the three known-failing shapes (R5 prior art) + at most one novel shape.
  Live trigger ⇒ golden. Otherwise record `:1915` inconclusive with every attempt's
  exact error text.
- **`:2390`:** settle dead-guard vs live-trigger. Dead-guard route → documented
  `unreachable!`/`debug_assert` + regression test
  `cross_module_colliding_mono_call_is_no_dispatch_error` pinning the reachable failure
  as `mono_member_no_dispatch_error`. Live-trigger route → whole-program registry
  routing + triggering golden.
- Update both guards' doc comments to the whole-program `poly_env` reality.
- Rewrite the roadmap's S5 exit-clause text (`docs/roadmap/P7b-higher-kinded-types.md`)
  to current-state-only (R6): cross-module member-dispatch claim is false; golden #10
  keeps its `i64`/`str` split as a regression witness (the workaround is *not*
  removed). Describe what holds now, no history.

Exit: `:2390` is provably dead (regression test) or provably live (golden); `:1915`
has a live-trigger golden or an explicitly-recorded inconclusive verdict; the roadmap
reflects current state; no speculative routing code ships without a witness.

## Signals re-check at phase exit (CLAUDE.md growth structure)

`src/check/poly.rs` is already large; per [[project_poly_rs_split_deferred]] a split is
deferred (3/5 signals, no clean cut). Phase 2a threads a field through the `Overload`
fan-out and adds `select_overload` to `builtins.rs` (the LCA of `terms.rs:956` and
`poly.rs:3260` — an elevate-to-lowest-common-ancestor move, not a new import
divergence); Phase 2b touches the audit sites; Phase 3 touches the mono-member arm.
Re-run the split signals at each phase exit against the files as they then stand; do
not preemptively split.

## Open questions (to be closed by the phase that resolves them)

- **Six-verdict audit outcome (R4)** — Phase 2b's first task; not pre-answered.
  `terms.rs:956` is the confirmed fix site; `poly.rs:3260` is the same
  find-by-input shape and presumed reachable; `poly.rs:3202` presumed
  rejects-not-cross-picks; `builtins.rs:46`, `poly.rs:6557`, `poly.rs:6771` presumed
  ctor-unreachable but each carries an explicit verdict (and `builtins.rs`' doc
  comment is fixed either way).
- **Legitimate same-input/different-output overload risk (R4)** — resolved by Phase
  2b's guard test.
- **`mint_fallback` module-provenance (R4/Fix D)** — Phase 2b's first task; decides
  whether tiers 2/3 reach fallback-sourced candidates. Tier 1 applies either way.
- **(a) `:2390`** — dead vs live, recorded once Phase 3's probe lands.
- **(a) `:1915`** — live-trigger vs explicitly-inconclusive (the static no-edge read
  does **not** settle it dead — R5), recorded once Phase 3 lands.
</content>

</invoke>
