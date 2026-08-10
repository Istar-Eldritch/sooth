# Phase 4 Slice 8b: polymorphic `drop` + module-scoped operator dispatch (brief)

Two halves, joined by one question: *which* name-resolution path a call goes through.
`drop`'s half is the one ROADMAP names (polymorphic `drop`, the disposal constraint, the
declared disposal word). The operator half is the module-scoped `env` fix 8a measured and
deliberately left open, folded in here.

The recon inverted the expected answer on the first half. ROADMAP's disposal-scope invariant
is built on a premise — "a module importing `File` without its disposal word would pop the
`i64` and never `close`" — that **does not hold against the built compiler**. Disposal today
never touches the word environment at all. So the leak that invariant defends against is not
a pre-existing hole this slice closes; it is a hole that *moving `drop` onto 8a's table would
open*, and which the declared-disposal-word mechanism then exists to close back up.

**That trade is taken deliberately, and the recon reframes rather than blocks it (D1 below).**
The measured "disposal crosses a module boundary for free" is exactly the implicit behaviour
this language is declining to have: a destructor that runs in a module that never named it is
magic, however convenient. The slice's job is therefore not to preserve the fallback but to
**delete** it — the leak only exists in a design that keeps a silent structural default *and*
import-scoped reachability, and the decisions below keep neither.

## Recon 1: disposal is already whole-program and type-directed

Four programs, compiled and run on `main` at `d58b52d`.

**A. Import the type, not its disposal word — the destructor still runs.**

```sooth
\ lib:  type: R tag i64 ;  : drop ( R -- ) | r | "closing " . r R>tag . ;
\       : make ( i64 -- R ) R ;  export: R make ;
\ main: import: lib | R make | "res_lib.sth" ;
\       : main ( -- ) 7 make | r | r drop ;
=> closing 7
```

**B. An orphan override (declared in the *importing* module, for an imported type) works.**
Same lib without the override, `: drop ( R -- )` in `main` instead => `closing 7`.

**C. Disposal executed in a module that cannot see the override works.** Override in `main`,
`: use-it ( R -- ) drop ;` in `lib`, `main` calls `use-it` => `closing 7`.

**D. Linearity is program-wide too, not per-module.** With the override in `main`, a
`: dup-it ( R -- R R ) dup ;` in `lib` is rejected: "`R` is linear because it defines
`drop`". `lib` cannot see the override and is bound by it anyway.

The mechanism, read rather than inferred: `drop` is intercepted in `check_shuffle`'s `"drop"`
arm (`check.rs:10389`) and `lower_call`'s (`ir.rs:3573`) before any env lookup; overrides live
in a registry keyed by `StructId`, never by name (`find_drop_overloads`, `check.rs:1736`), are
explicitly excluded from `env` (`check.rs:2116`), and are the one name `resolve::mangle`
refuses to touch (`resolve.rs:31`). Dispatch happens in `emit_drop` (`ir.rs:4779`) off the
value's `IrType` and the layout's `is_linear`/`drop_generation`. Nothing in that path can ask
what a module imported, which is exactly why A–D behave as they do.

**DESIGN.md described this as a feature and must be amended** (slice 5a's decision record,
"Disposal crosses the export boundary for free"): "`drop` is compiler-known and dispatches on
the concrete type… a destructor runs without being named… the ROADMAP's hypothesized 'an
exported linear type must also export its discharging word' rule has nothing to fire on yet."
That paragraph is the design this slice reverses; "a destructor runs without being named" is
the sentence that fails the magicless test. Amending it is in scope — a stale DESIGN.md is
what put these two documents in conflict in the first place.

One genuine tension the spec inherits rather than resolves away: ROADMAP keeps
`find_drop_overloads`' uniqueness program-wide ("scope-local uniqueness alone would let two
modules declare disposal for one `File` … and dispose the same value two different ways") while
making *reachability* import-scoped. Both halves survive D1, but they are different scopes and
the spec must state each rule's scope explicitly rather than saying "scoped".

