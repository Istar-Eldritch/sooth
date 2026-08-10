# Phase 4 Slice 9: `Bool` as a library enum (implemented; `if`/`cond` split out)

`Bool`/`bool` was a primitive scalar. This slice retired it into a two-variant
zero-payload enum, `type: Bool | False | True ;`. The ROADMAP framed the whole
slice — `Bool`-as-enum and `if`-as-word together — as a mechanical migration.
That was true for the `Bool` half, which is what this document now describes.
It was false for the `if` half: making `if` an ordinary word turned out to need
a row variable inside a quotation's declared effect, which does not parse
before ROADMAP slice 10a. That half is split out as **slice 10c**
(`docs/phase4-slice10c-brief.md`) — numbered into slice 10's lineage rather than
slice 9's, because it extends 10a's row mechanism and depends on nothing this
slice shipped beyond `Bool` being an enum. Not attempted here.

The original brief (`docs/phase4-slice9-brief.md`) is the discovery document and
remains the record of why the ROADMAP's framing was wrong on both halves.

## The decision this slice settles

**`Bool` gets a general zero-payload-enum → scalar-discriminant layout, not a
`Bool`-specific carve-out and not the tagged-aggregate cost every other enum
pays.** Every existing enum lowers to a fixed `i32` discriminant plus a payload
region, eliminated only through clause dispatch — never fed directly to `Jnz`.
Naively making `Bool` "just an enum" would have routed every comparison,
`and/or/xor/not`, and internal branch condition (bounds checks, loop back-edges)
through that aggregate machinery, a real regression on the single hottest
control-flow path in every program. Instead: an enum every variant of which has
an empty payload lowers to a bare scalar discriminant, register-resident, no
payload region. `Bool` is that rule's first client, not a special case bolted
onto it — `ir_type_of(Bool)` still yields the scalar QBE already emits as `w`,
so `Cmp`/`Jnz`/bitwise/internal-condition codegen is byte-for-byte unchanged.
`.`'s primitive `bool` printable row is retired in favour of a library
`: . ( Bool -- ) ;`, reached through 8a's `builtin_overloads` dispatch — the
concrete case the ROADMAP always cited as the reason to wait on 8a.

Surface spellings `true`/`false`/`bool` are retained (they now construct
`True`/`False` and spell `Type::Enum(Bool)`); the distinct primitive
`Type::Bool` is gone from the type layer.

## Two accepted deviations from strict byte-for-byte

Found and resolved during phase-1/2 review, both narrowly scoped and
behaviourally inert:

- **`list.ssa`/`refs.ssa`: `sooth_enum_drop_0` → `sooth_enum_drop_1`.** Reserving
  a fixed `BOOL_ENUM_ID = EnumId(0)` (so `Type::from_name("bool")` resolves with
  no registry access) shifts every user enum's id up by one, renumbering
  `List`'s destructor symbol. A pure rename — no instruction, block, or codegen
  shape differs, and every bool-print-free, non-enum-drop baseline
  (`gcd.ssa`/`countdown.ssa`/`bool_abi.ssa`/`shapes.ssa`/`vm.ssa`) stays
  byte-identical. The alternative (numbering enum-drop symbols to skip the
  no-drop `bool` slot) was rejected: it breaks the drop-symbol naming scheme
  struct/enum/cell already share and re-introduces exactly the kind of
  `bool`-shaped carve-out this slice exists to remove.
- **`leap.ssa`: inline `$boolstrs` index → `call $.2e.(w %vN)`.** `leap.sth`
  prints three `Bool`s, and once `.` is a library word its print call sites
  necessarily reroute to a real call. `$leap`'s own function body, every
  condition/branch, and runtime output are unchanged; this is what "ship `.` as
  a library overload" means, not a codegen regression.

`IrType::Bool`'s now-unreachable-from-source `Print` codegen arm and the
unconditionally-emitted `$boolstrs`/`$true_str`/`$false_str` QBE header are
deliberately left in place — deleting them now would itself violate the
byte-for-byte baseline above. Retiring them (and the codegen test that exercises
them, `emit_print_on_bool_indexes_boolstrs_via_sfmt`) is later cleanup, once
something actually depends on their absence.

## Implementation

Two phases, both merged (`c5db035`, a partial merge of
`impl/phase4_slice9_spec-2608100454` — only through the phase-2 tip; the
worktree's phase-3 attempt was a stub and is not part of `main`'s history):

- **Phase 1 — the layout rule and the declaration.** `16171ae`, `a94ef1f`,
  `8e98591`. The general zero-payload-enum scalar layout in `EnumLayout`/the
  registry builder (`ir.rs`), the `Bool` declaration with `False`/`True` at
  discriminants `0`/`1` (`ast.rs`), `True`/`False` replacing `BoolLit`, retained
  surface spellings, the `and/or/xor/not` `Bool` rows in `builtin_table`
  (`check.rs`), exhaustiveness over `{False, True}`. Touches `src/ast.rs`,
  `src/ir.rs`, `src/check.rs`, `src/backend/qbe.rs`.
- **Phase 2 — `.` as a library overload.** `c847af3`, `b4dcfaa`, `3922930`.
  Deletes `.`'s primitive `bool` row from `printable_types`, ships
  `: . ( Bool -- ) ;` dispatched through `Module::builtin_overloads` on both the
  native and REPL paths, verified against 8a's wiring (the exact path a prior
  8a fix cycle had to repair after a silent mis-lowering). Touches `src/check.rs`,
  `src/ir.rs`, `src/repl.rs`, `tests/phase3_strings.rs`.

Verified at the merge: `cargo fmt --check` and `cargo clippy -- -D warnings`
clean, 1,664 tests pass (0 failed), including
`corpus_qbe_stays_byte_identical_to_baseline` and
`operator_i64_lowers_identically_after_table`.

## Not implemented here

`if` as an ordinary clause-bodied word and `cond` as a fixed-arity combinator
built from it. The dispatch half is sound (clause dispatch on `Bool` is an
independent primitive, no circularity, and the guard currently blocking a
quotation-taking clause body is stale — see the brief), but the signature it
needs cannot be declared today: a row variable is legal at a word's own top
level but not inside a nested quotation's declared effect, which is ROADMAP
slice 10a's mechanism. Findings, the exact repro, and what survives unchanged
are recorded in `docs/phase4-slice10c-brief.md` — including why that brief's
original target signature (`[ ..a -- ..b ]`, two rows differing per side) was
itself wrong, and the `~[ ..i -- ..o ]` inline-only-quotation direction that
replaced it. Do not re-derive them; 10c gates on 10a phases 1–2, not on
anything in this document.
