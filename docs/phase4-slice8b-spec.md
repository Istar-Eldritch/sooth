# Phase 4 Slice 8b: `drop`'s import scope + 8a's operator module-scoping gap (spec)

Derived from `docs/phase4-slice8b-brief.md`. Three independent items, one shared
primitive. None of them is about naming a disposal word: the `disposal:` design is
abandoned, every disposal word here is `drop ( 'T -- )`, and the leaf-handle convention
already ships (`examples/resources.sth`).

## Grounding facts (verified against the repo, 2026-08-10)

The mechanism below rests on these, each re-checked against the current source; where the
brief's line number had drifted it is corrected here.

- **`drop`'s check-time arm records, never resolves.** `check_shuffle`'s `"drop"` arm
  (`check.rs:10389`) pops the top slot and pushes its type onto `prov.dropped` (skipping a
  compile-time-only quotation marker). No lookup runs. `check_shuffle`'s *direct* callers are
  the body walker (`check.rs:8407`) and one unit test (`check.rs:10790`) — but that body
  walker is `check_terms_relaxed` (`check.rs:7952`), which neither it nor its plain entry
  point `check_terms` (`check.rs:7913`) currently take a `modules` parameter. Both are called
  from ~10 sites inside `check.rs` (word bodies, `if`/`case` arms, `infer_line`), none of
  which pass anything module-shaped today. **Corrected from the brief: this is not a
  two-line change at `check_shuffle` alone — `modules` has to be threaded through this whole
  chain**, not just its two direct call sites (R3 below is written to that reality).
