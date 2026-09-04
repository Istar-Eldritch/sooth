# P7b.S5 brief — member-word routing and env-dispatch residuals (recon round)

Scope input for the S5 spec. Produced by a recon round against the clean tree
(worktree `p7b-s5`, HEAD `4cfb887`): a read-only machinery map plus five live probes
under `/tmp/p7bs5-probes/` (verbatim log: [slice5-probes.md](./slice5-probes.md)).
Repo untouched throughout (`git status --porcelain` empty at finish). Baseline
`cargo test --no-fail-fast` at HEAD is green, so every rejection below is a
pre-existing convention, not baseline breakage.

S5's scope (from the [phase doc](../P7b-higher-kinded-types.md), line 141) is three
items carved out of S2's golden #10: (a) mono member routing should share the poly
path's registry; (b) same-named ctor env dispatch is module-blind; (c)
`nested_receiver_member_error` hardcodes `'F`.

## What the round established

**(c) is exactly as described, and is a pure text fix.** Probe `pc` (trait header
`'G`, no application) gets the literal `'F` in the diagnostic regardless. Anchor:
`nested_receiver_member_error` (`src/check/declarations.rs:444`), which formats a
constant string rather than reading `decl.members[0].sig.ty_var_names[0]` (or
equivalent) off the trait declaration it is already holding. This is the
lowest-risk, best-isolated item in the slice — no design decision, just interpolate
the real variable name and re-pin the two goldens the task brief names.

**(b) is live, but the failure mode is not what the task brief's framing assumes.**
Two identically-shaped, same-named ctors across modules do not silently cross-pick
(probe `pb`): the checker's own `Type` equality is `StructId`-keyed underneath, and
it **rejects** the mismatched call with a type-mismatch error. What breaks is
*legibility*, not soundness: the error prints `Widget[i64]` on both sides of "leaves
X where the declaration requires Y" because rendering goes through
`generic_surface_name` and `Type::name()`, both of which strip exactly the
information (the per-module mangle tag) that would let the message distinguish the
two `Widget[i64]`s. Golden #10 works around this by choosing non-colliding payload
shapes (`i64` vs `str`) rather than by any pinned disambiguation rule — there isn't
one to invoke yet.

**(a) could not be witnessed as a live routing bug within this round.** Every
attempt to construct "a mono caller in a module that can see both impls dispatches
a colliding member word per its operand's constructor" (the task brief's own target
shape for a) ran into the pre-existing cross-module generic-instantiation gap
first: `find_bound_impl` (whole-program, no module filter) never got as far as
matching the impl target against the operand, because the operand's `Box[i64]` and
the impl's `Box['T]` target don't resolve to the same registry identity across a
module boundary (probe `pa2`; `export:` cannot even spell the workaround, probe
`pa4`). That is `mono_member_no_dispatch_error`, not `mono_member_unroutable_error`
— a different, earlier gate. No test in the tree exercises
`mono_member_unroutable_error` end-to-end (only a comment references it), and given
`poly_env` is built once, whole-program, off the fully flattened `assemble_module`
output (`src/driver.rs:512`, `src/check.rs:684-698`), the error's own doc comment
("not visible from this module") describes a per-module split in `poly_env` that
does not exist in the current architecture. This is either dead code guarding
against a scenario the current pipeline cannot produce, or the round simply failed
to find the right shape to trigger it — the spec needs to settle which before
budgeting work on "share the poly path's registry."

| # | Finding | Probes |
| --- | --- | --- |
| F1 | **(c) confirmed, isolated, no design question.** `nested_receiver_member_error` hardcodes `'F` regardless of the trait header's real variable spelling. Fix: interpolate; re-pin two goldens. | pc |
| F2 | **(b) does not silently cross-pick.** Same-named, same-shaped ctors across modules are rejected by a genuine `StructId`-level type mismatch. Not a soundness hazard. | pb |
| F3 | **(b)'s real defect is a diagnostic legibility gap, not a dispatch gap.** The mismatch message renders both sides through `generic_surface_name`/`Type::name()`, which strip the per-module mangle tag (`src/resolve.rs:799`) that would disambiguate them, so the message prints `Widget[i64]` vs `Widget[i64]` with no way to tell which is which. | pb |
| F4 | **(a) is not independently reachable from the module-instantiation gap.** Every cross-module construction hits `mono_member_no_dispatch_error` (a whole-program `find_bound_impl` match failure rooted in `project_generic_instantiation_cannot_cross_modules`) before it can reach the `mono_member_unroutable_error` guard three branches later. | pa2, pa4 |
| F5 | **`mono_member_unroutable_error` has no live trigger found in this round, and no test exercises it.** Its doc comment's premise (a per-module `poly_env`) conflicts with the whole-program `poly_env` the current checker actually builds. | pa2, pa3 (control) |