## Recon 2: what is actually broken about disposal

**1. Destructuring bypasses the override.** Verified, single file:

```sooth
type: R tag i64 ;
: drop ( R -- ) | r | "dropping " . r R>tag . ;
: main ( -- ) 7 R | r | r R>tag . ;
=> 7          \ compiles clean; the destructor never runs
```

A `File` can have its fd extracted, the linear obligation discharged, no `close`, no
diagnostic. This is the real hole, it is independent of tables and scoping, and DESIGN.md
names this slice as its owner (a Rust-E0509-shaped rule).

**2. Overrides are struct-only, so user extension — not `drop` itself — is what is
non-polymorphic.** `: drop ( E -- )` on an enum is a located error ("must take a
`type:`-declared struct"), from `drop_overload_struct_id` (`check.rs:1763`), which matches
only `Type::Struct`. Cells, arrays, and scalars likewise. Meanwhile `drop` already accepts
*any* `'T` at a call site and disposes it correctly. So "polymorphic `drop`" as a headline is
close to already-shipped; the missing capability is a user-declared destructor for a non-
struct type, which ROADMAP's entry never mentions.

**3. There is no container of resources to test the container-boundary rule on.** Linear
array elements are still rejected ("linear array elements are not supported yet"), `Vec` is
Phase 6, and the only container that can hold a resource today is a struct with a linear
field — which already disposes correctly through generated traversal (`type: Holder r R n
i64 ;` then `h drop` => `closing 7`). **ROADMAP's exit criterion "disposing a `Vec[File]`
reports the same error at the container's disposal site" is therefore unwritable**, the same
shape as 8a's exit criterion that had to be amended mid-implementation. Decide the
container-boundary question on paper if it is worth deciding early; do not exit on it.

**4. "Disposal may require inputs beyond the value" has no consumer yet.** `free ( &!'A ^T
-- )` needs plural allocators, which arrive in Phase 6 (`qbe.rs` has exactly one global
allocator behind `allocate`/`free` today). Settling the *general form* here is defensible
because Phase 6's rework is the logged consumer; shipping mechanism for it here is not.

## Recon 3: the operator half is a live 8a regression, not just the logged gap

`env` is `HashMap<String, Vec<Overload>>` (`check.rs:2068`), threaded through 21 function
signatures. But it is **already module-scoped in practice**: `resolve::mangle` renames every
decl to `name__m{module}` (`resolve.rs:345`) and rewrites call sites to match. The exception
is the ~20 names in `is_operator_dispatch_name` (`resolve.rs:43`), whose *call sites* are
deliberately left bare so `check_operator`'s operand-type dispatch can run — while their
*declarations* are mangled like everything else. Bare call site, mangled definition: the two
halves no longer meet.

**New finding — an own-module operator overload is unreachable as soon as a second module
joins the closure.** Byte-identical bodies:

```sooth
type: Vec2 x i64 y i64 ;
: + ( Vec2 Vec2 -- Vec2 ) | a b | a Vec2>x b Vec2>x + a Vec2>y b Vec2>y + Vec2 ;
: add2 ( Vec2 Vec2 -- Vec2 ) + ;
```

Single file with a `main` => runs, prints `4`. The same two words in a `lib` imported by a
one-line `main` => `error:`+` requires two operands of the same numeric type, found `Vec2`
and `Vec2`` — raised inside `add2`, in the very module that declares the overload. 8a's exit
criterion ("no definition left silently unreachable") holds only below two modules.

The logged half still reproduces: selectively importing an operator (`import: v | Vec2 + |`)
rewrites *every* bare use of that name in the importing module onto the imported overload, so
an unrelated `1 2 +` fails with "`+` expected `Vec2`, found `i64`" (`resolve.rs:265`, the
selective branch, which has no `is_operator_dispatch_name` skip — unlike the own-module branch
at `resolve.rs:241`, where 8a added one).

**The qualified form already behaves correctly** and is the target semantics: `1 2 v::Vec2 3 4
v::Vec2 v::+ v::Vec2>x .` beside `1 2 + .` prints `4` then `3`. Bare names should resolve the
way qualified ones already do — per call site, by operand type, against candidates visible to
the *calling* module. That is a bounded change (thread the caller's module id into operator
dispatch and key operator candidates by `(module, name)`), not a rewrite of `env`'s shape:
every non-operator name is already module-unique by mangling.

## Inherited from ROADMAP, not reopened

- **A structurally-total generic `drop` is not on the menu.** Accepting a resource type
  structurally discharges the obligation while leaking the resource; linearity buys
  use-exactly-once, not use-correctly.
- **Program-wide uniqueness of a type's disposal word stays program-wide.**
- **Phase 3 slice 8b's shipped machinery is reinterpreted, not extended**: overrides, REPL
  retention, epoch-suffixed destructor symbols and `examples/resources.sth` all read as
  "declared disposal word that happens to be named `drop`". That reading is load-bearing.
- **No partial guard for the destructuring hole**, and no visibility-rule workaround for it
  (DESIGN.md declined that explicitly when making exported types transparent).

## Decisions taken on this brief (the magicless reading)

The governing principle, stated by the language's author against this brief's recon: **Sooth
has little to no implicit behaviour, and disposal is not exempt. `drop` is explicitly defined
and explicitly imported.** Three decisions follow, and they are the premise the spec starts
from rather than questions it reopens.

**D1. `drop` rides the 8a table, import-scoped, and the structural fallback is deleted where
it would run user code.** Rule 3 applies to disposal exactly as it applies to `+`: a type's
disposal word must be in scope at the site that disposes it, and its absence is a located
error naming the word to import — never a silent structural pop. **"In scope" means imported
by name** (`import: lib | File close |`, or qualified `lib::close`): a disposal word is *not*
carried by its type the way a constructor or getter is, so importing `File` alone leaves a
disposal site rejected. Settled against the alternative after the first spec draft took it:
letting the type carry its word would leave recon 1.A — the very program that motivated
calling this magic — compiling and running the destructor exactly as it does today, which is
no change at all. The distinction that justifies the asymmetry with generated accessors: those
have no body, while a disposal word runs code the author wrote, which is D2's line. This is the trade recon 1
measured, taken with eyes open: the convenience being given up is precisely the implicit
behaviour the language rejects. What this costs, so the spec does not rediscover it: ROADMAP's
goal of retiring the bespoke registry is met (it becomes table rows), but the corpus pays —
see the golden inversion below.

**D2. Derived disposal survives only where it emits no user-declared word.** A plain
`type: Point x i64 y i64 ;` still disposes structurally with no declaration and no import,
because its "destructor" is nothing at all: no code, no behaviour, no magic. The moment
generated traversal would *call* a user's disposal word — a struct holding a `File` — that
outer disposal requires the inner type's word in scope, reported at the outer disposal site.
The traversal is still generated (nobody hand-writes field-walking glue), but nothing runs
that the disposing module did not name. This is the line between "derived" and "implicit", and
it is where "little" rather than "no" implicit behaviour lands. The stricter alternative — no
generated traversal at all for a type owning a declared-disposal type, so every wrapper
hand-writes its own disposal word — was considered and not taken: the tax grows with nesting
depth and buys no additional visibility, since the required import already makes the call
visible in the source.

**D3. The container-boundary question resolves against implicit threading**, which D1/D2
decide rather than defer. Generated traversal may call an inner disposal word (D2's import
rule makes it visible), but it may not *conjure inputs*: once allocators are explicit, a
disposal word requiring an allocator (`free ( &!'A ^T -- )`) can only be called where that
allocator is in scope, so a container whose elements need one is disposed where the inputs
are, not by compiler-generated glue that invents them. This is the answer Phase 6's allocator
rework is waiting on; recon 2.3 means it ships as a stated rule with a struct-field witness,
not as a `Vec[File]` test.

**A shipped golden inverts.** `tests/phase4_modules.rs:384`
(`imported_linear_type_is_disposed_by_drop`, slice 5a Criterion 17) asserts today's behaviour
in so many words: "the module's own destructor runs whether or not it was itself exported
(D6/R19)". Under D1 that program is an error. The spec must invert this test, not delete it,
and record the criterion as superseded in ROADMAP's slice 5a entry — a slice-5a exit criterion
is being reversed by a later slice, which is exactly the kind of change that must not happen
silently.

## Open questions for the spec

1. **Struct-only or not.** Does the declared disposal word extend to enums, cells, and arrays
   (recon 2.2), or does 8b keep `drop_overload_struct_id`'s struct restriction and record the
   rest? An enum with a resource payload is constructible today, so this is reachable, not
   hypothetical.
2. **The declaration's syntax on `type:`** — the only genuinely new surface in 8a/8b. It must
   also say what a type declaring a disposal word named something other than `drop` does to
   `drop` itself at a call site: an error naming the real word, or an alias.
3. **The destructuring rule's exact shape.** Reject destructuring any type with a declared
   disposal word (E0509), or only when a linear field would be left unaccounted? What is the
   remedy the diagnostic names — today `T>` *is* the remedy the recursion check points at.
4. **Where the operator fix lands in the pipeline**: mangle operator decls and give operator
   dispatch a module-aware lookup, or leave them bare and filter candidates by the caller's
   module at `check_operator`. The first is a resolve.rs change with a check.rs lookup change;
   the second is check.rs-only but leaves two modules' `+` overloads sharing one env key.
5. **How `drop`'s own import spells itself.** `drop` is currently unmangled and unimportable
   by name (`resolve.rs:31`, `check.rs:2116`); under D1 a disposal word is an ordinary
   importable name, so the exemption dies — but the spec must say what `import: lib | drop |`
   means when several modules export a `drop` overload for their own types, given 8a rule 4
   (one arity per name in scope) and rule 1 (no shadowing) now apply to it.

## Out of scope

- **`Vec`, growable containers, plural allocators** — Phase 6. The `free ( &!'A ^T -- )` shape
  informs the general form (open question 1) but ships nothing here.
- **Lifting the linear-array-element restriction.** It is what makes the container case
  untestable, and it is its own slice of work (an element-wise drop loop in the synthesized
  destructor), not a subordinate step of this one.
- **`if`/`cond` as words** (slice 9b, blocked on 10a) and **rows in quotation effects** (10a).
- **General module-scoped visibility beyond bare-name operator dispatch.** Every other name is
  already module-unique by mangling; widening `env`'s key type across 21 signatures buys
  nothing this slice needs.

## Exit (ROADMAP's list, amended by the recon)

- A type declares its disposal word, and the checker requires *that word* rather than
  `has_drop_overload`'s boolean.
- Disposing a value whose type declares a disposal word, without that word in scope, is a
  located error at the disposal site naming the word to import (D1) — while importing the type,
  holding it, forwarding it, and `&`-reading it all still compile.
- Disposing a struct holding such a type reports the same error at the outer disposal site
  (D2), and a plain data struct still disposes with no declaration and no import.
- Destructuring a type with a declared disposal word is a located error naming the word.
- `examples/resources.sth` and `tests/phase3_resources.rs` (single-file) are unchanged;
  `tests/phase4_modules.rs:384` is inverted, with slice 5a's Criterion 17 recorded as
  superseded.
- A module's own operator overload is reachable from its own module in a ≥2-module build, and
  a selectively imported operator no longer hijacks unrelated bare uses of that name in the
  importing module — with the single-module corpus unchanged.
- **Amended:** the `Vec[File]` container criterion is unwritable (recon 2.3); the struct-field
  container is the witness, and D3 ships as a stated rule rather than a test.
