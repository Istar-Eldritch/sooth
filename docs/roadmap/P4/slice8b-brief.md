# Phase 4 Slice 8b: `drop`'s import scope + 8a's operator module-scoping gap (brief)

Three items, none of them about naming a disposal word — that design (`disposal: close` on
`type:`) is abandoned; every disposal word in this design is `drop ( 'T -- )`, and the
leaf-handle convention it's replaced by already ships (`examples/resources.sth`).

## Recon 1: `drop`'s current dispatch has two halves that never interact

**Check-time:** `check_shuffle`'s `"drop"` arm (`check.rs:10389`) unconditionally pops and
records the type into `prov.dropped`. No lookup of any kind runs.

**Codegen-time:** `ir.rs`'s `lower_call` `"drop"` arm (`ir.rs:3573`) calls `self.emit_drop(v)`
(`ir.rs:4779`), which dispatches purely by the value's concrete type against a program-wide
`StructId`-keyed registry it rebuilds independently (`find_drop_overloads`, `ir.rs:1172`,
mirroring `check.rs:1736`).

**Uniqueness:** `find_drop_overloads` (`check.rs:1736`) also runs inside `check::check`
(`check.rs:2044`) to set `has_drop_overload` on every struct with an override
(`check.rs:2050`) and to reject a second override for the same struct — program-wide,
unconditionally, independent of scope.

**Exclusion from `env`:** the env-population loop (`check.rs:2130`–`2141`) skips every
`drop`-named word (`check.rs:2132`), and `mangle` (`resolve.rs:31`) exempts the literal name
`drop` from `__m{module}` renaming, on the stated premise (`resolve.rs`'s own doc comment,
lines 25–30) that dispatch never goes through `env`, so mangling it "would only break that
lookup for no benefit."

Net: nothing in this whole path asks what a module imported. Verified again live,
2026-08-10, against `imported_linear_type_is_disposed_by_drop`
(`tests/phase4_modules.rs:384`): `lib.sth` exports `mk`/`Res`, never `drop`, and the
consumer's bare `drop` still runs `lib`'s destructor.

## Recon 2: the fix is a gate, not a rewrite

A natural first instinct is to move `drop` through the same table `+` rides (8a's
`env`/`check_operator`/`resolve_overload` path). It doesn't need to. `find_drop_overloads`'s
job (program-wide uniqueness, `has_drop_overload`) and `emit_drop`'s job (structural dispatch
by concrete type, no symbol threaded from `check`) both already do exactly what this slice
needs and neither changes. The only new logic is a **check-time visibility gate**: at the
point `check_shuffle`'s `"drop"` arm pops a struct type with `has_drop_overload = true`,
verify that struct's override is visible to the calling module before accepting the pop; if
it isn't, a located error naming the word to import. A plain struct (`has_drop_overload =
false`) is untouched — its existing structural path has no gate to add.

`env` and `ctx` are already in scope exactly where this needs to fire: `check_shuffle` and
`check_operator` are called back-to-back from the same stack frame (`check.rs:8407`–`8410`),
`env` passed to the second call on the very next line. No plumbing change to reach it; the
arm just needs to consult it.

## Recon 3: the actual missing primitive is shared with 8a's operator gap

Neither `drop` nor an operator name has real per-module visibility in `env` today. `env` is
`HashMap<String, Vec<Overload>>`, flat and module-unaware. Operators fake scoping by
convention: `resolve::mangle` renames every ordinary decl to `name__m{module}` but leaves the
~20 `is_operator_dispatch_name` (`resolve.rs:43`) decls unmangled so `check_operator`'s
bare-name, operand-type dispatch can find them — the convention that breaks the moment a
second module joins the closure (8a's own gap, unchanged by `aaafa91`: that fix explicitly
preserved bare declarations for a ≥2-module closure's operators, only forcing mangling for
the single-file case). Per-module selective-import data already exists
(`selective_by_module: Vec<Vec<check::SelectiveName>>`, `driver.rs:205`) but nothing at
dispatch time consults it to build a per-module visible-candidate set.

`drop` has none of even that convention today, being excluded from `env` outright. Building
the one missing primitive — "is name X, overload set S, visible to module M" — once, and
giving both the operator fix and `drop`'s new gate the same answer, is smaller than solving
them twice.

## Recon 4: destructuring bypasses a `drop` override entirely, unaffected by everything above

Verified live, 2026-08-10:

```sooth
type: R tag i64 ;
: drop ( R -- ) | r | 999 . r R>tag drop ;
: main ( -- ) 5 R | r | r R>tag . ;
```

prints `5`, never `999` — the override never ran. `File>fd`-style field extraction is a
generated accessor (`struct_generated_sigs`, `check.rs:3437`: `{Name}>`/`{Name}>{field}`),
registered into `env` as an ordinary word with signature `(struct_ty -- field_ty)` and checked
like any other call — nothing consults `has_drop_overload` there the way recon 2's gate must.
Rust rejects this outright (E0509: cannot move out of a type implementing `Drop`) rather than
trying to run the destructor on "the rest"; Sooth should too — full extraction, not partial.
Scoped much smaller than the abandoned design's version: a plain composite (`File`, no
override) has nothing to bypass, so `File>fd` moving a still-linear `Fd` out stays legal; the
guard only fires where the struct being destructured itself has `has_drop_overload = true`.

## What doesn't change, so the spec doesn't reinvent it

- The poly-body `"drop"` arm (`check.rs:5252`) is a trivial pop for a generic `'T` value
  inside a poly word's body; dispatch to the concrete override happens later at
  monomorphization (verified this session: a generic `discard ( 'T -- ) drop` instantiated at
  a resource type calls the concrete override). Out of scope, don't touch.