- **`infer_line` (the REPL's bare-line checker, `check.rs:3627`) has no module concept at
  all, and neither does `repl.rs`.** `infer_line`'s signature carries `env`/`structs`/
  `enums`/`poly_env`/`combinators` but nothing resembling `Module.modules`; grepping
  `src/repl.rs` for `ModuleInfo`/`modules:` returns nothing. A REPL session's own `import:`
  lines go through a structurally separate mechanism (`compile_import_closure`, epoch-tagged
  `dlopen` retention, `repl.rs`), never through `Module.modules`/`selective`. So the brief's
  open question 4 ("verify REPL retention directly") resolves to: **there is nothing to
  thread the new gate through on the REPL path without inventing new session state**; D1/R12
  below choose not to invent it (see R8/R12's scope note).
- **`check_operator`'s poly-mode call site is behind a second, separate call chain.**
  `check.rs:5452` is inside `poly_delegate_op` (`check.rs:5429`), called only from
  `poly_call_term`, reached only from `check_poly_body` (`check.rs:4962`) — itself called
  from *two* places: `check::check`'s per-word loop (`check.rs:2234`, a real `&Module` in
  scope) and `repl.rs:2390` (a REPL session retaining a poly word, no `Module` in scope).
  R12's "two call sites" for `check_operator` therefore span two independent chains with two
  independent REPL/native splits, not one.
- **`drop`'s codegen arm dispatches by concrete type.** `ir.rs`'s `lower_call` `"drop"` arm
  (`ir.rs:3573`) calls `emit_drop` (`ir.rs:4779`), which selects the override from a
  `StructId`-keyed registry it rebuilds independently. Nothing from `check` is threaded to
  it.
- **Program-wide uniqueness / `has_drop_overload`.** `find_drop_overloads` (`check.rs:1736`)
  runs inside `check::check` (`check.rs:2041`), and its result sets
  `structs[id].has_drop_overload = true` (`check.rs:2051`), rejecting a second override for
  one struct. Unconditional, scope-independent.
- **`drop` is excluded from `env` and from mangling.** The env-population loop skips every
  `drop`-named word (`check.rs`, the `drop_overload_indices.contains(&idx)` guard around
  `check.rs:2135`); `resolve::mangle` (`resolve.rs:31`) exempts the literal name `drop`,
  on the stated premise that dispatch never goes through `env`.
- **Per-module import data already rides on the `Module`.** Every `WordDef`/`StructDecl`
  carries an owning `module: u32`; `Module.modules: Vec<ModuleInfo>` (`ast.rs`) holds, per
  module, `imports: HashMap<qualifier, u32>`, `exports`, and
  `selective: HashMap<name, target_module>` (`ast.rs:86`). `check::check(&mut Module)`
  already has, without any new parameter to *itself*, both "which module owns this decl" and
  "which bare names did module M import selectively." The driver's `selective_by_module`
  (`driver.rs:205`) is a parallel copy used only by the separate `check_selective_imports`
  pass; the primitive below reads `Module.modules[..].selective` instead. **This does not mean
  no new plumbing is needed anywhere** — `check::check` has it, but getting it from there down
  to `check_shuffle`/`check_operator` is exactly the chain the two bullets above measured;
  R3/R12 carry `Option<&[ModuleInfo]>` down that chain rather than assuming it arrives for
  free.
- **Operator decls are mangled per module in a ≥2-module build; their bare call sites are
  not.** `resolve_modules` (`resolve.rs`) mangles an operator-named decl to `+__m{k}`
  whenever the closure has ≥2 modules, and leaves it bare only in the forced single-module
  case (`single && is_operator_dispatch_name(&w.name)` at the decl-mangle loop). The body
  rewrite deliberately leaves a *bare* operator call unrewritten
  (`!is_operator_dispatch_name(core)` at `resolve.rs:241`) so `check_operator`'s
  operand-type dispatch can claim it. Net for ≥2 modules: `env` holds the overload under
  `+__m{k}`, but `check_operator` is handed `env.get("+")` (`check.rs:8410`,
  and the poly path `check.rs:5452`), which is `None` — so a module's own operator overload
  is reachable only through the qualified form (`v::+`, itself rewritten to `+__m1` and
  resolved as an ordinary word), never bare. This is 8a's residual gap.
- **`Ctx::Word` (`check.rs:1640`) does not carry the caller's module id today.** It has
  `name`, `mangled`, `effect`, `structs`, `enums`; the module is only recoverable from the
  `__m{k}` suffix on `mangled`. `Ctx::Line` (the REPL / bare-line context, built by
  `infer_line` at `check.rs:3627`) has no module and denotes module 0.
- **Generated struct accessors are ordinary env words.** `struct_generated_sigs`
  (`check.rs:3437`) registers, per struct `S`, a constructor `S`, a destructure
  `S> ( S -- T1..Tn )`, per-field getters `S>f ( S -- Tf )`, and functional setters
  `S<f ( S Tf -- S )`. The two moving accessors (`S>` and `S>f`) move fields out of the
  struct; `check_access_word` (`check.rs:9900`) handles only `@`/`!`/`+!`, so these
  accessors flow through the ordinary env call path.
- **The golden that must invert.** `tests/phase4_modules.rs:384`
  (`imported_linear_type_is_disposed_by_drop`): `lib.sth` exports `mk`/`Res` but never
  `drop`; `main.sth` does a qualified-only `import: lib "lib.sth" ;`, then `lib::mk drop`,
  and today the module's own destructor observably runs (prints `7`). Under this slice that
  is an error.

## The three decisions this slice implements

**D1 — a check-time visibility gate at the existing `drop` call site, not a move into 8a's
operator table.** `find_drop_overloads` and `emit_drop` are unchanged. `check_shuffle`'s
`"drop"` arm gains one check: when the popped type is a struct with
`has_drop_overload = true`, that struct's override must be *visible to the calling module*,
or the call is a located error naming the remedy.

**D2 — one module-visibility primitive, built once, consumed twice.** A single function
answers "is name X, owned by module L, visible to module M" against `Module.modules[M]`'s
own module identity plus its `selective` map. D1's gate and the 8a operator fix both consult
it; neither invents its own scoping mechanism.

**D3 — destructuring a type with `has_drop_overload = true` is a located error, full stop.**
A `S>`/`S>f` call whose struct `S` has an override is rejected regardless of which field is
extracted or how many remain, matching Rust's E0509. A composite with no override (`File`)
is unaffected; the leaf wrapper that owns the resource (`Fd`) is exactly where it fires.

## Requirements

Numbered, each with its test. "Located" means the diagnostic carries the offending line and
names the concrete word/type, per CLAUDE.md's "diagnostics are behaviour."

### D2: the shared visibility primitive

- **R1.** Add one function to `check.rs`, the sole authority on module visibility for a
  scoped name:
  `is_name_visible_to_module(modules: &[ModuleInfo], caller: u32, defining: u32, name: &str) -> bool`,
  returning `true` iff `defining == caller` (declared in the caller's own module) **or**
  `modules[caller as usize].selective.get(name) == Some(&defining)` (the caller selectively
  imported that bare name from that module). No other route makes a scoped name visible; a
  qualified-only import (`import: lib "lib.sth"`) does not.
  - Unit test beside it: `visibility_own_module_is_visible`,
    `visibility_selectively_imported_is_visible`,
    `visibility_qualified_only_import_is_not_visible`,
    `visibility_unrelated_module_is_not_visible`. Construct `ModuleInfo` values directly
    (do not route through a full build): the primitive is a pure function of
    `(modules, caller, defining, name)` and must be tested as one.

- **R2.** Add `module: u32` to `Ctx::Word` (`check.rs:1640`), populated by `word_ctx` from
  `word.module`; `Ctx::Line` denotes module 0. Expose `Ctx::module(&self) -> u32`
  (`Word { module, .. } => module`, `Line => 0`). This is the caller-module source both D1's
  gate and the operator fix read.
  - Unit test: `ctx_word_carries_owning_module`, `ctx_line_is_module_zero`.

### D1: `drop`'s import-visibility gate

- **R3.** Thread `modules: Option<&[ModuleInfo]>` down the *whole* `check_terms`/
  `check_terms_relaxed` chain (`check.rs:7913`/`:7952`, all ~10 internal call sites) to
  `check_shuffle`, not just its two direct callers. `check::check`'s per-word call passes
  `Some(&module.modules)`; `infer_line` (`check.rs:3627`, the REPL path) passes `None` — see
  R8, this is a deliberate scope cut, not an oversight. In `check_shuffle`'s `"drop"` arm,
  after popping the top slot and before the existing `prov.dropped.push`, when the popped
  type is `Type::Struct(id, _)`, `ctx.structs()[id.index()].has_drop_overload` is true, *and*
  `modules` is `Some(m)`, call a new helper `check_drop_import_visibility`. When `modules` is
  `None` (REPL), the arm is byte-for-byte what it is today — no gate, no lookup. The
  `prov.dropped` recording itself is unchanged either way.

- **R4.** `check_drop_import_visibility(ctx, span, m: &[ModuleInfo], decl) -> Result<(), String>`
  returns `Ok(())` when `is_name_visible_to_module(m, ctx.module(), decl.module,
  demangle(&decl.name))` holds, else a located error (R5). The name passed to the primitive
  is the struct's *source* (demangled) name, since `modules[..].selective` is keyed by
  source names and the struct decl's `name` is mangled (`Res__m1`) in a ≥2-module build.
  - Unit test: `drop_of_locally_declared_override_is_ok`,
    `drop_of_selectively_imported_type_is_ok`,
    `drop_of_qualified_only_imported_type_is_error`,
    `drop_of_plain_struct_no_override_is_ungated` (a struct with
    `has_drop_overload = false` never reaches the helper, disposes structurally),
    `check_shuffle_with_no_modules_is_ungated` (the `None` path: a struct with an override,
    checked with `modules: None`, is not gated — the REPL contract R8 relies on, tested at
    the unit level where `None` is trivial to construct rather than only through a full REPL
    session).

- **R5.** The located diagnostic names the demangled type, the qualifier the caller binds it
  under (from `modules[caller].imports`, the qualifier whose value is `decl.module`), and the
  remedy — importing the type by name. Exact text (a `Ctx::Word` site):

  ```text
  error: cannot `drop` a value of type `lib::Res` in `main` (line N)
    disposing it runs a `drop` destructor declared in module `lib`, which this module has not imported by name
    note: add `Res` to the import (`import: lib | Res | "..."`), or dispose it in a module that declares `Res`
  ```

  The `Ctx::Line` arm drops the `in`main`` clause. `demangle` is `resolve::demangle_word`
  / `demangle_call`, already used for diagnostics.
  - Test asserts the exact message on the error path (`drop_of_qualified_only_imported_type_is_error`),
    not merely that it fails.

- **R6.** Golden inversion. `tests/phase4_modules.rs:384`
  (`imported_linear_type_is_disposed_by_drop`) is rewritten: under the qualified-only import
  it now expects the R5 error at build time (rename to
  `imported_linear_type_dropped_without_importing_it_is_error`). Add the positive companion
  `imported_linear_type_dropped_after_selective_import_ok`: `main.sth` does
  `import: lib | Res | "lib.sth" ;` and bare `drop` runs `lib`'s destructor (prints `7`,
  exit 0). Both are exit-criterion goldens.

- **R7.** The non-disposing uses of a qualified-only imported resource type still compile:
  importing the type, holding a value of it, forwarding it to another word, and `&`-reading
  it through a reference all pass, because none of them reaches the gate (only bare `drop`
  does). Golden `imported_resource_qualified_only_non_disposal_uses_compile`.

- **R8.** REPL enforcement is explicitly out of scope, and this is a scope cut, not the
  "already fine" claim the brief's open question 4 hoped for — verifying directly against
  `repl.rs` (as instructed) found the opposite of what was assumed. `infer_line` has no
  `Module.modules` concept at all today (grepping `src/repl.rs` for `ModuleInfo`/`modules:`
  returns nothing), and a REPL session's `import:` lines go through a structurally separate
  mechanism (`compile_import_closure`, epoch-tagged `dlopen` retention) that has no
  `selective`/`ModuleInfo` shape to hand D1's primitive. Inventing one is real new session
  state with its own design questions (how a REPL-side selective import gets recorded, how
  epoch renaming interacts with a `defining` module id) and is not this slice's job. So
  `infer_line` passes `modules: None` (R3), the gate never fires on the REPL path, and a
  bare `drop` in a session disposes exactly as it does today — both a locally-defined
  override and one retained via `import:`. Regression golden in `repl.rs` tests:
  `repl_dispose_of_session_defined_override_is_unaffected` — define a `drop` override,
  construct a value, `drop` it, assert the destructor's observable output and that the
  residual stack line is exactly empty (do not assert a `contains` on the residual — the
  REPL prints the whole residual stack per line). A second regression golden,
  `repl_dispose_of_imported_override_without_selective_import_is_unaffected`, pins the
  specific case that would panic if `None` were ever replaced by an empty `Some(&[])` instead
  of a real opt-out: import a library's resource type into a session and dispose it with a
  bare `drop` with no selective import, asserting it still runs the destructor exactly as
  today (not the new R5 error — that error is native-only).

### D3: destructuring bypasses a `drop` override

- **R9.** When the body walker resolves a call whose name is a moving generated accessor of a
  struct `S` with `has_drop_overload = true` — i.e. `S>` (destructure) or `S>f`
  (field getter), matched against `struct_generated_sigs`' spelling — it is a located error,
  regardless of the extracted field's own type or the field count. The functional setter
  `S<f` (which returns `S`, so the struct stays live) and reference reads are **not** guarded.
  The guard fires only where `S.has_drop_overload` is true; a composite like `File` whose
  own `has_drop_overload` is false is unaffected, so `File>fd` moving a still-linear `Fd`
  out stays legal.

- **R10.** Insertion point: in the ordinary env call path (the same site that already applies
  a generated accessor's signature), before the accessor's effect is applied, detect
  `name == format!("{}>", s.name)` or `name == format!("{}>{}", s.name, field)` for the
  struct `s` the top operand names, and reject when `s.has_drop_overload`. Prefer a dedicated
  located diagnostic (R11) over a name-deny that would surface a generic "unknown word" — the
  latter is cheaper but violates "diagnostics are behaviour." (D3 resolves the brief's open
  question 3 in favour of the dedicated diagnostic.)

- **R11.** Exact text:

  ```text
  error: cannot destructure `Fd` in `main` (line N): it defines `drop`, so moving its fields out would skip its destructor
    note: dispose it with `drop`, or read a field through a borrow (`&`) instead of moving it out
  ```

  (demangled type name; `Ctx::Line` arm drops the `in`main`` clause.)
  - Tests: `destructure_of_drop_overloaded_type_is_error` (`Fd>`),
    `field_move_of_drop_overloaded_type_is_error` (`Fd>n`),
    `field_move_of_composite_holding_resource_is_ok` (`File>fd`, `File` has no override),
    `setter_on_drop_overloaded_type_is_not_guarded` (`Fd<n` still checks as before).

### 8a: the operator module-scoping fix (consumes D2)

- **R12.** A bare operator call resolves against the operator overloads *visible to the
  calling module*, the way a qualified call already does. `check_operator`'s two call sites
  sit behind two independent chains, each needing the same `Option<&[ModuleInfo]>` threaded
  down as R3 (this is not a rerun of R3's plumbing — it is separate work, on a separate
  chain): the concrete path (`check.rs:8410`, inside `check_terms_relaxed`, so it rides R3's
  threading for free once that lands) and the poly path (`check.rs:5452`, inside
  `poly_delegate_op` ← `poly_call_term` ← `check_poly_body` at `check.rs:4962`, called from
  `check::check`'s per-word loop with `Some(&module.modules)` at `check.rs:2234` and from
  `repl.rs:2390` with `None`). At each call site, when `modules` is `Some(m)`, replace the
  flat `env.get(name)` candidate lookup with a caller-scoped assembly; when `None` (REPL),
  fall back to `env.get(name)` exactly as today — the operator fix has the identical
  native/REPL split as D1's gate, for the identical reason (R8). Operator decls stay mangled
  per module in a ≥2-module build (unchanged from `aaafa91`); the assembly unions, for bare
  operator `name` and caller module `M = ctx.module()`: the overload under `mangle(name, M)`
  (own module) and, for each `k` such that `m[M].selective.get(name) == Some(&k)`, the
  overload under `mangle(name, k)` (selectively imported). Membership is decided by the R1
  primitive, not re-derived. In a single-module build the operator decl is left bare by
  `resolve_modules`, so the assembly degenerates to `env.get(name)` and the single-file
  corpus is byte-for-byte unchanged.

- **R13.** Consequences, each a golden in `tests/phase4_modules.rs`:
  - `own_module_operator_overload_reachable_bare_in_multi_module`: a module declaring `+`
    for its own struct and a second module in the closure — the bare `+` on that struct
    resolves to the own overload (today it misses and falls to the builtin, rejecting the
    struct operands).
  - `selectively_imported_operator_does_not_hijack_unrelated_module`: module N that did not
    import module X's `+` overload does not see it; a bare `+` in N whose operands match only
    X's overload is the ordinary "operator operands mismatch" error, not a silent dispatch to
    X's word.
  - `single_module_operator_overload_unchanged`: a single-file program overloading an
    operator name compiles and runs byte-for-byte as before (regression guard;
    mutation-check by deleting the single-module fallback branch and confirming this test
    fails).

  (R12 resolves the brief's open question 5: filter/assemble at `check_operator` using the
  shared D2 primitive; do not re-spell operator decls beyond the existing per-module mangle.)

## What this slice does not change

- **`emit_drop` / `ir.rs` drop lowering / epoch symbols** — D1 is check-time only.
- **The poly-body `"drop"` arm** (`check.rs:5252`) — a generic `'T` pop, resolved to the
  concrete override at monomorphization; out of scope, do not touch.
- **`check_tail_call_cycles`** (`check.rs:3962`) — its exclusion of drop-override indices
  stays; update only its comment to mention the new gate, not its logic.
- **`check_duplicate_word_names`'s skip of `drop`-named words** — `find_drop_overloads` still
  owns program-wide uniqueness by struct id, unconditionally.
- **`examples/resources.sth`** (leaf-handle shape) and **`tests/phase3_resources.rs`** — must
  remain green untouched.

## Documentation amendments (part of this slice)

- **`DESIGN.md`**, the "Disposal crosses the export boundary for free…" paragraph (the one
  asserting "a destructor runs without being named"): amend to state the current rule —
  disposing an imported resource type with a bare `drop` requires the type to be visible to
  the calling module (imported by name or declared locally). State the rule as it now is;
  do not narrate the reversal.
- **`ROADMAP.md`**: record slice 5a's Criterion 17 as superseded by this slice's gate. State
  the current criterion; no change narrative.

Both are current-state edits per the project's "ROADMAP/DESIGN state current design only"
convention. (This spec file is the only file this task itself writes; the amendments are done
during implementation.)

## Out of scope

- The `disposal:`/named-disposal-word apparatus — abandoned.
- Whether *derived* disposal can thread an allocator to a nested resource field — Phase 6.
- `Vec`, growable containers, plural allocators, lifting the linear-array-element restriction
  — Phase 6.
- General module-scoped visibility for every name — every non-operator name is already
  module-unique by mangling; D2 is scoped to the ~20 operator names plus `drop`, not a
  rewrite of `env`'s key type.
- **REPL enforcement of D1's gate and R12's operator fix.** Both are native-build-only
  (`modules: Option<&[ModuleInfo]>` is `None` on the REPL path); a REPL session's own
  `import:` mechanism (epoch-tagged `dlopen` retention) has no `ModuleInfo`/`selective`
  shape to give the shared primitive, and inventing one is its own slice, not a rider on
  this one. R8's two goldens exist to keep this a stated cut, not a silent regression.

## Exit criteria (goldens)

1. A module disposes an imported resource type's value with a bare `drop` **only** if that
   type's override is visible to it (imported by name, or declared locally); disposing it
   without that visibility is a located error naming the remedy, while importing, holding,
   forwarding, and `&`-reading it under a qualified-only import all still compile (R5–R7).
2. `tests/phase4_modules.rs`'s former `imported_linear_type_is_disposed_by_drop` is inverted
   to require the import, with a positive selective-import companion (R6).
3. A plain struct with no override anywhere disposes structurally with no gate (R4's
   `_ungated` test).
4. Destructuring a struct with an override (`S>`/`S>f`) is a located error naming the remedy;
   a composite with no override is unaffected (R9–R11).
5. A module's own operator overload is reachable bare in a ≥2-module build; a selectively
   imported operator does not leak to modules that did not import it; the single-module
   corpus is unchanged (R13).
6. `DESIGN.md`'s disposal paragraph and `ROADMAP.md`'s slice 5a Criterion 17 read as
   current-state under the new rule.
7. `examples/resources.sth` and `tests/phase3_resources.rs` remain green untouched.
8. REPL disposal of both a session-defined and an `import:`-retained resource type's `drop`
   override is byte-for-byte unaffected — neither gains the new gate nor regresses (R8's two
   goldens).
9. Green: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "D2 module-visibility primitive plus D1 drop import-visibility gate: is_name_visible_to_module, Ctx caller module, threading Option<&[ModuleInfo]> through check_terms/check_terms_relaxed to check_shuffle (Some from check::check, None from infer_line), the drop-arm gate and diagnostic, the inverted golden and its selective-import companion, the non-disposal-uses golden, and both REPL no-regression goldens; amend DESIGN.md and ROADMAP.md to current-state",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "D3 destructure-bypass guard: reject S> and S>field moving accessors when the struct has a drop override, with a dedicated located diagnostic; composite-without-override and setter paths stay legal",
      "difficulty": "standard"
    },
    {
      "phase": 3,
      "focus": "8a operator module-scoping fix consuming the D2 primitive: caller-scoped candidate assembly at check_operator's two call sites, threading the same Option<&[ModuleInfo]> through check_poly_body's chain (Some from check::check, None from repl.rs) in addition to phase 1's check_terms_relaxed threading, so a bare operator resolves against overloads visible to the calling module in both concrete and poly bodies; own-module-reachable, no-cross-module-leak, and single-module-unchanged goldens",
      "difficulty": "hard"
    }
  ]
}
```
