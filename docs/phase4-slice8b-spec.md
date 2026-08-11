# Phase 4 Slice 8b: `drop`'s import scope + 8a's operator module-scoping gap

Three independent items over one shared primitive. `disposal:` is abandoned; every
disposal word is `drop ( 'T -- )` and the leaf-handle convention already ships
(`examples/resources.sth`). Delivered across three phases.

## Grounding facts

- `drop`'s check-time arm (`check_shuffle`'s `"drop"` arm) only **records** the popped
  type onto `prov.dropped`; it runs no lookup. Its callers are `check_terms_relaxed` /
  `check_terms`, neither of which took a `modules` parameter, so gating `drop` meant
  threading `modules` down that whole ~10-site chain, not a two-line edit.
- `infer_line` (REPL bare-line checker) and `repl.rs` have **no `Module.modules`
  concept**. REPL `import:` lines use a structurally separate mechanism
  (`compile_import_closure`, epoch-tagged `dlopen` retention) with no `selective`/
  `ModuleInfo` shape. So the REPL path is a deliberate scope cut (see R8), not a gap to
  patch.
- `check_operator`'s poly call site sits behind a second chain (`poly_delegate_op` ←
  `poly_call_term` ← `check_poly_body`), reached from `check::check`'s per-word loop
  (real `&Module`) and from `repl.rs` (no `Module`). Two independent native/REPL splits.
- `drop`'s codegen (`emit_drop`) dispatches by concrete `StructId`; nothing from `check`
  is threaded to it. Program-wide override uniqueness (`find_drop_overloads` →
  `has_drop_overload`) is unconditional and scope-independent. `drop` is excluded from
  `env` and from `resolve::mangle`.
- Per-module import data rides on `Module.modules: Vec<ModuleInfo>` (`imports`, `exports`,
  `selective: HashMap<name, target_module>`). `check::check(&mut Module)` has it, but
  getting it down to `check_shuffle`/`check_operator` is the chain above.
- In a ≥2-module build, operator decls are mangled per module (`+__m{k}`) but bare
  operator call sites are left unrewritten for `check_operator`'s operand dispatch. Net:
  `env` holds the overload under `+__m{k}` while `check_operator` is handed `env.get("+")`
  = `None`, so a module's own operator overload was reachable only qualified. This is 8a's
  residual gap.
- Generated moving accessors (`S>`, `S>f`) flow through the ordinary env call path;
  `check_access_word` handles only `@`/`!`/`+!`.

## The three decisions

