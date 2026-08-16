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
branching structures stays out of scope, moved to Phase 7.
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

**Bug found in review (6f, slice 6f review round 2), fixed on the 6f branch: binding a
reborrow used to lose the suspend rule.** The stack-resident shape was always rejected
(`reborrow_while_projected_reference_still_live_is_error`, `tests/phase3_refs.rs:654`), but
naming the first projection's result before taking the second was not:

```
type: Buf data ^[u8 64] len usize ;
: f ( &!Buf -- )
  | p |
  p &!Buf>len | e |
  p &!Buf>len 1 +!
  e 1 +! ;
: main ( -- ) ;
```

used to build clean; now rejected on the same grounds as the stack-resident case. Root
cause: `Provenance::bind` deliberately cleared a derivation's `reborrow` flag on bind, as a
workaround for bound references living for the whole block (so `push-byte` could name its
buffer parameter again after binding a projection off it). Predates 6f, but its fix
depends on 6f: `bind` no longer needs to lie, because last-use liveness ends the
suspension honestly once the bound name's last use has passed. Landed here because it
could not land on main without 6f (verified: the same one-line change breaks `push-byte`
pre-6f). `Provenance::bind` was provably an identity function once the flag-clear was
removed (proven by replaying the whole suite with the call site reading `slot.deriv`
directly), so it is deleted rather than kept as a no-op.

**Second bug found in review (6f, slice 6f review round 3), fixed on the 6f branch: the
previous fix's own dependency turned latent conservatism into real over-rejections.**
D6's original rule never relaxed an outer-bound name inside a nested invocation (an `if`
arm, a `times`/quotation body), which was harmless until the fix above made the
reborrow-suspend rule fully depend on this liveness table. From then on, binding a
reference outside a nested block, consuming it before the block, and reborrowing its
root inside the block was rejected even though the bound reference was provably dead --
four distinct shapes reproduced it (an `if` arm, a `times` body, a deeper projection, and
use-then-reborrow within one arm), and one, consuming a field into a local then mutating
through the root in a loop, is an ordinary way to write a buffer routine. Fixed by
generalizing D6 rather than reverting the first fix: a name is granted into a nested
invocation only once the caller proves no residual use anywhere past the block
(`releasable_into`), and a granted name is then tracked by the nested invocation's own
`Liveness::scan` exactly like a name it binds -- fine last-use for an execute-once `if`
arm, pinned live for the whole body once used anywhere inside for a `times`/quotation
body (`back_edge`, since a per-iteration use must not die before the back-edge). Also
surfaced and corrected in the same pass: a pre-existing test
(`borrowed_local_carried_across_back_edge_is_error`) had been asserting the D6 gap
itself as the desired behaviour, on a self-tail loop's own back-edge arm; its own prior
comment already said the borrow would be legal if bound inside the arm, which is exactly
what this generalization now grants, so it is rewritten as an accept test with two new
reject neighbours (residual use elsewhere in the program; named again after the
recursive call within the same arm) rather than left as a stale reject. See
`docs/phase4-slice6f-spec.md`'s D6 and M4/M5 for the mechanism and its mutation matrix.

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
deferred to Phase 7, where it joins `Box`/`Vec`/`Map`/`String` in the `alloc` layer.
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
rejection instead of a panic. `times ( ..s i64 ~[ ..s i64 -- ..s ] -- ..s )` is an ordinary
exported word in `lib/combinators.sth` (slice 10b), a thin wrapper over a
self-tail-recursive `times-helper`: its recursive call in tail position lowers to a
`begin_loop`/`finalize_loop` back-edge like any other self-tail combinator, giving a runnable
constant-stack loop (a header `Phi`/`Jnz` reached by a back-edge `Jmp`, no per-iteration
`Instr::Call`, every loop-body `Alloc` entry-hoisted, verified at 1M+ iterations under a 1MB
stack), and loops nest at any depth. `examples/times.sth`
(`0 1000000 ~[ 1 + + ] times .`) dogfoods it, printing the same total as
`examples/countdown.sth`'s hand-threaded self-recursion.
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
is rejected at the `export:` declaration. Disposing an imported resource type
requires that type to be visible to the disposing module (Phase 4 slice 8b): a bare
`drop` on an imported linear value runs a destructor the owning module declared, so
a qualified-only import that never names the type is a located error at the `drop`,
naming the remedy. Selective import,
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

