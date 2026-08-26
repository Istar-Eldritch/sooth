# Phase 7 Slice 5: linear array elements (brief)

**Sequence.** After S4b (bounds on impl variables), which this slice does not depend
on. S4b's generic-impl bounds are orthogonal to the array element gate, and landing
either first is fine; S5 is briefed separately because it touches the IR destructor
synthesis and the checker's array element gate, neither of which S4b or S6 (surface
syntax) touches.

**Motivation.** `[T N]` rejects a linear element for every linear type:
`type: Arr xs [Spy 2] ;` and `type: Arr xs [owning [ -- ] 2] ;` both fail with
`linear array elements are not supported yet` (`src/check.rs:3227`), while the same
type as a struct field (`type: Box s Spy ;`) builds. A linear struct is storable but a
*collection* of them is not, which is the one gap that keeps the linear spine from
reaching arrays. The restriction dates to P3 (`P3-linear-spine.md:56`,
`P3/slice1-spec.md:61`) and has been re-observed from four slices since
(`P3/slice3-brief.md:70`, `P4/slice6b-brief.md:142`, `P7/slice3c-brief.md:242`, and
**P7.S3v**), always as somebody else's blocker, never as a slice.

Three things make it a real slice rather than a predicate flip: a construction path
that does not replicate, a disposal path that does not exist yet, and the
partially-initialized window during construction.

## Probe-verified findings

Five read-only probes (model `litellm/syn-large-text`, worktree-isolated, no commits)
verified every code-level claim in the roadmap text against the current tree. Their
findings, with corrected line numbers:

**`fill`'s IR lowering** (`src/ir/func_builder/word_families.rs:394-455`). The pattern
is: `save_loop_state` → `alloc_array(id)` (emits `Instr::Alloc`, producing raw
unzeroed storage, `:268`) → `begin_loop` (carries the induction index; the "seed" in
the loop header is `Const(0)` = the *loop index init*, not the element) → header
`Cmp(Lt, index, Const(n))` / `Jnz` → body: `elem_addr` + `store_elem` (writes the
element value into each slot) → back-edge `index + 1` / `Jmp` → `finalize_loop` →
`push dst`. The element value (`elem_v`) is replicated: the same SSA value is stored
into every slot. `fill` has no registered intrinsic signature — it is dispatched by
name in both the checker (`check_array_word`, `src/check/word_families.rs`) and the
IR (`lower_array_word`, via `src/ir/func_builder/calls.rs:580`). The `alloc_array` →
store loop → `push dst` pattern allocates raw, uninitialized storage that never
surfaces as a type-system value until every slot is written — exactly the boundary a
generation combinator needs. No new storage category is required: the
"no uninitialized memory" rule is a type-system boundary, not an IR constraint, and
the IR already crosses it inside `fill`.

**The element gate** (`check_array_element_gate`, `src/check.rs:510`). Checks
`is_copy` on the element *type* — `is_copy` (`src/check/builtins.rs:219`) is a
type-level predicate that returns false for any type with a transitive linear field.
The gate receives a `Type`, not a `Slot` or value, so it cannot distinguish a
nullary-variant seed from a data-carrying one. The diagnostic
(`fill_of_linear_element_error`, `src/check.rs:3224`) emits at `:3227` (REPL) and
`:3230` (build), naming the element type and the site.