## Machinery map (verified anchors, this tree, HEAD `4cfb887`)

- **(c) site.** `nested_receiver_member_error` (`src/check/declarations.rs:444-455`),
  called from `check_trait_decls` (`src/check/declarations.rs:360`). The trait
  header's own variable spelling is available on `decl` at that call site — the
  function signature already takes `decl: &TraitDecl`, so no plumbing is needed,
  only reading the right field instead of the literal.
- **Mono member dispatch entry.** `resolve_mono_member_call`
  (`src/check/poly.rs:2146`): candidate collection over the whole-program trait
  registry (`:2196-2210`, no module filter), viability via `find_bound_impl`
  (`:2210-2232`), then a concrete-target branch (`:2233-2382`, span-keyed
  `builtin_overloads` write) or a generic-target branch (`:2383-2398`, the
  `mono_member_unroutable_error` guard at `:2389-2393`).
- **`find_bound_impl`** (`src/check/poly.rs:8110-8188`): whole-program `tr.impls`
  scan, `match_impl_target` against the operand type, `select_most_specific` on
  ties. No module-visibility filter exists here — visibility is not the blocker;
  registry-identity match is.
- **The ctor env-dispatch key.** `struct_generated_sigs`
  (`src/check/declarations.rs:1815-1839`) and its enum/variant twins register the
  env key as `generic_surface_name(&decl.name)` (`src/ast.rs:848-853`, splits on
  the first `[`), while the lowering **symbol** stays `decl.name` verbatim (already
  module-mangled by `resolve::mangle`, `src/resolve.rs:36-38, 799`). So the env key
  a source term actually calls by is deliberately bare; the symbol is what stays
  module-distinct.
- **The rendering collapse.** `Type::name()` (module-blind per the standing notes
  at `src/ast.rs:766`, `src/ast.rs:5229`, `src/driver.rs:1171`) is what
  `type mismatch` (and every other diagnostic naming a `Type::Struct`) renders
  through — this is the site F3 points at, not the ctor lookup itself.
- **The `poly_env` construction.** `src/check.rs:684-698`: one `poly_env: PolyEnv`
  built once over `module.words` (the whole `assemble_module`-flattened program,
  `src/driver.rs:512`) with no module partition. This is what F5 rests on:
  `mono_member_unroutable_error`'s premise of a module-local `poly_env` a symbol
  could be absent from does not match this construction.
- **Golden #10 (S2-12).** `same_named_ctors_in_two_modules_dispatch_distinct_impls`,
  `tests/phase7b_slice2.rs:664-698`. The doc comment at `:633-663` is the task
  brief's source text; its workaround (b's `Widget` payload is `str`, not `i64`) is
  exactly what F2/F3 explain the necessity of.

## Open questions and scope recommendation

**Q1 (the core (a) question) — is `mono_member_unroutable_error` reachable at all,
and does S5 owe it anything?** This round could not construct a case where
`find_bound_impl` succeeds (`viable.len() == 1`) on a cross-module operand *and*
the resulting `word_sym` is absent from the whole-program `poly_env`. Two readings
are both consistent with what was found:

- **Dead-guard reading:** the guard was written for the S2-era architecture (a
    per-module env) and is now unreachable in the whole-program-flattened one; S5
    should either delete it (with a comment explaining why it can't fire) or add a
    regression test proving it, one way or the other, before any "share the
    registry" work is speced.
- **Blocked-behind-a-different-gap reading:** the guard is reachable, but only
    once the cross-module generic-instantiation gap (F4,
    `project_generic_instantiation_cannot_cross_modules`) is fixed first — in which
    case S5 either depends on that fix landing, or needs a same-module fixture
    shape this round didn't find (e.g. a concrete, non-generic impl target reached
    through a combinator re-entry path, `src/check/poly.rs:1900-1922`, which this
    round did not probe).

  Recommend the spec open by settling which reading holds — a targeted probe at the
  `:1900-1922` re-entry path, or a direct unit test constructing `PolyCtx` by hand
  with a deliberately partial `poly.env` — before committing to "route mono member
  lookup through the poly registry" as the fix shape.

**Q2 (the core (b) question) — module scope or qualified spelling?** The task
brief poses this as the design decision; here is what each costs mechanically,
given F3's finding that the *lookup* already routes correctly and the *identity*
is already `StructId`-correct:

- **Module-scope option:** change nothing about dispatch; only make the
    diagnostic renderer module-aware. Since the underlying `StructId`s already
    differ (`src/resolve.rs:799`'s per-module mangle already makes them distinct
    registry entries), the fix is confined to the *rendering* path — teach the
    type-mismatch message (and anywhere else that renders a `Type::Struct`/`Enum`
    for a user, per the other module-blind-render notes) to disambiguate two
    same-surface-name structs by appending something identifying (module path, or
    the registry id `type_arg_key` already uses for type *arguments* at
    `src/ast.rs:775-780` — the same disambiguation rule could extend to the base
    name). This is a diagnostics-only change; no dispatch, no env-key, no
    resolve.rs change.
- **Qualified-spelling option:** let a user *write* `a::Widget` vs `b::Widget`
    to force a specific module's ctor even when both are in scope unqualified.
    This needs `generic_surface_name`'s env key to admit a qualifier (mirroring
    `resolve_mono_member_call`'s existing `name.split_once("::")` handling,
    `src/check/poly.rs:2179-2192`) for the *ctor* call path, which currently has no
    such split at all (`struct_generated_sigs` never sees a qualifier). This is a
    parser/checker surface change, not just a diagnostic one, and only helps a
    user who already knows they have a collision — it does nothing for the
    unqualified call in golden #10's actual shape (both modules' `run` calls their
    own private `mk`, never a cross-module-qualified ctor).

  Given F2 (the collision is already fails-closed, not silently wrong), the
  module-scope/diagnostic-only option looks like the smaller, load-bearing fix; the
  qualified-spelling option solves a problem (a user *deliberately* wanting a
  specific same-named type from two visible modules in one unqualified call site)
  that golden #10 does not actually exhibit. Recommend the spec ask explicitly
  whether S5 needs to solve *that* problem, or only needs the render fix — this
  round found no fixture where the current fails-closed behavior produces a wrong
  answer, only a confusing one.

**Q3 — scope fence: does (b)'s render fix reach every `Type::name()` call site, or
just the type-mismatch message?** The module-blind rendering notes catalogued
above (`ast.rs:766`, `ast.rs:5229`, `driver.rs:1171`) suggest this is a
crate-wide convention, not a single format string. A minimal S5 fix likely touches
only the messages golden #10's shape can actually trigger (ctor-generated type
mismatches); widening it to every diagnostic is a separate, larger sweep the spec
should explicitly scope in or out.

**Q4 — (c)'s two goldens.** The task brief names "the two pinned goldens that
currently expect the literal `'F`" as needing updates once (c) is fixed; this round
did not enumerate them by name (out of scope for a read-only recon pass that
touches no `tests/`), but a `grep -rn "expected the trait's variable" tests/` at
spec time will find them directly — flagging so the spec's phase-1 slice includes
that grep as a checklist item, not a re-discovery.

## Round 2 (2026-09-03) — correcting the (b) fails-closed claim

A spec review reproduced a silent cross-pick the Round 1 `pb` fixture did not
exhibit. This section **supersedes F2, F3, and Q2/Q3's premises**; F2/F3/F4/F5 and
Q1–Q4 above are left unedited on the record per
[[workflow_review_preexisting_claim_check_the_parent_commit]]-style correction
convention — do not trust them for (b) without reading this section. Verbatim
fixtures, commands and revert confirmations: Round 2 section of
[slice5-probes.md](./slice5-probes.md).

