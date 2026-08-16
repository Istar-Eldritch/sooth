[← ROADMAP](./ROADMAP.md)

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

