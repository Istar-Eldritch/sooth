# Phase 3 Slice 6 — Reference types, places, escape checking (condensed, as implemented)

Base: `main` @ `0e2763f`. Delivered in three phases; all criteria green. Design input: [the brief](./phase3-slice6-brief.md).

## What the slice adds

Second-class references `&T`/`&!T`, borrows of locals, projection through a reference, access via `@`/`!`/`+!`, a per-place aliasing rule, and structural escape prevention. No lifetime system, no new IR instruction.

## Requirements (final decisions only)

**R1 — Two reference types.** `&T` shared and `Copy`; `&!T` mutable, neither `Copy` nor linear (a third disposal category). Neither owns. Constructed only by R2 or by projection (R3, R16). Ordinary types in type position, input side of an effect only (R8).

**R2 — Borrow is prefix on a local: `&a` / `&!a`.** A *place* is exactly a local name. Applied to a literal, a computed value, a word result, or a projection expression: located error. Prefix, because naming a linear local moves it and nothing in the codebase folds backward into a place term; `&a` lexes as one token. `& a` is two tokens; `a&!` is one unknown word. A reference-typed *parameter* takes no sigil (`b`, not `&!b`). Path places (`a Buf>len &!`) unsupported — borrow the whole local, then project. Rejected: the stack-value form `& ( T -- T &T )` (purely additive later).

**R3 — Accessor family: one spelling per shape *and* per mutability.**

| shape | shared | mutable |
|---|---|---|
| struct field | `&T>fi` `( &S -- &Ti )` | `&!T>fi` `( &!S -- &!Ti )` |
| array element | `&>` `( &[T N] usize -- &T )` | `&!>` (same bounds trap as `get`) |
| cell payload | `&^` `( &^T -- &T )` | `&!^` |

Mutability is in the token, never inherited from the receiver. A projected type may itself be linear (`&!Buf>data : &!^[u8 64]`); R4's Copy gate governs access, not projection.

**Suspend rule (mutable only).** Naming a `&!` local is a *reborrow*, so consuming the parent value is not sufficient. Stated over the place: taking a `&!` derived from a place suspends that place while any reference derived from it (any number of projection steps) is live; naming it again in that window is a located error. Shared references carry no exclusivity, so no suspension.

**Name reservation.** `&`/`&!`-led names rejected at every declaration site, mirroring `is_reserved_caret_name`. Additionally `:` may not declare exactly `@`, `!`, or `+!` (no pre-existing shadowing protection for exact-name builtins).

**Type-position splitting, three cases.** Bare (`&!Buf`, splits within one `Word`); `^`-composed (`&!^List`, remainder handed to the existing `parse_owning_cell_type_expr`); `[`-delimited (`&![u8 64]`, splits across tokens). Implemented as `parse_ref_type_expr` (src/parser.rs:687) with unit tests for all three plus the missing-referent error.

**R4 — `@` / `!` / `+!`.** `@` fetches, typed for both `&T -> T` and `&!T -> T` (so no demotion coercion exists); `!` stores `( &!T T -- )`; `+!` adds in place `( &!T T -- )` over integer `T`, with the usual inferred-type bare-literal carve-out (`usize`/`isize`). Restricted to **Copy `T`** — Copy-vs-linear, never scalar-vs-aggregate: a Copy aggregate fetches via `Alloc`+`Blit` and stores via `Blit`, the shape `dup`'s Copy-aggregate arm already uses. Linear `T` is rejected for both (`@` would make a second owner; `!` would silently leak the overwritten value). Criterion: the fetched copy survives later mutation of its source. `Alloc` is entry-block-hoisted, so that golden must not sit in a loop.

**R5 — Exclusivity is the whole aliasing rule, symmetric.** At most one live `&!` per place; no `&` while a `&!` is live; no `&!` while a `&` is live. Consequences, not separate rules: `&T` is `Copy`, `&!T` is not; `dup` of a `&!` is rejected; reborrow is not a move; two live `&!` at *different* places never conflict.

**R6 — Checked at consumption points, keyed on provenance.** On move, dispose, or a conflicting (re)borrow, ask whether anything on the virtual stack or in the locals map traces provenance back to this place through any number of projections — not literal `Value` identity, since a projection yields a new `Value`. `@` terminates provenance. Counters count outstanding derivations. Located error naming both place and conflicting borrow. Rejected: NLL-style liveness.