**Phase 4 Slice 6c (quotation-taking words at the REPL) is complete**: its brief
(`docs/phase4-slice6c-brief.md`) falsified the roadmap's own frozen-resolver framing before
any code changed — a combinator mints no `IrFunc` and no symbol, so it has no compile event
of its own to freeze against, and it is re-checked and re-lowered at every splice site
against that site's own live env, unlike a polymorphic word's body, which is checked once.
The fix is a session-level store (`Session.combinators: HashMap<String, WordDef>`, mono and
poly alike in one store, D2, replaced wholesale on redefinition, D1), projected on demand
into the two shapes the checker's `collect_combinators` and the lowerer's `combinator_bodies`
already read, threaded into every REPL entry point that hardcoded an empty map: `check_def`,
`check_def_collecting_drop_sites`, and `infer_line` on the checker side; `lower_word`,
`lower_instantiation`, and `lower_line` (a fifth, under-counted site) on the lowering side.
Defining a combinator (`eval_combinator_def`) skips lowering entirely — check, then store, no
`.so`, no symbol, no `dlopen` (D3) — checks the definee against a view that already includes
itself (so a self-reference dispatches through the inline path, not unknown-word), runs
`check_combinator_cycles` over that view so a cycle formed across session lines is still the
located error, and routes a polymorphic combinator through a new standalone check bypassing
`eval_poly_def`'s `>= 2`-outputs deferral (a combinator is spliced inline and never lowered to
a bundle-returning `IrFunc`, so that gate cannot apply). The three now-mutually-exclusive
name-shape stores (`self.env`, `self.poly_words`, `self.combinators`) evict each other
symmetrically on redefinition (D4), generalizing the existing env/poly-words rule, since
combinator dispatch runs before both and a stale entry in the wrong store would silently win.
Import reuses the same store (D5): the R24 rejection of a closure exporting a
quotation-taking word is dropped, and a module-0 exported combinator is retained under its
import-internal spelling with its body's calls — including a self-tail call — rewritten to
internal spellings, so the self-tail recognizer still fires on an imported `while` rather than
splicing forever. A review pass then closed a hygiene gap the brief's recon missed: a
retained combinator's body call to a module-0 *private* word was left unrewritten and fell
through to whatever the session's own env held under that bare name, a silent-wrong-answer
risk on a name collision, plus a forgeable variant (a REPL-declared name matching a
multi-file closure's internal mangled spelling). Both are closed: the body-call rewrite now
covers every module-0 word, not only exports, and a REPL-declared name may no longer end in a
resolver-mangled or import-epoch-tagged spelling. `examples/filter_while.sth`'s scenario is
dogfooded again as a REPL session transcript, pinned to the same computed values.

**Phase 4 Slice 6d (nested constant-stack loops: the hoist-target split) is complete**: the
cause was one field doing two jobs, `FuncBuilder::entry_block`, simultaneously the alloca
home (where a hoisted `Alloc` must land, since QBE's frame-bumping `alloc*` never reclaims
within a function) and the loop preheader (where a carried aggregate's seeding `Blit` must
land, so it re-runs once per entry to that loop). Those two blocks coincide at exactly one
loop level and diverge the moment loops nest, which was the whole of the bug. The fix keeps
`entry_block`'s meaning as the per-loop preheader (it was already correct at any depth) and
adds a separate, invariant `alloca_home` field that `push_alloc` routes into instead;
`begin_loop` sets it once, on the outermost loop only, to the block current when that loop
opens. This is narrower than this entry originally prescribed below ("split the field into an
invariant alloca home and a per-loop preheader", implying both roles move) and inverts which
half moves: only the alloca role does. The four-field loop-state save/restore duplicated at
the two mid-body call sites collapsed into one shared helper first, as an inert de-risking
step, before `alloca_home` joined it as a fifth field. With the split landed, the R18
nested-loop rejection retired outright (both checker call sites, the dead
`times_nested_in_loop_error` function, and the now-unread `loop_depth` bookkeeping), so all
five 6a/6b combinators compose inside a `times` body and inside each other, at any depth,
in constant stack; `examples/combinator_in_times.sth` dogfoods `each` inside a `times` against
a hand-threaded twin, and a recursive-enum value's destructor call inside a `times` body
inherits the fix for free, since a destructor's fused loop already opens at its own
`IrFunc`'s true entry.

**Phase 4 Slice 6e (`if` in a polymorphic body) is complete**: the unconditional rejection at
`poly_term`'s `TermKind::If` arm is replaced by a full arm mirroring the monomorphic one
(condition pop, `Bool` check, per-arm walk, move-state join) with no quotation handling
(a `PolyType` is never a quotation) and no lowering path (checker-acceptance-only; the
concrete instantiation already lowers correctly through `check_word`). `PolyScope`'s move
tracking is upgraded from a two-state `Option<Span>` map to the monomorphic `Moves`
three-state lattice (`Live`/`Moved`/`MaybeMoved`), joined at the `if` the same way branch
joins already are, and a keys-snapshot `leave_arm` rejects an arm-local linear value never
consumed inside its own arm before dropping it from scope. `choose` (arms `drop` different
operands) and `mymax` (the `Ord`-bounded polymorphic `max` the intrinsic could not
previously give up) both compile and run at two instantiations (`i64`, `f64`); a nested-`if`
`mymax3` dogfoods that recursion needs no special case. Slice 7's dependency on a
branching polymorphic body is now satisfied; the core library's intrinsic-vs-library split
for `max` is unblocked.

**Phase 4 Slice 6f (liveness ends at last use) is complete**: a reference bound to a local now
dies at its **last use**, the way one left on the stack already did, so a held-then-unused
borrow no longer blocks consuming its place — `&!acc &!Acc>arr | f | f 0 >usize &!> 5 ! acc
drop` compiles where before only its identical chained form did. Checker-acceptance-only (D1):
no new `Instr`/`Terminator`, no lowering, `Type`, or `IrType` change, and since `Deriv`/
`DerivId` never leave `src/check.rs`, no emitted code changes by construction. The rule is a
per-`check_terms` `Liveness` pre-pass keyed only on names *that* invocation binds; only the
`scope.bound` half of each table becomes use-bounded, both stack halves left byte-for-byte
(D2). The same analysis relaxes `aliasing_origin`'s name loop by each candidate's own last use
(D8), overlap preserved. Because a wrong answer on that half is a silent wrong *value*, the
alias half is mutation-tested rather than merely run green (D9). `examples/inplace_fold.sth`
dogfoods a named-accumulator in-place `fold` at both a `Copy` and a linear `Acc`, with no
per-iteration `blit` in the emitted QBE loop body.

Review found and fixed a soundness regression the first implementation introduced: a quotation
bound to a local (`[ ... ] | q |`) had its captures attributed to the *literal*, not the
binding, so a borrow captured by a quotation called later looked dead and a second `&!` to the
same place was silently accepted. Captures now die with the binding, transitively through a
quotation capturing a quotation. This is sound only because a capturing quotation still cannot
escape the block that binds it (7a ships *non-capturing* quotations as values; a capturing
literal reaching a materialization boundary is a located error naming 7b).

The syntactic capture-detection gap flagged after that fix is closed. The heuristic (a literal
immediately preceding a `Bind`, taking that bind's last name) is gone; capture propagation now
reads `Slot.quot`/`Binding.quot` directly, the association the checker already records on both
carriers a quotation can occupy. This closes the two known shapes (a quotation separated from
its `Bind` by another value; a `Bind` naming two or more quotations where a non-topmost one
captures) and a **third found while fixing the other two**: a conflict arising while the
quotation is still unbound on the stack, which had no `Bind` for the old heuristic to even look
for. A quotation's free-name set is a pure, cached property of its own body; whether a captured
name is actually still live is answered fresh at each query by walking which quotations are
currently reachable (on the stack, unconditionally; bound, transitively through whichever other
bound quotations are themselves still reachable) — mirroring `live_derivs`' existing stack-half/
scope-half shape rather than adding a second one beside it.

A third review round found and fixed an over-rejection the first fix's own dependency
introduced: making the reborrow-suspend rule depend on this liveness table meant D6's original
blanket "an outer name is never relaxed inside a nested invocation" turned real, four distinct
safe programs (bind-consume-then-reborrow across an `if` arm, a `times` body, a deeper
projection, and use-then-reborrow within one arm) went from accepted to rejected. Generalized
rather than reverted: a name is granted into a nested invocation only once the caller proves no
residual use anywhere past the block, and a granted name is then tracked by that invocation's
own scan exactly like one it binds — dying at its own last use in an execute-once `if` arm,
pinned live for the whole body once used anywhere inside a `times`/quotation body. See the
prose above ("Second bug found in review") and `docs/phase4-slice6f-spec.md`'s D6/M4/M5.

**Phase 4 Slice 6h (a raw array constructor, plus `fill`'s re-lowering) is complete**:
`[ Type ; Count ]` allocates one array slot and zero-initializes it with a byte-granular
runtime loop, sized exactly to `ArrayLayout::size` (an array is not word-padded). The
element name resolves and the shape interns at **parse time** (the parser already owns
`&mut Vec<ArrayDecl>`), so the term carries a finished `Type::Array` with no lowering-time
lookup. A recursive zero-validity gate rejects an element that transitively contains
`str`, `cstr`, or a quotation (through struct fields, every enum variant, and array
elements): each is `Copy` and pointer-shaped, so an all-zero slot would be a null pointer.
`fill` is re-lowered the same way — one `Alloc` plus a runtime counted-store loop (via
`ElemAddr`) replacing its N unrolled compile-time stores, closing a QBE-quadratic
compile-cost defect (emitted code size is now O(1) in the count); its type-checking is
otherwise untouched, and it keeps accepting `str`/`cstr`/quotation elements since it
replicates a real seed rather than minting one from zeroed memory. A polymorphic body
constructing its own array, typed by its own `'T`/`'N`, is out of scope: `subst_polytype`/
`array_id_of` only look up an already-interned shape and panic otherwise, and a poly body
has nothing to intern a body-internal shape against, so this is deferred to a future
slice. A concrete element inside a combinator body already works today, since a
combinator is monomorphized and checked by the ordinary concrete `check_word`.

**Phase 4 Slice 12 (combinator recognition becomes declared, not inferred) is complete**
and merged to `main`: a word declaring a `~[ ... ]` parameter must say `inline` (a located
error where it does not), `is_combinator`'s quotation-parameter inference leg is retired
(`inline` is the single route to "this word splices"), and every library combinator
(`times`, `each`, `map`, `fold`, `filter`, `while`, `if`, `unless`) is migrated. `~[ ... ]`
is now writable as a term literal and required at a `~` parameter — an ordinary `[ ... ]`
no longer silently satisfies one — closing the gap that kept a first-class capturing
quotation (7b's territory) from ever being a genuine call.

**Phase 4 Slice 13 (`PolyType::Ref` — poly borrow support) is complete** and merged to
`main`: a plain (non-combinator) generic word can borrow (`&x`/`&!x`, `&>`/`&!>`, `@`, `!`)
at its own top level, and a signature slot can declare a borrow of a still-generic type
(`&'T`, `&['T 4]`, `&!...`). `PolyType` gained a `Ref` variant threaded through the parser,
unification, substitution, Copy-gating, and IR lowering. A poly body's borrow gets full
exclusivity/liveness/use-after-move checking at check time (a conservative
`PolyScope`-local approximation, sound by construction: it can only over-reject, never miss
a hazard), since a plain poly word is checked once, abstractly, and never re-checked per
instantiation. Scope is bounded to the array case: `'N`-length (fully generic-length)
element access, `&^`/`&Struct>field` accessors in a generic body, `+!`, and full
`Provenance`/`Liveness` acceptance parity with the monomorphic checker are each a located
error or an explicit deferral, not a silent gap.

Slice 9 shipped `Bool` as a library enum (P1–P2 only, merged at `c5db035`); its `if`/`cond`
half was renumbered into slice 10's lineage as slice 10c, since what it depended on was
10a's row mechanism, not 9's own scope, and shipped a different mechanism than originally
assumed — full account in `docs/phase4-slice10c-spec.md`.

**Fixed: the row-typed-combinator quotation crash.** A bare quotation phantom riding
untouched through a self-tail combinator's carried row (`[ + ] 3 [ drop ] times drop`, and
the same shape over `while` or any user-declared row combinator) used to die at the backend
on an undefined phi operand (`qbe: invalid type for operand %v0 in phi %v4`): a phantom
carries a lying `IrType::I64` placeholder and no defining `Instr` (`lower_term`'s
`TermKind::Quotation` arm), so `begin_loop`'s `is_aggregate` check never recognized it and
it fell to the scalar-phi branch, which needs a real defining instruction the phantom never
has. `CarriedSlot::Phantom` (`src/ir/func_builder/mod.rs`), keyed on the `quot_bodies`
identity map rather than the placeholder type, now reuses the same id unchanged across every
iteration with no phi and no aggregate staging — sound because a phantom is compile-time-
fixed and reachable in only one form absent a materialized closure (7b, not built), which
already carries a genuine `IrType::Quotation` and already took the aggregate branch
correctly. Confirmed fixed live for both `times` and `while`, including nested composition,
and that the row-carried quotation's identity survives intact (callable afterward, not just
droppable). Goldens: `times_carries_an_untouched_quotation_through_the_row`,
`while_carries_an_untouched_quotation_through_the_row`,
`times_nested_inside_times_still_carries_a_row_quotation`
(`tests/phase4_slice10b.rs`).

The third shape `docs/phase4-slice10b-spec.md` named alongside these two (`while` over a
*materialized* quotation panicking in `control_flow.rs`) is a different crash, not this one:
it needs a way to feed a real `(code, env)` closure value into a `~[ ... ]` parameter. 7b's
capturing closures are built (`examples/capturing_dispatch.sth` materializes and `call`s
two same-frame closures via an array round-trip) and this text previously said otherwise;
the dead-code conclusion still holds for a narrower reason, confirmed directly rather than
read off the checker's source: loading a materialized quotation out of storage and passing
it to `while`'s `~[ i64 -- i64 bool ]` parameter is rejected on sight (`error: while expects
a quotation ~[...] here, found [...]`) regardless of whether a closure value exists
elsewhere in the program. Every combinator's `~` parameter carries the same restriction, so
this stays presumed dead code, verified by direct repro rather than by inference. Revisit if
any combinator's `~` parameter is ever loosened to accept a materialized value.

**Next action: none locked.** One previously-identified gap remains open and unscheduled
(`docs/phase4-slice13-brief.md`'s Deferred section): closing slice 13's conservative
borrow-liveness fallback to full mono-checker parity.

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

Full detail lives one file per phase, split out because this file had grown past 2500
lines. Each phase file is self-contained: exit criteria, dogfood, slice breakdown.

| Code | Phase | Weight |
| --- | --- | --- |
| **P0** | [Codegen spine](./P0-codegen-spine.md) | `[L]`  ✅ done |
| **P1** | [REPL and liveness](./P1-repl-and-liveness.md) | `[M]`  ✅ done |
| **P2** | [Typed core (monomorphic)](./P2-typed-core.md) | `[L]`  ✅ done |
| **P3** | [The linear spine](./P3-linear-spine.md) | `[XL]` — the point of the language |
| **P4** | [Minimal polymorphism + quotations](./P4-polymorphism-quotations.md) | `[L]` |
| **P5** | [Errors as values](./P5-errors-as-values.md) | `[S]` |
| **P6** | [Term-level enum elimination](./P6-enum-elimination.md) | `[L]` |
| **P7** | [Stdlib and `no_std` layering](./P7-stdlib-nostd.md) | `[L]` — where it becomes usable for real programs |
| **P8** | [Concurrency (library)](./P8-concurrency.md) | `[M]` |
| **P9** | [Bare metal](./P9-bare-metal.md) | `[M]` — the craft milestone |
| **P10** | [Self-hosting](./P10-self-hosting.md) | `[XL]` |

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
  **Phase 10** (self-hosting) is the other large lift but is well understood.
- Phases 4-9 are more independent than the numbering implies. Errors (5) is nearly
  free once ADTs exist (2). Concurrency (8) needs the linear model (3) but little
  else. Bare metal (9) needs the `fixed` layer (7) but not the hosted one. Reorder
  within that band by what you want to play with first, which for a craft project is
  a legitimate way to choose.
