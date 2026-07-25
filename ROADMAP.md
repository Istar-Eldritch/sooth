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
`tests/phase1.rs`. **Phase 2 is in progress**, sliced into vertical increments: **Slice 1
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
`ElemAddr` op; `examples/stack.sth` dogfoods it. `isize` deferred to Slice 8 (its only
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
**Next action: Phase 2 Slice 8** (`Copy` marker + optional / non-null pointer, the Phase
2 -> 3 bridge). Not yet locked.

Host language: Rust is the sensible default (ADT + pattern-matching-heavy compiler
workload, `no_std` for the runtime/intrinsics library), but nothing now requires
it, since LLVM and Z3 were dropped. Free choice.

## Guiding principles

- **De-risk novel-before-laborious.** Prove the uncertain, novel parts (the codegen
  model, then the affine memory model, which is the whole point of the language)
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

### Phase 2 — Typed core (monomorphic)  `[L]`  🚧 **in progress** (typed core + VM dogfood done: Slices 1-7 + floats/bitwise/bool; only the `Copy`/pointer bridge remains)

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
remains is the `Copy`/pointer bridge (Slice 8); the VM dogfood (Slice 7, the Phase 2 exit)
is done.

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
   scrutinee and pushes the variant's fields onto the stack (affine destructor dispatch);
   exhaustive-only, no `_` wildcard yet; no recursive enums (infinite size is a located error,
   since recursion needs a pointer, Slice 8). Also **renames the control-flow closer `then` ->
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
   deferred to Slice 8 (no consumer until pointer differences exist). Arrays are inline `Copy`
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
8. **`Copy` marker + optional / non-null pointer**: the `Copy`-vs-affine distinction as a
   built-in type property (so Phase 3 has it to build on), plus explicit optional and
   non-null pointer types.

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
data are deferred (Phase 4 / Slice 8 / Phase 3).

`(value, type)` slot from day one, concrete types only. Numeric tower (i8..i64,
u8..u64, f32/f64; `*/` widening primitive; literal defaults). Records/structs, enums/ADTs, exhaustiveness-checked
pattern matching. Non-null pointers + explicit optional type. The **`Copy` vs
affine distinction** as a built-in property of types (primitives Copy; anything
owning a resource affine), so Phase 3 has it to build on. Stack-effect checking now
unifies **type and arity** at branch join points (loops arrive with the loop
primitive in Phase 4). Still heap-free: value types and fixed-size arrays only.
**Exit:** typed programs with structs/enums/match; type and arity errors are sharp
compile errors.
**Dogfood:** a small parser or a fixed-size VM for some toy bytecode.

### Phase 3 — The affine spine  `[XL]`  `[highest novelty: this is the point of the language]`

Move semantics as the default; `dup` (a plain int-copy since Phase 0) becomes the
explicit copy **gated on `Copy`**, and `drop` (a plain discard since Phase 0) becomes
the statically-known destructor point; deterministic drop (destructor at the
statically-known end of ownership). Hylo-style
mutable value semantics: parameter conventions (`let`/`inout`/`sink`/`set`) and
second-class references (can't be stored, can't escape scope), so no borrow checker
and no lifetimes. Opt-in RC (`Rc`/`Arc`-equivalent). **Heap arrives here**, under
ownership. Resources (fds, later locks) modelled as affine values; `dup` on them is
a compile error.
**Exit:** memory-safe heap programs, no GC, deterministic destruction, resources as
affine values that can't be duplicated or leaked.
**Dogfood:** a program that opens/reads/closes files and manages owned buffers,
with the compiler catching a deliberate double-use.

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
inherited from the affine spine (send = move) and non-escaping refs, no separate
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
  → native path holds. The remaining mass and risk is **Phase 3** (the affine memory
  model, the most novel work and the reason the language exists); do it carefully.
  **Phase 9** (self-hosting) is the other large lift but is well understood.
- Phases 4-8 are more independent than the numbering implies. Errors (5) is nearly
  free once ADTs exist (2). Concurrency (7) needs the affine model (3) but little
  else. Bare metal (8) needs the `fixed` layer (6) but not the hosted one. Reorder
  within that band by what you want to play with first, which for a craft project is
  a legitimate way to choose.
