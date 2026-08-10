# Phase 4 Slice 8b: declared disposal words + module-scoped operator dispatch (spec)

Two halves joined by one question: *which* name-resolution path a call goes through.
The disposal half is the one ROADMAP names (polymorphic `drop`, the disposal constraint,
the declared disposal word); the operator half is the module-scoped `env` gap 8a measured
and left open, folded in here. Both ride 8a's overload table and its six rules
(`docs/phase4-slice8a-spec.md`); this spec must stay consistent with what 8a actually
built, not with 8a's original plan (8a's own phase notes record where the plan and the
delivery diverged, e.g. the `lower_call` read that was wired late).

Anchors below were most recently re-verified against the built tree at `ceb2507`; the
brief's `file:line` citations were re-checked, not trusted.

---

## Settled premise (do not reopen)

The brief's section **"Decisions taken on this brief (the magicless reading)"** (D1/D2/D3)
is settled by the language's author, against the brief's own recon recommendation, on
philosophical grounds: **Sooth has little to no implicit behaviour, and disposal is not
exempt; `drop` is explicitly defined and explicitly imported.** This spec starts from
D1/D2/D3 as premise and does not re-litigate them or offer the alternative (keep the
import-scoped structural fallback). Restated so the requirements below have a fixed target:

- **D1 — disposal rides the 8a table, import-scoped, and the structural fallback is
  deleted where it would run user code.** A type's disposal word must be in scope at the
  site that disposes it; its absence is a located error naming the word to import, never a
  silent structural pop that runs a user destructor the disposing module never named.
- **D2 — derived disposal survives only where it emits no user-declared word.** A plain
  data struct (`type: Point x i64 y i64 ;`) still disposes structurally with no declaration
  and no import: its "destructor" is *nothing at all* (no code, no behaviour, no magic). The
  moment generated traversal would *call* a user's disposal word, that outer disposal
  requires the inner type's word in scope, reported at the outer disposal site. Traversal
  glue is still generated (nobody hand-writes field-walking), but nothing *runs* that the
  disposing module did not name.
- **D3 — the container boundary resolves against implicit threading.** Generated traversal
  may call an inner disposal word (D2 makes it visible) but may not *conjure inputs*: once
  allocators are explicit (Phase 6), a disposal word requiring an allocator
  (`free ( &!'A ^T -- )`) can only be called where that allocator is in scope. Ships here as
  a **stated rule with a struct-field witness**, not as a `Vec[File]` test (recon 2.3: no
  container of resources is constructible yet).

Also inherited from ROADMAP and not reopened: a structurally-total generic `drop` is off the
menu (accepting a resource type structurally discharges the linear obligation while leaking
the resource); program-wide uniqueness of a type's disposal word stays program-wide; Phase 3
slice 8b's shipped machinery (overrides, REPL retention, epoch-suffixed destructor symbols,
`examples/resources.sth`) is *reinterpreted* as "a declared disposal word that happens to be
named `drop`", not extended; and there is **no partial guard** for the destructuring hole and
no visibility-rule workaround for it (DESIGN.md declined that when it made exported types
transparent).

---

## Codebase map (verified anchors at `ceb2507`)

Disposal, as read rather than inferred — nothing on this path can ask what a module imported,
which is why disposal is whole-program and type-directed today (brief recon 1, A–D):

- `check_shuffle`'s `"drop"` arm — `src/check.rs:10389` (a second check-side `"drop"` arm
  lives at `src/check.rs:5252`; both intercept before any env lookup).
- `MirBuilder::lower_call`'s `"drop"` arm — `src/ir.rs:3573`, calling
- `MirBuilder::emit_drop` — `src/ir.rs:4779`, the universal disposal primitive; dispatches
  off the value's `IrType` and the layout's `is_linear`/`drop_generation`.
- `find_drop_overloads` — `src/check.rs:1736`, the override registry keyed by `StructId`
  (never by name); a second override for one struct is a located error here.
- `drop_overload_struct_id` — `src/check.rs:1763`, validates a `: drop` word's shape
  (one input, zero outputs) and returns the struct id; matches **only** `Type::Struct`
  (`src/check.rs:1770`), so enums/cells/arrays/scalars are rejected
  (`drop_overload_non_struct_input_error`).
