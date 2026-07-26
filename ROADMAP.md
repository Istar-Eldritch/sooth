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
`fill`/`get`/`set`/`len` words (non-consuming `get`, functional `set`), and dynamic indexing
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
restriction), while a by-value cycle stays a located error; `isize`, a signed mirror of
`usize`; disposal is reversed to free-before-drop-payload and made pre-order; and a
directly self-recursive type (list or tree) gets one fused iterative destructor loop
instead of recursive `cell_drop`/`struct_drop`/`enum_drop` calls, giving constant-stack
disposal (verified at 1M+ nodes under a 1MB stack) for that shape specifically —
indirect cycles, `^^Self`, and mutually recursive types keep the recursive path and its
depth limit. The OOM trap stays: there is still no compiler-known optional/non-null
pointer type to return failure through (that needs Phase 4's generics), so this was
never this slice's revisit to make. `examples/list.sth` dogfoods it.
**Next action: Phase 3 Slice 4** (second-class references + parameter conventions). Not
yet locked.

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
   exhaustive-only, no `_` wildcard yet; no recursive enums (infinite size is a located error,
   since recursion needs a pointer, which arrives in Phase 3). Also **renames the control-flow closer `then` ->
   `end`** (`if … else … end`), unifying it, and extends **top-of-scope `| … |` locals** to
   clause bodies (bind names at the top of a word body or a clause body, extent = that scope;
   no mid-body binding, no closer: factor a word instead). Design locked in
   `docs/phase2-slice4-brief.md`. Prefer-the-stack stays the culture; locals stay opt-in.
   ✅ done.
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
field type, a duplicate type name, a recursive (infinite-size) struct, constructor
arity/type mismatch, an accessor applied to the wrong type, `.`/`=`/arithmetic on a
struct, and a malformed declaration; a zero-field unit struct; and the
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
struct field may be an enum and vice versa, but a value-cycle is a located compile
error, never a hang); `.`/`=`/arithmetic on an enum are sharp located errors; a REPL
`<TypeName>` placeholder display; and size-aware carried-stack marshalling generalized
to enum slots across both the word-call and REPL-line boundary. The `examples/shapes.sth`
dogfood (`Shape`'s `Circle`/`Rect` via `area`, `MaybeInt`'s `None`/`Some` via
`unwrap-or`) runs both as a native binary and in the REPL. Generics, `Option<T>`/
`Result<T,E>`, open multimethods, the `_` wildcard, inline `match`, and recursive/heap
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
   `drop` is overridable (destructor bodies in slice 6, polymorphic dispatch in Phase 4). Aggregates are in scope via
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
   positional restriction); a by-value cycle keeps its existing located error. Disposal is
   reversed to free-the-cell-before-dropping-the-payload (uniformity, not a correctness
   requirement) and made pre-order (a node's own fields drop and its cell frees before
   descending). A directly self-recursive type gets one fused iterative destructor loop
   (looping the *last* recursive field in declaration order, recursing any others) instead
   of mutually recursive `cell_drop`/`struct_drop`/`enum_drop` calls, giving verified
   constant-stack disposal at 1M+ nodes under a 1MB stack for that shape only — indirect
   cycles, `^^Self`, and mutually recursive types keep the recursive path and its depth
   limit, which is a documented limitation, not a bug. `isize`, a signed mirror of `usize`.
   **Slice 2's OOM-trap-and-abort decision stays closed, not revisited**: an earlier plan
   assumed this slice would introduce a compiler-known optional/non-null pointer type for a
   failed allocation to return through; there is no such type here or anywhere before Phase
   4's generics (a compiler-synthesized per-payload `Option` is exactly what generics would
   delete), so the allocator has nothing privileged to return and the trap stays. Dogfood:
   `examples/list.sth`, a linked list that builds, walks (sums via a non-consuming or
   consuming pass), and disposes what remains via the fused loop.
   A **zipper** (focus + stored path of one-hole steps) remains a sharper future exercise
   for the recursive drop glue, and the one shape slice 4's second-class references
   provably cannot express, since the path must be stored; not attempted this slice.