- **D1** — a check-time visibility gate at the existing `drop` call site (not a move into
  8a's operator table). `find_drop_overloads`/`emit_drop` unchanged; when the popped type
  is a struct with `has_drop_overload`, its override must be visible to the calling module
  or the call is a located error.
- **D2** — one module-visibility primitive, built once, consumed by D1 and the 8a fix.
- **D3** — destructuring a type with `has_drop_overload` (`S>`/`S>f`) is a located error,
  matching Rust's E0509. Composites with no override (`File`) are unaffected; the leaf
  wrapper (`Fd`) is where it fires.

## Requirements

### D2: shared visibility primitive

- **R1.** `is_name_visible_to_module(modules, caller, defining, name) -> bool`, the sole
  authority: `true` iff `defining == caller` **or**
  `modules[caller].selective.get(name) == Some(&defining)`. A qualified-only import does
  not make a scoped name visible. Unit tests construct `ModuleInfo` directly:
  `visibility_own_module_is_visible`, `visibility_selectively_imported_is_visible`,
  `visibility_qualified_only_import_is_not_visible`,
  `visibility_unrelated_module_is_not_visible`.
- **R2.** `Ctx::Word` gains `module: u32` (from `word.module`); `Ctx::Line` denotes module
  0. Expose `Ctx::module(&self) -> u32`. Tests: `ctx_word_carries_owning_module`,
  `ctx_line_is_module_zero`.

### D1: `drop`'s import-visibility gate

- **R3.** Park `modules: Option<&'a [ModuleInfo]>` on `Ctx::Word` itself, read back via
  `ctx.modules()`; `Ctx::Line` carries no such field and structurally returns `None` (so
  `infer_line`'s REPL path never threads one, deliberate, R8), with no parameter added to
  `check_terms`/`check_terms_relaxed`/`check_shuffle`'s own signatures. `check::check`
  builds every word's `Ctx::Word` with `Some(&module.modules)`. In the `"drop"` arm, when
  the popped type is `Type::Struct(id,_)`, `has_drop_overload` is true, and `ctx.modules()`
  is `Some(m)`, call `check_drop_import_visibility`. `None` = today's behaviour, no gate.
  `prov.dropped` recording unchanged.
- **R4.** `check_drop_import_visibility` returns `Ok` when
  `is_name_visible_to_module(m, ctx.module(), decl.module, demangle(&decl.name))` holds,
  else the R5 error. The passed name is the struct's demangled source name (`selective` is
  keyed by source names). Tests: `drop_of_locally_declared_override_is_ok`,
  `drop_of_selectively_imported_type_is_ok`,
  `drop_of_qualified_only_imported_type_is_error`,
  `drop_of_plain_struct_no_override_is_ungated`,
  `check_shuffle_with_no_modules_is_ungated`.
- **R5.** Located diagnostic names the demangled type, the qualifier the caller binds it
  under, and the remedy:

  ```text
  error: cannot `drop` a value of type `lib::Res` in `main` (line N)
    disposing it runs a `drop` destructor declared in module `lib`, which this module has not imported by name
    note: add `Res` to the import (`import: lib | Res | "..."`), or dispose it in a module that declares `Res`
  ```

  `Ctx::Line` arm drops the `in`main`` clause. Test asserts exact text.
- **R6.** Golden inversion. `imported_linear_type_is_disposed_by_drop` →
  `imported_linear_type_dropped_without_importing_it_is_error` (qualified-only import now
  errors at build). Positive companion
  `imported_linear_type_dropped_after_selective_import_ok` (`import: lib | Res | ...`;
  bare `drop` runs the destructor, prints `7`, exit 0).
- **R7.** Non-disposing uses of a qualified-only imported resource type still compile
  (hold, forward, `&`-read); none reach the gate. Golden
  `imported_resource_qualified_only_non_disposal_uses_compile`.
- **R8.** REPL enforcement is out of scope (a stated cut, not "already fine"). `infer_line`
  has no `Module.modules`; REPL `import:` uses epoch-tagged `dlopen` retention with no
  `ModuleInfo`/`selective` shape. `infer_line` builds a `Ctx::Line`, which carries no
  `modules` field and structurally returns `None`; a bare `drop` in a session disposes
  exactly as today. Regression goldens:
  `repl_dispose_of_session_defined_override_is_unaffected` (assert destructor output and an
  exactly-empty residual stack line, not a `contains`) and
  `repl_dispose_of_imported_override_without_selective_import_is_unaffected` (pins the case
  that would panic if `None` were ever an empty `Some(&[])`).

### D3: destructuring bypasses a `drop` override

- **R9.** A moving generated accessor (`S>` or `S>f`) on a struct with `has_drop_overload`
  is a located error regardless of field or count. The functional setter `S<f` (returns
  `S`) and reference reads are not guarded. `File>fd` (composite, no override) stays legal.
- **R10.** Insertion point: the ordinary env call path applying a generated accessor's
  signature, before its effect is applied; detect `name == format!("{}>", s.name)` or
  `format!("{}>{}", s.name, field)` and reject when `s.has_drop_overload`. Dedicated
  diagnostic (R11), not a name-deny.
- **R11.** Exact text:

  ```text
  error: cannot destructure `Fd` in `main` (line N): it defines `drop`, so moving its fields out would skip its destructor
    note: dispose it with `drop`, or read a field through a borrow (`&`) instead of moving it out
  ```

  Tests: `destructure_of_drop_overloaded_type_is_error` (`Fd>`),
  `field_move_of_drop_overloaded_type_is_error` (`Fd>n`),
  `field_move_of_composite_holding_resource_is_ok` (`File>fd`),
  `setter_on_drop_overloaded_type_is_not_guarded` (`Fd<n`).

### 8a: operator module-scoping fix (consumes D2)

- **R12.** A bare operator call resolves against the operator overloads visible to the
  calling module. Both `check_operator` call sites get `Option<&[ModuleInfo]>` threaded:
  the concrete path rides R3's threading; the poly path needs the same threaded through
  `check_poly_body`'s chain (`Some` from `check::check`, `None` from `repl.rs`). When
  `Some(m)`, replace flat `env.get(name)` with a caller-scoped union: the overload under
  `mangle(name, M)` (own module) plus, for each `k` with
  `m[M].selective.get(name) == Some(&k)`, `mangle(name, k)`. Membership decided by R1.
  `None` (REPL) falls back to `env.get(name)`. Single-module builds leave the decl bare,
  so the assembly degenerates to `env.get(name)`; single-file corpus byte-for-byte
  unchanged. Operator decls stay mangled per module (unchanged from `aaafa91`).
- **R13.** Goldens in `tests/phase4_modules.rs`:
  `own_module_operator_overload_reachable_bare_in_multi_module`,
  `selectively_imported_operator_does_not_hijack_unrelated_module` (bare `+` in an
  unrelated module → ordinary operands-mismatch error, not silent dispatch),
  `single_module_operator_overload_unchanged` (regression; mutation-check by deleting the
  single-module fallback branch).

## What this slice does not change

`emit_drop` / `ir.rs` drop lowering / epoch symbols (D1 is check-time only); the poly-body
`"drop"` arm; `check_tail_call_cycles` (comment-only update);
`check_duplicate_word_names`'s skip of `drop` words (`find_drop_overloads` still owns
program-wide uniqueness by struct id); `examples/resources.sth` (untouched).
`tests/phase3_resources.rs`'s existing tests are untouched; it gained one new REPL
regression golden (R8).