- `StructDecl::has_drop_overload` — set at `src/check.rs:2051`, read at
  `src/check.rs:493`, `:6580`–`6581` (`cannot_copy_error`'s `defines_drop`), and in `src/ir.rs`
  (`has_drop_overload || any linear field`); the boolean that D1's resolution replaces with a
  *named* word. `StructLayout::has_drop_overload` (`src/ir.rs:279`) is a separate, IR-side copy
  of the same fact, populated at `src/ir.rs:859`–`867` for lowering to read.
- Recognized `drop` overloads are excluded from the concrete env — the exclusion set
  (`drop_overload_indices`) is built at `src/check.rs:2045` and applied at `src/check.rs:2132`
  (env declared at `src/check.rs:2068` as `HashMap<String, Vec<Overload>>`).
- `resolve::mangle` — `fn mangle` at `src/resolve.rs:31`, the `main`/`drop` exemption check at
  `src/resolve.rs:32`–`34` — exempts `main` and `drop` from `name__m{module}` mangling;
  everything else is mangled in the sibling loops at `src/resolve.rs:338`–`349` (structs,
  enums, words, externs each get their own loop; `:344` is the words loop specifically).

Sibling hole (destructuring bypasses the override): the consuming `{Struct}>`/`{Struct}>{field}`
field-extraction discharges the linear obligation without invoking the disposal word (exact
surface verified in R6). The recursion check already names `S>` as the sanctioned decomposition
inside a `drop` body (`src/check.rs:4362`), and the linear-field peek diagnostic points at
`S>` too (`src/check.rs:9242`); `S|>`/`^|>` are the non-consuming peeks (`src/check.rs:9242`,
`:9268`).

Operator half:

- `check_operator` — `src/check.rs:8935` — 8a's operand-class coercion + diagnostic fallback.
- `is_operator_dispatch_name` — `src/resolve.rs:43` — the ~20 names whose call sites are left
  bare for operand-type dispatch while their declarations are mangled.
- The own-module rewrite branch **has** the `is_operator_dispatch_name` skip —
  `src/resolve.rs:239`–`241` (`!is_operator_dispatch_name(core)`); the selective-import
  branch at `src/resolve.rs:262` **does not**, which is the hijack bug.

Corpus / goldens / design that this slice touches:

- `examples/resources.sth` — declares `: drop ( File -- )` and disposes with bare `drop`
  (single file; must stay byte-for-byte).
- `tests/phase3_resources.rs` — single-file drop-override goldens (must stay unchanged).
- `tests/phase4_modules.rs:384` — `imported_linear_type_is_disposed_by_drop` (slice 5a
  Criterion 17): the multi-module program D1 reverses; **inverted, not deleted** (R9).
- `DESIGN.md:547` — the slice-5a paragraph "Disposal crosses the export boundary for free…"
  and its "a destructor runs without being named" sentence: the design being reversed;
  amended in this slice (R9).

---

## Requirements

The five **"Open questions for the spec"** in the brief are answered by R1, R2, R5, R6, and
R8 below, each with a rationale. The remaining requirements (R3, R4, R7, R9–R11) carry
D1/D2/D3 and the corpus contract into checkable rules.

### R1 — The disposal-word declaration syntax (answers open question 2)

A type declares its disposal word with a trailing clause in the `type:` body, introduced by
the reserved marker `disposal:`:

```sooth
type: File fd i64 disposal: close ;
: close ( File -- ) | f | f File>fd close-fd ;
```

- The clause is optional, appears at most once per `type:`, follows all field declarations,
  and precedes the closing `;`. Parsed in `parse_typedef` (`src/parser.rs:1652`): while
  reading fields, the word `disposal:` switches to reading exactly one word (the disposal
  word's name), then `;` is required. `disposal:` becomes a reserved word that cannot be a
  field name (same class as `type:`/`:` already rejected by `expect_field_type_token`,
  `src/parser.rs:1701`).
- The named word must be defined as `: close ( File -- ) ;` — exactly one input equal to the
  declaring type, zero outputs. This is `drop_overload_struct_id`'s existing shape rule
  (`src/check.rs:1763`), generalized to key off *the declared name* instead of the literal
  `"drop"`. A `disposal: close` naming a word that is absent, or whose shape is wrong, is a
  located error at the `type:` declaration naming the word and the shape it must have.
- **Resolution is by unmangled name and input type, program-wide — never by a mangled name
  derived from the declaring type's module.** `disposal: close` records the pair (`File`,
  `"close"`); the checker's table-building pass then finds the word declaration, anywhere in
  the build closure, whose *unmangled* name is `"close"` and whose single input type is
  `File`, exactly as `find_drop_overloads` already does today (`src/check.rs:1736`, no
  same-module check). This is what makes R7's orphan overrides work at all: `close` can be
  mangled to `close__m3` while `File` is declared in module 1, and the association still
  resolves, because it was never keyed by module in the first place. Mangling (R11) changes
  what a *call site* resolves `close` to when it is in scope — it does not change how this
  type-to-word association is built.
  **Scoped, not general:** this shape rule is exactly what this slice needs; R10's `free
  ( &!'A ^T -- )` example has two inputs and is Phase 6's concern (explicit allocators) —
  generalizing the shape rule to admit an allocator input is not attempted here.

**Back-compat — the implicit `: drop` path is retained.** A `: drop ( T -- )` word with no
`disposal:` clause on `T` continues to declare `T`'s disposal word (the "declared disposal
word that happens to be named `drop`" reading, which ROADMAP marks load-bearing). This is
what keeps `examples/resources.sth` and `tests/phase3_resources.rs` byte-for-byte. Declaring
both `disposal: close` and a `: drop ( T -- )` for one `T` is a located error (two disposal
words for one type; program-wide uniqueness, R7).

**What a non-`drop` disposal name does to `drop` at a call site: a located error naming the
real word — not an alias.** `drop` applied to a value whose type declares a disposal word
named `close` is a located error at the `drop` site:
``error: `File` is disposed by `close`, not `drop` (line N)``. `drop` is *not* aliased to
`close`.

*Rationale.* Aliasing `drop`→`close` would reintroduce exactly the magic the slice removes
("a destructor runs without being named"): the reader sees `drop` and a `close` fires. Under
D1 the disposal word must be named at the site. Naming the disposal word on the type (rather
than only inferring it from a word named `drop`) is the mechanism ROADMAP requires so the
checker has a *named word to require rather than a boolean* — `has_drop_overload`
(`src/check.rs:493`) cannot distinguish a plain struct from a resource whose disposal word is
not called `drop`; both leave the bit `false`, and the second must reject structural disposal.

### R2 — The declaration is struct-only; enum/cell/array/scalar user disposal is recorded, not shipped (answers open question 1)

`drop_overload_struct_id` keeps its `Type::Struct`-only restriction (`src/check.rs:1770`):
a **user-declared** disposal word may be declared only for a `type:`-declared struct. A
`: drop ( E -- )` / `: close ( E -- )` on any non-struct stays the existing located error
(`drop_overload_non_struct_input_error`, `src/check.rs:1800`), reworded to name the declared
word rather than the literal `drop`.

**A `disposal:` clause on an enum is a parser-level rejection, not `drop_overload_struct_id`'s
error.** Enums parse through `parse_enum_typedef` (`src/parser.rs:1786`, selected by
`current_typedef_is_enum` at `src/parser.rs:1776`), a different production from `parse_typedef`
(`src/parser.rs:1652`) where R1 adds `disposal:` clause-reading. `parse_enum_typedef` must
also recognize the `disposal:` marker and reject it with a located error naming the enum and
pointing at the clause — the same class of rejection as R1's absent-field-name check
(`expect_field_type_token`) — rather than falling through to a generic "unexpected token"
parse error, which would name neither the enum nor the reason. This is one small addition to
the enum production (recognize-then-reject), not new clause semantics for enums. Message
shape, in the same "parse error: ..." voice as `expect_field_type_token`'s existing messages:
``parse error: enum `Opt` cannot declare `disposal:` at line N, col C (declared disposal
words are struct-only)``.

*Rationale, with the reachability analysis so it is not rediscovered.* Recon 2.2 frames this
correctly: `drop` already disposes **any** `'T` at a call site correctly; the only thing
missing beyond structs is a *user-declared* destructor for a non-struct type, which is a
feature, not a soundness hole. The hole would be a resource that structural disposal leaks:

- An enum carrying a resource carries it as a resource *struct* payload
  (`type: Opt | Some File | None ;`), because Sooth tracks resources only through declared
  disposal (a raw `i64` fd is not linear and never was). Structural disposal of `Opt` is
  generated traversal that **calls `File`'s disposal word** — covered by D2/R4 (the word must
  be in scope at `Opt`'s disposal site), and extracting the `File` out of `Opt` by matching
  hands you a still-linear `File` whose obligation survives. No leak.
- The only way to leak through an enum is to destructure the inner `File` down to its `i64`
  fd — which is the destructure hole R6 closes, at the `File`, independent of the enum.

So the resource-safety property is closed by D2 (R4) plus the destructure rule (R6) without
the enum itself carrying a *user* disposal word. Extending `drop_overload_struct_id` to enums
buys only *custom* enum disposal, which no corpus program needs and which ROADMAP's entry
never asks for; adding a second `drop_overload_*_id` path and enum-shaped linearity
registration for that is surface without payoff this slice (CLAUDE.md growth rule: split under
pressure, not preemptively). Recorded for a later slice: user-declared disposal words for
enums/cells, if a real consumer appears. Arrays are out anyway (linear array elements are
still rejected — a separate slice); scalars own nothing.

### R3 — Disposal rides the 8a table under rule 3, import-scoped (carries D1)

`drop`'s hardcoded interception (`check_shuffle` `src/check.rs:10389`/`:5252`, `lower_call`
`src/ir.rs:3573`) and the `StructId`-keyed registry (`find_drop_overloads`,
`src/check.rs:1736`) are retired into the 8a overload table (R11), so a call that disposes a
value is resolved the way `+` is: against candidates visible to the calling module.

- **A value whose type declares a disposal word `W`, disposed without `W` in scope at the
  disposal site, is a located error naming `W` to import** — 8a rule 3 applied to disposal.
  This is the observable D1 behaviour. "Disposed" means `drop` on it, the named word on it,
  or disposing a container that holds it (R4). Message shape:
  ``error: `File` is disposed by `close` (line N), which is not in scope; import it``.
- **Destructuring is never this error — R6 preempts it unconditionally.** R6's guard fires on
  *any* destructure of a declared-disposal type regardless of whether its word is in scope,
  so a destructure never reaches R3's scope check: R6 is the only diagnostic a destructure can
  produce.
- **Importing the type, holding it, forwarding it, storing it, and `&`-reading it all still
  compile.** The error fires only at a disposal site, never at import; a module that only
  forwards a resource never needs its disposal word in scope. (This is the shape D1 is careful
  to preserve.)
- **A plain data struct still disposes structurally with no declaration and no import**
  (D2): its derived destructor emits no user word, so there is nothing to require in scope.

The disposal-site check runs in the checker (single source of truth, mirroring 8a R7);
`emit_drop` (`src/ir.rs:4779`) keeps dispatching on concrete `IrType` at lowering and never
re-runs resolution.

### R4 — Nested traversal requires the inner disposal word at the outer site (carries D2)

Disposing a struct that (transitively) owns a declared-disposal type requires that inner
type's disposal word in scope, reported at the **outer** disposal site. The traversal glue is
still generated (`src/ir.rs`: a struct is linear when `has_drop_overload || any field is
linear`), but the checker requires every user disposal word that generated traversal would
call to be visible at the site that triggers the traversal.

- `type: Wrapper f File n i64 ;` then `w drop` where `File` declares `close`: located error at
  the `w drop` site naming `close`, unless `close`/`File` is in scope. Message shape:
  ``error: disposing `Wrapper` (line N) requires `close` in scope: it owns a `File`, which is
  disposed by `close``` (mirrors R1's "disposed by" phrasing).
- A plain data struct (all-`Copy`, or nesting only plain data structs) requires nothing.
- **References are not traversed.** A field behind a reference (`&File`) imposes no inner-word
  requirement on the outer disposal site: the outer struct does not own what the reference
  points at, so disposing the outer value never disposes through the reference, and there is
  nothing generated traversal would call there.

*Rationale.* This is the line between "derived" and "implicit" (D2). The stricter alternative
(no generated traversal at all for a type owning a declared-disposal type, forcing every
wrapper to hand-write disposal) was considered and rejected in the premise: the tax grows with
nesting depth and buys no additional visibility, since the required import already makes the
call visible in the source.

### R5 — A disposal word is imported by name, never carried by its type (answers open question 5)

Under D1 the `resolve::mangle`/env exemptions for `drop` (`src/resolve.rs:31`–`34`,
`src/check.rs:2045`/`:2132`) die. The question is what `import: lib | drop |` means when several
modules each export a `drop` overload for their own type. **Answer: the importer names the
word — the disposal word does not travel with its type.** Importing `File` and never writing
`close` anywhere would leave the destructor running at a distance the moment the type is
imported, which is the magic D1 removes: the flagship case (recon 1.A) would keep behaving
exactly as it does today, with nothing observably changed.

- **A declared disposal word is an ordinary exported word, imported by name.** Qualified
  (`import: lib "lib.sth"` → `lib::close`) or selective (`import: lib | File close |`). It is
  *not* part of the type's name-scope: importing `File` alone does **not** bring `close`, and a
  disposal site that has the type but not the word is R3's located error. This is the one place
  a disposal word differs from a type's generated constructor/getters, and deliberately so:
  those are generated words with no body, while a disposal word runs code the author wrote,
  which is exactly the derived-vs-implicit line D2 draws.
- **`drop` is importable by name when it is a type's declared disposal word.**
  `import: lib | R make drop |` is legal and brings `lib`'s `drop ( R -- )` as one more
  candidate. The universal structural `drop` remains always available as a builtin row (it is
  not a name any module owns), so disposing a plain data struct still needs no import (D2).
- **Several modules exporting a `drop` overload never collide.** Candidates key by
  `(module, name, input_types)` under 8a R1 with *distinct* input types (`R` vs `S`), so no
  shadowing (8a R1) fires; all disposal words are arity 1, so 8a R4 (one arity per name in
  scope) is satisfied; dispatch selects by operand type, as for any overloaded name.
- **This dissolves the R5/R7 orphan tension rather than papering over it.** Because the word is
  imported from wherever it is declared, an orphan disposal word (declared in a module other
  than the type's, R7) obeys the identical rule and R3's diagnostic names a word and its
  module, never "import the type". One rule covers both shapes.
- **Precedence between the builtin structural `drop` row, an own-module `: drop`, and an
  imported disposal word is moot.** R7's program-wide uniqueness means a given concrete type
  has at most one disposal word in the whole program — there is never more than one real
  candidate for that type, only the question of whether it (or the plain-data builtin row,
  D2) is in scope at the site.

**Struck: an earlier draft of this spec added "R5a", rejecting `export:` of a
declared-disposal type that does not also export its disposal word. The author declined it —
not implied by D1/D2/D3, and out of scope for this slice.**

### R6 — Destructuring a declared-disposal type is a located error (answers open question 3)

Destructuring the value of a type that has a declared disposal word — the consuming
`{Struct}>` (full destructure, all fields) and `{Struct}>{field}` (single-field getter) forms
— is a located error (Rust-E0509-shaped), **unconditional on the type**, not "only when a
linear field would be left unaccounted".

**The exhaustive surface: two of `struct_generated_sigs`'s four generated families consume,
and nothing outside that table consumes either.** `struct_generated_sigs` (`src/check.rs:3437`)
generates, per struct, exactly four word families: the constructor, the bare `{Struct}>`
full-destructure (pops the struct, pushes every field), `{Struct}>{field}` per field (pops the
struct, pushes one field, silently dropping the rest — the exact shape recon 2.1 exploited),
and `{Struct}<{field}` (a struct-to-struct setter: takes the struct plus one field's
replacement value, returns the *same aggregate type* — nothing is decomposed, so it is not
part of this guard's surface). Only the destructure and the per-field getter consume the
struct operand; the constructor takes no struct input and the setter returns one, so this
guard's surface is those two forms, from that table.

**The non-consuming peek `{Struct}|>{field}` (R10) bypasses `struct_generated_sigs` entirely
by construction, not because it was pruned from it.** `check_struct_peek_word`
(`src/check.rs:10252`) resolves it by splitting the call name on `|>` and looking it up
against the struct registry directly — it is never registered as a `Sig` in that table at
all. It stays outside this guard's surface because of what it actually does, verified
independently of the table: its signature is `( S -- S field )`, and its implementation never
pops the struct operand off the stack, only pushes the field's value alongside it, so the
aggregate stays live and its linear obligation is untouched — there is nothing for this guard
to reject. Cell unwrap (`^>`/`^|>`) is likewise unaffected — cells can never wrap a
declared-disposal type as *their own* type (R2 restricts declared disposal to `Type::Struct`),
and unwrapping a cell moves its payload out whole, decomposing nothing. Enums are also out of
scope for this guard: matching an enum hands you a still-linear payload (R2), it never
decomposes a struct down to raw fields the way `{Struct}>`/`{Struct}>{field}` do.

- **Boundary.** `&`-reading the value and the non-consuming peek `{Struct}|>{field}`
  (`check_struct_peek_word`, `src/check.rs:10252`) still compile: they do not discharge the
  obligation. Peeking a field that is itself linear is already rejected on unrelated grounds
  (`peek_of_linear_field_error`, `src/check.rs:9242`) — a disposal-type test of this boundary
  must peek a `Copy` field, or it fails for that reason instead of the one being tested. (`^|>`,
  `src/check.rs:9268`, is the cell peek, irrelevant here per above.)
- **Remedy named by the diagnostic:** call the type's declared disposal word (`close`/`drop`),
  not `{Struct}>`/`{Struct}>{field}`. Message shape, at the destructure site:
  ``error: cannot destructure `File`, which declares the disposal word `close` (line N)``
  `` destructuring discharges the linear obligation without running `close`; dispose it with
  `close` ``.
- **Exception — the type's own disposal word body.** Within the body of `T`'s declared
  disposal word, destructuring the receiver is permitted: it is the sanctioned means of
  reaching fields for disposal, and the recursion check already points a `drop` body at `S>`
  as the remedy (`src/check.rs:4362`). Without this exception `examples/resources.sth`'s
  `: drop ( File -- ) | f | f File>fd close-fd ;` would not compile (R9).

*Rationale for unconditional.* A declared disposal word may do arbitrary work keyed off a
*non-linear* field (close an fd stored as an `i64`), so "account for every linear field" is
not equivalent to disposal — the whole point of the declared word is that structural
decomposition is not disposal. Unconditional also matches E0509 and is simpler to specify and
test. This closes the hole recon 2.1 measured (`7 R | r | r R>tag .` printing the field and
never running the destructor), which slice 5a's transparent-type export made reachable across
a file boundary for the first time.

### R7 — Program-wide uniqueness stays program-wide; reachability is import-scoped (state both scopes explicitly)

Two different scopes, and the spec states each rather than saying "scoped":

- **Uniqueness of a type's disposal word is program-wide.** `find_drop_overloads`'
  program-wide uniqueness (`src/check.rs:1736`) survives the migration to table rows:
  scope-local uniqueness alone would let two modules declare disposal for one `File`, never
  collide, and dispose the same value two different ways. A second declared disposal word for
  one type, anywhere in the closure, is a located error: the existing
  `duplicate_drop_overload_error`, generalized off the name so it names the declared word
  instead of the literal `drop` — today it reads ``error: `File` already defines its own
  `drop` (line N, col C)``; generalized, it reads ``error: `File` already defines its own
  `close` (line N, col C)`` when `close` is the type's declared word.
- **Reachability (callability) of a disposal word is import-scoped** (R3/R5): the word must be
  in scope at the disposal site.
- **Open sub-question, decided: orphan overrides stay legal.** Nothing today requires a
  disposal word to live in the module declaring the type (`drop_overload_struct_id` derives
  the id from the input type, no same-module check); this slice does not add that restriction.
  Uniqueness is the load-bearing safety property; restricting to the declaring module is
  optional and not taken (it would break the legitimate orphan-override shape recon 1.B/1.C
  measured, for no safety gain given program-wide uniqueness). Under R5's by-name import this
  needs no special case: an orphan word is imported from its own module exactly like any
  other, so uniqueness stays program-wide while reachability stays per-name.

### R8 — The operator fix: module-aware operator dispatch (answers open question 4)

Take the first option in the brief: fix operator dispatch's `resolve.rs`/`check.rs` gaps so it
becomes module-aware, rather than leaving decls bare and filtering by caller module in
`check.rs` only. Operator *declarations* are already mangled like every other name — `mangle`
(`src/resolve.rs:31`) exempts only `main` and `drop`, so the per-kind decl-mangling loops
(`src/resolve.rs:338`–`349`) already rename a `: + ( Vec2 Vec2 -- Vec2 )` declaration the same
as any other word; there is no mangling change to make here. What is missing is the
`is_operator_dispatch_name` skip on the selective-import call-site rewrite, and a
module-aware lookup at the call site itself:

- **`resolve.rs`:** add the `is_operator_dispatch_name` skip to the selective-import branch at
  `src/resolve.rs:262`, mirroring the own-module branch at `src/resolve.rs:239`–`241`, so a
  selectively imported operator no longer rewrites *every* bare use of that name in the
  importing module (the hijack: `1 2 +` failing with "`+` expected `Vec2`, found `i64`"). A
  selectively imported operator becomes a *candidate* visible to the importing module, not a
  static rewrite.
- **`check.rs`:** key operator candidates by `(module, name)` and thread the **caller's**
  module id into `check_operator` (`src/check.rs:8935`) so a bare operator call site resolves
  against candidates owned by the caller's module plus candidates imported into it, selected
  by operand type. This is what fixes the own-module-unreachable regression (recon 3: a
  module's own `: + ( Vec2 Vec2 -- Vec2 )` unreachable from its own module the moment a second
  module joins the closure). `env`'s shape (`HashMap<String, Vec<Overload>>`,
  `src/check.rs:2068`) is **not** widened across its 21 signatures; every non-operator name is
  already module-unique by mangling, so only operator dispatch gains the module key.

Target semantics (already correct for the qualified form `v::+`, brief recon 3): bare names
resolve the way qualified ones already do — per call site, by operand type, against candidates
visible to the calling module. 8a's R1/R4/R5 collision/arity/overlap checks are already
`(module, name)`-keyed, so this makes operators consistent with the scoping the rest of 8a
already uses, rather than special-casing them.

*Rationale for option one over "check.rs-only, filter by caller module".* The check.rs-only
option leaves two modules' `+` overloads sharing one `env` key, which is precisely the
collision that makes own-module reachability fail in a ≥2-module build; it cannot distinguish
module A's `Vec2 +` from module B's. Adding the `is_operator_dispatch_name` skip to the
selective-import branch plus a module-aware `check_operator` lookup is a bounded change that
reuses the module id `resolve::mangle` already computes for every declaration, and it removes
the "bare call site, mangled definition" mismatch at its root instead of layering a filter on
top of it.

### R9 — Single-file corpus byte-for-byte; the multi-module golden inverts; DESIGN.md amended

- **`examples/resources.sth` and `tests/phase3_resources.rs` are unchanged** (single-file,
  disposal word named `drop`, disposed in the same module: R6's own-body exception and R3's
  same-module scope keep them green). Capture/keep their goldens.
- **`tests/phase4_modules.rs:384` (`imported_linear_type_is_disposed_by_drop`) is inverted,
  not deleted.** Today it asserts the destructor runs across a module boundary with the word
  out of the importer's scope. Under D1 that program
  (`import: lib "lib.sth" ; : main ( -- ) lib::mk drop ;`, with `drop` declared in `lib` and
  not brought into `main`'s scope) is a located error at the `drop` site naming `Res`'s
  disposal word to import. Rewrite the test to assert that error, rename it to reflect the
  inverted behaviour (e.g. `imported_linear_type_disposed_without_its_word_in_scope_is_error`),
  and add a positive companion asserting that importing **the disposal word by name** (R5 —
  not the type; importing the type alone is exactly what the negative case shows staying
  rejected) makes the same disposal compile and run.
- **Record slice 5a Criterion 17 as superseded in `ROADMAP.md`'s slice 5a entry** — a slice-5a
  exit criterion reversed by a later slice must not happen silently.
- **Amend `DESIGN.md:547`** — the paragraph "Disposal crosses the export boundary for free, so
  this slice adds no new disposal rule", including "a destructor runs without being named" and
  "the ROADMAP's hypothesized 'an exported linear type must also export its discharging word'
  rule has nothing to fire on yet." Rewrite it to record that 8b reverses this: disposal is
  import-scoped, the disposal word must be in scope at the disposal site, and a destructor
  never runs in a module that did not name its disposal word (R5) — the destructor becomes
  callable only when the disposing module imports that word itself, never merely by holding or
  importing a value of the type. A stale DESIGN.md is what put the two documents in conflict;
  amending it is in scope.

### R10 — The container boundary ships as a stated rule with a struct-field witness (carries D3)

Record D3 as a stated rule and exercise it with the only container of a resource that is
constructible today — a struct with a linear field — not a `Vec[File]` test (recon 2.3:
linear array elements are rejected, `Vec` is Phase 6, so the `Vec[File]` exit criterion is
unwritable, the same shape as the 8a exit criterion amended mid-implementation). The rule:
generated traversal may *call* an inner disposal word (D2/R4, exercised by the `Wrapper f File`
witness) but may not *conjure inputs* a disposal word requires; a disposal word needing an
allocator (`free ( &!'A ^T -- )`) can only be called where that allocator is in scope, so a
container whose elements need one is disposed where the inputs are, not by generated glue that
invents them. Phase 6's allocator rework (its *Generic struct declarations* item and Slice 2
shim-to-FFI rework) is the logged consumer waiting on this answer; ship the rule, not the
mechanism.

### R11 — Retire the bespoke registry into 8a table rows; reinterpret shipped machinery

- The `StructId`-keyed override registry (`find_drop_overloads`, `src/check.rs:1736`), its
  `check_duplicate_word_names` exemption (retained through 8a specifically to die here), the
  `resolve::mangle` `drop` exemption (`fn mangle`/exemption check, `src/resolve.rs:31`–`34`),
  and the env exclusion (`drop_overload_indices`, built `src/check.rs:2045`, applied
  `src/check.rs:2132`) are all removed; disposal words become ordinary rows in 8a's table,
  keyed by `(module, name, input_types)`, with the `StructId → declared-word` association
  living where the table's candidate resolution can read it. ROADMAP's stated goal of retiring
  the bespoke registry is met by this migration.
- `StructDecl::has_drop_overload` (the boolean, `src/check.rs:493`/`:2051`/`:6580`–`6581`, with
  the IR-side copy `StructLayout::has_drop_overload` at `src/ir.rs:279`) is superseded by "does
  this struct have a declared disposal word" carrying the *word*, not a bit — the checker
  requires the named word (R1/R3) and lowering still reads linearity for traversal
  (`has_drop_overload || any field is linear`) off the presence of a declared word.
- **Non-struct disposal must not regress.** `check_shuffle`/`lower_call`'s hardcoded `"drop"`
  arms today dispatch `drop` on *any* `'T` — scalars, cells, plain enums, not just structs —
  via `emit_drop`'s `IrType`-based dispatch (R2's whole safety argument rests on this already
  being true). Retiring those arms into table rows must not narrow that: the universal
  structural `drop` survives as an always-visible builtin table row (R5), so `drop` on a
  scalar/cell/plain-enum with no declared disposal word keeps resolving and lowering exactly
  as it does today, with no import required (D2).
- Phase 3 slice 8b's overrides, REPL retention, epoch-suffixed destructor symbols, and
  `examples/resources.sth` all read unchanged as "a declared disposal word that happens to be
  named `drop`" (R1 back-compat). This reinterpretation is load-bearing and is stated, not
  discovered. The REPL must adopt the table shape without regressing its goldens (8a's warning:
  a mis-threaded overload record segfaulted the session rather than merely failing to
  dispatch; keep the resolution record threaded through the REPL lowering path).

---

## Exit criteria

1. A type declares its disposal word (`disposal:` clause, or the back-compat `: drop`), and
   the checker requires *that word* rather than `has_drop_overload`'s boolean (R1, R11).
2. The `disposal:` clause is struct-only: a non-`drop`-named declared word is rejected on a
   non-struct the same way `: drop` is today, and `parse_enum_typedef` rejects a `disposal:`
   clause on an enum with a located error naming the enum, rather than falling through to a
   generic parse error (R2).
3. `drop` on a scalar, cell, or plain enum with no declared disposal word still compiles and
   dispenses with it structurally, with no import, exactly as before the table migration (R11).
4. Disposing a value whose type declares a disposal word, without that word in scope, is a
   located error at the disposal site naming the word to import (R3/D1) — and importing the
   *type* alone does not discharge it (R5): the word is imported by name, or disposal is
   rejected. Holding, forwarding, and `&`-reading such a value all still compile, as does
   importing the type without the word.
5. A second declared disposal word for one type, anywhere in the build closure — not just
   within one module — is a located error: uniqueness is program-wide, not scope-local (R7).
6. Disposing a struct holding such a type reports the same error at the outer disposal site
   (R4/D2), and a plain data struct still disposes with no declaration and no import.
7. `drop` applied to a value whose declared disposal word is not named `drop` is a located
   error naming the real word (R1) — not an alias.
8. Destructuring a type with a declared disposal word is a located error naming the word (R6),
   with the type's own disposal-word body exempt.
9. `examples/resources.sth` and `tests/phase3_resources.rs` (single-file) are unchanged;
   `tests/phase4_modules.rs:384` is inverted with a positive companion, slice 5a's Criterion
   17 is recorded superseded in ROADMAP, and `DESIGN.md:547` is amended (R9).
10. A module's own operator overload is reachable from its own module in a ≥2-module build,
    and a selectively imported operator no longer hijacks unrelated bare uses of that name in
    the importing module — with the single-module corpus byte-for-byte unchanged (R8).
11. The container-boundary rule (D3) is recorded and exercised by the `Wrapper f File`
    struct-field witness (R10); the `Vec[File]` criterion is explicitly recorded as unwritable.

Green throughout: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.

---

## Testing

Unit tests beside stage code, `thing_condition_expected`, asserting exact message text;
mutation-check each new guard (delete what the test guards, prove it fails — Sooth has shipped
placebo tests before). Key tests:

- Syntax (R1): `typedef_disposal_clause_registers_word`,
  `disposal_clause_naming_absent_word_is_error`,
  `disposal_clause_word_wrong_shape_is_error`,
  `disposal_and_drop_for_one_type_is_error`,
  `drop_on_type_with_named_disposal_word_is_error` (names the real word).
- Struct-only (R2): `disposal_clause_on_enum_is_error`,
  `drop_overload_on_enum_is_error` (reworded message).
- Import-scoping (R3/R5): `dispose_imported_type_without_its_word_in_scope_is_error`,
  `importing_type_alone_does_not_bring_its_disposal_word` (the flagship: the type is in scope,
  the word is not, disposal is rejected), `importing_disposal_word_by_name_allows_disposal`
  (both the `close` and the back-compat `drop` spelling),
  `forward_imported_resource_without_word_compiles`, `read_imported_resource_through_ref_compiles`,
  `two_modules_drop_overloads_do_not_collide`,
  `orphan_disposal_word_imported_from_its_own_module` (R7 × R5).
- Traversal (R4/R10): `dispose_struct_holding_resource_requires_inner_word_at_outer_site`,
  `enum_holding_resource_struct_disposes_via_inner_word` (an enum's resource payload disposes
  through the inner word without the enum itself declaring one, R2's safety argument),
  `plain_data_struct_disposes_with_no_import`, `dispose_through_ref_field_requires_nothing`
  (R4's reference-boundary rule).
- Destructure (R6): `destructure_full_of_disposal_type_is_error` (bare `{Struct}>`),
  `destructure_field_getter_of_disposal_type_is_error` (`{Struct}>{field}`),
  `destructure_in_own_disposal_body_is_allowed`, `peek_and_ref_of_disposal_type_compile`,
  `struct_setter_of_disposal_type_is_not_a_destructure` (`{Struct}<{field}` stays legal, since
  nothing is decomposed).
- Uniqueness (R7): `two_disposal_words_for_one_type_program_wide_is_error` (asserts the
  existing `duplicate_drop_overload_error` message with the declared name substituted for the
  literal `drop` — no new message text to design here),
  `orphan_disposal_override_in_importing_module_is_allowed`.
- Non-regression (R11): `drop_on_plain_scalar_and_enum_unaffected_by_table_migration`.
- Operator half (R8): `own_module_operator_overload_reachable_in_two_module_build`,
  `selective_operator_import_does_not_hijack_bare_use`,
  single-module operator goldens unchanged.
- Corpus (R9/R11): the R9 baseline goldens (`examples/resources.sth`,
  `tests/phase3_resources.rs`) byte-for-byte; the inverted + companion
  `tests/phase4_modules.rs` tests; REPL goldens unchanged.

---

## Out of scope (hard boundary)

- `Vec`, growable containers, plural allocators (Phase 6); `free ( &!'A ^T -- )` informs the
  general form (R10) but ships nothing here.
- Lifting the linear-array-element restriction (its own slice).
- User-declared disposal words for enums/cells/scalars (R2: recorded, no safety payoff, no
  consumer this slice).
- `if`/`cond` as words (slice 9b, blocked on 10a) and rows in quotation effects (10a).
- General module-scoped visibility beyond bare-name operator dispatch: every other name is
  already module-unique by mangling; widening `env`'s key across 21 signatures buys nothing
  this slice needs (R8).
- A structurally-total generic `drop` (ruled out in the premise).

---

## Phases (JSON)

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Module-scoped operator dispatch: add the is_operator_dispatch_name skip to resolve.rs's selective-import branch (operator declarations are already mangled like every other name; only call-site dispatch is inconsistent), and thread the caller's module id into check_operator so operator candidates are keyed by (module, name); own-module overloads reachable in a >=2-module build and selective operator imports stop hijacking bare uses, single-module corpus byte-for-byte (R8)",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "Declared disposal word mechanism: parse the type: disposal: clause (parse_typedef, reject it in parse_enum_typedef with a located error), generalize drop_overload_struct_id off the literal name (struct-only, R2), keep the : drop back-compat reading and the non-drop-name call-site error (R1); do NOT retire find_drop_overloads or the drop mangle/env exemptions yet -- that migration is phase 3's, alongside import-scope enforcement, so that tests/phase4_modules.rs:384's still-unmodified cross-module assertion keeps passing unchanged; single-file corpus byte-for-byte, no import-scope enforcement yet; a disposal: close type is declarable but not yet disposable via close (the interception arms are still hardcoded on the literal drop until phase 3), so phase 2's tests are registration and negative cases only, no dispose-and-run e2e",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "Import-scoped disposal core: retire find_drop_overloads, the resolve::mangle drop exemption, and the env exclusion into 8a table rows keyed by (module, name, input_types) (R11), preserving drop's existing any-'T dispatch as an always-visible builtin row so scalars/cells/plain-enums are unaffected; require a value's declared disposal word in scope at every disposal site, with importing the type alone insufficient -- the word must be imported by name (R3/R5); enforce program-wide uniqueness of a type's disposal word (R7); invert tests/phase4_modules.rs:384 with a positive companion importing the word by name, record slice 5a Criterion 17 superseded in ROADMAP, and amend DESIGN.md's slice-5a disposal paragraph (R9)",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "Nested traversal and container boundary: require the inner disposal word in scope at the outer disposal site when a struct owns a declared-disposal type, riding the existing is_linear/has_drop_overload structural fold to find which inner words generated traversal would call rather than a new transitive-ownership pass (R4/D2); references are not traversed; record the D3 container-boundary rule (generated traversal may call a named disposal word but may never conjure inputs) exercised by the Wrapper f File struct-field witness, with the Vec[File] criterion recorded unwritable (R10)",
      "difficulty": "hard"
    },
    {
      "phase": 5,
      "focus": "Destructure guard: an E0509-shaped located error rejecting the exhaustive consuming-decomposition surface of a type with a declared disposal word -- struct_generated_sigs' bare `{Struct}>` full destructure and `{Struct}>{field}` per-field getter, and no other form (the setter `{Struct}<{field}` and all peeks stay legal) -- with the type's own disposal-word body exempt and the diagnostic naming the disposal word as the remedy (R6)",
      "difficulty": "standard"
    }
  ]
}
```