**R7 — Path disjointness is not modeled.** Two simultaneously live references into disjoint fields of one place are rejected; sequencing them is the workaround (`push-byte` names `i` and `arr` as they are produced, which does the sequencing a `swap` would otherwise do). R16's clause payload bindings are a narrow exemption, sound by construction.

**R8 — Escape prevented by six positional rejections over transitive containment.** Struct field, enum variant payload, array element (`fill`), cell payload (`^`), effect **output** side, and a value surviving to the end of a REPL line into the session's carried stack. Enforced at construction sites, not only declarations (`^` interns any payload; `fill` gates only on `is_copy`, and `&T` is `Copy`). The input-side carve-out accepts only a type that *is itself* `&T`/`&!T` at top level. The REPL case is genuinely reachable since Slice 5 gave REPL lines locals.

`drop` of a reference frees nothing. A leftover reference on the **stack** is still a surplus-value error; a reference-typed **local** is never surplus-checked and expires silently. Consequence: a projection can never be factored into a helper word (`( &!Buf -- &!usize )` is banned output).

**R9 — Loops.** A reference **parameter**, or a reference projected from one, may cross a self-tail-call back-edge (referent lives in an ancestor frame) — this is what makes `walk` legal. Two errors: a reference derived from a current-scope local may not cross; a currently-borrowed local may not be loop-carried (locals rebind at `header_phis`). *As shipped,* the second has no separate implementation or loop-specific wording: it is subsumed by R21's naming-side rule, which already refuses to name a place whose mutable borrow is live, and so fires on the loop-carried case without mentioning the loop. The golden is written so the arms balance, leaving the borrow rule as the only possible cause of rejection.

**R10 — Branch joins: borrow state must agree.** Type unification already rejects shape mismatch; R10 adds that the suspended-place attribution must agree too (identical shapes suspending different places). Tested on both the disagreement and the agreement side. Rejected: a `MaybeBorrowed` lattice.

The *aliasing* side of the same join (R21's identity, not R10's borrow state) is deliberately not symmetric with it: **no aliasing rejection ever happens at a join.** A value carries a *set* of regions rather than one, interned in `Provenance` behind an `AliasSetId` so a `Slot` stays `Copy` (the same device `DerivId` already uses), and the merge takes the union of the two arms. The merge cannot know which arm ran, so keeping every region either arm could have left is what preserves R21's own rule that the error fires at the borrow, where the diagnostic can name both ends and point at `dup`. A projection out of a merged value projects the field out of every member, and the borrow-site scan tests pairwise overlap across the two sets.

Two cheaper merges were tried and both are wrong. *Rejecting the join* whenever the arms disagree is too blunt: it rejects selecting one of two named aggregates (`: bigger ( V V -- V ) | a b | ... if a else b end ;`) and a merge of a named local against a fresh value, which `examples/list.sth`'s `build` does, neither of which ever takes a borrow, and the `dup` it forces is exactly the implicit copy R21 refused to impose. *Keeping one arm's alias* and discarding the other is unsound, verified: borrowing the discarded arm's place is then accepted and prints the mutation back through the merged name.

**R11 — Only aggregate or cell locals may be borrowed.** Scalar local → located error ("borrow a field or an aggregate"); scalars are SSA temporaries with no address. A projection whose *result* is scalar is unaffected.

**R12 — No new IR; a reference is always `IrType::Ptr`.** `PtrOffset` for field, `ElemAddr` for element, `Load` for cell, `FieldLoad`/`Alloc`+`Blit` for `@`, `FieldStore`/`Blit` for `!`, `FieldLoad`+`Bin(Add)`+`FieldStore` for `+!`. `Ptr` stays opaque; no surface pointer arithmetic. `&!Buf` must **not** be `IrType::Struct(id)`: QBE's C-ABI passes a `:Buf` parameter by value, so a callee's store would mutate a caller temporary (measured). Only `ir_type_of` gains arms; `width`/`qbe_abi_ty` unchanged. Soundness, not mechanics: `is_copy` true for `&T`/false for `&!T`, every `is_linear`-shaped predicate false for both, and `Moves::new` plus the back-edge check need explicit reference-local exclusions. References are interned through a `RefId` registry of `(inner, mutable)`, mirroring `ArrayId`/`OwnedCellId`.