4. **Second-class references + parameter conventions (`let`/`inout`/`sink`/`set`) + escape
   checking.** Hylo mutable value semantics: pass a borrow, mutate in place, no move, with
   the escape checker keeping refs from being stored or escaping scope. Comes after heap
   because "hand the value back" already works by threading it through the stack. Dogfood:
   in-place mutation of an owned buffer through `inout`.
   **Design question this slice's brief must answer:** do `inout` projections into nested
   fields subsume a reified take/fill pair (`S/fi` yielding a residual `∂S/∂fi`, refilled
   exactly once)? A second-class projection is the same residual, made implicit and
   lexically bounded, and it also covers whole-value borrows, so the expectation is yes
   for every statically known path, leaving reified residuals worth having only where the
   focus must escape (slice 3's zipper, as a stdlib type). Answer it here rather than
   letting nested-path ergonomics get solved twice.
5. **Opt-in RC (`Rc`/`Arc`-equivalent).** Shared ownership, last ref frees. The softest
   slice; could slip toward Phase 6 if it wants a stdlib home.
6. **Resources as linear values (fds, hosted) + user-definable destructor bodies.** The
   Phase 3 exit dogfood: open/read/close files, with the compiler catching a deliberate
   double-use and a forgotten `close`. **This slice is where a user can first attach their
   own cleanup code to a type**, rather than only inheriting disposal by composition. It
   lands here, not earlier and not in Phase 4, because a mechanism wants two dissimilar
   real clients to be designed against: `free` (pointer + size) from slice 2 and `close`
   (an integer handle, and it can fail) from this one. Designing it in slice 2 from the
   buffer alone would be guessing. Open questions to settle here, not before: whether a
   user destructor runs *before* or *instead of* the synthesized field glue, and how a
   destructor body is stopped from dropping its own receiver and recursing forever. Note
   this is destructor *bodies* only; `drop` becoming a polymorphic word is still Phase 4.

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
loops rather than a `call` per element. Escaping quotations use the uniform-runtime-
stack fallback and depend on the alloc layer (Phase 6). With quotations in hand, `if`
is redefined as an ordinary combinator (`cond [ then ] [ else ] if`, Factor-style) and
stops being a keyword, and a `cond` multi-way combinator lands alongside the others.

**Dispatch and uniformity (bundled here on purpose).** Several deferred ideas are one
conversation, and none is clean without quotations, so they land together in this phase:
(a) **`if` becomes an ordinary combinator** over quotations (above); (b) **generics /
minimal polymorphism** (above); (c) **ad-hoc dispatch**, both static **overloading** (one
word name, several statically-known input types, e.g. `+` over `i64`/`f64`/`Vec2`) and
**open multimethods** (`generic:`/`method:` on a sum, the open/dynamic dual of Slice 4's
closed clause-style match, trading closed exhaustiveness for module-level extensibility,
the expression-problem tradeoff); and (d) **`Bool` as a library enum**
(`type: Bool | False | True ;`) rather than a primitive. (d) waits for here specifically
because bool's specialness is not the *type* but that strict, quotation-less two-way
branching needs inline syntax; once `if` is a quotation combinator, `Bool`-as-enum +
`if`-as-word unify, and only then does making it a library type avoid re-adding special
cases (at `if`, at the `True`/`False` literals, and at type-directed printing). Slice 4's
clause-style match and `if/else/end` are deliberately designed not to foreclose any of
this: clause structure maps 1:1 onto a future quotation arm-table, and `if` staying a
keyword for now is the honest strict-eval choice, not a commitment.

**Exit:** polymorphic `dup`/`swap`/`max`; a constant-stack `each`/`fold` over a
collection; combinators verified to inline to loops, not per-element calls.
**Dogfood:** write the combinator library (`each`/`map`/`fold`/`while`) in Sooth
itself, then rewrite an earlier program to use it.

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
wrappers). Tag every stdlib word with the layer it needs.
**Exit:** real hosted programs using libc via safe wrappers; a usable standard
library; the `fixed` layer works with no allocator present.
**Dogfood:** a genuinely useful small tool (a line-oriented text utility, a small
static-site or markdown thing) written entirely in Sooth.

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