- `check_tail_call_cycles` (`check.rs:3962`) excludes drop-overload indices from the
  tail-call graph so a trailing scalar `drop` doesn't fabricate a phantom cycle; its comment
  needs updating to describe the new gate, its logic doesn't change.
- `check_duplicate_word_names`'s skip of `drop`-named words (`check.rs:2978`) stays:
  `find_drop_overloads` still owns program-wide uniqueness by struct id, unconditionally,
  same as today.
- REPL retention and epoch-suffixed destructor symbols (`repl.rs`) assume today's
  structural, unconditional dispatch; since `emit_drop`/`ir.rs` don't change, this should be
  unaffected — confirm directly in the spec rather than assuming.

## Decisions taken on this brief

**D1. The fix is a visibility gate at the existing `drop` call site, not a move into the 8a
table.** `find_drop_overloads` and `emit_drop` are unchanged. `check_shuffle`'s `"drop"` arm
gains one new check: when the popped type is a struct with `has_drop_overload`, its override
must be visible to the calling module or the call is rejected, naming the word to import.
Settled against literally moving `drop` through `check_operator`/`env`'s `resolve_overload`
path (mechanically possible, since a drop override's `(struct_ty -- )` signature would
resolve by exact input-type match exactly like any operand-type-dispatched operator) because
nothing downstream of `check` needs the resolved symbol — `emit_drop` already finds it
independently by type — so routing it through the operator machinery would duplicate
`find_drop_overloads`'s registry for no gain.

**D2. The module-scoped visibility primitive is built once, consumed twice.** Rather than a
bespoke scoping mechanism for `drop` and a separate patch for the operator gap, build "is
name X visible to module M" against `selective_by_module` plus each module's own local
declarations, and have both D1's gate and the operator fix consult it.

**D3. Destructuring a type with `has_drop_overload = true` is a located error, full stop —
not scoped to "what's left over."** Matches Rust's E0509 shape: reject any
`{Name}>`/`{Name}>{field}` call whose struct has an override, regardless of whether the
extracted field is itself linear or how many fields remain. `File`-shaped composites (no
override) are unaffected; `Fd`-shaped leaf wrappers (the only place an override lives, by the
leaf-handle convention) are exactly where this fires.

## Open questions for the spec

1. **Exact insertion point and diagnostic wording for D1's gate** — inside `check_shuffle`'s
   `"drop"` arm directly, or a small helper called from it; the message should name the
   missing word the same way an operator's rule-3 error would, once D2's primitive exists to
   ask it.
2. **Exact shape of D2's visibility primitive** — a function taking `(module_id, name) ->
   &[Overload]` filtering `env`'s existing per-name `Vec<Overload>` by which module each
   candidate belongs to (requires each `Overload` to carry or derive its owning module, not
   currently tracked), versus building a separate per-module env at `check::check`'s top.
   Pick whichever fits `env`'s existing shape with the least churn across its ~21 call sites.
3. **Whether D3's rejection is by name (deny calling the generated accessor at all) or a
   dedicated diagnostic at the call site** — the former is cheaper but produces a generic
   "unresolved word" error; CLAUDE.md's "diagnostics are behaviour" convention argues for the
   latter.
4. **REPL retention's actual behaviour**, not assumed: does a REPL session redefining a
   `drop` override, or importing one, interact with D1's new gate or epoch-suffixed symbols
   in a way the native build path doesn't hit?
5. **The operator fix's own mechanism choice** (mangle operator decls and give dispatch a
   module-aware lookup, vs. filter existing bare-call candidates by caller module at
   `check_operator`) is still open and unaffected by anything here; D2 answers "what
   visibility check to run," not "how operator decls are spelled."

## Out of scope

- The disposal-word-naming apparatus (`disposal:` clause, by-name import of a *named* word,
  program-wide uniqueness of that name) — abandoned.
- Whether *derived* disposal can thread an allocator to a nested resource field — open,
  Phase 6's question (ROADMAP), not this slice's.
- `Vec`, growable containers, plural allocators — Phase 6.
- Lifting the linear-array-element restriction.
- General module-scoped visibility for every name — every non-operator name is already
  module-unique by mangling; D2's primitive is scoped to the ~20 operator names plus `drop`,
  not a rewrite of `env`'s key type.

## Exit

- A module can dispose an imported resource type's value with a bare `drop` only if that
  type's override is visible to it (imported by name, or declared locally); disposing it
  without that visibility is a located error naming the word, while importing the type,
  holding it, forwarding it, and `&`-reading it all still compile.
- A plain struct with no override anywhere disposes structurally with no gate, no
  declaration, no import — unaffected.
- `tests/phase4_modules.rs:384` (`imported_linear_type_is_disposed_by_drop`) inverts;
  ROADMAP's slice 5a Criterion 17 is recorded superseded; `DESIGN.md:547`'s "disposal crosses
  the export boundary for free" paragraph is amended.
- Destructuring a struct with an override (`{Name}>`/`{Name}>{field}`) is a located error
  naming the remedy.
- A module's own operator overload is reachable from its own module in a ≥2-module build; a
  selectively imported operator no longer hijacks unrelated bare uses of that name in the
  importer; single-module corpus unchanged.
- `examples/resources.sth` (leaf-handle shape) and `tests/phase3_resources.rs` are unaffected.