**R13 — Mutation emits no rebuild.** Measurable form: `push-byte`'s emitted body contains no `alloc` and no `blit`. Its computed-index `&!>` still emits `bounds_check`'s `Cmp`/`Jnz`/trap block/`Call sooth_oob_trap`, so the count ceiling is set from the measured body including the guard.

**R14 — No parameter-convention keywords.** `let`/`inout`/`sink`/`set` not added: the reference type *is* the convention, unannotated stays `sink`, no signature changes meaning. (`set` is also already a user-callable array word. Separately noted pre-existing bug, out of scope: a general word with two declared outputs panics at `print: value`; `get`'s two outputs work only as a checker/IR special case.)

**R15 — ROADMAP's parked question, answered** (recorded at ROADMAP.md:498-504). `inout` projections **do** subsume a reified take/fill pair for every statically known path, and cover whole-value borrows too. No residual form added; reified residuals wait for the zipper slice's RC.

**R16 — Reference-mode enum elimination.** When a word's declared top input is `&Enum`/`&!Enum`, the existing clause form applies in reference mode, same syntax, four differences: the scrutinee reference is consumed by the dispatch (tag `FieldLoad`, nothing freed or moved, clause bodies start from the value-mode stack shape); payload bindings are references inheriting the scrutinee's mutability (`&!List` binds `v : &!i64`, `next : &!^List`); no clause may consume a payload binding, which *as shipped* turns out to be unrepresentable rather than separately rejected (moving one out is a type error, since the binding is a reference and `^>` wants an owning cell, and fetching a linear referent is R4's existing rejection); a clause's bindings are exempt from R7. `lower_clauses` threads the scrutinee's `EnumId` from the checked frontend `Type`, since under R12 the lowered scrutinee is `Ptr` and the old `unreachable!` would fire. `dispatch_on_tag` short-circuits for single-variant enums, so tag-read goldens use `List`.

**R17 / R18 — Test discipline.** Every criterion is a runnable golden except R13's no-rebuild shape, which asserts on emitted IL. R12's mapping is asserted behaviourally (caller-visible mutation). Structural tests live beside `backend/qbe.rs`, scoped to one named body via `func_body` and pinned to the mangled symbol (`$push_byte`, never `push-byte`). Diagnostics assert the specific error text and location; each rejection gets its own program, since checking fails fast.

**R19 — Purely additive**, demonstrated by `git diff --name-status 0e2763f -- examples/ tests/phase0.rs tests/phase1.rs` showing only `A`. Checked by running that command, deliberately not by a test: an in-suite version hardcoded the base hash, so any squash, rebase or shallow clone that dropped the object would fail the suite for a reason unrelated to the compiler, and it would assert a one-time property forever.

**R20 — `get`/`set` superseded, not migrated here.** `&> @` replaces non-consuming two-output `get` (whose every read-only call site pays a `swap drop`); `&!> !` replaces `set`'s whole-array rebuild. `fill`/`len` are unaffected. The vocabulary only genuinely shrinks because R4 covers Copy aggregates (`vm.sth`'s `[Op N]`). Difference worth stating: `get` on an aggregate element aliases the array's storage; `&!> @` copies out. Migration (rewrite `examples/stack.sth`, `examples/vm.sth`, `tests/phase1.rs`, then delete `get`/`set`) is a standalone follow-up commit — no brief, no spec, no slice.

**R21 — Two live names for one aggregate place: rejected where a `&!` makes it observable.** Naming an aggregate local reuses the same `Value` (one frame slot), and non-consuming projections (`S|>fi`'s `Peek`, `get`) push interior addresses — two routes by which distinct locals denote one region. Pre-existing and Copy-only (every route to a linear value is already closed by move tracking, `S|>fi`'s linear-field rejection, and `fill`'s `is_copy` gate), so no double-free: the failure mode is a wrong *value*, which is exactly what this language converts into a compile error. Fires at the borrow, not the naming, so repeated naming stays legal and `examples/vm.sth` (320-byte `Vm`, `vm` named 38 times) is untouched at zero cost. Diagnostic names both the borrow and the aliasing origin and points at `dup`. No implicit copy: `dup` is the language's explicit copy, and WCET reasoning requires instruction counts readable off the source.

## Load-bearing invariants (survived)

QBE only; `Ptr` opaque, no arithmetic exposed, WASM lowering still possible. Linear spine intact: references never own, R4's Copy gate stops a borrow manufacturing a second owner or leaking an overwritten one, R8 stops a reference outliving its referent, and `&!T`'s third-category disposal is stated rather than left to fall through. `core` stays `no_std`; no JIT, no comptime interpreter.

**Tripwire acknowledged:** `&T`/`&!T` are the third and fourth ad-hoc payload-interned constructors after `Array` and `OwnedCell` (docs/phase3-slice2-spec.md:9 called a third the signal to switch to Phase 4 generics). Sequencing: references are needed in Phase 3, generics are Phase 4. **Revisit trigger:** when Phase 4's ad-hoc dispatch lands, re-examine collapsing `&`/`&!` and the accessor family into overloads of `S>fi`/`get`/`^|>`.

## Delivery, as shipped

**Phase 1 — types, places, projection, access, escape** (`144fe11f`, `aadc18be`). `RefId` registry and the `is_copy`/linearity answers; prefix `&`/`&!` with name reservation and the `@`/`!`/`+!` shadowing rejection; the three-case splitter; the accessor family; `@`/`!`/`+!` incl. Copy aggregates; R11; R8's six rejections plus drop-as-no-op and the surplus rule; R12's lowering. Commit disclosed that R5/R6/R7/R9/R10/R21 do not exist yet, so conflicting borrows, unsafe back-edge crossings and aliased mutable borrows are accepted at that commit. Exit: criteria 1-6, 13, 15.

*Delta:* the follow-up fix added two rejections the spec did not enumerate — borrowing a **moved** local, and borrowing a **reference-typed** local (`&`/`&!` applies to aggregates, not to a reference already in hand): `borrow_of_moved_local_is_error`, `borrow_of_reference_local_is_error`.

**Phase 2 — borrow rules and diagnostics** (`b8e77e35`, `d2914197`, `e2481ca7`). Provenance threading, R5 in both directions, R6's consumption-point scan, R7's rejection plus its sequenced accept-case, R21 over both aliasing routes. Exit: criteria 7-9, 17.

*Delta:* R21 needed more routes and a naming-side symmetry than the spec listed — struct aliased by peeked field and the converse, array aliased by an element name, an `if`-join result, and both aliases sitting on the stack rather than in locals; plus `naming_a_place_while_mutably_borrowed_is_error` / `..._whose_mutable_borrow_is_bound_is_error` / `naming_a_place_after_its_borrow_ends_is_accepted`. Spec text was amended in place.

**Phase 3 — loops, joins, reference-mode enums, dogfood, docs** (`15cfe6ce`, `2678bd08`). R9 both sides, R10 both sides, R16 end to end with the `EnumId` threading, `examples/refs.sth`, R15 into ROADMAP.md, R19's regression check. Exit: criteria 10-12, 14, 16.

*Delta:* added `borrow_join_disagreeing_on_reborrowed_parameter_is_error` (join disagreement over a reborrowed parameter, not just over locals) and `reference_mode_clause_payload_bindings_are_simultaneously_live` (R16's R7 exemption asserted positively).

There is no phase 4; see R20.

## Criterion → test map

Goldens in `tests/phase3_refs.rs` (new file, so criterion 16's addition-only check has nothing pre-existing to reason about). Structural criteria 6 and 13 as unit tests beside `src/backend/qbe.rs` (`mutation_through_reference_emits_no_rebuild`, `rebuild_style_equivalent_still_emits_alloc_and_blit`), plus splitter unit tests in `src/parser.rs`.

| # | criterion | phase |
|---|---|---|
| 1 | `&`/`&!` yield references; literal / arithmetic / word-result borrows are separate errors; `&`-led names reserved; `@`/`!`/`+!` unshadowable | 1 |
| 2 | scalar-local borrow rejected; scalar-*field* projection accepted; `dup` of `&T` ok, of `&!T` not; (added) moved-local and reference-local borrows rejected | 1 |
| 3 | projection reads through field / element (incl. bounds trap) / cell; store through a shared-spelled projection is an error | 1 |
| 4 | `@`/`!`/`+!` read/write/increment; linear fetch and linear store are two errors; Copy aggregate reads and writes; fetched copy survives source mutation | 1 |
| 5 | six escape rejections incl. the REPL carried stack; input-side accepted; `drop` frees nothing | 1 |
| 6 | structural: `push-byte`'s body has no `alloc`/`blit`, under a ceiling budgeting the bounds guard; a rebuild-style control word still has both | 1 |
| 13 | structural: a callee's mutation through a `&!` parameter is visible to the caller | 1 |
| 15 | leftover reference on the stack is surplus; a reference local expires without `drop` | 1 |
| 7 | exclusivity both directions, plus the reborrow-while-derivation-live case; different-places, `&`-is-Copy, and post-consumption reborrow accepted | 2 |
| 8 | consuming or disposing a borrowed place is an error (borrow on stack or in locals); after the borrow ends, accepted | 2 |
| 9 | simultaneous disjoint-field borrows rejected; sequenced accepted | 2 |
| 17 | R21 over every aliasing route (naming, peek, peeked field both ways, array element, `if`-join incl. one-sided arms in both orders, stack-side) plus naming-while-borrowed; a merge of two aliased arms is caught at the borrow from either end, while a join that never gets borrowed is accepted; `dup` fixes it; plain repeated naming still accepted | 2 |
| 10 | reference parameter crosses a back-edge over 1,000,000 nodes in constant stack with the mutation read back; local-derived crossing and loop-carried borrowed local are errors | 3 |
| 11 | borrow live on one arm only is an error at the join (incl. the reborrowed-parameter variant); both arms or neither joins cleanly | 3 |
| 12 | reference-mode clauses bind payloads as references, simultaneously live; fetching a linear payload's referent is an error (consuming a binding is unrepresentable, see R16) | 3 |
| 14 | dogfood end to end: `72`, `90`, `2`, `2` | 3 |
| 16 | full suite green; the regression diff above shows only additions (checked by command, not by a test) | 3 |

## Dogfood (`examples/refs.sth`, as shipped)

```forth
type: Buf  data ^[u8 64]  len usize ;

: new ( -- Buf ) 0 >u8 64 fill ^ 0 >usize Buf ;

: push-byte ( &!Buf u8 -- )
  | b x |
  b &!Buf>len @ | i |
  b &!Buf>data &!^ | arr |
  arr i &!> x !
  b &!Buf>len 1 +! ;

: byte-at ( &Buf usize -- u8 ) | b i | b &Buf>data &^ i &> @ ;

: copy-byte ( &!Buf &Buf usize -- ) | dst src i | dst src i byte-at push-byte ;

: walk ( &!List -- )
  | Nil
  | Cons | v next | v 1 +! next &!^ walk ;
```

`main` borrows two buffers directly (Slice 5's mid-body `| names |`), prints `72 90 2 2`: the byte written into `a`, the byte `copy-byte` moved from `b`, `a`'s length, and a 10-node list's head value after `walk` increments every node in place.

`push-byte` reborrows `b` three times; each derivation is fully consumed before the next reborrow, so `b`'s place is never suspended at a naming and R7 never sees two live derivations. Naming `i` and `arr` as they are produced does the sequencing a `swap` would otherwise do. `byte-at` is entirely shared, so the suspend rule never engages. `walk`'s `next &!^` traces provenance to `walk`'s own parameter, making the tail call a legal R9 back-edge; ownership stays with the caller.

## Out of scope

The stack-value borrow `& ( T -- T &T )` (revisit if examples become build-then-configure pipelines). Path-disjoint borrows. Borrowing a scalar local and its spill. `!` over a linear value with drop-on-overwrite. Raw/foreign pointers (a third pointer kind's only client is hosted FFI, and it must stay an opaque handle). Collapsing the accessor family into Phase 4 overloads. RC, storable references, zippers. User-defined destructor bodies. Worklist-based branching disposal. The `get`/`set` migration itself. Making an aliased *read* an error, and eliminating the aliasing by implicit copying or by fixing the peek's lowering.