**Array drop glue** (`emit_drop`, `src/ir/func_builder/quotation.rs:394`). Arrays
today are stack-allocated (`Instr::Alloc` maps to QBE's `alloc4`/`alloc8`/`alloc16`,
`src/backend/qbe.rs:1339`). Non-linear arrays hit the `_ => {}` arm — no instructions,
no per-element disposal, no free call (the stack frame is cleaned up automatically).
Linear arrays hit `unreachable!` at `:412` ("checked: a linear array element is
rejected wherever an array type is named"), because the gate prevents a linear array
from ever reaching drop. The destructor synthesis pass
(`synthesize_aggregate_destructors`, `src/ir/destructors.rs:37`) handles structs,
enums, and cells — but *not* arrays. There is no `array_drop_symbol`, no
`synthesize_array_destructor`. The `ArrayLayout::is_linear` field already exists and
is computed from the element type, so the plumbing for "is this array linear?" is in
place; the missing piece is the drop arm and the synthesized destructor.

**The `[Type; Count]` constructor** (`parse_array_ctor_term`,
`src/parser.rs:4021`). The syntax is `[ Type ; Count ]` (spaces around the
semicolon), producing a `TermKind::ArrayCtor`. The lowering
(`src/ir/func_builder/calls.rs:74`) is a byte-granular zero-init loop over
`ArrayLayout::size` bytes — a memset-equivalent, not a per-element constructor
call. It takes a *type*, not a value, so it cannot produce valid linear elements
(zeroed memory is not a constructed value). It shares the same
`check_array_element_gate` as `fill`, but with `zero_safety = true` (rejecting
zero-unsafe types) in addition to the linearity gate. It is recognized in the term
parser by `array_ctor_ahead` (`src/parser.rs:3990`), a two-token lookahead that
distinguishes `[Type; Count]` from a quotation — the same class of disambiguation
heuristic S6 deletes from the type parser by naming the array type. Only two runtime
construction forms exist today: `[Type; Count]` and `fill`. No literal array syntax
`[1 2 3]` exists — that parses as a quotation. No `tabulate` word exists.

**`times` and the splice mechanism.** `times` is a *library word*
(`lib/combinators.sth:40`), not a builtin IR word family — the IR has `fill` and
`len` as array word families, but `times` is ordinary source, spliced via the D5/D8
inline-combinator mechanism (self-tail recursion lowers to a loop back-edge, so
`times` runs in constant stack). Its quotation is spliced at the call site: the
quotation body becomes the loop body. `times` forbids carrying move-state across
the back-edge (a linear value consumed inside the quotation cannot "escape" the
iteration boundary), which is the linear-spine constraint a `tabulate` combinator
must respect: each iteration produces a fresh value that is stored, not a value
that persists across iterations.

**Nullary-variant detection** (`Slot`, `src/check.rs:278`). The `Slot` struct has
`ty`, `literal`, `int_val`, `alias`, `deriv`, `quot`, `surviving` — no
variant-identity field. When `None` is called, the checker pushes `Type::Enum(...)`
onto the slot; the slot does not record *which variant* produced it. So the checker
cannot today, at the point where `fill` checks its element, determine whether the
seed was a nullary variant. Variant discriminants are assigned in declaration order
(`variant_idx as i64` in lowering, `src/ir/func_builder/calls.rs:555`), so the
first-declared variant is discriminant 0, and an all-zero bit pattern is valid for a
discriminant-0 nullary variant. `Option` is a regular generic enum
(`lib/option.sth: type: Option 'T | None | Some 'T ;`), not a compiler primitive.

## The three pieces

### 1. Construction: `tabulate`, nullary-variant `fill`, and dropping `[Type; Count]`

**`tabulate`.** A generation combinator — `tabulate ( usize ~[ -- T ] -- [T N] )` —
whose lowering is `fill`'s loop body with one swap: instead of storing a replicated
seed each iteration, the loop calls the quotation (spliced, so `tabulate` is `inline`
like `times`), gets a fresh `T`, and stores that. The IR pattern is already proven:
`fill`'s `alloc_array` → store loop → `push dst`
(`src/ir/func_builder/word_families.rs:394-455`) allocates raw, uninitialized storage
that never surfaces as a type-system value until every slot is written. `tabulate`
reuses this pattern, replacing `store_elem(fptr, elem_v, elem)` (the replicated
seed) with a quotation call that produces a fresh value per iteration.

`tabulate` is a new builtin array word family (like `fill` and `len`), not a library
word. `times` is library code, but `tabulate` must allocate an array and manage the
storage boundary, which is IR-level work — the same reason `fill` is a word family
and not a library word. The quotation is spliced the same way `times`'s is (D5/D8
inline splicing), so each iteration's quotation body is inlined at the loop body.

The linear-spine constraint from `times` applies: each iteration produces a fresh
value that is stored into the array; the value does not persist across iterations.
This is exactly what makes `tabulate` safe for linear elements — no value is
replicated, each slot gets a distinct, freshly-constructed value.

**Nullary-variant `fill` relaxation.** `fill` could admit a *nullary-variant seed*
(e.g. `None 3 fill`) even when the enum type is linear, because a nullary variant
carries no linear data to replicate — only a discriminant. This does not solve the
general case (a linear array of *distinct* values still needs `tabulate`) but it
covers the sentinel-initialized backing array the `fixed`-layer collections (P9.S1)
need: an `array[Option[T] N]` initialized with `None` in every slot, overwritten as
values arrive.

The gate today (`check_array_element_gate`, `src/check.rs:510`) checks `is_copy` on
the *type* and receives a `Type`, not a `Slot` or value. To admit a nullary-variant
seed, the checker must determine whether the *seed value* is a nullary variant,
which requires information the `Slot` (`src/check.rs:278`) does not carry today —
the slot records `ty: Type::Enum(...)` but not which variant produced it. The
relaxation needs one of:

- A new `Slot` field (analogous to `int_val`) tracking the variant index of the
  value that produced the slot, set when a variant constructor is called and read
  by the gate. This is the cleanest path: it threads the information the gate
  already needs but cannot see.
- A provenance span on `Slot`, tracing back to the constructor call, so the gate
  can look up which overload/variant was chosen from `builtin_overloads`. This is
  more general but heavier, and `Slot` is `Copy` — adding a `Span` (which is
  `Copy`) is mechanically fine but widens every slot.

The lowering choice follows from the discriminant: a discriminant-0 nullary variant
(the first-declared, like `None`) can memset to zero (its bit pattern is all zeros,
same as the `[Type; Count]` path); a non-zero nullary variant needs `fill`'s store
loop writing the correct discriminant. This does not require `Option` to be
intrinsic (it is an ordinary generic enum, `lib/option.sth`), and it does not require
a `Default` trait (a `Default`-based construction would still be replication under
another name, the same contradiction `fill` hits for a truly linear type with a real
destructor obligation).

**Dropping `[Type; Count]`.** With `tabulate` for distinct values and the
nullary-variant `fill` relaxation for sentinel-init, the `[Type; Count]` constructor
is the redundant third construction path — and the one that is semantically at odds
with the rest. It takes a *type*, not a value, and zero-initializes via memset, which
is exactly why it cannot produce valid linear values (zeroed memory is not a
constructed value). It works only for the case S5 is *not* extending (copy types)
and fails for the case S5 *is* extending (linear types). Keeping it means
maintaining a third parser path (`parse_array_ctor_term`, `src/parser.rs:4021`), a
third checker path (`TermKind::ArrayCtor`, `src/check/terms.rs:1033`), a third IR
path (`src/ir/func_builder/calls.rs:74`), the `array_ctor_ahead` term-parser
lookahead (`src/parser.rs:3990`), and the `zero_safety` flag in
`check_array_element_gate` — all for a form that overlaps with `fill`.

Dropping it also eliminates `array_ctor_ahead`, the term-level lookahead that
distinguishes `[Type; Count]` from a quotation. That is the same class of heuristic
S6 deletes from the type parser by naming the array type; removing it here makes
S6's "bare `[` unambiguously opens a quotation" truly clean in the term parser too,
not just the type parser.

Migration is mechanical: 6 usages in `examples/array_ctor.sth`, all rewriting to
`fill` — `[i64; 10]` → `0 10 fill`, `[Bool; 4]` → `False 4 fill`,
`[i8; 10]` → `0 >i8 10 fill`. The example itself stays (it still tests the store
loop overwriting dirty stack residue); it becomes a `fill` test rather than a
separate ctor test.

No performance is lost. The memset-zero path survives as a `fill` lowering
optimization: when the seed's bit pattern is all zeros (a discriminant-0 nullary
variant, or integer `0`), `fill`'s store loop can lower to the same byte-granular
memset the `[Type; Count]` path uses today. The optimization is a lowering choice
inside `fill`, not a separate surface construction form. This keeps the language's
construction story uniform: every array is built value-level, through a constructor
that takes a seed or a quotation — never a type.

### 2. Disposal: array destructor synthesis

An array of linear elements needs synthesized element-wise glue with a static trip
count. Today, `emit_drop` (`src/ir/func_builder/quotation.rs:394`) has
`unreachable!` for linear arrays (`:412`), and
`synthesize_aggregate_destructors` (`src/ir/destructors.rs:37`) does not handle
arrays at all. Non-linear arrays drop as a no-op (the `_ => {}` arm) — no
per-element disposal, no free call (arrays are stack-allocated).

The work is a `synthesize_array_destructor` (mirroring
`synthesize_struct_destructor`, `src/ir/destructors.rs:310`) that emits a loop: for
each element 0..N, load the element, call `emit_drop` on it. Since the array is
stack-allocated, there is no allocation to free — only per-element disposal. The
`array_drop_symbol` (mirroring `struct_drop_symbol` / `enum_drop_symbol`) names the
synthesized function, and `emit_drop`'s `IrType::Array` arm calls it instead of
`unreachable!`.