## Documentation amendments (done in-slice)

- **`DESIGN.md`** disposal paragraph: amend to the current rule (disposing an imported
  resource type via bare `drop` requires the type visible to the caller: imported by name
  or locally declared). Current-state only, no reversal narrative.
- **`ROADMAP.md`**: record slice 5a's Criterion 17 as superseded by this gate; current
  criterion only.

## Out of scope

`disposal:`/named-disposal apparatus (abandoned); derived disposal threading an allocator
to nested fields (Phase 6); `Vec`/growable containers/plural allocators/linear-array
restriction (Phase 6); general module-scoped visibility for every name (non-operator names
are already module-unique by mangling; D2 is scoped to operator names plus `drop`); **REPL
enforcement of D1's gate and R12's operator fix** (both native-build-only; REPL's
`import:` mechanism has no `ModuleInfo`/`selective` shape; R8's two goldens keep this a
stated cut).

## Exit criteria (goldens)

1. A module disposes an imported resource type with bare `drop` only if its override is
   visible (imported by name or locally declared); otherwise a located error naming the
   remedy, while import/hold/forward/`&`-read under a qualified-only import compile (R5–R7).
2. Former `imported_linear_type_is_disposed_by_drop` inverted to require the import, plus a
   positive selective-import companion (R6).
3. A plain struct with no override disposes structurally, no gate (R4 `_ungated`).
4. Destructuring a struct with an override (`S>`/`S>f`) is a located error; a composite
   with no override is unaffected (R9–R11).
5. A module's own operator overload is reachable bare in a ≥2-module build; a selectively
   imported operator does not leak; the single-module corpus is unchanged (R13).
6. `DESIGN.md` disposal paragraph and `ROADMAP.md` slice 5a Criterion 17 read as
   current-state.
7. `examples/resources.sth` remains untouched; `tests/phase3_resources.rs`'s existing
   tests are untouched, with one new REPL regression golden (R8) added.
8. REPL disposal of both a session-defined and an `import:`-retained override is
   byte-for-byte unaffected (R8's two goldens).
9. Green: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

## Implementation summary

- **Phase 1 (D2 primitive + D1 gate):** `src/check.rs` (`is_name_visible_to_module`,
  `Ctx::module`, `modules` threading, `check_drop_import_visibility`, R5 diagnostic),
  `DESIGN.md`, `ROADMAP.md`, inverted/companion/non-disposal goldens in
  `tests/phase4_modules.rs`, REPL no-regression goldens in `tests/phase4_repl_imports.rs`;
  `tests/phase3_resources.rs` confirmed green. Commits `c8af16f7`, `9eca2e6b` (review
  cycle 1).
- **Phase 2 (D3 guard):** `src/check.rs` destructure/field-move rejection with dedicated
  diagnostic; tests in `tests/phase4_combinators.rs`, `tests/phase4_generics.rs`. Commit
  `b0b09db5`.
- **Phase 3 (8a operator fix):** caller-scoped candidate assembly at both `check_operator`
  sites, poly-chain threading across `src/check.rs`, `src/repl.rs`, `src/resolve.rs`;
  goldens in `tests/phase4_modules.rs`. Commits `10bdb9d4`, `b272a8f0` (review cycle 1).
