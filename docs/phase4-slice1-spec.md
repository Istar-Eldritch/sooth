# Phase 4 Slice 1: Type variables + row variable + length variables + monomorphization (native)

Base: `main` @ `9f8644c`. Design input: [the brief](./phase4-slice1-brief.md), whose D1–D7
are locked and not reopened here. This spec answers the brief's "Open questions the spec must
answer": the concrete `Sig`/environment representation change, the multi-output call-boundary
ABI (D4), whether `Copy` is an ordinary or privileged constraint (D5), whether the core
shuffles need any lowering change (D3), the surface syntax for each variable and for a bound,
the `max`-over-floats surface (D6), and which existing example is the dogfood (D7).

**Native only.** REPL monomorphization is Slice 2 (D2); nothing here touches `src/repl.rs`.

## What ships

A user `:` word may declare a polymorphic stack effect using three new variable forms:
a **type variable** `'T`, the **row variable** `..s`, and a **length variable** in an array
count position `['T 'N]`, optionally bounded (`'T: Copy`). The checker unifies each such word
against the concrete stack at every call site, checks the bounds against the concrete
instantiation Kitten-style, and records the instantiation; the backend emits one monomorphized
`IrFunc` per distinct concrete instantiation. The long-standing multi-output call panic is
closed by a synthesized aggregate-return ABI, which is also the path a row variable lowers
through once monomorphization has resolved it to a concrete output count. `max` (integers) and
`max-total` (floats, total-ordered) ship as new builtins. The core shuffles
(`dup`/`swap`/`over`/`rot`/`drop`) are unchanged in both stages: they were already polymorphic
by construction.

## Why this is a real slice

Three things that cannot be written today become writable, and one compiler panic is removed:

1. **A length-polymorphic user word.** `[i64 8]` and `[i64 4]` are distinct types (confirmed:
   recon 2), the builtin `len` accepts both but no user word can, so `each`/`fold` over a
   fixed-size array (the whole Phase 4 combinator library and the phase's own exit criterion)
   are unwritable without `'N`. This slice makes them expressible; the combinators themselves
   are Slices 4–5.
2. **A `Copy`-bounded type-polymorphic user word.** `dup`'s soundness depends on `'T: Copy`
   (the linear spine), so a *constraint* appears whether wanted or not (D5). A user word that
   `dup`s a `'T` must be able to require the bound and have it checked at each instantiation.
3. **A multi-output user word that can be called.** Defining `( i64 -- i64 i64 )` checks and
   lowers today (ten tests assert it), but *calling* it panics the compiler: `lower_call`
   builds a result only when `out_arity == 1` and silently drops the rest, desyncing the
   checker-verified stack from the lowering stack (recon 3, reproduced at `src/ir.rs:2233-2246`).
   A `..s` in output position is exactly a word with a statically-unknown output count, so the
   row variable cannot ship on a path that panics on two: closing this hole and lowering a row
   variable are one question, answered once (D4).

Polymorphic `dup`/`swap`/`max` (the literal exit-criterion phrase) is a **test**, not new
code for the shuffles: `check_shuffle` (`src/check.rs:4717`) moves `Slot`s verbatim and gates
only on `is_copy`, and `lower_call`'s shuffle arms (`src/ir.rs:2032`+) dispatch on the runtime
`value_type`, so both are already type-transparent. The novelty is entirely in *user*
polymorphic words, the multi-output ABI, and `max`/`max-total`.

## Locked decisions

Restating D1–D7 as they bind this spec, then the six decisions this spec adds (**S1–S6**),
which are this spec's actual job.

- **D1.** `'T`, `..s`, and `'N` land together as one change to what a signature is. `Sig`
  stops being purely concrete exactly once.
- **D2.** Native only; REPL is Slice 2. No `src/repl.rs` change.
- **D3.** No inliner. Confirmed by S3 below: the core shuffles need no lowering change at all.
- **D4.** The multi-output ABI is decided here (S2), and is the same mechanism a `..s` in
  output position lowers through.
- **D5.** `Copy` is per-variable, not phase-wide. S4 decides *how*.
- **D6.** The float total-order surface lands here (S6).
- **D7.** Dogfood is rewriting an existing example (S5 identifies it), or plainly reporting that
  nothing is touched.

**S1: A signature stops being `Vec<Type>` in exactly one bounded place; the simulated stack
stays concrete.** The blast radius is bounded by *not* adding variable variants to `Type` (which
is `Copy` and threaded through every match and through the checker's `Slot` stack). Instead a new
`PolyType`/`PolySig` (checker-side) represents variables and lives only in a word's declared
effect and in call-site unification. `Type`, the `Slot` virtual stack, and every existing
concrete path are untouched: a monomorphic word still resolves to a concrete `Sig` exactly as
today. See R1.

**S2: The multi-output ABI is synthesized aggregate return.** Weighed against the two
alternatives:

- *Out-parameters* (caller allocs N slots, passes pointers, callee stores through them): needs a
  new calling convention and out-pointer IR, hand-rolls a QBE by-ref ABI, and threads output
  through pointer stores rather than the linear spine's move discipline. Rejected.
- *Carried runtime stack*: that is the escaping-quotation uniform-runtime-stack fallback, which
  depends on Phase 6's alloc layer and defeats the register/WCET goals. Out of scope here.
  Rejected.
- *Synthesized aggregate return* (recon 4's candidate): reuses the **already-shipping**
  `out_arity == 1` struct-return path (`vm-pop ( Vm -- VmPop )` in `examples/vm.sth` returns a
  struct today), synthesizes the exact bundling users write by hand (`VmPop`/`Fetched`/`Popped`),
  needs no new `Instr`, and is count-agnostic so a row-variable-expanded count is free. **Chosen.**
  See R7–R9.

**S3: The core shuffles are checker-only, and in fact need no signature representation at
all.** Confirmed by direct reading: `check_shuffle` (`src/check.rs:4717`) is fully
type-transparent (`swap`/`rot` move slots; `dup`/`over` add an `is_copy` gate) and is intercepted
*before* `env` lookup, so it never consults a `Sig`; `lower_call`'s `dup`/`swap`/`over`/`rot`/`drop`
arms (`src/ir.rs:2032`+) dispatch on the runtime `value_type`, emitting no `Instr::Call`. The
shuffles therefore acquire no `PolySig`, no unification, and no monomorphized `IrFunc`. The
new machinery (R1–R6) is **for user-declared polymorphic words only**; the shuffles' "honest
polymorphic signatures" are a documentation/`hover` concern, not an enforcement one.

**S4: `Copy` is an ordinary required-operation constraint, not privileged.** A type variable
carries a bound set; `Copy` is one entry, resolved at the concrete instantiation by the existing
`is_copy` predicate, exactly as `>` for `max` resolves against the numeric tower, Kitten-style,
no trait objects, no formal trait system. Inside a polymorphic body, copy-ness and `Ord`-ness of a bare variable are answered from its
bound set by the separate `PolyType` body-check (R7), which does **not** modify `is_copy`/`is_linear`
(they keep their concrete-only matches): a `'T: Copy` may be `dup`ed; an unbounded `'T` may not.
Privileging `Copy` in the variable-binding machinery is rejected because
it would foreclose the identical treatment a polymorphic `drop ( 'T -- )` needs in Slice 6 (the
per-type `drop` overload resolution parked by 8b): `drop`, `Copy`, and `Ord` are then one
mechanism pointed at three operations. See R6, R7.

**S5: The dogfood is `examples/stack.sth`, rewritten to return multiple values directly,** and
the plain D7 finding that polymorphic `dup`/`swap` touch no existing example (they already
accepted every type) is reported, not papered over. See "Dogfood".

**S6: `max` is integer-only; `max-total` is the float surface.** `max ( 'T -- 'T )` (informal
spelling; two inputs, one output) is defined for any type whose `>` is a total order (the
integer tower and `usize`/`isize`) and lowers inline to a compare-and-select on the concrete
operand type. Over a float it is a located error naming `max-total`. `max-total ( 'F -- 'F )` is
defined for `f32`/`f64` and orders by the Rust-`total_cmp` rule (a bit-pattern total order that
sorts `-0.0 < +0.0` and places NaN at the ends), surfaced explicitly at the call site rather than
pretending IEEE `>` is total (D6). Both are builtins (inline arms, like `>`); neither is a
library word and neither is monomorphized. See R12, R13.

## Requirements by stage

Requirement IDs `Rn`; diagnostics `Xn` (each a behavioural negative test asserting the specific
message *and* the named identifiers, per the test convention). "Golden" means source-in →
expected-output or source-in → expected-diagnostic, runnable, never an IL-string assertion,
except the one structural requirement (R10) explicitly marked as an emitted-IR assertion.

### Surface syntax and parsing (`src/lexer.rs`, `src/parser.rs`, `src/ast.rs`)

**R1: Three variable forms in a stack-effect declaration.** `'` and `..` are not lexer
delimiters, so each form arrives as a single `Token::Word` and is recognised during effect
parsing (`parse_slot` / `parse_type_expr`, `src/parser.rs:663`/`:709`), never as a resolvable
concrete type name:

| form | spelling | position | meaning |
|---|---|---|---|
| type variable | `'T` | any type-expression slot | one unknown type, ranged over per instantiation |
| length variable | `'N` | the **count** slot of an array type, `['T 'N]` | one unknown array length |
| row variable | `..s` | the **deepest** (leftmost) slot of the input and/or output list | an unknown-length stack prefix, passed through unchanged |

A `'`-led word is a **binding occurrence** the first (leftmost, deepest-first) time it appears in
the effect and a **use** thereafter; the same name in a *type* position and in an array *count*
position is two different variables and is a located error (X1). Lexically `'T` and `'N` are
identical (a leading-apostrophe word); they are distinguished purely by grammatical position
(count slot vs. type slot), so no separate length-variable lexeme is introduced. `..s` may appear
at most once on each side and only deepest; anywhere else is a located error (X2).

**R2: A word with no variables is unchanged.** `parse_effect` still produces a concrete
`StackEffect` for a monomorphic word; the polymorphic representation is attached only when at
least one variable is present, so every existing example and test parses and resolves byte-for-byte
as before (regression-checked, R15).

**R3: Bound syntax: `'T: Copy` at the binding occurrence.** A bound is written immediately after
a type variable's binding occurrence as `'T: Copy` (colon then a capability name), the capability
names in this slice being `Copy` and `Ord`. Multiple bounds are space-separated after the colon
(`'T: Copy Ord`); this slice's tests exercise `Copy` alone (the forcing case) and `Ord` only via
`max`'s internal bound. A bound on a use occurrence rather than the binding occurrence, and an
unknown capability name, are two located errors (X3). The bound binds to the nearest preceding
type variable, terminating at the next slot word or `--`.

### Representation and checking (`src/check.rs`, `src/ast.rs`)

**R4: The representation (S1).** A new checker-side `PolyType`
(`Concrete(Type) | Var(TyVarId) | Array(Box<PolyType>, Len)`, with
`Len = Concrete(u32) | Var(LenVarId)`) and a `PolySig`
(`{ row_in: Option<RowVarId>, inputs: Vec<PolyType>, outputs: Vec<PolyType>, row_out:
Option<RowVarId>, bounds: Vec<(TyVarId, Bound)>` where `Bound ∈ {Copy, Ord}`}`). A`WordDef`
gains an optional `PolySig`(`None` for a monomorphic word). `Type` gains **no** new variant and
the `Slot` virtual stack stays concrete `Type`.`sig_of` is unchanged for the monomorphic path; a
new `poly_sig_of` builds a `PolySig` from an effect containing variables.

**R5: Call-site unification and substitution.** At a call to a word carrying a `PolySig`, the
checker unifies `PolySig.inputs` (deepest-first) against the concrete top-of-stack `Slot` types,
plus `row_in` against any deeper prefix, producing a substitution
`θ: {TyVar→Type, LenVar→u32, RowVar→Vec<Type>}`:

- a repeated `'T` must unify to the *same* concrete type at every occurrence, else X4 (`'T`
  resolved to both `i64` and `bool`, naming both);
- a repeated `'N` likewise for lengths, X4;
- underflow (fewer stack slots than fixed inputs) is the existing `underflow_error`, unchanged;
- a `'N` unifies only against an actual array's count; a non-array where `['T 'N]` is expected is
  the existing type-mismatch error.

θ is applied to `PolySig.outputs` (with `row_out` expanded to `θ(row_in)`), and the resulting
**concrete** `Type`s are pushed onto the simulated stack. From this point every downstream check
(`infer_line`, branch join, must-consume, aliasing) sees concrete types and is unchanged.

**R6: Bound checking at the instantiation (S4).** For each `(v, bound)` in `PolySig.bounds`, the
checker verifies `θ(v)` satisfies it: `Copy` via `is_copy` (`src/check.rs:177`), `Ord` via the
numeric-tower/total-order predicate. Failure is a located error at the *call site*, naming the
variable, the concrete type θ bound it to, and the unsatisfied capability (X5 for `Copy`, X6 for
`Ord`). The message for `Copy` carries the linear-spine reason (a linear value cannot be
duplicated), mirroring 8b's reason-carrying diagnostic.

**R7: Body checking of a polymorphic word, over a `PolyType` stack (not the concrete `Slot`
stack).** A polymorphic word's body is checked once by a dedicated pass, `check_poly_body`, whose
virtual stack holds `PolyType` (R4), **not** `Slot`/`Type`. This is deliberately a *separate*
mechanism from `infer_line`, and that separation is what makes the representation possible: no
placeholder is ever pushed onto `Slot.ty` (a concrete `Type` with no variable variant, S1/R4); the
concrete `Slot` stack and every concrete path stay untouched; `is_copy` (`src/check.rs:177`) and
`is_linear` (`src/check.rs:204`, derived from `is_copy`) gain **no** new arm, so their exhaustive
matches and `is_copy`'s `_ => true` tail are unchanged and an unbounded `'T` can never fall through
a catch-all and read as `Copy`; and `is_aggregate` (`src/ast.rs:546`) plus the ~116 concrete
`Type::` match sites are never reached with a variable at all. `check_poly_body` is seeded from
`PolySig.inputs`, with `row_in` an opaque row marker, and dispatches per slot kind with **no
catch-all**:

- **`PolyType::Concrete(t)`**: copy-ness, `Ord`-ness, and every type-directed check delegate to the
  existing predicates on the unwrapped `t` (`is_copy(t)`, the numeric-tower predicate, …), giving
  byte-for-byte the monomorphic answer. Concrete literals and concrete sub-computations in the body
  push `Concrete(_)` slots and are checked exactly as today.
- **`PolyType::Var(v)`**: a *bare* variable supports **only**: the five core shuffles, an operation
  its bound set permits, binding to and reading from a local `| x |`, being passed to a call slot
  that is itself the same variable, and being returned. `dup`/`over` require `Copy ∈ bounds(v)` (else
  X7, naming `v` and the missing `Copy` bound); `>`/`max` require `Ord ∈ bounds(v)` (else X8, naming
  `v`). Every other type-directed operation on a bare `Var` (arithmetic, `.`, a field/array/`@`
  access, a conversion, a concrete-typed call argument) is a located error naming `v`: because a
  `Var` slot satisfies **no** concrete-type predicate, a body a real instantiation would reject can
  never slip through (Key-risk 2). A length variable is opaque except through the already
  length-agnostic `len`/`&>`/`fill`/`@`; the row variable is pass-through only.

Forgetting a bare linear `Var` (unbounded, non-`Copy`) at body end is the existing must-consume
error, checked against the residual `PolyType` stack vs `PolySig.outputs` (`row_out` matched against
the carried row marker). Branch joins inside a polymorphic body reuse `infer_line`'s join rule lifted
to `PolyType` (both arms' residual `PolyType` stacks must be equal); the slice's exercised
polymorphic bodies (criteria 3–5) are straight-line, so this rule is stated, not stressed. A
polymorphic body calling *another polymorphic* word with a variable propagated is out of scope this
slice (see R14); the bodies under test call only the inlined shuffles/`max` and monomorphic words.

**R8: Instantiation recording.** Each distinct ground θ for each polymorphic word is recorded in
a per-module specialization set (deduped structurally, mirroring `intern_array_type`), consumed by
lowering (R9). A word instantiated at only one concrete shape yields one specialization; at K
shapes, K. Bundle-struct interning (R10) happens in the same check-time step and into the same
`module` registries: whenever a word's *concrete* output count is >= 2, whether a monomorphic word
like `pair` or a resolved instantiation of a polymorphic one, the checker interns its bundle struct
into `module.structs` (deduped by output tuple, exactly as `intern_array_type` interns a shape into
`module.arrays`), and the specialization carries the bundle's `StructId`. Bundle interning is
therefore gated on output count, not on polymorphism, so a monomorphic multi-output word gets a
bundle even though it has no θ entry. R8 and R10 name the same home, `module.structs`, filled at
check time; lowering only reads it, it never interns.

### Lowering: monomorphization and the multi-output ABI (`src/ir.rs`, `src/backend/qbe.rs`)

**R14: The check→lower instantiation table (the per-call-site carrier).** The name-only `Resolver`
(`src/ir.rs:966`, `&dyn Fn(&str) -> String`) and name-keyed `env` (`Arity`, `src/ir.rs:961`) cannot,
by construction, map one polymorphic word called at two concrete shapes to two symbols or two output
arities: both are keyed by name alone. So the checker emits a side table
`instantiations: HashMap<Span, CallInst>`, keyed by the call site's `Span` (already on every `Term`,
`src/ast.rs:598`/`:5`), and passes it into `ir::lower` alongside `&Module`, the same way
`find_drop_overloads`' result already flows check→lower. `Span` gains a `Hash` derive and the full
`term.span` (not just `term.span.line`, as `lower_term` passes today at `src/ir.rs:2015`) is threaded
into `lower_call`, so a call site's identity survives to lowering. Each `CallInst` records exactly
what `env`/`Resolver` structurally cannot supply for that one call site: the ground `θ`
(`{TyVar→Type, LenVar→u32, RowVar→Vec<Type>}`), the **mangled callee symbol** for that instantiation
(R9's scheme), the instantiation's **concrete `out_arity`**, and its **ordered output `IrType`s**
(the bundle tuple when `out_arity ≥ 2`). `lower_call`, for a call whose callee carries a `PolySig`,
reads `instantiations[term.span]` and emits `Instr::Call` to `CallInst.symbol` (not
`(self.resolve)(name)`), using `CallInst.out_arity`/`CallInst.output_types` for R10/R11's pack/unpack
(not the name-keyed `env`). A monomorphic call has no table entry and takes the existing
`env`/`Resolver` path unchanged. Because a polymorphic body calls nothing polymorphic in this slice
(R7), each call-site `Span` resolves to exactly one ground `CallInst`, so the `Span` key is
unambiguous in-slice; nested polymorphic calls (which would need one entry per enclosing
instantiation) are out of scope, deferred to the Slice 5 inliner.

**R9: Monomorphization: one `IrFunc` per instantiation.** For each recorded specialization
`(word, θ)`, `ir::lower` substitutes θ into the word's effect and body types and emits one
`IrFunc` under a **mangled** name keyed on θ's ground types, reusing the existing symbol-mangling
scheme (`struct_drop_symbol`'s epoch-suffix shape, `src/ir.rs:288`; the same `mangled_symbol`
device 8b used). The mangling is a **pure, deterministic function of `(word, θ)`** with no
lowering-order dependence, and that function is the single shared source of truth for the
instantiation's symbol: Phase 2 computes `CallInst.symbol` in the R14 table by calling it, and
Phase 3 emits this `IrFunc.name` by calling the same function on the same `(word, θ)`, so the
call-site key and the emitted symbol can never disagree even though they are produced in different
phases. A monomorphic word lowers once under its plain name, unchanged. A call site
resolves to the mangled symbol for its own instantiation through the R14 instantiation table (not the
name-only `Resolver`, which cannot key on θ). Because θ is
ground, the monomorphized body carries concrete array types with concrete `N`, so
`lower_array_word`, `&>`, `@`, and `len` need no length-variable handling: length polymorphism
is fully discharged by monomorphization (confirming recon 2's "the compiler is already
length-polymorphic by hand"). Builtins are exempt (S3).

**R10: The multi-output aggregate ABI (S2), callee side.** A word (or monomorphized
instantiation) whose **concrete** output count is ≥ 2 gets a synthesized *bundle struct*
`__ret$<tuple>` **interned at check time into `module.structs`**, deduped by its output-type tuple,
exactly the way `intern_array_type` interns an array shape into `module.arrays` (R8). The bundle is a
`StructDecl` carrying a new `is_bundle: bool` flag (`false` for every user `type:` struct, `true`
only for a synthesized bundle), a separately-set bit mirroring how `StructDecl::has_drop_overload`
is set rather than re-derived from the fields. Interning is done by the checker, not by lowering:
`Registries` holds only shared `&structs` refs with no interior mutability (`src/ir.rs:518`), so
`lower_word`/`lower_call` can never mint a new `StructLayout`. Because the bundle lands in
`module.structs` before the layout pass, `build_registries` lays it out into `structs.layouts` like
any user struct, computing its `size`/`align`/`is_linear` from its fields automatically and copying
`is_bundle` onto a new `bundle: bool` flag of its `StructLayout` (`src/ir.rs:188`, beside
`is_linear` at `:195`), exactly as it already copies `has_drop_overload` (`src/ir.rs:717`). That
flag is how the registry tells a bundle from a user struct, and it suppresses destructor synthesis
(R11): `synthesize_aggregate_destructors` (`src/ir.rs:1106`) iterates `structs.layouts` (bundles
included, because they were interned before the layout pass) and filters
`layout.is_linear && !layout.bundle`, so a bundle acquires no drop glue even when a field is linear.
The backend emits it through `IrModule.structs = structs.layouts` like any other struct. This is why
the filter is *enforced by construction*, not asserted (B4): were the bundle instead interned into a
side `Vec` during lowering, `synthesize_aggregate_destructors` would never iterate it and the
backend would never emit it. Both `lower_word` touch-points move:
the single-output ret projection `let ret = word.effect.outputs.first().map(...)` (`src/ir.rs:1705`)
must, for arity ≥ 2, yield the bundle's `IrType::Struct(bundle_id)` (the `StructId` the checker
interned for this word's output tuple, R8) rather than only the first output's type; and the
finalization `let result = if ret.is_some() { b.stack.pop() }`
(`src/ir.rs:1761`) allocates the bundle (`alloc_struct`), stores the top `out_arity` stack values
into its fields deepest-first, and returns it via `Terminator::Ret(Some(bundle))`. `IrFunc.ret`
becomes `Some(IrType::Struct(bundle_id))` and `env`'s `ret_ty` follows (derived at `src/ir.rs:1005`
from the word's outputs; for arity ≥ 2 it is the bundle `StructId`, which is what lets a monomorphic
multi-output caller read the bundle type straight from `env`, R11). `Instr::Call` keeps its
single `Option<Value>`. No new IR variant. Structural check: a two-output word's emitted body ends
in one `Ret` of a struct value, with the two outputs stored into it.

**R11: The multi-output aggregate ABI, caller side (closes recon 3).** `lower_call`'s fallthrough
(`src/ir.rs:2233`) stops discarding results when `out_arity >= 2`. Where the `out_arity` and bundle
output types come from splits by mono-vs-poly: For a **monomorphic** multi-output call (`pair`, the
dogfood's `pop`/`peek`, the `( -- ^i64 i64 )` cell word: criteria 2, 8, 10) they come straight from
the name-keyed `env`: its `Arity` already carries the output count (`src/ir.rs:961`) and its `ret_ty`
is the bundle `IrType::Struct(bundle_id)` R10 set (derived at `src/ir.rs:1005`), so the caller takes
the existing `env`/`Resolver` path with no table lookup (R14). Only for a **polymorphic per-θ
instantiation** (a type- or row-variable word whose output count and/or bundle tuple vary per θ:
criteria 3, 5) does a single `Arity` per name fail to represent the per-θ shape; there the
per-instantiation `out_arity` and bundle output types come from the R14 table (`CallInst`), keyed by
call-site `Span` (the same root cause as the caller-side symbol-resolution gap, fixed once by R14).
Either way `lower_call` receives the single bundle value and immediately **unpacks**
it into `out_arity` field loads pushed back onto the stack deepest-first (the reverse of R10's pack,
reusing the destructure path the generated `S>` word already uses), so the caller's lowering stack
matches the checker-verified stack exactly and the `print: value` / subtract-overflow panic is gone.
A field that is itself linear is moved out by the unpack exactly as `S>` moves a linear field; the
bundle shell is then dead with no owned bytes. It never runs a destructor, and this is *enforced*,
not asserted: R10 flags the interned bundle struct (`bundle: bool`) and
`synthesize_aggregate_destructors` skips flagged structs, so no drop glue is ever synthesized for it:
the one mechanism that could have double-freed the moved-out linear field. The shell is also never
bound to a local and never surplus-checked (it exists only across the two adjacent pack/unpack
steps). This is the same code path a row variable reaches: monomorphization (R9) has already
resolved `row_out` to a concrete count, so a row-variable word and a fixed `( i64 -- i64 i64 )` word
lower through R10/R11 identically. **One mechanism, D4 satisfied.**

### `max` / `max-total` (`src/check.rs`, `src/ir.rs`, `src/backend/qbe.rs`)

**R12: `max` (integers).** A new builtin arm in the shuffle/operator dispatch (checker) and in
`lower_call` (lowering): checker signature `( 'T 'T -- 'T )` with an internal `Ord` bound, accepted
for any two operands of one integer-tower type (`i8..i64`, `u8..u64`, `usize`, `isize`) and
rejected for two floats (X9, naming `max-total`) and for a type mismatch or a non-`Ord` type (the
existing operator-type errors). Lowers inline to a `Cmp(Gt)` plus a conditional select of the
larger operand (a two-block select or QBE's compare-and-pick), on the concrete operand width. No
`Instr::Call`, no monomorphization.

**R13: `max-total` (floats).** A new builtin arm accepting exactly two `f32` or two `f64`
operands, lowering to a total-ordered maximum by the `total_cmp` bit-pattern rule (map the IEEE
bits to a monotone key: flip all bits if the sign bit is set, else flip only the sign bit, then
integer-compare), inline, no `Instr::Call`. Applied to non-floats it is a located error (X10,
directing integer operands to `max`). The lowering emits no float `>`; the golden asserts the
total-order result on operands where IEEE and total order agree, and R13's negative test asserts
that `max` refuses floats (X9) so the two surfaces stay disjoint.

### Regression (`tests/`, existing suite)

**R15: Addition-only regression (referenced by R2).** No existing golden or unit test changes its
expected output as a result of this slice: every monomorphic word parses, checks, and lowers
byte-for-byte as on `main`, the `Slot` stack stays concrete `Type`, and `is_copy`/`is_linear` are
untouched. The check is the existing suite passing unmodified, plus the two `stack.sth`-diff goldens
(criteria 1, 8) confirming the rewrite's output is unchanged; a diff to any pre-slice
`.expected`/assertion is a regression, not an update.

## Success criteria

Every criterion maps to a runnable golden; each Xn maps to a behavioural negative test asserting
the specific message and named identifiers. Goldens live in a new `tests/phase4_generics.rs`;
unit tests sit beside their stage (`src/lexer.rs`, `src/parser.rs`, `src/check.rs`, `src/ir.rs`,
`src/backend/qbe.rs`).

| # | criterion | kind | maps |
|---|---|---|---|
| 1 | polymorphic `dup`/`swap` on `i64`, `bool`, and a struct in one program run correctly (already type-transparent; pins it) | golden, run | S3, R2 |
| 2 | a user word `: pair ( i64 -- i64 i64 ) dup ;` called as `5 pair . .` prints `5` then `5` (recon-3 repro, no longer panics) | golden, run | R10, R11 |
| 3 | a user word with a `'T: Copy` type variable, called at two concrete types, runs and prints both | golden, run | R1, R4–R7, R9, R14 |
| 4 | a length-polymorphic user word over `[i64 4]` and `[i64 8]` runs (the recon-2 "unwritable" case, now written) | golden, run | R1, R5, R9 |
| 5 | a row-variable user word (e.g. `( ..s 'a 'b -- ..s 'a 'b 'a 'b )`, `'a`/`'b: Copy`) runs, exercising a ≥2-output row expansion through R10/R11 | golden, run | R1, R5, R9, R10, R11, R14 |
| 6 | `max` over `i64`, over `u8`, and over `usize` prints the larger operand each | golden, run | R12 |
| 7 | `max-total` over two `f64` and two `f32` prints the total-ordered larger | golden, run | R13 |
| 8 | the dogfood (`stack.sth`, rewritten) runs with output unchanged: `3` `3` `2` `1` `16` | golden, run | R10, R11, S5 |
| 9 | a two-output word's emitted body ends in one struct `Ret` with both outputs stored (not a dropped second value) | structural (emitted IR) | R10 |
| 10 | a two-output word with a linear output field (`( -- ^i64 i64 )` via a Phase 3 owned cell) runs, freeing the owned cell exactly once (no double-free, no leak), and its interned bundle struct carries no synthesized destructor | golden, run + structural | R10, R11 |
| X1 | one `'`-name used in both a type slot and a count slot is a located declaration error | negative, message + `'T` | R1 |
| X2 | `..s` in a non-deepest position, or twice on one side, is a located error | negative, message | R1 |
| X3 | a bound on a use occurrence, or an unknown capability name, are two located errors | negative, message + name | R3 |
| X4 | `'T` (or `'N`) forced to two different concretes at one call site is a located error naming both | negative, message + both types | R5 |
| X5 | instantiating a `'T: Copy` word with a linear concrete type is a located call-site error naming the variable, the type, and the linear reason | negative, message + `'T` + type | R6 |
| X6 | instantiating a `'T: Ord` requirement with a non-`Ord` type is a located error | negative, message | R6 |
| X7 | `dup` of an unbounded `'T` inside a polymorphic body is a located error naming the missing `Copy` bound | negative, message + `'T` | R7 |
| X8 | `>` on an unbounded `'T` inside a body requires `Ord` (located error) | negative, message + `'T` | R7 |
| X9 | `max` on two floats is a located error naming `max-total` | negative, message + `max-total` | R12 |
| X10 | `max-total` on two integers is a located error directing to `max` | negative, message + `max` | R13 |

## Dogfood

`examples/stack.sth`, rewritten. Today `pop`/`peek` bundle their two results (the updated `Stack`
and the read value) into a hand-written `Popped` struct precisely because "a user-defined word may
only return one value through a call" (the file's own comment). With the multi-output ABI (R10,
R11) they return directly:

```
: pop  ( Stack -- Stack i64 ) ... ;
: peek ( Stack -- Stack i64 ) ... ;
```

`type: Popped`, and every `Popped>` destructure at the call sites, are deleted; `main` calls
`pop .` / `peek .` directly. Output is unchanged (`3` `3` `2` `1` `16`), so the golden pins the
rewrite behaviourally. `Stack` is all-`Copy`; the trailing `drop` discards the items array that `Stack>items` yields
(having consumed the `Stack`), exactly as in the original.

**The plain D7 finding (reported, not papered over):** polymorphic `dup`/`swap` simplify **no**
existing example, because they already accepted every type: the shuffles were type-transparent
before this slice (S3). `max`/`max-total` also touch no existing example (no checked-in program
computes a maximum), so they are introduced by golden only. The honest example-level win of this
slice is the multi-output ABI removing hand-bundled result structs; `list.sth` (`Popped`/`Summed`)
and `vm.sth` (`VmPop`/`Fetched`) are the same pattern and could be migrated later, but are left
untouched here to keep the dogfood to one example (the other four combinator-driven simplifications
arrive with Slices 4–5).

## Non-functional

- **Green** unchanged: `cargo fmt --check && cargo clippy -- -D warnings && cargo test`.
- **No new `Instr` variant** and no new `Terminator`; the aggregate ABI reuses
  `alloc_struct`/`Blit`/field-load and `Instr::Call`'s existing `Option<Value>` (R10, R11).
- **`Type` gains no variant**; the `Slot` virtual stack stays concrete (S1/R4), so the checker's
  existing invariant (the simulated stack carries concrete `Type`) holds.
- **Backend stays QBE**; `Ptr` stays opaque; the bundle struct rides QBE's existing aggregate C-ABI
  (the same one `vm-pop` uses today). No LLVM, no native backend, no WASM assumption broken.
- **`core` stays `no_std`**; no JIT, no comptime interpreter, no inliner (D3).
- Monomorphization is **compile-time only**; no runtime dispatch, no vtable, no boxed generic.

## Out of scope

Quotations and `call` (Slice 4). The combinator library `each`/`map`/`filter`/`fold`/`while`/`times`
(Slices 4–5). The inliner (Slice 5; D3). Static overloading of `+` and open multimethods
(`generic:`/`method:`), `if`-as-combinator, `Bool`-as-enum (Slices 6–7). REPL monomorphization
(Slice 2; D2), no `src/repl.rs` change, and a polymorphic word defined at the REPL is out of
scope. Generic `type:` declarations (Slice 3): this slice's variables are consumed by word
signatures only, never by a `type:` declaration, even though Slice 3 parameterizes `type:` with
the same variables. A polymorphic `drop ( 'T -- )` (Slice 6): S4 keeps its door open but does not
build it. Migrating `list.sth`/`vm.sth` off their bundle structs. HM inference: unification here is
one-directional (declared signature against a concrete stack), never full type inference. `max`'s
`Ord` as user-writable on arbitrary types beyond the numeric tower.

## Key risks

- **Bundle-struct disposal (R11).** The synthesized bundle must never run a destructor on itself:
  its fields are the real outputs, moved out by the unpack in the same breath. If a bundle field is
  linear and the interning or the disposal fold treats the bundle as an owning aggregate, a double
  free or a leak results. Mitigation: the bundle exists only across the adjacent pack/unpack, is
  never bound to a local, never surplus-checked, and its fields are moved out through the existing
`S>` destructure path, the same shape `Popped` already exercises. A multi-output word with a linear
output field **is** reachable this slice (`( -- ^i64 i64 )` via Phase 3 owned cells), so this is
live, not hypothetical, and is *enforced*, not asserted: R10 marks the interned bundle with a
`bundle` flag on its `StructLayout` and `synthesize_aggregate_destructors` skips flagged structs
(`is_linear && !bundle`), so no destructor is ever synthesized for a bundle even when a field is
linear. Pinned by criterion 10: a `( -- ^i64 i64 )` word run that frees the owned cell exactly once
(no double-free, no leak), with a structural check that the bundle struct carries no destructor
symbol.
- **Body checking with opaque variable placeholders (R7).** A placeholder must unify only with
  itself and must not accidentally satisfy a concrete-type predicate (e.g. `is_aggregate`,
  numeric-op dispatch) and thereby accept a body that a real instantiation would reject. Mitigation:
  the placeholder answers `is_copy`/`is_linear`/`Ord` *only* from its bound set, and every other
  type-directed operation on a bare `'T` is rejected (X7, X8).
- **Instantiation explosion.** K distinct concrete shapes yield K `IrFunc`s. Acceptable at this
  scale (a handful of goldens); no mitigation beyond structural dedup (R8). Flagged for Slice 5,
  where combinator inlining changes the calculus.
- **Row-variable scope creep.** Without quotations there is no library consumer of `..s`; the
  temptation is to over-build its unification. Mitigation: `..s` is passed through opaquely (R7),
  its only exercised consumer is criterion 5's synthetic word, and its lowering is *entirely*
  subsumed by R9+R10+R11 (monomorphization resolves it to a concrete count before lowering sees it).

## Current-state anchors (confirmed against `9f8644c`)

- `Sig { inputs: Vec<Type>, outputs: Vec<Type> }`, `sig_of`: `src/check.rs:22-31`. Fully concrete.
- `check_shuffle`: `src/check.rs:4717`. Type-transparent; `dup`/`over` gate on `is_copy`
  (`src/check.rs:177`); intercepted before `env` lookup. → S3.
- `lower_call` shuffle arms (`dup`/`swap`/`over`/`rot`/`drop`): `src/ir.rs:2032`+. Dispatch on
  runtime `value_type`, emit no `Instr::Call`. → S3.
- The multi-output desync, `src/ir.rs:2233-2246`: `ret = if out_arity == 1 { Some(...) } else
  { None }`, second output silently dropped. → R11.
- `lower_word` finalization pops one output: `src/ir.rs:1761` (and the single-output ret projection
  `let ret = word.effect.outputs.first().map(...)` at `src/ir.rs:1705`); `IrFunc.ret: Option<IrType>`,
  `Terminator::Ret(Option<Value>)` single-valued. → R10.
- `Arity = (usize, usize, Option<IrType>)`: `src/ir.rs:961`; `Instr::Call(Option<Value>, ...)`:
  `src/ir.rs:861`. → R10, R11.
- Aggregate return already ships: `vm-pop ( Vm -- VmPop )` returns a struct via the `out_arity == 1`
  path: `examples/vm.sth:54`. → S2 precedent.
- Symbol mangling precedent: `struct_drop_symbol(id, epoch)`: `src/ir.rs:288`. → R9.
- `parse_effect`/`parse_slot`/`parse_type_expr`/`parse_array_type_expr`:
  `src/parser.rs:644`/`:663`/`:709`/`:804`. `'`/`..` are not delimiters. → R1, R3.
- `type: Popped` bundling in `examples/stack.sth` (and `list.sth`, `vm.sth`): the D7 dogfood. → S5.
- `Term { kind, span }` / `Span { line, col }` (`#[derive(... Eq)]`, no `Hash` yet):
  `src/ast.rs:598`/`:5`; `lower_term` passes only `term.span.line` into `lower_call`: `src/ir.rs:2015`.
  → R14 (call-site key).
- `StructLayout` (bundle-flag home, beside `is_linear`): `src/ir.rs:188`/`:195`;
  `synthesize_aggregate_destructors` filters on `is_linear`: `src/ir.rs:1106` (filter at `:1122`).
  → R10/R11 (bundle-destructor suppression).

## Phases JSON

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "Multi-output aggregate-return ABI (S2), monomorphic path only (criteria 2, 9, 10 are all monomorphic multi-output words, so this phase needs no variable machinery and no R14 table): at check time, intern a bundle struct into module.structs for any word with a concrete output count >= 2 (a new StructDecl is_bundle flag that build_registries copies onto a new StructLayout bundle flag at src/ir.rs:717, mirroring has_drop_overload), so synthesize_aggregate_destructors filters is_linear && !bundle and never double-frees a linear output field (B4, B5); extend lower_word's two touch-points (the ret projection at src/ir.rs:1705 and the finalization at src/ir.rs:1761) to pack the top out_arity stack values into the bundle and Ret it, setting IrFunc.ret and env's ret_ty (src/ir.rs:1005) to the bundle Struct type (R10); stop lower_call's fallthrough discarding results when out_arity >= 2 and unpack the bundle back onto the stack via the S> destructure path, sourcing the monomorphic caller's out_arity/bundle type straight from the name-keyed env/ret_ty, no instantiation table (R11). Closes the recon-3 panic (src/ir.rs:2233). No new Instr, no dependency on the R14 table (built in phase 2). Exit: criteria 2, 9, 10.",
      "effort": "high",
      "difficulty": "hard"
    },
    {
      "phase": 2,
      "focus": "Checker-side variable machinery (S1/S4): PolyType/PolySig representation (R4) with no new Type variant and a concrete Slot stack; lex/parse 'T, 'N in count position, and ..s (R1) plus the 'T: Copy bound (R3); call-site unification and substitution (R5); bound checking at the instantiation (R6); polymorphic-body checking over a separate PolyType stack (R7) with is_copy/is_linear left unmodified (bare-variable copy/Ord answered from the bound set, no catch-all); instantiation recording (R8); build the check-to-lower instantiation table keyed by call-site span, carrying theta + mangled symbol + concrete out_arity + bundle output types (R14), the mangled symbol minted by the existing struct_drop_symbol-style mangling primitive (src/ir.rs:288) applied deterministically to (word, theta), the single source of truth both this table key and phase 3's IrFunc.name (R9) are minted from, so they provably agree without phase 2 consuming any phase-3 artifact; reuse phase 1's check-time bundle interning for any instantiation whose resolved output count is >= 2. Checker-only; polymorphic words do not lower yet. Exit: X1, X2, X3, X4, X5, X6, X7, X8 as diagnostic goldens.",
      "effort": "high",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "Monomorphization lowering (R9): emit one mangled IrFunc per recorded instantiation, substituting the ground theta into effect and body types, reusing the struct_drop_symbol-style mangling; resolve each call site to its instantiation's symbol through the R14 instantiation table (not the name-only Resolver); length variables discharged by concrete N in the monomorphized body. Depends on phase 1 (a polymorphic instantiation may be multi-output / row-expanded) and phase 2 (the recorded instantiations and the R14 table). Makes phase-2 polymorphic words run. Exit: criteria 3, 4, 5.",
      "effort": "high",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "max and max-total builtins (S6/D6): inline max over the integer tower (R12) with a compare-and-select lowering and a located rejection of floats naming max-total (X9); inline max-total over f32/f64 by the total_cmp bit-pattern rule (R13) with a located rejection of integers naming max (X10). Builtins only, no monomorphization. Exit: criteria 6, 7.",
      "effort": "medium",
      "difficulty": "medium"
    },
    {
      "phase": 5,
      "focus": "Dogfood and docs (S5/D7): rewrite examples/stack.sth so pop/peek return ( Stack -- Stack i64 ) directly, delete type: Popped and every Popped> destructure, output unchanged 3/3/2/1/16 (criterion 8); record the plain D7 finding that polymorphic dup/swap/max touch no existing example; note the slice's decisions in DESIGN.md/ROADMAP.md; run the addition-only regression check (R2, R15). Exit: criterion 1, criterion 8.",
      "effort": "low",
      "difficulty": "easy"
    }
  ]
}
```
