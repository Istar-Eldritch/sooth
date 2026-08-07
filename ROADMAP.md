# Sooth — roadmap

Implementation roadmap for the language in [DESIGN.md](./DESIGN.md). Milestones,
not a schedule.

## Current status / next action

Design phase complete (see DESIGN.md, Decided section). Backend decided: **QBE**
(the joy is the language, not codegen). **Phase 0 (codegen spine) is complete** and
merged to `main`: the core architectural bet held (compile-time virtual stack →
backend-neutral IR → QBE IL → native binary), with `gcd`/`factorial`/`lerp`
compiling to native binaries that run. **Phase 1 (REPL / liveness) is complete**:
`cargo run -- repl` compiles each line to a `.so` and `dlopen`s it into the session,
with a persistent stack, generation-mangled redefinition, and the golden sessions in
`tests/phase1.rs`. **Phase 2 is complete**, sliced into vertical increments: **Slice 1
(typed-core spine) is complete** and merged to `main`, carrying a `Type` per stack slot
(`i64` and `bool`), checking operand/condition/output types, unifying types at branch
joins, and lowering `bool` to QBE `w`. **Slice 2 (integer tower + conversions) is
complete** and merged to `main`: the fixed-width integer tower (`i8`..`i64`, `u8`..`u64`),
target-only conversion words (`>i8`..`>u64`), homogeneous no-implicit-promotion arithmetic
and comparison, and width/signedness-correct QBE codegen. The **floats axis** (`f32`/`f64`,
carved out of the tower) has also **landed** and merged to `main`: IEEE `+ - * /`, ordered
comparison, `<digits>.<digits>` literals, type-aware `.` printing, and int<->float conversions.
**Slice 3 (structs/records) is complete** and merged to `main`: the `type:` struct
declaration form, a user-extensible type namespace, an inline-aggregate layout model
(offsets/size/alignment computed from field widths, word-width-neutral) backed by QBE
aggregate types, generated constructor/getter/setter/destructure words, nesting, and
size-aware carried-stack marshalling across word and REPL boundaries.
**Slice 4 (enums/ADTs + clause-style pattern matching) is complete** and merged to
`main`: the `type:` form extended with `|`-separated variants, a separate enum registry
sharing the Slice 3 layout machinery, a tagged inline-aggregate representation,
exhaustiveness-checked clause-style elimination (no inline `match`), the `then` -> `end`
control-flow-closer rename, clause-body locals, and enum values crossing word-call and
REPL boundaries; `examples/shapes.sth` dogfoods it natively and in the REPL.
**Slice 5 (fixed-size arrays + `usize`) is complete** and merged to `main`: fixed-size
heap-free value arrays `[T N]` (structurally interned into an `ArrayId` registry so `Type`
stays `Copy`, reusing the Slice 3/4 layout machinery), the target-width `usize` index/length
type (width from a single threaded `WORD_WIDTH` parameter, never a hardcoded 8),
`fill`/`len` words (element access later moved to second-class references, Slice 6), and dynamic indexing
with a runtime bounds trap (Sooth's first runtime failure path) via a backend-neutral
`ElemAddr` op; `examples/stack.sth` dogfoods it. `isize` deferred to Phase 3 (its only
motivation, pointer differences, arrives with pointers).
**Slice 6 (self-tail-call → loop lowering) is complete** and merged to `main`: a word
whose body or clause body ends in a tail call to itself compiles to a back-edge `Jmp` to a
phi'd loop header instead of a `Call`, giving guaranteed constant-stack self-tail-recursion
(verified at 1M+ iterations under a 256KB stack). Reuses existing IR (blocks / `Phi` /
back-edge `Jmp`) with no new instruction; back-patching is a small deferred `back_edges`
accumulation; loop-body allocs are hoisted to the entry block (QBE `alloc*` never reclaims
within a function). Mutual tail recursion is a located compile error (3-color DFS over the
tail-call graph); tier-2 SCC contraction stays deferred. `examples/countdown.sth` dogfoods
it.
**Slice 7 (bytecode-VM dogfood, the Phase 2 exit) is complete** and merged to `main`:
`examples/vm.sth` is a small fixed-size stack machine (opcode enum, operand-stack array + a
memory array, a self-tail-recursive `run` dispatch word) that computes sum 1..N via a
bytecode loop with a backward branch, exercising the whole typed core at once (arrays,
`usize`, enums/clauses, structs, and the Slice 6 dispatch loop) in constant stack over
~1.1M dispatch steps. It shipped with **zero compiler machinery** (no `src/` change), which
is itself the exit verdict: the typed core is sufficient to write a real interpreter.
**Phase 2 is complete.** The old Slice 8 (`Copy` marker + optional / non-null pointer) was
dissolved: in a heap-free phase the `Copy` marker has no non-`Copy` type to reject and
pointers have nothing to point at, so `Copy`/linear, pointers, recursive/heap data, and
drop moved to Phase 3. Optional / non-null pointers had no compiler-known type to attach
to (Phase 3's cells are always non-null) and moved further out, to Phase 4's generics.
**Phase 3 Slice 1 (linear analysis + move-by-default + `dup` gated on `Copy` + explicit
`drop`) is complete**: move tracking on linear locals (a `Live`/`Moved`/`MaybeMoved`
lattice reconciled at branch joins), `dup`/`over` rejected on non-`Copy` types, `drop`
lowered to a destructor call, the test-only `__spy` drop-spy bootstrap primitive,
destructure-whole struct/enum aggregates (`S>fi` drop-the-rest, the non-consuming
`S|>fi` Copy-field peek, `S<fi` drop-on-overwrite) with synthesized recursive/
tag-dispatched drop glue, a located error for a linear value across a Slice 6
back-edge, and REPL `:quit` disposing residual linear values LIFO.
**Phase 3 Slice 2 (heap + owning pointer + allocator) is complete**: `^T`, a compiler-known
single heap cell, always linear, propagating linearity transitively into structs and enums and
able to hold a linear payload (the cell is freed before the payload is dropped). `^`
constructs, `^>` unwraps and frees, `^|>` peeks a Copy payload; unwrap materialises the
payload before releasing the cell and peek copies out rather than aliasing, so neither
hands a freed pointer to the stack.
A single global allocator sits behind a compiler-emitted `malloc`/`free` shim with an OOM trap
that exits non-zero and a `max(size,1)` adjustment. Disposal is observable through an
allocation trace gated on `SOOTH_TRACE_ALLOC`, on stdout so program order equals transcript
order, silent by default.
**Phase 3 Slice 3 (recursive/heap data + `isize`) is complete**: a type cycle is legal iff
it passes through at least one `^` (struct field or enum variant payload, no positional
restriction), while a by-value cycle stays a bare, span-less error; `isize`, a signed mirror of
`usize`; disposal is reversed to free-before-drop-payload and made pre-order; and a
directly self-recursive type (list or tree) gets one fused iterative destructor loop
instead of recursive `cell_drop`/`struct_drop`/`enum_drop` calls, giving constant-stack
disposal (verified at 1M+ nodes under a 1MB stack) for that shape specifically —
indirect cycles, `^^Self`, and mutually recursive types initially kept the recursive path
and its depth limit, closed the same phase by Slice 4 below. The OOM trap stays: there is
still no compiler-known optional/non-null pointer type to return failure through (that
needs Phase 4's generics), so this was never this slice's revisit to make.
`examples/list.sth` dogfoods it.
**Phase 3 Slice 4 (generalized recursive disposal) is complete**: `recursive_loop_field`'s
exact-match detection is replaced by `recursive_disposal_path`, a backtracking walk over
the static type graph that finds a path of typed steps (`Project`/`Unwrap`/`Branch`) back
to the entry type through any composition of intervening structs, cells, and enum
dispatches, keeping every independently recursive enum variant rather than restricting to
one. The fused loop's codegen walks a path of any length — byval projections, per-level
field drops, mid-path tag dispatch with the reset-then-check `terminated` discipline
applied at every dispatch it introduces, and per-aggregate-slot copyout ordering — so a
wrapper-struct indirection, `^^Self`, and a mutually recursive multi-type cycle each get
their own fused loop (every type on a cycle synthesizes its own loop from its own shape;
none calls another's destructor to traverse the shared cycle), all verified in constant
stack at 1M+ nodes under a 1MB stack. A **struct** with more than one simultaneously-live
recursive field still narrows to one chosen edge (D1, unchanged); an **enum**'s
mutually-exclusive variants are not that restriction. Worklist-based disposal for
branching structures stays out of scope, moved to Phase 6.
**Phase 3 Slice 5 (general locals) is complete**: `| names |` binding at any point in a
word body, clause body, or `if`/`else` arm (extent = the rest of the enclosing block, no
new closing token), the checker's locals map evolving during the walk instead of a
precomputed borrow, and locals at the REPL line with the session-stack depth as the
binding's frame floor. No new IR instruction, and no header phi added for a mid-body name
in a self-tail-recursive loop. `examples/vm.sth`'s `run` word dogfoods it, naming a
`vm-pop` result mid-clause instead of shuffling it with `swap`/`over`/`rot`.
**Phase 3 Slice 6 (second-class references) is complete**: `&T`/`&!T`, prefix `&a`/`&!a`
borrows of a local, the projection family (`&T>fi`/`&!T>fi`/`&>`/`&!>`/`&^`/`&!^`), and
`@`/`!`/`+!` through a reference, with a reference staying `IrType::Ptr` to the backend (the
QBE backend needed no non-test change). Escape is closed structurally in six positions plus
the REPL line; exclusivity is per place; a back-edge may carry a reference parameter but not
one derived from a current-scope local. Two things fell out of specifying it. Because naming
an aggregate does not copy it, **two names for one region plus a mutable borrow is an error**,
with `dup` as the remedy: no copy is ever inserted implicitly, since worst-case-timing
reasoning needs instruction counts readable off the source. And because an `if`/`else` merge
can denote either arm's place, a value carries a *set* of regions rather than one, so the
merge unions the arms and **no aliasing rejection happens at a join**: selecting one of two
owned records compiles, and the error lands at the borrow where it can name both ends.
`examples/refs.sth` dogfoods it.
**Phase 3 Slice 8a (typed foreign calls + string slices) is complete**: one `extern:` declaration
form (a C symbol string plus a stack effect), registered into the ordinary word environment so
existing arity/type checks apply unchanged; the boundary type set is the numeric tower, `&T`/`&!T`,
and `cstr` (amended during implementation to *exclude* `str`, which is a descriptor handle rather
than a scalar or a single opaque `Ptr`, so C would receive a pointer to a descriptor rather than a
`char*`); owned aggregates, `str`, and an
owned/reference output are rejected at the declaration. Two new string types: `str` (one opaque
pointer to a static two-word `{bytes_ptr, len}` descriptor, `IrType::Str`) and `cstr` (a bare
NUL-terminated pointer, `IrType::Cstr`), both `Copy`, both static-rooted except that a `cstr` may
also come back from a declared foreign call, where the declaration site's trust governs instead
(no `unsafe` marker). String literals (`Token::Str`, `\n \t \\ \" \0` escapes) lower to static
data with an uncounted trailing NUL, which is what makes `len` (carried, no scan) and C's `strlen`
agree on NUL-free content, and what the explicit, one-way `cstr` conversion relies on. `.` prints
a `str` via `%.*s` bounded by the carried length and a `cstr` via `%s`. `examples/strings.sth`
dogfoods it.
**Phase 3 Slice 8b (resources and user destructor bodies) is complete**: `drop` is the first
overloaded-by-input-type word, a miniature early instance of Phase 4's dispatch — a user body
substitutes directly into the existing `struct_drop_symbol` slot rather than an env-name lookup,
since `drop` is intercepted by hardcoded match arms before any lookup, at both check and
IR-lowering time, and runs *instead of* the synthesized field glue. Self-recursion (direct,
through a helper word, or through a containing aggregate) is caught by whole-program call-graph
reachability, both natively and at the REPL. Full REPL support: session-level retention of
overrides, an epoch/generation-suffixed destructor symbol avoiding `RTLD_GLOBAL` collisions on
redefinition (stamps every linear struct/enum/cell, not just the overridden one), and
composing-glue refresh on redefinition. A fused-disposal-cycle boundary fix (`expand_path`) stops
an override from being silently bypassed. Unifying `Type::Spy`'s hardcoded drop dispatch into this
mechanism was considered mid-spec-review and **cut entirely**, not partially folded in: it would
have been a pure no-op refactor of code Slice 8c deletes outright anyway. `examples/resources.sth`
(open/read/close a file, `close` reached only through `File`'s own `drop` override, a deliberate
double-use and a forgotten `close` both compile errors) is the Phase 3 exit dogfood.
**Phase 3 Slice 8c (retire `__spy`) is complete**: `Type::Spy`/`IrType::Spy` and every
hardcoded match arm across `ast.rs`/`check.rs`/`ir.rs`/`src/backend/qbe.rs` — the
builtin-table entry, `is_copy`'s special case, and the backend's compiler-synthesized
`sooth_spy_drop`/`$spyfmt` destructor shim — are gone, found via Rust's exhaustiveness
checker rather than a manual audit. Every property the primitive existed to demonstrate is
now expressed with an ordinary `type: Spy tag i64 ;` plus a user `drop` overload
(`| s | "drop " . s Spy>tag . ;`), verified to reproduce the primitive's exact runtime trace
byte-for-byte; ~280 call sites migrated across five test files and four in-crate unit-test
modules, with all six integration-test binaries keeping identical counts to before
(migration, not silent deletion). Five unit tests were deleted outright with no
replacement, each confirmed to test only the deleted primitive's own bootstrap machinery
(the backend's synthesized destructor IL, its hardcoded extern-boundary rejection wording,
its own name/identity), leaving no coverage gap. A deletion-plus-migration slice with no
design decisions (per its roadmap entry below), so it skipped the brief/spec/multi-round
review pipeline used for 8a/8b: one async implementation pass plus one fresh-context review
pass, proportional to migration blast radius rather than design risk.

**Phase 3 is complete.** Slice 7's opt-in RC is
deferred to Phase 6, where it joins `Box`/`Vec`/`Map`/`String` in the `alloc` layer.
**Phase 4 Slice 1 (type variables + row variable + length variables + monomorphization,
native) is complete**: a user `:` word may declare `'T` (type variable), `..s` (row
variable), and a length variable in an array count position `['T 'N]`, optionally bounded
(`'T: Copy`); the checker represents these in a checker-only `PolyType`/`PolySig` living
beside a word's declared effect (`Type` gains no variant, the `Slot` virtual stack stays
concrete), unifies them against the concrete stack at each call site, checks bounds
Kitten-style against the concrete instantiation, and the backend emits one monomorphized
`IrFunc` per distinct instantiation, keyed through a check→lower table so the call-site
symbol and the emitted symbol can never disagree. The same slice closes the long-standing
multi-output lowering panic (`lower_call` silently dropped every result past the first):
a synthesized aggregate-return ABI (a bundle struct interned at check time, flagged so no
destructor is ever synthesized for it) is both the fix and the mechanism a row variable in
output position lowers through once monomorphization has resolved it to a concrete count —
one mechanism for both. `max`/`max-total` ship as new builtins, `max` over the integer
tower and `max-total` over floats by the `total_cmp` bit-pattern rule, kept as two disjoint
surfaces rather than pretending IEEE `>` is total. `examples/stack.sth` is the dogfood:
`pop`/`peek` now return `( Stack -- Stack i64 )` directly instead of hand-bundling into a
`Popped` struct; the honest finding is that polymorphic `dup`/`swap`/`max` touch no
existing example, since the core shuffles were already type-transparent before this slice
and no checked-in program computed a maximum. REPL monomorphization, the combinator
library, and the inliner stay out of scope, deferred to later slices.
**Phase 4 Slice 2 (REPL monomorphization) is complete**: the REPL sees polymorphic words.
`eval_def` routes a `word.poly.is_some()` definition through the native `check_poly_body`
pass instead of the concrete `check_def`/`check_word` path, replacing the recon-1 silent
miscompile (a bogus `( -- )` mismatch, or a silent `defined` that never checked the body)
with either a real located error or real acceptance; a poly def resolving to two or more
outputs is a clean located deferral rather than a silent single-output truncation, since
REPL lowering interns no return bundle. A session store, `poly_words`, retains a
polymorphic word's body alongside a generation and a **frozen resolver snapshot** captured
at its *defining* line (not the instantiating line's), so an instantiation's callees bind
to the generations live when the word was defined, matching the frozen-binding rule every
ordinary REPL word already follows (the larger question of REPL late binding on
redefinition stays deferred, DESIGN.md). One session poly-env threads into both check paths
(a bare expression line and a defined word's own body, so a defined word can call a
retained polymorphic word) and one instantiation table plus poly-arity map threads into
both lowering entry points; a shared emit step lowers one monomorphized `IrFunc` per
not-yet-exported instantiation into the compiling module with external linkage, deduped by
an `exported_insts` set keyed by the instantiation symbol (which already encodes name,
generation, and substitution) so a repeated same-type instantiation recompiles nothing.
`instantiation_symbol` gained a `generation: Option<u64>` parameter (`None` natively,
reproducing every existing symbol byte-for-byte; `Some(g)` at the REPL, appending a
`__gen{g}` component matching `mangled_symbol`'s existing device), so the checker's mint
and lowering's independent re-mint stay two deterministic computations of one function and
can never disagree. Redefining a polymorphic word follows the ordinary-word generation
rule (bump a shared per-name counter across `self.env` and `poly_words`, retain a fresh
resolver snapshot, leave every old instantiation symbol resident and resolvable) rather
than 8b's blanket restamp, so an earlier line's compiled call stays frozen to its old
generation's body while a new call site binds the new generation — closing the symbol-
collision hazard the brief traced (re-instantiating a redefined word at an already-seen
type would otherwise mint the old body's symbol and silently run it under `RTLD_GLOBAL`
first-loaded-wins). `examples/*.sth` and the native `tests/phase4_generics.rs`/
`tests/phase3_resources.rs` suites are unchanged (every native call site still passes
`None`); the exit is a `tests/phase1.rs` golden session covering the whole sequence: define
once, instantiate at two different types, instantiate twice at one type without
recompiling, redefine, and see the new body take effect on a new call while an earlier
line's call keeps the old one.
**Phase 4 Slice 3 (aggregate-return aliasing: the loop-carried copy) is complete**: a
self-tail-recursive loop carrying an aggregate across the back-edge now gets one
entry-hoisted stable stack slot per carried slot (no header phi for it) plus an
unconditional read-before-write staged blit on each back-edge, so a value from
iteration *k* can no longer alias the slot iteration *k+1* overwrites; scalars,
references, ordinary (non-loop) join phis, and the fused destructor loops are
unchanged. The cause was storage reused across loop iterations generally, not the
aggregate-return ABI specifically: a by-value aggregate return is the common instance,
but an aggregate constructed inline each iteration with no call at all reused its
entry-hoisted storage the same way and is fixed by the same change.
**Phase 4 Slice 4 (quotations + the internal loop primitive) is complete**: a quotation
literal `[ ... ]` parses to `TermKind::Quotation` and stays a **compile-time-only marker**
until slice 7 gives it a runtime type — no `Type`/`PolyType`/`IrType` variant — carrying the
identity of its literal body on a `Slot`/`Binding` side-channel (`quot`), forwarded through
shuffles and binds, and consumed only by fusion: `call` splices a literal's body at the
consumption site (type-checking identically to writing the body inline), and every other
position that would need a runtime quotation value (an array element, a branch join, a
user or polymorphic word argument, an operator operand, a REPL residual stack) is a located
rejection instead of a panic. The one compiler-known intrinsic, `times ( ..s i64
[ ..s i64 -- ..s ] -- ..s )`, drives the existing `begin_loop`/`finalize_loop` staging with a
synthesized index, giving a runnable constant-stack loop (a header `Phi`/`Jnz` reached by a
back-edge `Jmp`, no per-iteration `Instr::Call`, every loop-body `Alloc` entry-hoisted,
verified at 1M+ iterations under a 1MB stack); a `times` nested inside another loop is
rejected in the checker, which also restores loop state after a `times` returns so a second
loop in the same word still runs. `while` was weighed as a second floor member and declined:
its condition quotation returns a `bool` on a passthrough row, strictly harder than `times`
needs. `examples/times.sth` (`0 1000000 [ 1 + + ] times .`) dogfoods it, printing the same
total as `examples/countdown.sth`'s hand-threaded self-recursion.
**Phase 4 Slice 5a (native multi-file compilation, word and type imports, and
encapsulation) is complete**: a file is a compilation unit, and `import: q "path.sth" ;`
resolves the import graph from the entry file, canonicalizes and dedupes by path,
orders it topologically, and rejects a cycle or self-import with a located error naming
both files. The whole closure is lexed and pre-passed once into one shared registry
set (structs/enums/arrays/cells/refs) — the parser's pre-pass over raw tokens
(`prepass_type_decls`) needs every imported type name present before any body parses,
so an import cannot be a post-parse merge — then bodies parse per file against that
shared set and `check::check` runs once over the assembled `Module`. A qualified
`q::name` resolves own-module-first, then by qualifier; two modules may each declare
`Point` (duplicate-type-name checking is per-module) and same-named words in two
modules mint distinct symbols via a module-disambiguating component minted the way
`generation` already is, so no `::` ever reaches the symbol sanitizer and a
single-module closure is byte-for-byte unchanged. **Encapsulation is default-private**,
with a per-file `export:` list (multiple lines accumulate); naming a type in `export:`
is **transparent** (D3, no opacity mechanism in this slice): it exports the type
*and* its five generated words (constructor, getter, peek, setter, destructure) as
one unit, since Sooth structs are dumb data and hiding accessors buys little against
no UB, trapped indexing, and linearity — and since destructure already bypasses a
`drop` override today, single-file, visibility never protected resource discipline
anyway (that gap is real, newly reachable across a file boundary, and stays
out of scope for slice 8's ownership checker to fix properly, `E0509`-style, rather
than a partial guard here). A qualified accessor (`q::Type>field`, `q::Type<field`,
`q::Type|>field`) resolves by splitting on the *first* `::`, since `>` is not a
delimiter. Using an unexported name qualified is a located `not exported` error,
distinct from unknown-word; an exported word naming a private type of its own module
is rejected at the `export:` declaration. Disposal crosses the boundary for free: a
bare `drop` dispatches on the concrete type whether or not its destructor glue was
exported, so this slice adds no export-site disposal rule (that waits on slice 8,
where a polymorphic `drop` could first be structurally total). Selective import,
`import: q | a b | "path.sth" ;`, additionally exposes the listed names unqualified
(a type brings its generated words too); two selective imports of one name, or a
collision with a local word, is a located error at the second, naming both. REPL
imports were not in this slice: `import:` at the REPL was a located rejection naming
the construct until slice 5b (below) shipped the real thing, so no phase shipped a
degraded REPL in between. `examples/modules.sth`
dogfoods it: a `Point` type exported from `examples/modules_point.sth`, `add`/`len2`
exported from `examples/modules_ops.sth` (itself importing the type file), used
together by the entry file via both a qualified accessor and a qualified word call.

**Phase 4 Slice 5b (REPL imports) is complete**: `import:` at the REPL reuses the native
pipeline unchanged (`discover_closure` / `assemble_module` / `check::check`, elevated to
`pub(crate)`), resolving the REPL's own top-level path relative to the process cwd while
every transitive import inside the closure keeps 5a's importer-relative rule. The whole
closure bulk-lowers to one `.so` via the same call sequence `eval_def` already uses for a
single word, `dlopen`ed and retained under `RTLD_GLOBAL`. Each `import:` line is an
ordinary redefinition event applied to a batch of names rather than a new rule: it mints
one fresh, session-wide import epoch, symbol-tags every compiled word by it
(`{name}__import{epoch}`, collision-free by construction against an ordinary word's
`{name}__gen{N}` and against every other import epoch), and assigns the event its own
module id, so a re-import recompiles every word fresh and a caller already compiled
against the old epoch stays frozen exactly as any other redefinition freezes its callers
(DESIGN.md's separate REPL-late-binding question is unchanged, not reopened). Splicing an
imported closure into the session's flat, positionally-indexed registries remaps every
type id it carries (struct/enum fields, arrays, cells, refs, and every spliced word's
`Sig`) from closure-local indices to session indices, the append-with-remap the
`StructId = index` invariant forces once a session — unlike a fresh native compile — has
an already-populated registry to append onto. A body-position user-facing spelling
(`q::name`, or a selective import's bare name) resolves through an alias indirection to
its current internal, epoch-tagged entry, while a type-*position* reference (a signature,
a `type:` field) resolves through the same module-aware resolver a native multi-file
closure's own `q::Type` already uses; a REPL-declared name containing `::` is rejected up
front so nothing can forge the internal tag. Registry growth on re-import is accepted, not
derived, so the growth is not deduplicated or capped, matching a redefined word minting a fresh
generation every time. Only module 0's `export:`ed names are nameable; a third file
imported *by* the imported file contributes no session-visible name (transitive
re-export stays closed, as 5a already declined). An imported file declaring `main` is a
located rejection naming the file and the word, at import time, before any codegen — the
same `main`-collision on the *native* path (recon #4) stays unfixed and recorded below.
Selective import, `import: q | a b | "path.sth" ;`, ships at native parity: `q` binds and
`a b` are additionally spliced unqualified, pointing at the same internal entry the
qualified splice already created (a value built through either spelling is the same
type), with 5a's collision rule extended to session scope (a selectively-exposed name
colliding with an existing session name is a located error at the second, naming both).
Any failure in `eval_import` (parse, missing file, cycle, `main`-in-library, selective
collision, check error) leaves the session untouched, matching `eval_line`'s existing
commit-only-on-success contract. `tests/phase4_repl_imports.rs` dogfoods it end to end in
one piped-stdin session: a qualified word and accessor, a redefine-and-reimport with a
frozen caller beside a fresh resolution, the `main`-in-a-library rejection, and a
selective import called unqualified.

**Phase 4 Slice 6a (quotation types in signatures + the inliner + `each`/`map`/`fold`) is
complete**: `Type`/`PolyType` gain a `Quotation` variant carrying an interned declared
effect (`[ 'T -- ]`), with unification and `apply_subst` following, so a word may declare a
quotation parameter and be checked standalone against it — no `IrType` variant and no
"statically known" bit (D6): knownness stays a predicate on the value (`Slot.quot`), and
every other type position (a struct field, an array element, a cell payload, a reference
referent, a word's output, an `extern:` boundary, `main`, nesting inside another effect) is
a located rejection naming slice 7. `call`/`times` accept an *abstract* quotation typed only
by a declared parameter, beside the literal they accept today; a quotation literal passed to
a declared parameter is checked directionally against the declared effect, enforcing a
`Copy`-only capture restriction (D3) at the literal. Every call to a quotation-taking word is
inlined by term-splicing the callee's AST body against the caller's live stack — the
compiler's only inliner, forced by there being no `IrFunc` for a quotation-taking word to
call (D2) — transitively and totally: anything un-inlinable, starting with recursion among
quotation-taking words, is a located error, never a silent real call (D5). The transitive
case is a combinator forwarding its own quotation *parameter* to a nested combinator, which
splices through both frames. `each`/`map`/`fold` are ordinary polymorphic Sooth words at
`lib/combinators.sth` (D8), each a **leaf** combinator driving a `times` loop directly over
an array's elements and handing one to its quotation parameter per iteration, verified to
lower to a tight loop with no per-element `Instr::Call`, and verified to match a
hand-threaded `times` twin across a sweep of stack limits. `map`/`fold` are *not* built on
`each`, but on cost grounds, not impossibility: `fold` and `map` over `each` are both
expressible (the accumulator rides a captured one-element array reached by balanced `&`/`&!`
borrows, which D3 accepts). Because inlining is total, library composition depth is code
size at every call site, so building `map` on `each` would make every `map` call site depth
2 plus an extra array copy and a counter cell, where a leaf keeps the library flat at depth

1. **"When to inline" becomes a real question only at slice 7, when a runtime representation
first makes a genuine choice possible; until then "always" is the only implementable answer,
and a budget would be actively harmful, since exceeding it could only be a compile error.** Native only: the REPL is a located rejection at
both the defining line and an imported closure exporting a quotation-taking word (D7),
lifted by 6c. `examples/array_totals.sth` dogfoods it against its
hand-threaded twin `examples/array_totals_hand.sth` (three manual `times` loops): three
one-line combinator calls run to the same total and doubled elements the twin does.

Two pre-existing defects 6a measured but deliberately did not fix, for a later slice to pick
up: `fill`'s compile cost is superlinear in the array length (10k ~ 0.36s, 100k ~ 25s, 1M >
300s, and a hand-threaded loop is equally slow, so it is the array machinery and not the
inliner), which is why 6a's constant-stack criterion is an equivalence-plus-correctness
witness at 10k (equal exit code and stdout against the hand-threaded twin) rather than the
1M run first specified; and every native `build` diagnostic prints a doubled `error: error:`
prefix, because the ~165 error constructors embed `error:` and `src/main.rs` prepends
another. A separate repo-wide diagnostic defect *was* fixed during the review sequence:
every diagnostic naming a word leaked the internal `__m0` monomorphization mangling whenever
the module had an import (predating 6a, arriving with module support), and the now-unreachable
`demangle_local` was deleted as dead code.

**Phase 4 Slice 6b (`filter`/`while`, and the self-tail combinator loop) is complete**: its
paper pre-check (`docs/phase4-slice6b-brief.md`) falsified the slice's original charter
before any code changed, by building the programs — `filter` needed no compiler change at
all, and `while`'s actual blocker was 6a's own combinator-cycle rejection, not either
"polymorphic-path gap" the roadmap had named, both of which stay in place, untouched
(`src/check.rs:3672`'s polymorphic-`if` rejection, `src/ir.rs`'s poly-instantiation
`self_tail` hardcode). `filter` ships as a `Copy`-element combinator compacting an array in
place through 6a's inliner unchanged, no compiler change at all. `while`'s deliverable is
the self-tail combinator loop: `check_combinator_cycles` is relaxed so a self-edge is
permitted iff every occurrence of the self-name is in tail position (comparing
`all_calls` against `tail_position_calls`); a non-tail self-call or a mutual cycle is still
`combinator_cycle_error`, unchanged. `inline_combinator` gains a self-tail branch: while
splicing a self-tail combinator's body, a tail-position call back to that same combinator is
not re-spliced but treated as the loop back-edge, running the same two obligations the
whole-word self-tail transform already runs (`check_linear_across_back_edge`,
`check_reference_across_back_edge`) before terminating that branch. Lowering composes two
existing ingredients rather than inventing a third: `times`'s mid-body `begin_loop` open and
the whole-word transform's self-call-driven back-edge, including the `stage_aggregates`
stable-slot path for a carried aggregate state (the slice-3 aliasing fix, reused verbatim).
`while` inherits the R18 nested-loop limit in both directions (a `while` sited inside an open
`times`, and a `times` sited inside a self-tail combinator body), which required renaming the
guard's counter from `times_depth` to `loop_depth` since it now counts two kinds of loop; the
limit itself is not lifted here (6d lifts it for all five combinators at once). The REPL
definition chokepoint already rejects a self-tail combinator with no change, since it keys on
the declared quotation parameter alone. `examples/filter_while.sth` dogfoods both words
against a hand-threaded twin, with the array passed straight from its producer word into
`filter` rather than bound to a local first, so it does not trip 6a's bind-then-pass alias
limitation.

**Next action: Phase 4 Slice 6c, 6d, or 6e.** None depends on the others; 6c (quotation-taking
words at the REPL), 6d (nested constant-stack loops, lifting the limit 6b inherited), and 6e
(`if` in a polymorphic body) may land in any order. 6e was added after 6b shipped, once it
became clear that no slice owned the polymorphic-`if` gap even though slice 7 depends on it
and the core library's intrinsic-vs-library split is gated on it.

Host language: Rust is the sensible default (ADT + pattern-matching-heavy compiler
workload, `no_std` for the runtime/intrinsics library), but nothing now requires
it, since LLVM and Z3 were dropped. Free choice.

## Guiding principles

- **De-risk novel-before-laborious.** Prove the uncertain, novel parts (the codegen
  model, then the linear memory model, which is the whole point of the language)
  early. The larger-but-understood parts (stdlib, self-hosting) can wait.
- **Vertical slices with a dogfood program each phase.** Every phase ends with a
  language you can run a real (if small) program in, and you actually write that
  program. This is the antidote to the failure mode named in DESIGN.md: a beautiful
  half-built compiler no one writes code in. If a phase produces no runnable
  program, the phase isn't done.
- **Liveness early.** A REPL and immediate feedback arrive in Phase 1, not at the
  end, for the same reason.
- **No calendar estimates** (they'd be fiction). Effort weights (S/M/L/XL) are
  relative, to show where the mass is.

## Phases

### Phase 0 — Codegen spine  `[L]`  ✅ **done** (go/no-go on the architecture: **go**)

Lexer/parser for a minimal concrete-typed core (`: ;`, literals, arithmetic,
comparisons, `if/else/end` (originally `if/else/then`; the closer was renamed to `end`
in Slice 4), the core stack
shuffles `dup`/`drop`/`swap`/`over`/`rot`
(monomorphic, int-only here; widened later), and `| locals |`). Compile-time virtual
stack → a
backend-neutral IR → **QBE** IL → `qbe` → system assembler + linker → native binary.
No LLVM, no hand-written native backend. Keep the IR's `Ptr[T]` abstract from the
start so a WASM sibling lowering can be added later. Static stack-effect (arity)
checking. One concrete int type, no heap.
**Exit (met):** `gcd`, `factorial`, and `lerp` compile to standalone native binaries
and run correctly (`5` / `120` / `30`), plus a negative golden for the stack-effect
diagnostic. Proved the virtual-stack → IR → QBE → native path end-to-end.

### Phase 1 — REPL and liveness  `[M]`  ✅ **done**

No in-process JIT (that left with LLVM), and no comptime interpreter (there are no
immediate words; see DESIGN Declined). The REPL runs on the **backend** via `dlopen`:
each new word is compiled to a shared object and loaded into the live session, so the
process holds natively-compiled code it can call at once; redefinition loads a new
object and swaps the name→symbol entry. Whole-program `run` uses compile-to-binary +
subprocess. Factor's in-image model minus the sub-millisecond compile, without owning
a backend.
**Exit (met):** define/test words interactively; redefinition works; the first
throwaway-but-real interactive session exists.
**Dogfood (met):** a tiny interactive calculator session (`tests/phase1.rs`,
`calculator_session_dogfood`).

### Phase 2 — Typed core (monomorphic)  `[L]`  ✅ **done** (Slices 1-7 + floats/bitwise/bool; the VM dogfood was the exit. The old Slice 8 `Copy`/pointer bridge moved to Phase 3.)

Sliced into vertical increments (each green and runnable). **Slice 1 (typed-core spine)
is done**: two concrete types (`i64`/`bool`), a type-carrying checker that unifies type
and arity at branch joins, and `bool` lowered to QBE `w`. **Slice 2 (integer tower +
conversions) is done**: the fixed-width integer tower (`i8`..`i64`, `u8`..`u64`), target-only
conversion words, homogeneous arithmetic/comparison, and width/signedness-correct codegen
with single-point sub-word canonicalization. The **floats axis is done** too: `f32`/`f64`
with IEEE arithmetic (including float `/`), ordered NaN-correct comparison, float literals,
type-aware `.` printing, and numeric-generalised int<->float conversions. The **bitwise axis is done**:
`and`/`or`/`xor`/`not` plus `shl`/`shr` with a single type-directed right shift (arithmetic
`sar` for signed, logical `shr` for unsigned), i64 shift count masked mod the operand's bit
width. The **boolean/comparison surface is complete** too: `and`/`or`/`xor`/`not` are
type-directed over `bool` (logical) as well as integers (bitwise), and the comparison set is
filled out with `<= >= <>` (numeric, signedness- and NaN-correct), making `bool` a
first-class operand type rather than an `if`-only token. With this the Phase 2 **scalar core**
is complete. **Slice 3 (structs/records) is also done**: user-declared struct value types,
an inline-aggregate layout model, and generated construction/field-read/functional-update/
destructure words. **Slice 4 (enums/ADTs + clause-style pattern matching) is also done**:
sum types via the `type:` form, a tagged inline-aggregate representation sharing the Slice 3
layout machinery, exhaustiveness-checked clause-style elimination, the `then` -> `end`
rename, and clause-body locals. **Slice 5 (fixed-size arrays + `usize`) is also done**:
heap-free value arrays `[T N]` (interned `ArrayId`, reused layout machinery), target-width
`usize`, `fill`/`get`/`set`/`len`, and dynamic indexing with a runtime bounds trap. What
nothing remains in Phase 2: the VM dogfood (Slice 7) was the exit, and the old `Copy`/pointer
bridge (Slice 8) moved to Phase 3.

**Slice plan** (dependency-ordered; each its own brief -> spec -> implement -> review
cycle, each green and runnable). Slices 3+ are a plan, not yet locked specs:

1. **Typed-core spine** (`i64` + `bool`): a `Type` per stack slot, unifying type (not just
   depth) through bodies and at branch joins. ✅ done.
2. **Integer tower + conversions**: `i8`..`i64` / `u8`..`u64`, target-only `>iN`/`>uN`
   conversions, homogeneous arithmetic/comparison, width/signedness codegen. ✅ done.
3. **Structs / records**: aggregate value types. ✅ done.
4. **Enums / ADTs + clause-style pattern matching**: sum types via the `type:` form with
   `|`-separated variants; exhaustiveness-checked elimination **folded into word definition**
   (a word whose top input is an enum is defined by `|`-led clauses, one per variant, with no
   inline `match` keyword); Result/Either fall out as ordinary monomorphic enums. Variants are
   not standalone types (a variant constructor yields the enum); a clause consumes the
   scrutinee and pushes the variant's fields onto the stack (linear destructor dispatch);
   exhaustive-only, no `_` wildcard yet; no recursive enums (infinite size is a bare,
   span-less error, since recursion needs a pointer, which arrives in Phase 3). Also **renames the control-flow closer `then` ->
   `end`** (`if … else … end`), unifying it, and extends **top-of-scope `| … |` locals** to
   clause bodies (bind names at the top of a word body or a clause body, extent = that scope;
   no mid-body binding, no closer: factor a word instead). Design locked in
   `docs/phase2-slice4-brief.md`. Prefer-the-stack stays the culture; locals stay opt-in.
   ✅ done. **The mid-body half of this decision is reversed by Phase 3 Slice 5 (general
   locals):** specifying the references slice (Phase 3 Slice 6) hit six separate places where
   the only way to name a projection's result was a word that exists purely as a binding site
   (`run`, `build-into`), not a meaningful abstraction, so "factor a word instead" failed on
   its own terms and the restriction is lifted. The **no-closing-token** half stands: a
   mid-body binding's extent is simply the rest of its enclosing block, and that needs no
   closer either way.
5. **Fixed-size arrays** (still heap-free). Introduce **`usize`** (and likely **`isize`** for
   pointer differences) here, as the target-width index/length type, so array indices are
   `usize` from the first use rather than a hardcoded `i64` retrofitted later. Its defining
   property (target-defined width, consistent with the opaque `Ptr[T]` invariant) only
   becomes load-bearing and testable once a real consumer (indexing) or a non-64-bit backend
   exists, which is why it waits until now rather than landing with the integer tower. `isize`
   deferred to Phase 3 (no consumer until pointer differences exist). Arrays are inline `Copy`
   value aggregates, `get` non-consuming, `set` functional; dynamic indexing has a runtime
   bounds trap. ✅ done.
6. **Self-tail-call → loop lowering** (mandatory TCO for self-recursion). A word whose
   body (or any clause body) ends in a tail call to *itself* is compiled to a back-edge
   jump to a phi'd entry header instead of a `Call`, so self-tail-recursion runs in
   constant stack and cannot overflow. No new surface syntax: existing recursive words
   simply stop growing the stack in tail position. It's a **guarantee**, not a
   best-effort optimisation (code may rely on it), which is why it precedes the VM: the
   dispatch loop is self-recursive and would otherwise overflow, and pulling quotations
   (Phase 4) forward to get a loop is the larger change. Reuses the IR's blocks / `Phi`
   / back-edge-capable `Jmp` (already emitted for `if`/clause dispatch). **Mutual** tail
   recursion (a tail-call cycle A→B→A) is **out of scope this iteration** and rejected
   with a located error; tier 2 (SCC contraction into one tagged loop, explicitly not a
   trampoline and not QBE backend TCO) is a planned follow-on, see DESIGN.md. Drop-at-
   back-edge is vacuous in Phase 2 (all-`Copy`) but the back-edge is the defined drop-
   insertion point for Phase 3. ✅ done.
7. **Bytecode-VM dogfood**: the Phase 2 exit dogfood, a small fixed-size VM for a toy
   bytecode, exercising the whole typed core (arrays, `usize`, enums/clauses, structs,
   and the self-tail-call dispatch loop from Slice 6). Shipped as `examples/vm.sth` with
   zero compiler machinery. ✅ done.
The old **Slice 8** (`Copy` marker + optional / non-null pointer) is **dissolved**: with no
heap and no linear type in Phase 2, the marker had nothing to reject and pointers had
nothing to point at. `Copy`/linear, pointers, recursive/heap data, and drop land in Phase
3, where their first real clients exist; optional/non-null pointers had no compiler-known
type to attach to there either and land in Phase 4's generics instead. See the Phase 3
slice plan.

Numeric axes carved out of Slice 2 have all landed: **floats** and **bitwise operators**
(`and`/`or`/`xor`/`not`/`shl`/`shr`, type-directed right shift), both merged to `main`. The
`*/` widening primitive is still deferred. **`i128`/`u128` are not planned:** a first-class 128-bit type is completeness-think for a craft language, and the one
real need behind it (a 64x64->128 widening multiply, e.g. for hashing or `*/`) is better
served by a narrow widening-multiply primitive if a concrete consumer ever appears, not by a
type.

**Floats axis, delivered** (brief + spec: `docs/phase2-slice-floats-brief.md`,
`docs/phase2-slice-floats-spec.md`): `f32`+`f64`; homogeneous `+ - * /` (float `/` is in, no
`mod`); IEEE-754 with **silent NaN/inf propagation** (no trapping, no static rejection:
NaN/inf are inherently runtime and Sooth's compile-error lever cannot reach them); float
literals `<digits>.<digits>` (digits required both sides so they cannot collide with the `.`
print word), defaulting to `f64`; printing is the type-directed `.` (every scalar, unsigned
printed as unsigned); the target-only conversion family generalised to numeric (`>f32`/`>f64`, and
float->int truncating toward zero, out-of-range/NaN unspecified). Comparison `< > =` are
plain IEEE ordered compares: `=` is **exact** bit equality (a documented footgun), never
epsilon. NaN is user-detectable via `x = x`; `isinf` and any epsilon/approximate comparison
are deferred to the stdlib. **Note:** the unsigned int<->float conversions emit QBE ops
(`uwtof`/`ultof`/`stoui`/`dtoui`) that need a reasonably modern QBE; Debian's packaged 1.2 is
too old (see README build note).

Float ordering is **partial** (NaN compares false to everything, so there is no total
order). This slice ships no generic `sort`/`Ord` bound to attach that to, so nothing is
owed now; when generics land (Phase 4) and a `>`-requiring polymorphic word or a
sortable/hashable collection needs a total order over floats, revisit then (Rust's model:
expose the partialness at the sort/key site, e.g. a `total_cmp`, rather than silently
lying). Tracked so it is not lost.

**Structs axis, delivered** (brief + spec: `docs/phase2-slice3-brief.md`,
`docs/phase2-slice3-spec.md`): the `type:` struct declaration form (bare `name type`
field pairs, `;`-terminated); a user-extensible type namespace (`Type::Struct` +
a per-program registry); an inline-aggregate value model (one typed stack slot per
struct, backed by QBE aggregate types and frame-local `alloc`, heap-free); layout
(offsets/size/alignment) computed from field sizes/alignments, never a hardcoded
machine word; generated constructor/getter/setter(functional)/destructure words per
struct; nesting via juxtaposed accessor calls; all structs trivially `Copy` (byte-copy
`dup`, no-op `drop`); size-aware carried-stack marshalling generalized to per-slot
byte sizes across both the word-call boundary and the REPL line boundary; a REPL
struct-placeholder display (`<TypeName>`); sharp located diagnostics for an unknown
field type, a duplicate type name, constructor arity/type mismatch, an accessor applied
to the wrong type, `.`/`=`/arithmetic on a struct, and a malformed declaration (a
recursive (infinite-size) struct is the one exception: bare and span-less, not located);
a zero-field unit struct; and the
`examples/vectors.sth` dogfood (`Vec2`/`Segment`, `sub`/`len2`/`span`/`shift-x`),
running both as a native binary and in the REPL.

**Enums axis, delivered** (brief + spec: `docs/phase2-slice4-brief.md`,
`docs/phase2-slice4-spec.md`): the `type:` form extended with `|`-separated variants
(each a name plus zero or more `name type` field pairs); a separate enum registry
(`Type::Enum` + `EnumId`) sharing the Slice 3 layout machinery rather than merging with
the struct registry; a tagged inline-aggregate representation (a fixed-width `i32`
discriminant plus a max-variant payload, word-width-neutral); generated per-variant
constructor words; exhaustiveness-checked clause-style word definition as the sole
eliminator (`| Variant … | Variant … ;`, no inline `match`, exact one-clause-per-variant
coverage folded into the word's single declared output effect); clause-body `| names |`
locals (extent = the clause); the control-flow closer rename **`then` -> `end`**
(behaviour-preserving, migrated across every live example/test/doc); a D8 variant-name
pre-pass disambiguating `|` as clause-marker vs. locals-delimiter, with a variant-named
local/parameter rejected as a sharp error; combined struct+enum recursion detection (a
struct field may be an enum and vice versa, but a value-cycle is a bare, span-less compile
error, never a hang); `.`/`=`/arithmetic on an enum are sharp located errors; a REPL
`<TypeName>` placeholder display; and size-aware carried-stack marshalling generalized
to enum slots across both the word-call and REPL-line boundary. The `examples/shapes.sth`
dogfood (`Shape`'s `Circle`/`Rect` via `area`, `MaybeInt`'s `None`/`Some` via
`unwrap-or`) runs both as a native binary and in the REPL. Generics, `Option<T>`/
`Result<T,E>`, the `_` wildcard, inline `match`, and recursive/heap
data are deferred (Phase 4 / Phase 3).

`(value, type)` slot from day one, concrete types only. Numeric tower (i8..i64,
u8..u64, f32/f64; `*/` widening primitive; literal defaults). Records/structs, enums/ADTs, exhaustiveness-checked
pattern matching. Non-null pointers + explicit optional type. The **`Copy` vs
linear distinction** as a built-in property of types (primitives Copy; anything
owning a resource linear), so Phase 3 has it to build on. Stack-effect checking now
unifies **type and arity** at branch join points (loops arrive with the loop
primitive in Phase 4). Still heap-free: value types and fixed-size arrays only.
**Exit:** typed programs with structs/enums/match; type and arity errors are sharp
compile errors.
**Dogfood:** a small parser or a fixed-size VM for some toy bytecode.

### Phase 3 — The linear spine  `[XL]`  `[highest novelty: this is the point of the language]`

**Linear** types (use exactly once), not affine: `dup` (a plain int-copy since Phase 0)
becomes the explicit copy **gated on `Copy`**, and `drop` (a plain discard since Phase 0)
becomes the explicit destructor. Move-by-default, use-after-move is an error, and
**forgetting to dispose a linear value is a compile error** caught by the existing
stack-effect check (nothing auto-drops; the destructor runs exactly where you write the
`drop`). Hylo-style mutable value semantics: parameter conventions
(`let`/`inout`/`sink`/`set`) and second-class references (can't be stored, can't escape
scope), so no borrow checker and no lifetimes. Opt-in RC (`Rc`/`Arc`-equivalent). **Heap
arrives here**, under ownership. Resources (fds, later locks) are linear values; `dup` on
them is a compile error, and leaking one is too.
**Exit:** memory-safe heap programs, no GC, deterministic destruction, resources as
linear values that can't be duplicated, and can't be silently forgotten.
**Dogfood:** a program that opens/reads/closes files and manages owned buffers,
with the compiler catching a deliberate double-use.

**Slice plan** (dependency-ordered; each its own brief -> spec -> implement -> review,
same as Phase 2). This absorbs the dissolved Phase 2 Slice 8.

1. **Linear analysis + move-by-default + `dup` gated on `Copy` + explicit `drop`.** ✅ done.
   The core novelty, isolated from heap. Move tracking (a second use of a moved value is a
   located error), `dup`/`over` rejected on a non-`Copy` type, `drop` lowered to a
   destructor call. **Linear, not affine**: no auto-drop, forgetting to dispose is a
   compile error via the existing surplus-value check (Copy and linear handled
   symmetrically). Bootstrap (1a): a **test-only builtin linear primitive** (a drop-spy
   with a print-on-drop destructor tagged by an `i64`) gives the analysis teeth before
   heap exists; it is not user-facing surface and dissolves into an ordinary type once
   `drop` is overridable (destructor bodies in slice 8b, the dissolution itself in slice 8c;
   polymorphic dispatch in Phase 4). Aggregates are in scope via
   destructure-whole (no partial moves): `S>fi` stays consuming and drops the non-extracted
   fields, `S|>fi` is a non-consuming Copy-field peek (forbidden on linear fields), `S<fi`
   drops the overwritten field; the compiler synthesises recursive/tag-dispatched drop
   glue. Deferred: loop-carried linear values across the Slice 6 back-edge (a later slice).
   Dogfood: a deliberate second-use is a compile error, a forgotten value is a compile
   error, and a destructor runs exactly once at its explicit `drop`.
2. **Heap + owning pointer + allocator.** ✅ done. The first linear type with a real
   destructor, spelled `^T`: a single heap cell, not a sized buffer, because slice 3's
   recursive data needs the *indirection* and a growable buffer wants Phase 6's `alloc`
   layer. A fixed-capacity heap buffer composes as `^[u8 N]`, size in the type. `^T` is a
   **compiler-known type constructor, not generics** (one interned entry per concrete
   payload, builtin words checked ad hoc at the call site, exactly as `[T N]` arrays work).
   **Tripwire**: `^` is the *second* such ad-hoc type constructor; a third is the signal that
   the special-casing has become the mechanism and Phase 4's generics should subsume all of
   them. Allocation is a single global allocator, deliberately not parameterized per value,
   since a swappable global is cheap to retrofit later while per-value allocators change every
   value's representation. See [the brief](./docs/phase3-slice2-brief.md) and
   [the spec](./docs/phase3-slice2-spec.md) for the full decision record.
   **Known limitation, and where it will first hurt**: because a cell is linear and slice 1
   rejects linear array elements, there is **no collection of resources** in this slice, and
   the restriction attaches to the array type itself, so nesting does not launder it
   (`^[^i64 4]` is rejected too). Lifting it needs an element-wise drop loop in the
   synthesized destructor. First real pressure is slice 6, if a set of file handles is wanted
   rather than one at a time.
   **Rework expected, two items.** The allocator is a **compiler-emitted shim** wrapping
   `malloc`/`free` (as slice 1's backend emits the drop-spy's `printf` helper), because there
   is no user-facing FFI yet; once Phase 6 lands FFI-to-libc, it should become ordinary bound
   foreign words rather than a backend special case. And the trace's gate is a `getenv` **per
   allocation and per free**, so it sits on the permanent allocator path in release builds,
   not merely a test path; caching it needs a mutable global, which has no precedent in the
   emitter.
   **Registry bundling done post-merge**: `ir.rs` threaded `structs`/`enums`/`arrays`/`cells`
   as four separate references through 7 functions, all passing the identical quartet with
   nothing consuming a subset; bundled into one `Registries` handle (`Copy`, mirroring the
   backend's `Layouts`), which removed every `#[allow(clippy::too_many_arguments)]` in
   `ir.rs` (three were already no-ops, found by removing all of them and reading which
   functions clippy actually flagged). `check.rs`'s four equivalent parameters were *not*
   bundled: `arrays`/`cells` are `&mut` there (interned during checking) while
   `structs`/`enums` are `&`, so there is no single handle, and its highest-arity function
   stays over threshold on its own real parameters regardless.
3. **Recursive/heap data + `isize`.** ✅ done. A type cycle is legal iff every cycle passes
   through at least one `^`, in struct field or enum variant payload position alike (no
   positional restriction); a by-value cycle keeps its existing bare, span-less error. Disposal is
   reversed to free-the-cell-before-dropping-the-payload (uniformity, not a correctness
   requirement) and made pre-order (a node's own fields drop and its cell frees before
   descending). A directly self-recursive type gets one fused iterative destructor loop
   (looping the *last* recursive field in declaration order, recursing any others) instead
   of mutually recursive `cell_drop`/`struct_drop`/`enum_drop` calls, giving verified
   constant-stack disposal at 1M+ nodes under a 1MB stack for that shape at the time; slice
   4 below generalizes the same loop to indirect cycles, `^^Self`, and mutually recursive
   types, closing that limitation within this phase. `isize`, a signed mirror of `usize`.
   **Slice 2's OOM-trap-and-abort decision stays closed, not revisited**: an earlier plan
   assumed this slice would introduce a compiler-known optional/non-null pointer type for a
   failed allocation to return through; there is no such type here or anywhere before Phase
   4's generics (a compiler-synthesized per-payload `Option` is exactly what generics would
   delete), so the allocator has nothing privileged to return and the trap stays. Dogfood:
   `examples/list.sth`, a linked list that builds, walks (sums via a non-consuming or
   consuming pass), and disposes what remains via the fused loop.
   A **zipper** (focus + stored path of one-hole steps) remains a sharper future exercise
   for the recursive drop glue, and the one shape slice 6's second-class references
   provably cannot express, since the path must be stored; not attempted this slice.
4. **Generalized recursive disposal, cycle generalization (the Slice 3 follow-on).** ✅ done.
   Slice 3's fused destructor loop covered only direct self-recursion, on a single type,
   looping the *last* recursive field and recursing the rest. `recursive_loop_field`'s
   exact-match predicate is replaced by `recursive_disposal_path`, a backtracking walk over
   the static type graph (`Registries`) that finds a path of typed steps
   (`Project`/`Unwrap`/`Branch`) from a type back to itself through any composition of
   intervening structs, cells, and enum dispatches, reusing the existing malloc/free and
   loop machinery (no new runtime primitive). This closes all three gaps Slice 3 left open:
   **indirect cycles** through an intervening struct (a wrapper type whose cell payload is a
   *different* type that eventually cycles back), **`^^Self`** (a cell of a cell of the
   enclosing type), and **multi-type cycles** (mutually recursive types, each getting its
   own independent fused loop from its own shape, never calling another's destructor to
   traverse the shared cycle). An enum's independently recursive variants (mutually
   exclusive at runtime, unlike a struct's simultaneously-live fields) all keep their own
   back-edge, not just one. Every value these types can build is still a tree, never an
   actual runtime cycle: `^T` ownership is exclusive (no aliasing), and struct/enum setters
   (`S<fi`) are purely functional (`( S Ti -- S )`, a whole-value transform, never a write
   through a pointer), so disposal never needs a visited-set or double-free guard; the fix
   was detection and loop-codegen reach, not aliasing safety. That stops being true once
   Slice 7's opt-in RC lands, since shared ownership is exactly what makes a real reference
   cycle constructible (and, without a `Weak` type, leak). **Worklist-based disposal for
   branching structures stays moved to Phase 6** (see there): it needs a growable
   pending-pointer structure and a new OOM-during-disposal interaction, neither of which
   this slice's gaps required.
5. **General locals.** ✅ done. `| names |` binding is no longer confined to the top of a
   word or clause body: it is permitted at any point in a body or an `if`/`else` arm,
   popping values off the stack at the point it appears (leftmost name binds the deepest
   value, exactly like the existing entry binding). Extent is the rest of the enclosing
   block, so a name bound in an arm is gone after `end`; no new closing token, since the
   block's existing terminator already marks it. Re-binding a name still in scope and
   binding more values than the frame holds are both located errors (the latter reuses the
   existing needs-N-holds-M underflow shape, with the frame floor context-dependent: a
   word's declared inputs, or a REPL line's current session-stack depth); a linear value
   left unconsumed at its block's terminator is now caught there, naming the scope that
   ended, rather than only at word end. The checker's `Ctx::Word` locals map is gone entirely:
   names now live in an independently threaded `&mut Scope`, evolving as terms are walked
   and saved/restored at block entry/exit — the slice's main structural change. No new IR instruction: a binding
   lowers to a pop off the lowering stack plus an insert into the locals map, truncated at
   block exit, since values are SSA and simply outlive the name; a mid-body binding inside
   a self-tail-recursive arm needs no new header phi, its extent ending at the arm's
   terminator where the back-edge sits. Also reaches the **REPL line**, which had no locals
   at all before (a bare line's checker context carried none) — a line now gains the same
   `| names |` form a word body has, scoped to the line, with the session stack persisting
   across lines while names do not. This reverses the mid-body half of Phase 2 Slice 4's
   "no mid-body binding, no closer: factor a word instead" (see the note there); the
   no-closing-token half stands, unchanged, for the same reason it always did. Dogfood: a
   REPL line that binds a local reaching a value an earlier line left, which could not be
   written before; `examples/vm.sth`'s `run` word now names a `vm-pop` result mid-body in
   its `Add`/`Sub`/`Mul`/`Store` clauses instead of shuffling it into position with
   `swap`/`over`/`rot`.
6. **Second-class references + places + escape checking.** ✅ done. `&`/`&!` prefix borrow
   operators on an aggregate/cell local, a per-place aliasing rule (not a lifetime-tracking
   borrow checker: exclusivity plus escape prevention, no lifetime apparatus), and a
   projection/accessor family (`&T>fi`/`&!T>fi`/`&>`/`&!>`/`&^`/`&!^`) that keeps a reference
   opaque (`IrType::Ptr`, never the referent's own shape) all the way to the backend.
   `@`/`!`/`+!` read/write/increment through a reference, restricted to a `Copy` referent
   (covering a Copy aggregate via `Alloc`+`Blit` as well as a Copy scalar). Escape is closed
   structurally: a reference cannot be stored in a struct field, enum payload, array
   element, or cell payload, cannot appear on an effect's output side, and cannot survive a
   REPL line. A self-tail-call back-edge may carry a reference parameter (or one derived
   from it by projection) but not a reference to a current-scope local, and a branch join's
   borrow-suspension state must agree across both arms.
   **The aliasing rule, which specifying this slice forced into the open**: naming an
   aggregate does not copy it, so two names can denote one region, and taking a `&!` of a
   place another live name denotes is an error whose remedy is `dup`. It fires at the
   *borrow*, never at the naming or at a join, because naming twice is harmless if nothing
   mutates through it, and because forcing a `dup` on a non-hazard would insert exactly the
   copy this language refuses to insert implicitly (instruction counts stay readable off the
   source for worst-case-timing work). The routes are naming, a non-consuming peek (`S|>fi`),
   the consuming getter of an aggregate field (`S>fi`, whose lowering pushes the field's
   interior address exactly as the peek's does), `over` (which reuses its operand rather
   than deep-copying like `dup`), and an `if`/`else` merge. A merge is why a value carries
   a *set* of regions interned behind an
   `AliasSetId` rather than a single region: the merge unions both arms, so a projection out
   of it projects the field out of every member and the borrow check tests pairwise overlap.
   The rule is Copy-only by construction: every route to a linear aggregate is already closed
   by move tracking, the peek's linear-field rejection, and `fill`'s `Copy` gate, so the
   failure mode was a wrong *value*, never a double free.
   Reference-mode clause elimination
   (a word whose declared top input is `&Enum`/`&!Enum`) binds each clause's payload as a
   reference inheriting the scrutinee's mutability, exempt from the disjoint-borrow
   limitation below since a variant's fields are statically known to be disjoint. Dogfood:
   `examples/refs.sth` — in-place mutation of an owned buffer through a `&!` reference with
   no rebuild (no `alloc`/`blit` in the emitted body), and `walk ( &!List -- )` mutating
   every node of a list in constant stack via reference-mode dispatch.
   **Known limitation, stated rather than modeled**: path disjointness. Two references
   derived from the same local conflict even when they project into disjoint fields, if
   both are simultaneously live; the workaround is sequencing (fully consume the first
   before taking the second), which mid-body binding (slice 5) makes free of `swap`.
   **Design question this slice's brief asked, answered (R15):** `inout` projections into
   nested fields **do** subsume a reified take/fill pair (`S/fi` yielding a residual
   `∂S/∂fi`, refilled exactly once) for every statically known path — a projection is the
   same residual made implicit and lexically bounded, and it also covers whole-value
   borrows. No residual form was added. Reified residuals remain worth having only where
   the focus must escape, which is a later slice's zipper; escape prevention forbids storing
   a reference, so the zipper waits for that slice's RC rather than for a residual type.
7. **Opt-in RC (`Rc`/`Arc`-equivalent).** **Deferred to Phase 6**, taking the stdlib-home
   escape hatch this entry always carried. It is not named in Phase 3's exit criteria, no
   current dogfood needs shared ownership, second-class references already cover sharing
   within a dynamic extent, and an arena-plus-index owning container covers graph-shaped data
   without it. It is also the one deliberate crack in the linear spine (refcount traffic, and
   cycle leaks without a `Weak`), which sits badly mid-phase in the slice whose point is
   nailing down deterministic linear disposal. In Phase 6 it lands beside `Box`/`Vec`/`Map`/
   `String`, which is the coherent home: it is a way to point at heap data, not a way to
   dispose of it.
8. **Resources as linear values (fds, hosted) + user-definable destructor bodies.** Split in
   two, because the two mechanisms are orthogonal and `close` needs to *exist* before the
   destructor mechanism can be designed against it (this entry always wanted "two dissimilar
   real clients": `free`, pointer + size, from slice 2, and `close`, an integer handle that can
   fail, from here).

   **8a — typed foreign calls + string slices. ✅ done** (brief + spec:
   `docs/phase3-slice8a-spec.md`). One `extern:` declaration form (a C symbol plus a stack
   effect) instead of per-syscall compiler builtins, so every future hosted call is library
   code. This is not new machinery so much as user-facing access to machinery the backend
   already uses six times over (`malloc`, `free`, `printf`, `dprintf`, `exit`, `getenv` are
   all already called by name). An untyped generic syscall word was considered and rejected:
   it would force `Ptr[T]` to an integer, breaking the backend-neutral invariant the WASM
   lowering depends on, and syscall numbers are neither OS- nor arch-portable. String slices
   land here because there are none today (no `Token::Str` exists; `"hi"` lexes as a word),
   which means **this phase's stated exit criterion was unreachable as written** until they
   do. `str`/`cstr` per DESIGN.md's Memory model; buffer slicing stays out (see DESIGN.md Open
   / deferred). **Exit criterion amended during implementation**: the original wording ("a
   foreign call declared in Sooth, taking a literal `str` and a reference, running") is
   unmeetable as written, since `str` is rejected at every `extern:` boundary (a descriptor
   handle matches no C prototype, R2/R3) — so the exit is a foreign call declared in Sooth,
   taking a literal `str` **converted with `cstr`**, and a reference, running;
   `examples/strings.sth` dogfoods it.

   **8b — resources and user destructor bodies. ✅ done** (brief + spec:
   `docs/phase3-slice8b-spec.md`). The Phase 3 exit dogfood: open/read/close a
   file, with the compiler catching a deliberate double-use and a forgotten `close`. **This is
   where a user can first attach their own cleanup code to a type**, rather than only
   inheriting disposal by composition. It needs *no new declaration form*: a user destructor
   is an overload of `drop` for a concrete type, and defining one forces that type linear
   (a struct holding one `i64` would otherwise be `Copy`), which is the same `Copy`/destructor
   exclusion Rust enforces as E0184. That makes `drop` the first overloaded-by-input-type word,
   a miniature early instance of Phase 4's planned ad-hoc dispatch rather than a parallel
   mechanism. The two questions this entry parked are answered: the body runs **instead of**
   the synthesized field glue ("nothing auto-drops" already makes it answerable for its own
   fields through the ordinary must-consume rule, whereas running both would double-dispose),
   and self-recursion is closed not by rejecting a bare direct self-call but by whole-program
   call-graph reachability — any cycle back to `T`'s own `drop`, including through helper
   words — generalizing the same tail-cycle-detection shape Slice 6's mutual-tail-recursion
   check already established, with `T>` destructure as the remedy either way. **Unifying
   `Type::Spy`'s hardcoded drop dispatch into this same table was considered during spec review
   and cut entirely**, not implemented: it would only have delivered a behavior-preserving
   refactor of code 8c deletes outright anyway, so `IrType::Spy`'s hardcoded arm
   (`src/ir.rs`'s `emit_drop`) is untouched here and stays fully in 8c's scope. Note this is
   destructor *bodies* only; `drop` becoming fully polymorphic is still Phase 4.

   **8c — retire `__spy`. ✅ done.** Once 8b's mechanism was proven, the Slice 1 bootstrap
   primitive was fully redundant: every property it existed for (linear-by-declaration, `dup`
   rejection, drop dispatch propagating through struct/enum/array/cell nesting, extern-boundary
   rejection) is now expressible with an ordinary `type:` plus a user `drop` overload —
   `type: Spy tag i64 ; : drop ( Spy -- ) | s | "drop " . s Spy>tag . ;` reproduces the old
   primitive's exact runtime trace byte-for-byte. `Type::Spy`/`IrType::Spy` and every hardcoded
   match arm across `ast.rs`/`check.rs`/`ir.rs`/`src/backend/qbe.rs` — the builtin-table entry,
   `is_copy`'s special case, and the synthesized native trace stub (`sooth_spy_drop`, a
   compiler-emitted `printf` shim) — are deleted, found via Rust's exhaustiveness check rather
   than a manual audit, the same technique that closed the carried-slot bug in 8a. ~280 call
   sites migrated across `tests/phase0.rs`, `tests/phase1.rs`, `tests/phase3_locals.rs`,
   `tests/phase3_refs.rs`, `tests/phase3_resources.rs`, and in-crate unit tests in
   `check.rs`/`ir.rs`/`parser.rs`/`src/backend/qbe.rs`, onto a small locally-defined resource
   type; all six integration-test binaries kept identical counts to before, and the five unit
   tests deleted outright (rather than migrated) each tested only the deleted primitive's own
   bootstrap machinery, with no coverage gap left behind. No design decisions (built directly on
   an async implementation pass plus one fresh-context review pass, skipping the brief/spec/
   multi-round-review pipeline 8a/8b used, since this slice carried migration blast radius, not
   design risk).

### Phase 4 — Minimal polymorphism + quotations  `[L]`

Not full HM inference. Type variables (`'T`) and a row variable (`..s`) so the
monomorphic Phase 0 shuffles (`dup`/`swap`/`over`/`rot`/`drop`), plus `max` and user
words, gain honest polymorphic signatures; monomorphise
per concrete stack shape, force-inline the small core words. Required operations
(e.g. `>` for `max`) resolved at the concrete instantiation, Kitten-style, no formal
trait system. When such a required operation is a total order over **floats**, this is the
point to decide the float total-ordering story deferred from the floats slice (float `<`/`=`
are IEEE-partial; a `max`/sort over floats needs an explicit total order, Rust-`total_cmp`
style, surfaced at the call site rather than pretending IEEE ordering is total). **Quotations** (`[ ... ]` + `call`) as the sole iteration primitive,
plus the **internal loop primitive** they compile down to for constant-stack
iteration. Combinators (`each`/`map`/`filter`/`fold`/`while`/`times`) are ordinary
**library words** written in Sooth on top of quotations, with the compiler inlining
the common ones and their quotation arguments at the call site so they lower to tight
loops rather than a `call` per element. Quotations then become genuine first-class code
values in slice 7 (functions as values: downward closures on a frame-local environment,
upward ones on a `^` cell), which is what makes the functional style the combinators
imply actually writable. With quotations in hand, `if`
is redefined as an ordinary combinator (`cond [ then ] [ else ] if`, Factor-style) and
stops being a keyword, and a `cond` multi-way combinator lands alongside the others.

**Dispatch and uniformity (bundled here on purpose).** Several deferred ideas are one
conversation, and none is clean without quotations, so they land together in this phase:
(a) **`if` becomes an ordinary combinator** over quotations (above); (b) **generics /
minimal polymorphism** (above); (c) **ad-hoc dispatch** as static **overloading** (one
word name, several statically-known input types, e.g. `+` over `i64`/`f64`/`Vec2`); and (d) **`Bool` as a library enum**
(`type: Bool | False | True ;`) rather than a primitive. (d) waits for here specifically
because bool's specialness is not the *type* but that strict, quotation-less two-way
branching needs inline syntax; once `if` is a quotation combinator, `Bool`-as-enum +
`if`-as-word unify, and only then does making it a library type avoid re-adding special
cases (at `if`, at the `True`/`False` literals, and at type-directed printing). Slice 4's
clause-style match and `if/else/end` are deliberately designed not to foreclose any of
this: clause structure maps 1:1 onto a future quotation arm-table, and `if` staying a
keyword for now is the honest strict-eval choice, not a commitment.

**Exit:** polymorphic `dup`/`swap`/`max`; a constant-stack `each`/`fold` over a
collection; combinators verified to inline to loops, not per-element calls; and a quotation
held as a value, invoked through a path the compiler did not inline (slice 7).
**Dogfood:** write the combinator library (`each`/`map`/`fold`/`while`) in Sooth
itself, then rewrite an earlier program to use it; and rewrite `examples/vm.sth`'s dispatch
around a table of quotations.

**Slice plan** (dependency-ordered; each its own brief -> spec -> implement -> review,
same as Phases 2 and 3). None of these is a locked spec yet. The dispatch-and-uniformity
bundle above is **one design conversation, not one delivery unit**: settle the story
whole, land it in order. Phase 3's evidence for slicing rather than one big push is that
its Slice 8 had to split into 8a/8b/8c mid-flight (`close` had to exist before the
destructor mechanism could be designed against it) and its Slice 3 shipped covering only
direct self-recursion, needing Slice 4 as a follow-on — both boundaries discovered late,
under load. Process is calibrated per slice, not uniformly: slices 1–7 carry real design
risk and want full specs, while 9 is a mechanical migration that can run 8c-style (no
spec, one implementation pass plus one review).

**The paper pre-check that shaped this plan.** Before committing to the order, the four
headline combinators (`each`/`fold`/`filter`/`while`) were hand-written against the
planned feature set to see what they actually need. The exercise inverted the expected
answer: quotation *capture*, the question that looked hardest, turned out to be a
quality-of-life issue (see slice 4), while two things absent from the plan turned out to
be load-bearing: length polymorphism (now slice 1) and generic struct declarations
(originally slice 3, since moved to Phase 6 once slice 1's synthesized return bundles
removed `filter`'s need for it). Same technique as `vm.sth`, which shipped with zero
compiler changes and made that fact the Phase 2 exit verdict: write the program first,
then find out what the compiler owes it.

1. **Type variables + row variables + length variables + monomorphization (native).** The
   deepest change, and first for that reason: `'T` and `..s` mean a `Sig` stops being purely
   concrete, so unification and substitution touch every signature-checking path (blast
   radius comparable to Phase 2 Slice 1's typed-core spine, which is the closest precedent
   for how invasive it is). Monomorphise per concrete stack shape. Required operations (`>`
   for `max`) resolve at the concrete instantiation, Kitten-style, no formal trait system.
   **Native only — the REPL is slice 2**, split off because this slice already carries more
   than 8b did and 8b was Phase 3's largest, needing three review rounds precisely because it
   did the native and REPL halves together.
   **No inlining here** (see slice 6). The phrase "force-inline the small core words" that
   this entry used to carry is probably already satisfied and costs nothing: `dup`/`swap`/`+`
   and friends are match arms in `lower_call` that never emit an `Instr::Call`, and
   `check_shuffle` already dispatches on the concrete operand type, so "honest polymorphic
   signatures" may be a checker-side change with no lowering work at all. The brief should
   confirm that; if it holds, this slice adds no inlining machinery of any kind.
   Dogfood: rewrite existing examples to use polymorphic words, rather than adding a new
   example. The exit criterion ("polymorphic `dup`/`swap`/`max`") is a test, not a program,
   and the existing corpus is the honest measure of whether the generics are usable.
   **A length variable (`'N`) is required, not optional** — the pre-check's main finding.
   An array's length is part of its type, so `[i64 8]` and `[i64 4]` are distinct and a word
   taking one rejects the other, while the builtin `len` accepts both: the compiler already
   does length-polymorphism by hand and user words cannot. Since the only Phase 4 collection
   is the fixed-size array (`Vec` is Phase 6), without `'N` every combinator is per-length
   and **the phase exit criterion — a constant-stack `each`/`fold` over a collection — is
   unwritable**. This is also the third hand-rolled ad-hoc polymorphism site after `.` and
   the numeric-tower operators, which is exactly the tripwire slice 2 of Phase 3 set for `^`:
   the third instance is the signal that the special-casing has become the mechanism.
   **The linear spine is what makes this more than textbook generics**: `dup ( 'T -- 'T 'T )`
   is only sound when `'T` is `Copy`, so a *constraint* appears here whether or not one is
   wanted, and the slice has to decide whether `Copy` is an ordinary required-operation
   constraint or a privileged one. The pre-check settles the sub-question of whether
   constraints can be global: they cannot. `each` reads elements through `&> @`, which Slice
   6 restricts to a `Copy` referent, so `each`'s element variable needs the bound — while
   `fold`'s accumulator needs none and may legitimately be linear. **Per-variable, not a
   blanket "generics are Copy-only" rule.** The signature of a polymorphic `drop ( 'T -- )`
   is the same question pointed at 8b's per-type overloads; parked for slice 8 (static
   overloading, which says the same thing from its side), but its answer must not be
   foreclosed here.
   **Closes the multi-output lowering hole**, which stops being deferrable here: a `..s` in
   output position *is* a word with a statically-unknown number of outputs, so row variables
   cannot ship on a lowering path that panics on two. The defect is long-standing and
   independent of this phase; it was noted and passed over three times (the Slice 6 spec, and
   8b's brief and spec, the latter rejecting multi-output `extern:` at the declaration
   precisely to avoid it) and has never had a home. It lands here rather than as a standalone
   warm-up fix because **it is not a separate decision**: "how do two values cross a call
   boundary" and "how does `..s` cross a call boundary" are one question, and answering it
   early, without the constraint that actually stresses it, risks answering it twice and
   inconsistently. A stopgap diagnostic rejecting multi-output calls is also not worth taking,
   since this slice removes it again immediately.
   Two things the brief should start from. **The defect is narrower than "multi-output words
   are broken"**: defining one checks and lowers fine (ten existing tests assert exactly that,
   including `: w ( -- bool bool bool )` lowering successfully), and only the *call* desyncs
   the stack — `lower_call` builds a result only when `out_arity == 1`, `Instr::Call` carries
   an `Option<Value>`, and `env` stores a single `ret_ty`, so there is nowhere to put a second
   output. And **the likely answer already exists in the tree**: aggregate returns work today
   (`vm-pop ( Vm -- VmPop )` in `examples/vm.sth` is a shipped word returning a struct), so
   multi-output can plausibly synthesize the bundling users currently do by hand, against
   machinery that is already load-bearing. Out-parameters and a carried stack are the
   alternatives to weigh against it.
   **The float total-ordering decision, deferred from the floats slice, lands here**: float
   `<`/`=` are IEEE-partial, so a `max`/sort over floats needs an explicit total order
   (Rust-`total_cmp` style) surfaced at the call site rather than pretending IEEE ordering
   is total.
2. **REPL monomorphization.** Split from slice 1 to keep the phase's risk out of one commit,
   and placed immediately after so no later slice builds on a REPL that cannot see a
   polymorphic word. **The problem is retention.** `Session` keeps signatures in `env` but
   discards ordinary word *bodies* once their line compiles to a `.so`; the only bodies it
   retains are `drop_overloads`, which 8b added. A polymorphic word has no concrete
   instantiation to compile at its defining line, and a later line's `5 id` needs `id@i64`
   lowered into *that* line's `.so`, from a body the session threw away. So this generalizes
   8b's retention scheme from "drop overrides" to "every polymorphic word" — and inherits its
   hazards, which are documented in place and worth reading before designing: the stale-env
   problem that `drop_dropped_sites` caches around (re-checking an earlier body against a
   later line's env), and `RTLD_GLOBAL` symbol collision when two lines instantiate the same
   word at the same type, which is what the epoch-suffixing machinery already solves for
   destructors. A further collision the brief traced directly (`docs/phase4-slice2-brief.md`):
   `instantiation_symbol` (S1) is a pure function of word name and substitution with no
   generation component, so redefining a polymorphic word and re-instantiating it at a type it
   was already instantiated at mints the same symbol as the old body and silently keeps running
   it under `RTLD_GLOBAL`'s first-loaded-wins rule. And one binding question the brief settles
   rather than leaves open: an instantiation lowers against the resolver live at the word's own
   *defining* line, not the instantiating line's, matching the frozen-binding rule every other
   REPL word already follows (see DESIGN.md's Open / deferred: REPL late binding, for the larger
   live-patching question this brushed against and deferred rather than decided here).
3. **Aggregate-return aliasing: the loop-carried copy (fixed).** A self-tail-recursive
   loop lowers to header phis rather than a new frame each iteration, so any aggregate
   *storage reused across iterations* that is carried across the back-edge could alias:
   a value from iteration *k* pointed at the same slot iteration *k+1* overwrote, silently
   becoming the wrong value with no diagnostic. A by-value aggregate return (`%r =:T call
   $f()`, one QBE stack slot per call site) was the common instance and the reason this
   became urgent, but it was one instance of the mechanism, not the mechanism itself: an
   aggregate constructed inline each iteration, with no call at all, reused its
   entry-hoisted storage and reproduced the identical bug. Fixed by giving each
   loop-carried aggregate slot one entry-hoisted stable stack slot (no header phi for it)
   and an unconditional read-before-write staged blit on the back-edge, immune by
   construction to both a swap (two carried slots exchanging places) and an interior
   pointer into a carried slot (`field_value`'s `PtrOffset`), with no aliasing analysis.
   Scalars, references, ordinary (non-loop) join phis, and the fused destructor loops'
   own lowering are unchanged.
   **This displaced generic struct declarations from the slot** (moved to Phase 6, see
   there). That slice's whole claim to not being speculative structure was one named
   consumer, `filter` needing to bundle a filtered array with a count, and slice 1's
   synthesized multi-output return bundles closed that need: `: pass-through ( [i64 'N] --
   [i64 'N] usize ) len ;` is `filter`'s exact shape and compiles and runs at two different
   lengths today, verified against the built compiler.
4. **Quotations + the internal loop primitive.** `[ ... ]` + `call`, plus the loop
   primitive they compile down to for constant-stack iteration, plus call-site inlining.
   **Scoped by `docs/phase4-slice4-brief.md`; the decisions below are settled, not open.**
   The floor is one compiler-known intrinsic, `times ( ..s i64 [ ..s i64 -- ..s ] -- ..s )`,
   passing the iteration index: its body quotation returns the row it received, so effect
   inference only ever unifies an inner row against itself. `while` was weighed as a second
   floor member (DESIGN.md:285 allows "one or two") and declined here, because its condition
   quotation returns a `bool` on a passthrough row, which is strictly harder inference.
   The slice ships a runnable constant-stack loop rather than inert plumbing, so the phase's
   riskiest integration (type machinery against quotations against the loop primitive) gets
   a witness in the slice that builds it; the headline is `0 1000000 [ + ] times .` printing
   `499999500000` in constant stack, next to `examples/countdown.sth`'s hand-threaded
   self-recursive equivalent. The floor is permanent, not a bootstrap: DESIGN.md:281-289
   makes the loop primitive internal ("not surface syntax, not user-facing") and the thin
   intrinsic floor user-facing by design, so slice 6 builds on `times` rather than retiring
   it. A quotation here is a **compile-time marker** carrying its inferred effect and body,
   fused at its `call`, never a runtime value: that defers the `Type`/`PolyType`/`IrType`/
   unification/mangling change to slice 7, where a consumer for it finally exists. The two
   "inlining"s the plan used to seem to double-book are different mechanisms and both are
   real: slice 4 owns **quotation-literal fusion** (splicing a literal's body at its `call`,
   never crossing a `:` word boundary), slice 6 owns the **interprocedural user-word
   inliner**.
   The brief's sharpest finding is what it rules *out* of this slice: a polymorphic word
   today can neither branch (`if` is rejected in a polymorphic body, `src/check.rs:2997`)
   nor loop in constant stack (`self_tail` is hardcoded `false` on the polymorphic
   instantiation path, `src/ir.rs:1176`), so combinators cannot be written as ordinary
   polymorphic library words at all. *This paragraph used to end "both gaps land in slice 6
   against their first real consumers"; slice 6b's paper pre-check falsified that by building
   the programs. Neither gap gates a combinator, because a combinator is checked by
   term-splicing at the concrete call site and never goes through `poly_term` at all. Both
   gaps survived 6a-6c untouched: the polymorphic `if` is now 6e, and the `self_tail`
   hardcode is unreachable until a polymorphic body can call a polymorphic word (slice 7).*
   After slice 1 because a combinator's signature has to *say* `[ 'a -- 'b ]`, which needs
   row variables to be expressible at all. **Capture is a quality-of-life question, not a
   soundness one** — downgraded by the pre-check, which expected the opposite. The fear was
   that a quotation capturing a linear value and called twice would dispose it twice, the
   split Rust spells `FnOnce`/`FnMut`/`Fn`. But `fold` and `while` need no capture at all:
   loop state rides the accumulator and the stack respectively, which is the concatenative
   structure doing the work. Capture is wanted only for read-only *ambient* context that is
   not loop state (`| threshold | arr [ threshold > ] each`), and even that has an
   expressible if ugly fallback in bundling the context into the accumulator. So the library
   is writable non-capturing, and the brief decides on ergonomics with a working fallback in
   hand rather than under threat. The likely resolution to reach for: capture is free
   precisely when the quotation is inlined, since the captured local is simply still in
   scope after inlining — no environment, no allocation, and no `FnOnce`/`FnMut`/`Fn` split
   if capture is restricted to `Copy` locals. Escaping quotations stay out of scope
   regardless — they need the uniform-runtime-stack fallback and Phase 6's alloc layer.
5. **Modules: multi-file compilation, word and type imports.** Pulled forward from Phase 6
   for two converging reasons, not one. Slice 6's combinator library needs somewhere to live
   besides copy-pasted into every dogfood — the original motivation — but the bigger one
   surfaced at this slice's brief conversation: a reusable component worth writing is rarely
   just words. A queue, a small `Result`-like type, a `Point`/`Vec2` bundle — the type is
   usually the component, the words around it are secondary. Word-only imports would serve
   the combinator library and almost nothing else worth calling a library, so this slice
   takes the harder half too: a file becomes a compilation unit, and an import brings
   another file's words *and* struct/enum declarations into scope by qualified name.
   **Split native/REPL, 8a-style**, on the precedent that already exists twice in this
   project: slice 1 held REPL monomorphization back to slice 2 rather than carry both, and
   Phase 3's 8b needed three review rounds precisely because it did the native and REPL
   halves together. The REPL has no file-loading path at all today and carries session state
   the native path does not (`drop_overloads` keyed by `StructId`, frozen resolver snapshots,
   per-name generations, every `.so` resident under `RTLD_GLOBAL`), so what an import *means*
   there is a separate design problem, not the same one repeated.
   **This settles the open design question Phase 6 had named**: how struct/enum
   declarations from separate files join one shared type registry without a name collision
   or a duplicate registration. Type resolution, move-tracking, and the R21 aliasing rule
   are value-flow-based, not file-based, so none of that checker machinery should need to
   know a boundary was crossed. But the *front* of the pipeline is not additive, and the
   brief should start from that: `driver::build` today is straight-line `read_to_string` ->
   `lex` -> `parse` -> `check` -> `lower` -> `emit`, and `check::check(&mut module)` takes
   exactly one module, so something has to locate imports, parse N files, reject import
   cycles, and merge before checking runs.
   **Encapsulation lands here too, because without it a type cannot hold an invariant.**
   A `type:` generates a constructor, getter, peek, setter, and destructure word per field,
   all ordinary words in the flat environment, so with no visibility rule an imported
   `Queue` hands every consumer `Queue<back` and the invariant it was written to protect is
   gone. Default private, with a per-file `export:` form naming what crosses. Because the
   generated accessors are just words, that list gives per-word granularity for free (export
   the type and its operations, withhold the raw setter) with no new visibility algebra.
   Elm's opaque types are the precedent and answer the one subtle question: exposing a type
   *name* and exposing its *constructors* are separable exports, and that separation is the
   whole encapsulation mechanism.
   Syntax proposal, to be confirmed at the brief: `import: queue "collections/queue.sth" ;`
   and `export: Queue push pop ;`, following the existing `name: ... ;` defining-word family,
   with access as a single token `queue::push`. `::` because `.` is the print word and `>`
   is already the generated field getter (`queue>push` would read as a getter on a type
   named `queue`). Qualified-only, so there is no cross-module collision rule to invent. A
   file with no `export:` exports nothing: it is a program, not a library, which keeps every
   existing single-file example valid.
   **Still narrower than Phase 6's eventual module system.** No serializable API description
   and no version diffing. Those are a packaging/publishing concern — letting other people
   depend on you with enforced semver — not a personal-reuse one, and
   `docs/dependency-management.md` still depends on Phase 6 for them; it consumes the export
   list this slice introduces rather than defining it.
   **Disposal crosses the export boundary, which constrains the export rule.** Two
   consequences of linearity meeting encapsulation, both wanting settled at this slice's
   brief. First, a destructor runs without being named, so an unexported `close` still
   executes in a consumer's program through the generated glue: the export list describes
   what a consumer can *call*, not what can *run*. Second, and sharper: whenever a type's
   disposal is not reachable through `drop` alone (see slice 8's constraint), an exported
   opaque linear type must also export the word that discharges its obligation, or a
   consumer who obtains one is stuck holding a value it cannot legally consume — unusable
   rather than encapsulated. That is a checkable rule; decide here whether the compiler
   enforces it or it stays the library author's job.

   **5a — native multi-file compilation, imports, and encapsulation.** Everything above, on
   the native path only. **`import:` at the REPL must be a clean located error in this
   slice, specified and tested, not left to whatever the parser happens to do.** This is the
   one lesson 5b cannot be trusted to carry: slice 1 shipped without pinning REPL behaviour
   for polymorphic words and slice 2's recon found the gap had produced a *silent
   miscompile* (a bogus `( -- )` mismatch, or a silent `defined` that never checked the
   body). An unimplemented feature that rejects is a deferral; one that quietly does the
   wrong thing is a defect, and the difference is one specified diagnostic.
   Open questions for its brief beyond the pipeline shape above: whether a qualified name
   becomes part of the `StructId` key and what that does to existing interning (structs are
   nominal, so two files each declaring `Point` collide); whether 8b's epoch/generation
   destructor-symbol scheme stays collision-free once two modules synthesize destructors for
   same-named types; and where privacy is enforced, since filtering unexported names at
   splice time is the simplest implementation but reports "unknown word" where it should say
   "`grow` is not exported by `queue`", and diagnostics are behaviour here.
   **Exit:** two files, one importing a type and a word from the other, compiling and
   linking as one program, with a non-exported word rejected at a use site in the second and
   `import:` at the REPL rejected with a located error.

   **5b — imports at the REPL.** What an import means in a session: whether a line may
   `import:` at all or only a loaded file may, how an imported module interacts with
   generation-mangled redefinition, whether re-importing after an edit reloads or is frozen
   the way every other REPL binding is frozen (DESIGN.md's deferred REPL late-binding
   question is adjacent and should not be reopened here by accident), and whether the export
   list is enforced against REPL lines or a session sees everything. Sized at its own brief;
   the honest possibility is that the frozen-binding rule makes this smaller than it looks,
   since an imported file is just more words entering the env at a known generation.
   **Exit:** a REPL session importing a file and calling an exported word from it, with the
   frozen-binding rule holding across a redefinition.

   **Dogfood (5a):** the combinator library (slice 6) lives in its own file; a small
   standalone type (e.g. a `Point`/`Vec2` or a stack-like struct) lives in another, exported
   opaquely, and is imported by an example that uses both.
6. **The combinator library in Sooth + inlining, and the machinery that work measured (the
   phase's headline exit).** *This heading used to end "+ closing the polymorphic-path gaps".
   It did not close them and no longer claims to: 6b's pre-check established that neither gap
   gates a combinator. 6a-6c are the library itself; 6d and 6e are gaps the library work
   surfaced but does not itself need, grouped here for ordering rather than subject matter.*
   `each`/`map`/`filter`/`fold`/`while` as ordinary library words over quotations,
   with the compiler inlining the common ones and their quotation arguments so they lower to
   tight loops rather than a `call` per element. Depends on 1–5a (5a is where the library
   gets its own file to live in instead of every consumer's; 5b is not a prerequisite), and
   comes *before* the dispatch slices deliberately: it is the first real integration test of the type machinery
   against quotations, so if the two are awkward together, that feedback should arrive before
   three more mechanisms are built on top. The paper pre-check is not a substitute for this,
   only a filter on the plan.
   **Split in three, on fault lines a paper pre-check against the built compiler measured**
   (findings below, all verified by compiling, not read off this document). `each`/`map`/
   `fold` are expressible over `times` alone and need none of the polymorphic-path machinery
   `filter` and `while` need; and the REPL half is its own design problem, exactly as it was
   at slice 1/2 and 5a/5b. Separate them rather than risk a slice splitting mid-flight the
   way Phase 3's 8 did.
   **What the pre-check overturned:** the combinator library is not "writable but slow"
   today, it is **not expressible at all**, so the inliner is the enabling mechanism rather
   than an optimization laid over a working library. Two independent walls, both measured: a
   quotation type cannot be spelled in a signature (`( i64 [ i64 -- i64 ] -- i64 )` fails in
   the *parser*, since `[ ... ]` is already the array-type syntax and the count must be a
   literal), and passing a quotation to any user word, monomorphic or polymorphic, is a hard
   located rejection. What the inliner must *produce*, by contrast, is fully expressible:
   hand-inlined `each`, `fold`, and `map` over a `[i64 4]` (a `times` loop, `&a i &> @`
   reads, `&!b i &!> v !` writes) compile and run correctly today, and the length/element
   polymorphism they need is already there (`: size ( ['T 'N] -- ['T 'N] usize ) len ;` runs
   at `[i64 4]` and `[f64 7]`). The gap is entirely front-end.

   **6a — quotation types in signatures + the inliner + `each`/`map`/`fold`** (native).
   **This slice owns the inliner, and it is the only one** — there is no inlining anywhere in
   the compiler today. Everything the source calls "inline" means "lowered straight to
   instructions rather than a call" (builtins and generated struct/enum words are match arms
   in `lower_call` that never emit `Instr::Call`); a user `:` word is always a real call.
   **Nothing downstream will do it either**: QBE is a per-function backend and emits
   `callq` even for a one-instruction function called with a constant (verified directly),
   and the driver runs `cc` on a `.s` with no `-O` flags, so that is only the assembler. The
   pass lands here rather than with monomorphization because combinators are *ordinary Sooth
   library words*, so inlining them is inlining monomorphized user words — there is one
   mechanism either way, and it should be designed against its first real consumer instead of
   two slices ahead of one. That library-words commitment is what forces the pass at all: the
   alternative is making combinators compiler-known and lowering them as match arms, cheaper
   but forfeiting the thing the phase is trying to prove, in the same way `vm.sth` shipping
   with zero compiler changes *was* the Phase 2 exit verdict.
   **A quotation becomes nameable in a signature here, and that is decided, not open.** For
   `: each ( ['T 'N] [ 'T -- ] -- )` to be checkable standalone, `Type`/`PolyType` gain a
   quotation variant carrying a declared effect, with unification, `apply_subst`, and
   mangling following, and `call`/`times` must accept an **abstract** quotation — one whose
   effect is known only from a declared parameter type — instead of demanding a
   statically-known literal as they do today (`src/check.rs:4763,4797`). New surface syntax
   is needed for the spelling, since `[ ... -- ... ]` collides with the array type. This is a
   slice-1-shaped change and the reason 6a is `hard`, **but it stops at the type**: no
   `IrType` variant, no calling convention, no per-literal environment struct, no `(code,
   env)` representation, no escape rules, no quotation in an array, struct field, or branch
   join. Every consumption site still resolves to a known literal, so lowering is untouched.
   Slice 7 keeps the representation half and gets cheaper by exactly what this pre-pays.
   Not split off into its own slice precisely because a quotation type with no inliner
   compiles no call site at all: it would be inert plumbing, which slice 4 refused on the
   same grounds, and "what is a quotation parameter's type" and "what happens at a call site
   that must inline" are one question in the way row variables and multi-output returns were.
   **Two constraints that fall out and belong in the brief.** Inlining stops being an
   optimization and becomes **total**: with a quotation type but no runtime value there is no
   fallback, so every call to a quotation-taking word must inline, transitively (`map` over
   `each` inlines twice), and anything un-inlinable — recursion among quotation-taking words,
   for one — must be a located error, never a silent real call. And the new variant must not
   bake in "a quotation is always statically known": keep that a predicate on the value, or
   slice 7 has to unpick it out of unification and the monomorphization walk to allow a
   genuine runtime closure.
   **The REPL is 6c, and 6a must pin its behaviour with a located rejection**, specified and
   tested — the 5a lesson, which slice 2's recon proved the hard way when slice 1 left REPL
   polymorphic words unpinned and the gap turned out to be a silent miscompile. The checker
   sees only signatures at a call site (`env: &HashMap<String, Sig>`), never bodies, so an
   inliner needs bodies threaded in; at the REPL bodies are discarded except drop overrides
   and `poly_words`, which is a retention problem, not the same problem repeated.
   Needs none of 6b's checker fixes, since `each`/`fold`/`map` need no branch.
   Accepts one known churn cost — the library is written against keyword `if` and gets
   rewritten when slice 9 turns `if` into a word — which is a handful of library words,
   cheaper than reordering the phase around it. **Exit:** `each`/`map`/`fold` written in
   Sooth over `times`, verified to inline to a tight loop rather than a `call` per element,
   with a quotation-taking word at the REPL a located rejection.
   Dogfood: rewrite an earlier program to use `each`/`map`/`fold`.

   **6b — `filter`/`while`, and the self-tail combinator loop.** *This entry previously said
   the slice closes the two "polymorphic-path gaps" slice 4's brief measured. Its own paper
   pre-check falsified that, by building the programs (see
   `docs/phase4-slice6b-brief.md`); the charter below is the corrected one.*
   Neither named gap is what `filter`/`while` need, and both are left in place. A combinator
   is checked by term-splicing at the *concrete* call site, never through `poly_term`, so the
   polymorphic-`if` rejection never gates one: `filter` compiles and runs today with **no
   compiler change at all**, `if`/`else` and all. And a polymorphic body cannot call any
   polymorphic word, self or other (`unknown word` in `poly_call_term`, long before
   lowering), which makes the `src/ir.rs` poly-instantiation `self_tail` hardcode currently
   *unreachable*: no poly word can self-call to reach it.
   `while`'s actual blocker is 6a's own D5 combinator-cycle rejection ("a quotation-taking
   word cannot be recursive"), which fires identically for a *monomorphic* self-recursive
   combinator, so it is not a polymorphism question at all. The work is therefore: relax D5
   for a self-*tail* combinator edge only (non-tail self-calls and mutual cycles stay hard
   errors, they need slice 7's runtime quotation), and lower that edge to a loop back-edge at
   splice time, reusing the mid-body loop `times` already opens (brief D8; specializing the
   combinator into a monomorphic `IrFunc` was weighed and rejected, since it reopens 6a's
   "inlining is total" and "a combinator mints no symbol"). The tail-vs-non-tail distinction
   is a *checker* change, not just IR work: `check_combinator_cycles` builds its edges from
   `all_calls`, which erases it.
   Ships `while` inheriting the R18 nested-loop limit that `each`/`map`/`fold` already have
   (brief D9); 6d lifts it for all of them at once. Depends on 6a for the inliner and the
   library's file/shape.
   **Exit:** `filter` and `while` written in Sooth and inlined the same way 6a's words are,
   with `while` running in constant stack and a non-tail combinator self-call still rejected.

   **6c — quotation-taking words at the REPL.** Lifts 6a's located rejection: what it means
   to define and call a combinator in a live session. The problem is retention, the same
   shape slice 2 solved for polymorphic words and 8b for drop overrides — a session discards
   ordinary word bodies once a line compiles to its `.so`, and an inliner needs the body —
   plus the frozen-binding question of which generation of a callee an inlined body binds to,
   which slice 2 already answered for instantiations (the *defining* line's resolver
   snapshot, not the instantiating line's) and this should follow rather than reopen. Last
   of the three because the phase exit is a native criterion and nothing else builds on it;
   its order against 6b is free. **Exit:** a session defining a quotation-taking word,
   calling it, and redefining it, with the frozen-binding rule holding across the
   redefinition.
   **6d — nested constant-stack loops (the hoist-target split).** Lifts R18, which today
   rejects any `times` reached while a loop is already open. The limit is not hypothetical
   and not confined to a future `while`: it bites every combinator 6a shipped, because each
   one drives its own `times`. `2 [ | i | mk [ . ] c::each ] times` is a hard error today
   ("a `times` cannot be nested in a loop yet"), so no combinator composes inside a loop,
   and the rejection is a deferral rather than a design decision.
   **The cause is one field doing two jobs.** `FuncBuilder::entry_block` (`src/ir.rs:2226`)
   is simultaneously the alloca home and the loop preheader. It must be the function's true
   entry block for allocation, since QBE's alloca bumps the frame pointer on every execution
   and never reclaims within a function, so an `Alloc` reached per-iteration grows the frame
   until the constant-stack guarantee is worthless. It must be the *loop's* preheader for a
   carried aggregate's stable-slot seeding blit, which has to run once per entry to that
   loop. Those two blocks coincide at exactly one loop level and diverge the moment loops
   nest, which is the whole of the bug. The fix is to split the field: an invariant alloca
   home, and a per-loop preheader. The rest of the loop state (`header`, `carried_slots`,
   `back_edges`) already saves and restores around a nested region (`src/ir.rs:2609-2687`,
   the `times` arm), so the phi bookkeeping is largely present already.
   **Handle with care, and not as a rider on another slice.** This is the same loop-lowering
   code where the aggregate-return aliasing bug landed (see the Phase 4 slice 3 note above),
   whose fix, one entry-hoisted stable slot per carried aggregate plus an unconditional
   read-before-write staged blit on the back-edge, is exactly the invariant that rearranging
   hoist targets can silently break. Its guards want mutation-testing, not just a green run.
   Depends on 6a for its consumers; independent of 6b and 6c, and orderable against either.
   **Exit:** a combinator called inside a `times` body compiles and runs in constant stack,
   with a nested-loop golden and the slice-3 aliasing guards still green.
   **6e — `if` in a polymorphic body.** Lifts the rejection at `src/check.rs:3690` (`` `if`
   in the polymorphic body of `{word}` is not yet supported ``), which has stood since slice
   1 deferred it and which no later slice picked up. **A 6-family letter for ordering and
   discovery, not subject matter:** 6b's pre-check is what measured this gap (while ruling it
   out of 6b's scope), and it has to read before slice 7, which needs it. Unlike 6d, whose
   consumers are the 6a combinators themselves, this one's consumers sit outside the family
   entirely, below.
   **Two consumers, both real, which is the bar this plan keeps applying.** Slice 7: any
   closure-taking word worth writing branches on something. And the core library: a word is
   only a library word if it can be written in Sooth, and `max` cannot be, so it sits in
   `BUILTIN_WORDS` next to `+` as a compiler builtin. `: mymax ( 'T: Copy Ord 'T -- 'T ) over
   over > if drop else swap drop end ;` is rejected today (verified by compiling; its
   monomorphic `i64` twin builds and runs), and the `Ord` bound it needs already exists. That
   makes this the gate on separating the irreducible intrinsics (`+`, the shuffles, `fill`,
   the `>uN` conversions: things that lower to instructions no Sooth source can express) from
   the words that are builtins only because the front end cannot yet express them.
   **Three pieces, and shipping a subset is worse than shipping none** (`docs/phase4-slice1-spec.md:80`,
   which is the only existing plan for this): pop the condition off the `PolyType` stack, run
   a per-arm unconsumed-linear check, and join the two arms' move-state. None of the three is
   lifted to `PolyType` yet. The rejection exists *because* the half-built arm it replaced
   both spuriously rejected valid programs (`: choose ( 'T 'T bool -- 'T ) ...`, whose
   monomorphic sibling builds) and panicked the compiler outright (a `^i64` allocated on one
   arm reaching `ir.rs`'s `drop: non-empty stack`). `max`'s own body is a fair test rather
   than a trivial one: its arms `drop` different operands, so it exercises exactly the
   move-state join that is missing.
   **Not in scope: the quotation-in-a-polymorphic-body rejection** (`src/check.rs:3708`).
   It is a sibling wall, not this one, and it belongs to slice 7, which is where a quotation
   acquires the runtime representation that would let a polymorphic body carry one.
   Depends on slice 1 only; independent of 6a-6d and orderable against all of them.
   **Exit:** a polymorphic word that branches, including one whose arms consume different
   operands, compiles and runs at two instantiations, with the linear checks that motivated
   the original deferral proven by tests that fail without them.
7. **Functions as values: closures.** The slice that makes a quotation a real runtime value
   rather than a compile-time marker, so it can be branched to, stored, returned, and passed
   to something that is not inlined: `cond [ fast ] [ slow ] if call`, a dispatch table as an
   array of quotations, a strategy in a struct field, and genuine non-inlined higher-order
   words. After slice 6a because the combinator library is the consumer that makes the
   calling convention concrete (designing it with no caller is the anti-pattern this plan
   keeps citing). *This entry used to add "and after slice 6b because it lifts the polymorphic
   `if`"; 6b did not, and never claimed to once its pre-check corrected the charter. The
   polymorphic `if` any interesting closure-taking word needs is 6e, which is therefore a real
   prerequisite here.*
   **Most of the machinery already exists, which is why this is a slice and not a phase.**
   The environment of a downward closure (passed in, never returned or stored beyond the
   frame) is an ordinary frame-local aggregate, so it needs no allocator; the escape
   discipline that keeps it sound is Phase 3 Slice 6's structural escape checking, pointed at
   a new carrier, since a closure simply inherits its captures' restrictions. The
   `Fn`/`FnMut`/`FnOnce` split does **not** need inventing: Rust needs it because the real
   question is how the call takes the closure, and Sooth already spells all three, `call`
   through `&q`, through `&!q`, and by value. A closure capturing a linear value is itself
   linear, so dropping it disposes the captures through the existing destructor mechanism.
   **Upward closures are not blocked on Phase 6.** `^T` is already an owning heap pointer
   backed by a real allocator (`src/backend/qbe.rs:672-685`, `malloc` plus an OOM trap) with
   Phase 3's full disposal story, so an escaping closure is `(code pointer, ^Env)`: the
   environment lives in a cell instead of a frame slot and drops through machinery that
   exists. DESIGN.md:512's "a non-escaping quotation is core but an escaping one is `alloc`"
   is a statement about which stdlib layer the feature belongs to, not about missing
   machinery. What is genuinely new is the environment's *type*: each capturing quotation has
   its own capture set, so the compiler synthesizes an env struct per quotation literal, the
   way slice 1 already synthesizes and interns bundle structs for multi-output returns.
   Known limit to take in with open eyes: `^` is single-owner, so a `^`-closure is linear and
   two owners of one callback needs Rc, which stays deferred.
   This also pays the real type cost slice 4 deferred: a quotation must become nameable, so
   `Type`/`PolyType`/`IrType` gain a variant and unification, `apply_subst`, `Subst`,
   `instantiation_symbol` mangling, the monomorphization walk, layout, and the backend all
   follow. That is a slice-1-sized representation change and the second-largest item in the
   phase; slice 4's brief sized it deliberately before deferring it here. Representation:
   one uniform `(code, env)` pair, with a non-capturing quotation carrying an unused env,
   revisited only if the RT subset demands a distinct bare-pointer type (DESIGN.md:480 names
   dynamic dispatch through escaping quotations as a hot-path enemy). Decide at its brief
   whether downward and upward land together; probably yes, since splitting means designing
   the environment layout twice.
   Dogfood: rewrite `examples/vm.sth`'s dispatch around a table of quotations and compare it
   against the enum-plus-clause version it replaces.

8. **Ad-hoc dispatch: static overloading.** One word name, several statically-known input
   types (`+` over `i64`/`f64`/`Vec2`). After slice 1 because a resolution rule defined over
   concrete types is a rule that gets rewritten once type variables exist. **The compiler
   already does this by hand and this slice is where it stops**: the numeric-tower operators
   and `.` (type-directed over any printable scalar) dispatch on the concrete operand type
   inside `check_operator`/`check_term` match arms rather than through any table — which is
   why `builtin_table` is empty. `len`'s length-polymorphism (slice 1) and 8b's `drop`
   overloads are the other such sites; the latter are explicitly parked for here ("`drop`
   becoming fully polymorphic is still Phase 4"), and absorbing them means retiring the
   hardcoded interception arms in `check.rs`/`ir.rs` that currently run before any env lookup.
   **One constraint from the destructor side, which the polymorphic `drop` must not
   violate: it cannot be structurally total.** A generic `drop ( 'T -- )` that accepts a
   resource type discharges the linear obligation while leaking the resource, turning
   today's compile error into a silent leak. `type: File fd i32 ;` is structurally an
   `i32`, so derived disposal pops it and never calls `close`, and the checker sees the
   debt paid. Linearity buys use-exactly-once, not use-correctly. So either `drop` carries
   a bound satisfied only by structurally-derivable types (declared-consumer types rejected
   outright, disposed by their named word), or the resource case resolves through the
   constraint system to that named word. A structural default is not on the menu. Note this
   is a hazard of the *destination*, not of what shipped: 8b's overrides are sound precisely
   because the user body substitutes for the derived glue rather than sitting beside it.
   The residual decision is the container boundary — whether generated traversal of a
   `List[File]` may call `close` implicitly (composability, some ambience) or must be
   hand-written (neither) — probably the former, since that call sits inside
   compiler-generated traversal nobody reads while direct code keeps `close` visible, but
   decide it rather than inherit it. Slice 5's export rule depends on the answer: whatever
   `drop` cannot dispose, a module must export a disposal word for.
   **A sibling hole, measured and pre-existing: destructuring a type bypasses its `drop`
   override entirely.** `type: R tag i64 ;` with a `drop` override, then `r R>tag .`, prints
   the field and never runs the destructor. So today a `File` can have its fd extracted and the
   linear obligation discharged with no `close` and no diagnostic. Rust closes exactly this with
   E0509 (cannot move out of a type implementing `Drop`); Sooth has no such rule. It belongs
   here rather than anywhere earlier because it is the same question as the constraint above,
   asked of a different consumer: *what counts as discharging a linear obligation*, where a
   structural answer silently drops the resource-specific one. Slice 5a's transparent type
   export makes it reachable across a file boundary for the first time (its earlier
   opaque-by-default draft would have papered over it, and only for types whose author chose
   opacity), which is an argument for fixing it properly here rather than with a visibility
   rule. Do not add a partial guard in an earlier slice that would foreclose the general form.
   **A second, unrelated sibling hole, measured while scoping slice 5b: two modules can each
   declare `main` and nothing rejects it.** `mangle` (`src/resolve.rs`) exempts `main`/`drop`
   from module-disambiguating suffixes, and `check_main_effect` (`src/check.rs`) finds the
   *first* word literally named `main` in the whole checked module with no uniqueness check.
   Slice 5a shipped with this latent (a library file has no reason to declare `main`); slice 5b
   closes it on the REPL path it's building (a located rejection of an imported file declaring
   `main`) but leaves the native path unfixed. Whoever picks this up: reject it the same way
   slice 5a already rejects an exported word naming a private type, at the declaration, naming
   both the file and the word.
   **A third sibling hole, measured while designing slice 5b's import-symbol scheme: two
   words of the same name in one file are not rejected by any check and leak a bare
   assembler `symbol already defined` error, native build included.** `check_duplicate_type_names`
   (`src/check.rs`) covers structs/enums only; the word env (`HashMap<String, Sig>` built by
   plain `insert`) silently overwrites on a repeat name, so both `WordDef`s still lower to
   codegen and only the linker notices. Slice 5b's REPL-side import-symbol design inherits
   this pre-existing gap unfixed (an imported closure with a duplicate word name gets the
   same leaky error) rather than patching it as a drive-by. Whoever picks this up: a located
   duplicate-word-name check, parallel to `check_duplicate_type_names`, naming both
   definitions' locations.
9. **`if` as an ordinary combinator + `Bool` as a library enum.** `cond [ then ] [ else ] if`
   Factor-style, `if` stops being a keyword, a multi-way `cond` combinator lands alongside,
   and `type: Bool | False | True ;` replaces the primitive. Last because it is the cleanup
   the other eight enable: it needs quotations (slice 4) for `if` to be a word at all, and
   dispatch (slice 8) so `Bool`'s type-directed printing becomes an ordinary overload instead
   of a re-added special case — which is the whole point of waiting, per the bundle note
   above. Mechanically it is a large migration rather than a design problem: `bool` has been
   in the test suite since Phase 2 Slice 1, so this is 8c-shaped work (delete the special
   cases, let the exhaustiveness checker find the arms, migrate the call sites) and should
   run 8c's lightweight process.

### Phase 5 — Errors as values  `[S]`

Result/Either as an ordinary ADT (mostly free from Phase 2), plus the `?`-style
short-circuit sugar and the convention that fallible words return it. Branch-on-
result codegen, no unwinding. FFI/C error returns map to Result at the (later)
safe-wrapper layer.
**Exit:** Result-based error handling with `?` sugar; no exception/unwind path
exists anywhere.

### Phase 6 — Stdlib and `no_std` layering  `[L]`  `[where it becomes usable for real programs]`

The four layers from DESIGN.md, with boundaries and the allocator *interface* fixed
now even though hosted is built first: **core** (already accreting), **fixed**
(allocation-free fixed-capacity vec/map/string/ringbuffer), **alloc** (growable
Vec/Map/String, Box, opt-in Rc/Arc, escaping closures, bignum, against core's
allocator interface), **hosted** (files, stdio, time, FFI-to-libc via safe
wrappers). Tag every stdlib word with the layer it needs. Escaping closures appear in that
list as a *layer tag*, not as unbuilt work: the feature itself lands in Phase 4 Slice 7 on
`^`, and what belongs here is only the classification that a closure which escapes its
frame needs an allocator present, so it is unavailable to the `fixed` layer.
**Exit:** real hosted programs using libc via safe wrappers; a usable standard
library; the `fixed` layer works with no allocator present.
**Dogfood:** a genuinely useful small tool (a line-oriented text utility, a small
static-site or markdown thing) written entirely in Sooth.

**Modules: what's left after Slice 5.** Phase 4 Slice 5 already pulled the whole
compilation-unit story forward: a file is a compilation unit, and an import brings a word
or a struct/enum declaration across a file boundary by qualified name, landed once writing
a reusable component — usually a type plus its operations — needed somewhere to live
besides copy-pasted into every consumer. `Vec`/`Map`/`String`/`Box`/`Rc`/`Arc` already have
somewhere to live, courtesy of that slice.
Encapsulation went with it: default private, a per-file `export:` list, and the Elm-style
split between exporting a type name and exporting its constructors. So "which words, types,
and externs are public" is already answered, and answered where it had to be, since a type
cannot hold an invariant while its generated setters cross the boundary unchecked.
What's left here is one thing, not two: a **serializable API description**, a compiler pass
that walks the checked AST, filters to the exported declarations Slice 5 already
distinguishes, and emits a file listing every exported signature for the API diff to
compare between versions. That is the remaining prerequisite in
`docs/dependency-management.md`, and it is a packaging/publishing concern (letting other
people depend on you with enforced semver) rather than a personal-reuse one, which is why
it waited.
**Exit:** a published package's API diff correctly classifies a PATCH/MINOR/MAJOR bump
across a two-file change.
**Dogfood:** `sooth publish --check` on a two-version bump of a small library, one that adds
a word (MINOR) and one that removes one (MAJOR).

**Accessors as lenses, and it lands before the stdlib.** Retire the per-field generated
accessor words in favour of separating the *location* from the *operation*: `q buf &>`
instead of `q Queue&>buf`, matching how arrays already read (`l 0 &>`). See DESIGN.md's
Open / deferred for the full case; the short version is that `>` / `<` / `|>` currently
conflate which field with what ownership transfer happens, lenses separate them, and the
generated-word count drops from O(fields x operations) to O(fields + operations), which is
also what makes the module export list stop needing three entries per field.
**Ordered first in this phase, before `Vec`/`Map`/`String`**, for exactly the reason modules
were pulled forward: writing the collections against the old accessors and migrating them
afterwards is the waste. It cannot land earlier than this phase either, since one `&>`
accepting both an array and a struct *is* static overloading (Phase 4 slice 8).
**Not a locked design.** The open question is what a selector *is*: a compile-time-only
marker (the machinery slice 4 built for quotations, cheap and known, but no composition) or
a first-class `Lens['S 'A]` value (composable, expressible once type variables exist, but it
needs unambiguous selector names, which means qualification, which undoes the terseness that
motivated the change). Its brief has to settle that before anything else, and should size
the corpus migration honestly: every struct access in `examples/` and the test suite, which
is 8c-shaped mechanical work on top of a real design decision.

**Generic struct declarations (moved from Phase 4 Slice 3).** A `type:` parameterized by
Phase 4's type and length variables, with layout and the `StructId`/`ArrayId` registries
keyed per instantiation. It was placed in Phase 4 on the strength of a single named
consumer, `filter` needing to bundle a filtered array with a count, and moved here when
that consumer evaporated: Phase 4 Slice 1's synthesized multi-output return bundles make
`( [i64 'N] -- [i64 'N] usize )` an ordinary word signature, verified against the built
compiler, so no user-declared generic type is needed to return the pair. The internal half
of the machinery exists either way, since `intern_bundle_struct` already keys an interned
struct per instantiation on concrete field types; what stays unbuilt is the user-facing
half, a parameterized declaration with name resolution, per-instantiation generated words,
and per-instantiation destructor synthesis, all of which the bundle path deliberately opts
out of (a bundle is an ABI detail, not a nameable type). `Vec['T]` and `Map['K 'V]` are the
real consumers and they live here, which is what makes this the right phase: specifying it
in Phase 4 would have designed it against a consumer that does not exist, the same test
that sends open multimethods' former slot to Phase 6.

**Worklist-based disposal for branching structures (moved from Phase 3 Slice 4).** A
multi-child recursive type's synthesized destructor loops only its *last* recursive field
and recurses the rest, so a left-leaning tree still disposes in O(depth); a worklist would
let every child dispose iteratively instead. Waits for here because it needs a growable
pending-pointer structure to hold onto siblings while descending, which is exactly the
`alloc` layer's job, and because a fallible push wants an
optional to report through, which only exists after Phase 4's generics. Building a private
version of either inside a Phase 3 destructor would be guessing at both. If the fixed-size
bound turns out to be enough, the `fixed` layer's ringbuffer covers it without waiting for
`alloc`. No dogfood forces this earlier: the first real pressure is Phase 9's self-hosted
AST, a genuinely deep branching structure.

### Phase 7 — Concurrency (library)  `[M]`

Core intrinsics only: **atomics + memory ordering** (LL/SC on arm64, or FFI to C11
atomics on QBE) and a **spawn** primitive (thin FFI to `pthread_create` at the
hosted layer). Everything else is library: split-endpoint channels, mutexes,
pools, and actors (mailbox + loop + move-only messages). Data-race freedom is
inherited from the linear spine (send = move) and non-escaping refs, no separate
`Send`/`Sync` apparatus. Ship two libraries: the convenient hosted one and a
constrained `no_std`/RT one (static topology, fixed mailboxes, no escaping
captures).
**Exit:** concurrent programs that are data-race-free by construction; a deliberate
attempt to alias a sent value is a compile error.
**Dogfood:** a small worker-pool or a producer/consumer pipeline.

### Phase 8 — Bare metal  `[M]`  `[the craft milestone: own the vertical to the metal]`

Cross-compile to arm64 (or Cortex-M) bare metal: per-target intrinsics
(memcpy/memset, integer-divide/soft-float helpers), linker script, entry point,
`no_std` core + `fixed` layer on-device, soft-float lint. Soft-real-time works out
of the box; demonstrate hard-RT-by-discipline (fixed layer + static-topology
concurrency, no allocation or spawning on the hot path) if you want it.
**Exit:** a program running on real hardware or QEMU with no OS and no allocator,
blinking an LED or driving a sensor, from your own source language down to the
machine code you emit.

### Phase 9 — Self-hosting  `[XL]`

Stabilise the self-hosting subset S (smaller than before: concrete types + ADTs +
pattern matching, growable collections + strings, words + modules, errors as
values, a modest C FFI for the hosted layer; no inference, no refinements, no effect
rows, no borrow analysis). Rewrite the compiler in S, fixpoint-verify
(bootstrap-compiled == self-compiled), retire/demote the host-language bootstrap.
No metacircular JIT: the self-hosted REPL/build path still runs on the backend.
**Exit:** the compiler compiles itself; fixpoint reached.

### Optional (any time after Phase 2) — WASM sibling backend  `[M]`

A second lowering off the backend-neutral IR, parallel to QBE, not through it: Sooth
IR → WASM (emit, hand to binaryen for optimisation and any structured-control
cleanup). No relooper needed, since the IR already carries structured control flow.
The hosted layer re-ports from libc-FFI to WASI imports; `core`/`fixed` compile
nearly for free. AOT-to-native via `wasm2c` when a native artifact is wanted.
Depends on `Ptr[T]` having been kept abstract since Phase 2.
**Exit:** a Sooth program runs both as a native QBE binary and as a `.wasm` module.

### Committed future target — RISC-V 32

rv32 is a committed eventual target (embedded). QBE gives arm64/x86_64/riscv64 but has no
rv32, so reaching it means patching rv32 into QBE or the hand-written backend, a decision
deferred to **post-bootstrap** (consistent with "reconsider the backend after self-hosting").
Nothing is built for it now; the only present-tense obligation is that the frontend stays
word-width-neutral: the IR never assumes a 64-bit machine word, and `usize`/`isize` arrive as
target-width types with arrays (Slice 5). See DESIGN.md, Codegen and backend.

## Cross-cutting — Tooling and diagnostics  `[ongoing from Phase 0]`

Not a terminal phase. Good, localised compile errors start at Phase 0, for the
author's own write-run-fix loop and for legibility, not for any LLM-authorability
goal (dropped). A formatter and an auto-generated reference doc (word list + stack
effects) once the surface stabilises around Phase 4. An LSP is optional and low
priority for a craft language; add it only if you're using it enough to want it.

The REPL (`src/repl.rs`, `src/editor.rs`) grew a hand-rolled raw-mode line editor
(prompt, cursor movement, history, Ctrl-C/Ctrl-D handling), multi-line continuation
for an open `:`/`type:` definition or bracket, typed/rich stack rendering on the tty
path, and `:help`/`:words`/`:type`/`:stack`/`:clear` meta-commands with tab
completion. The piped (non-tty) path is unchanged byte-for-byte.

## Shape of the risk

- **Phase 0 is done and the go/no-go came back *go***: the virtual-stack → IR → QBE
  → native path holds. The remaining mass and risk is **Phase 3** (the linear memory
  model, the most novel work and the reason the language exists); do it carefully.
  **Phase 9** (self-hosting) is the other large lift but is well understood.
- Phases 4-8 are more independent than the numbering implies. Errors (5) is nearly
  free once ADTs exist (2). Concurrency (7) needs the linear model (3) but little
  else. Bare metal (8) needs the `fixed` layer (6) but not the hosted one. Reorder
  within that band by what you want to play with first, which for a craft project is
  a legitimate way to choose.
