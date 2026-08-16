[← ROADMAP](./ROADMAP.md)

### Phase 5 — Errors as values  `[S]`

Result/Either as an ordinary generic ADT (`type: Result 'T 'E | Ok 'T | Err 'E ;`,
`lib/result.sth`), plus the convention that fallible words return it. Branch-on-result
codegen, no unwinding. FFI/C error returns map to Result at the (later) safe-wrapper
layer. No `?` short-circuit sugar: it has no DESIGN.md mandate, proves no new
mechanism, and is pure parser sugar that can be added later against a stable Result
without touching this phase's exit criteria.
**`Option['T]` (`type: Option 'T | None | Some 'T ;`, `lib/option.sth`) ships in the
same phase**, DESIGN.md's own named answer to pointer nullability (`^T` stays
non-null; a program wanting nullability names this type rather than each redeclaring an
equivalent enum, which would make two modules' "same" option type nominally distinct).
It is also the mechanism's cheapest second consumer: one type variable against Result's
two, so it is what proves the mechanism generalizes across arity rather than being
shaped by its first client. Not scoped here: rebuilding the allocator's OOM trap
(Phase 3 Slice 2) to return `Option`/`Result` instead of trapping — a real future
consumer, but a change to already-shipped allocator behavior, not a consequence of
this phase existing.

**P5.S1 — Phase 5 Slice 1 — generic `type:` declarations (structs and enums).** `type:` parses
only concrete field types today (`parse_enum_typedef`/`parse_variant_fields`,
`src/parser.rs`), so a user-declared generic struct/enum needs a `type:` header
parameterized by Phase 4's type variables, one `StructId`/`EnumId` minted per concrete
instantiation (mirroring how a polymorphic word already monomorphizes), plus
per-instantiation generated words and destructor synthesis. `intern_bundle_struct`
already keys an interned struct per instantiation, so the layout half of the machinery
exists; the user-facing declaration, resolution, and generation half does not. Scoped
without an allocator-parameter slot: the default-allocator-parameter question
(`Vec['T 'A = Global]`) stays Phase 7's, since this slice's own exit case allocates
nothing — `Vec`/`Map` reuse the mechanism in Phase 7 rather than being its motivating
case here. This slice does not need Result or Option to exist; any throwaway generic
struct/enum proves it.
**Exit:** a generic `type:` declaration monomorphizes per instantiation the way a
polymorphic word already does.

**P5.S2 — Phase 5 Slice 2 — Result and Option.** Built on Slice 1's generic `type:` machinery:
`Result 'T 'E` and `Option 'T` as ordinary generic enums, branch-on-result codegen, no
unwinding, `Option` importable from `lib/option.sth`.
**Exit:** Result-based error handling with no `?` sugar; `Option['T]` importable from
`lib/option.sth`; no exception/unwind path exists anywhere.