### What changed since Round 1

Round 1's `pb` fixture had **both** modules' ctor consumer (`mk`) leave the
instantiation type unspelled, inferred only from the ctor's own `( i64 --
Widget[i64] )` declaration. The reviewer's fixture instead has module `a`'s
consumer never spell the full instantiation (`Widget sized`, a poly-bounded call)
while module `b`'s does (`Widget[i64]` as a declared parameter type). That
difference determines which of two different code paths handles the collision —
one path is fails-closed (rejects), the other silently mis-answers.

### Corrected findings table

| # | Finding | Status | Probes |
| --- | --- | --- | --- |
| F2 | (b) does not silently cross-pick. | **Superseded, false as a general claim.** True only for the specific shape Round 1 tested (both consumers unspelled); false when one consumer's parameter type spells the instantiation. | pb2 (Round 2) |
| F3 | (b)'s real defect is a diagnostic legibility gap, not a dispatch gap. | **Superseded, incomplete.** The legibility gap is real (see F3-corrected below) but is not the only defect — a genuine dispatch bug coexists with it, in a different call shape. | pb2, pb3, pb4 (Round 2) |
| F3-corrected | The env key `struct_generated_sigs` registers a generic ctor under (`generic_surface_name(decl.name)`) is not merely under-disambiguated at *render* time — the *overload-selection* match at `check/terms.rs:955` picks among same-name candidates by **input signature only** (`sig.outputs` and the caller's module never enter the comparison), so two same-shaped ctors across modules are genuinely ambiguous at the call site, not just at the printer. Which candidate wins is first-in-`Vec` order (module-assembly order), independent of the calling module. | New | pb2 |
| F6 | `generic_structs`/`generic_enums` are never mangled by `resolve.rs`'s per-module scoping pass (`resolve.rs:798-803` mangles `structs`/`enums` only) — Round 1's claim that the two modules' `Widget` ctors register under distinct mangled env keys (`Widget__m1`/`Widget__m2`) is **false**; both register under the identical bare key `"Widget"`. | New | grep, direct read |
| F7 | The two `GenericStructDecl` headers (one per module) still mint **distinct `StructId`s** on instantiation — `instantiate_struct`'s dedup key is `(idx, module, args, lens)` and `idx` differs per module's own header entry. So the identity-vs-rendering framing Round 1 used (F2/F3) was not simply backwards; the `StructId`s genuinely are distinct. The bug is a third thing: overload *selection* among distinct-identity, same-input-signature candidates, blind to both output type and caller module. | New | pb2, driver.rs:1114 (existing) |
| F8 | A mangle-only fix (mangling `generic_structs`/`generic_enums` the way `structs`/`enums` already are) does **not** fix the silent cross-pick — spiked and measured `2 2` unchanged — and regresses 3 existing tests (`driver::tests::whole_closure_generic_pre_pass_registers_each_header_once`, two `generic_header_colliding_with_a_concrete_type_is_a_duplicate` variants) whose premises assume the generic header's bare name is unmangled. | New | pb2 + mangle spike, reverted |
| F9 | R4's scope fence ("the ctor-generated type-mismatch message only", `check_outputs`'s `SlotMatch::Mismatch` arm) misses a second live diagnostic call site: when both modules' consumers spell the full instantiation as a declared parameter type, the collision instead renders through `type_mismatch_error` (`check.rs:1506`, `` `{op}` expected `{expected}`, found `{found}` ``) — a different function with a different message shape. | New | pb3, pb4 |
| F10 | `poly.rs:2390` (mono generic-target branch) reachability verdict is unchanged from Round 1 (F4/F5): still no live trigger found, same architectural argument (whole-program `poly_env`) applies. `poly.rs:1915` (inline re-entry path) verdict is **inconclusive**: direct self-recursion in an inline generic-target member is intercepted by a distinct, dedicated "always-spliced word cannot be recursive" guard *upstream* of `:1915`, not by `:1915` itself; mutual recursion between two inline members (the next candidate shape) was not completed this round due to trait-member declaration syntax friction. | New | pa5_inline_reentry |

### Corrected root-cause mechanism (replaces Round 1's "Root of the `pb` mismatch")

Round 1 traced the collision to *rendering only* (`Type::name()` module-blind
display over already-distinct `StructId`s, with env keys assumed already
module-distinct via mangling). That trace is wrong at its first step: the env
keys are **not** module-distinct (F6), because `generic_structs` is never
mangled. But the `StructId`s **are** distinct (F7) — the dedup key includes
`module`. The actual mechanism is a third layer neither round initially found:
**overload-candidate selection at the call site** (`check/terms.rs:955`,
`candidates.iter().find(|o| ... o.sig.inputs ...)`) matches on input signature
alone. When `env["Widget"]` holds two `Overload`s (one per module, both `i64 ->
Widget[i64]`-shaped, with the *outputs* being the two distinct-`StructId`
`Widget[i64]`s that render identically), `.find` returns the first `Vec` entry
whose *inputs* match — never consulting which `StructId` the caller's own module
actually declared, or even the output type at all. This is why the *specific*
consumer shape matters: a consumer whose declared type spells `Widget[i64]`
(module `b`'s `usesize`) forces the checker to unify against a concrete
`StructId` sooner, in a context where a mismatch against that unified `StructId`
is checked directly (`pb3`/`pb4`'s fails-closed shapes) — but a consumer that
never spells the type (module `a`'s `run ( i64 -- i64 ) Widget sized`) leaves the
ctor call to resolve through the ambiguous multi-candidate `env` lookup with no
such later cross-check, and the wrong candidate's `StructId` (and therefore the
wrong impl, since impl dispatch is `StructId`-keyed) silently wins.

### Q1 — (a): unchanged from Round 1 for `poly.rs:2390`; open for `poly.rs:1915`

Round 1's Q1 recommended a targeted probe at `poly.rs:1900-1922` before ruling on
(a); this round attempted it and found a **different, dedicated guard**
("an always-spliced word cannot be recursive") intercepts the most direct
candidate shape (self-recursion) before `poly.rs:1915` can fire. This is
consistent with either reading Round 1 offered (dead guard, or blocked-behind-a-
different-gap) but does not settle which: it shows one *specific* path to
`:1915` is blocked by a *different* mechanism, not that all paths are. The
mutual-recursion shape (two inline generic-target members each calling the
other) remains untried; Phase 3's reachability probe should attempt it directly
rather than relying on this round's incomplete attempt (blocked on trait-member
declaration syntax, not a design question).

### Q2 — (b): the design choice is no longer "diagnostic vs qualified-spelling" —

it is now "diagnostic-only is insufficient; a real fix needs dispatch-selection
changes too"

R3's ruling ("module-scope/diagnostic-only... There is no soundness bug to fix")
is **falsified** by F2 (Round 2)/pb2: there is a soundness bug, in the
`env["Widget"]` multi-candidate resolution path, that a render-only fix cannot
touch — the wrong `Overload` (wrong `StructId`, wrong impl) is selected and
*executed*, not merely mis-displayed. A real fix needs at minimum:

1. Module-scoping `generic_structs`/`generic_enums`' names (F8 shows this alone
   is insufficient and has collateral against two other invariants that
   currently rely on the bare, unmangled header name — the pre-pass dedup at
   `driver.rs:1426` and `check_duplicate_type_names`'s cross-check against
   concrete `structs`/`enums` bare names at `declarations.rs:1231`).
2. Widening the overload-candidate match at `check/terms.rs:955` (and its
   `poly.rs:3202`/`poly.rs:3258` analogues, not audited this round) to
   disambiguate same-input-signature candidates by output type and/or the
   calling module, not input alone.

Both are real design/sizing questions for the spec, not diagnostic-string
changes. This round did not spike (2) at all (only traced it) — a Phase 2 sizing
pass should treat "is (2) itself sound, and does it interact with legitimate
same-input-different-output overloading elsewhere in the language" as an open
question, since the fix touches general overload resolution, not just this
ctor-collision shape.