The trip count is static (the array's `N` is a compile-time constant), so the loop
can be either a constant-trip-count IR loop or an unrolled sequence of
load-and-drop instructions. The constant-trip-count loop is simpler to generate and
matches `fill`'s loop pattern; unrolling is an optimization the QBE backend can
apply if it chooses.

### 3. The partially-initialized window

During `tabulate`'s construction, elements 0..i are live and i..N are uninitialized
— the first place in the language where a value is neither wholly live nor wholly
disposed. The IR already crosses this boundary inside `fill` (raw storage is
allocated, written in a loop, and only surfaces as a value after the loop
completes), but the type system does not: `fill`'s output is a complete `Type::Array`
that appears on the stack only after `finalize_loop` + `push dst`.

For `tabulate`, the same pattern holds: the array does not surface as a
type-system value until the loop completes and every slot is written. The
quotation's output type is `T` (a single value), and the array type is `[T N]` —
the type system sees the quotation producing one `T` per iteration and the array
appearing whole after the loop. The partially-initialized window exists only in the
IR, not in the type system, so no new type-system concept is needed.

A partially-constructed array abandoned mid-construction — the quotation calls
`drop` or panics — is either rejected with a located error (the quotation's effect
must produce `T`, not consume it) or disposes exactly the slots already
initialized. The simplest rule: the quotation's effect is `~[ -- T ]` (produces,
does not consume), so the quotation cannot `drop` the element it is building. If the
quotation itself calls a word that aborts (a runtime trap), the process exits and
the partially-initialized array is never observed. This is the same behavior `fill`
has today: if `fill`'s seed construction traps, the process exits.

## What this does not touch

- `is_copy` (`src/check/builtins.rs:219`) — unchanged. The relaxation is a new
  condition alongside `is_copy`, not a modification of it.
- `Slice[T]` views — a view does not own what it points at, so linear elements
  through a slice are unchanged (the slice borrows, the array owns).
- `fill`'s existing semantics for copy types — unchanged. `fill` still replicates a
  copy-type seed; the relaxation only admits nullary-variant seeds for linear types.
- The `len` constant fold (`src/ir/func_builder/word_families.rs:508`) — unchanged.

## Open questions for the spec

1. **Slot variant-identity tracking.** Adding a field to `Slot` (which is `Copy` and
   pervasive in the checker) is a mechanical change with wide blast radius. The
   spec should evaluate whether a narrower mechanism — threading the variant index
   only through the `fill` check path, without storing it on every slot — is
   possible, or whether the `Slot` field is the clean answer.

2. **`tabulate` as word family vs. library word.** `tabulate` must allocate an
   array and manage the storage boundary (IR-level work), which argues for a word
   family like `fill`. But the quotation splicing is the same D5/D8 mechanism `times`
   uses as library code. The spec should confirm that the allocation + splicing
   combination requires the word-family path, or whether a library word with an
   intrinsic hook suffices.

3. **Destructor: loop vs. unroll.** A constant-trip-count IR loop is simpler to
   generate; an unrolled sequence is simpler to verify. The spec should pick one
   and state why.

4. **`fill` memset optimization.** Whether to implement the all-zeros-seed memset
   optimization (lowering `fill` to a byte-granular memset when the seed is
   discriminant-0 nullary or integer `0`) in this slice or as a follow-up. The
   optimization preserves the performance of the dropped `[Type; Count]` path but is
   not required for correctness or for the linear-element exit criterion.

## Out of scope

- A dynamically-sized or growable array (that is a library `Vec` over an allocator,
  and needs a struct header length variable that **P7.S3n** named and did not land).
- A linear element reached through a `Slice[T]` view, since a view does not own
  what it points at.
- Zero-cost reservation without a sentinel (P11, pending a concrete RT consumer).
  The `Option[T]` sentinel approach initializes N `None` slots at construction and
  overwrites them as values arrive, which is bounded and predictable but not free.
  A zero-cost reservation (allocate raw, gate access by a runtime length, never
  let the type system see an uninitialized slot) is mechanically available at the
  IR level but would be a carve-out from the "no uninitialized memory in the type
  system" principle, the same shape as the static-storage carve-out from linearity
  (`docs/design/embedded.md`). Deferred to P11, where a concrete RT program can
  prove the sentinel init cost bites.
- A `Default` trait (a `Default`-based construction would still be replication under
  another name).

**Exit:** `[T N]` admits a linear `T`; such an array can be constructed without any
element being copied (via `tabulate`, which produces N distinct values, or a
nullary-variant seed that carries no linear data); dropping it disposes every
element exactly once (via synthesized array destructor glue with a static trip
count); a partially-constructed array abandoned mid-construction is either rejected
with a located error or disposes exactly the slots already initialized, with the
rule stated; the `[Type; Count]` constructor is gone, its usages migrated to `fill`,
and the `array_ctor_ahead` term-parser lookahead is deleted; and the `linear array
elements are not supported yet` diagnostic is gone rather than reworded.
