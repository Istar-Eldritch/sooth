[← ROADMAP](./ROADMAP.md)

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
   recursive data needs the *indirection* and a growable buffer wants Phase 7's `alloc`
   layer. A fixed-capacity heap buffer composes as `^[u8 N]`, size in the type. `^T` is a
   **compiler-known type constructor, not generics** (one interned entry per concrete
   payload, builtin words checked ad hoc at the call site, exactly as `[T N]` arrays work).
   **Tripwire**: `^` is the *second* such ad-hoc type constructor; a third is the signal that
   the special-casing has become the mechanism and Phase 4's generics should subsume all of
   them. Allocation is a single global allocator, deliberately not parameterized per value,
   since a swappable global is cheap to retrofit later while per-value allocators change every
   value's representation. **Half that rationale is wrong, corrected rather than rewritten**:
   under the type-parameter design Phase 7 will use (an allocator type parameter defaulted to
   the global one, carrying a zero-size handle in that case), a per-value allocator does *not*
   change representation. The deferral holds on the other half — it needs generics, which did
   not exist here. See [the brief](./docs/phase3-slice2-brief.md) and
   [the spec](./docs/phase3-slice2-spec.md) for the full decision record.
   **Known limitation, and where it will first hurt**: because a cell is linear and slice 1
   rejects linear array elements, there is **no collection of resources** in this slice, and
   the restriction attaches to the array type itself, so nesting does not launder it
   (`^[^i64 4]` is rejected too). Lifting it needs an element-wise drop loop in the
   synthesized destructor. First real pressure is slice 6, if a set of file handles is wanted
   rather than one at a time.
   **Rework expected, two items.** The allocator is a **compiler-emitted shim** wrapping
   `malloc`/`free` (as slice 1's backend emits the drop-spy's `printf` helper), because there
   is no user-facing FFI yet; once Phase 7 lands FFI-to-libc, it should become ordinary bound
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
   branching structures stays moved to Phase 7** (see there): it needs a growable
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
7. **Opt-in RC (`Rc`/`Arc`-equivalent).** **Deferred to Phase 7**, taking the stdlib-home
   escape hatch this entry always carried. It is not named in Phase 3's exit criteria, no
   current dogfood needs shared ownership, second-class references already cover sharing
   within a dynamic extent, and an arena-plus-index owning container covers graph-shaped data
   without it. It is also the one deliberate crack in the linear spine (refcount traffic, and
   cycle leaks without a `Weak`), which sits badly mid-phase in the slice whose point is
   nailing down deterministic linear disposal. In Phase 7 it lands beside `Box`/`Vec`/`Map`/
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

