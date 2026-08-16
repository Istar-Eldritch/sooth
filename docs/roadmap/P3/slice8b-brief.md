# Phase 3 Slice 8b — Resources as linear values + user destructor bodies (brief)

The Phase 3 exit: open/read/close a file, with the compiler catching a deliberate double-use and
a forgotten `close`. This is where a user can first attach their own cleanup code to a type,
rather than only inheriting disposal by composition.

Depends on **8a** (`phase3-slice8a-brief.md`) for `extern:` and string slices, so that `close`
exists as a real second client to design the mechanism against, alongside slice 2's `free`
(pointer + size) — ROADMAP's original entry asked for exactly those two dissimilar clients and
warned that designing from the buffer alone would be guessing.

Slice 7's opt-in RC is deferred to Phase 6.

## Recon: what already exists (measured, not assumed)

**1. Every destructor is synthesized field-composition glue; none has a user body.**
`synthesize_aggregate_destructors` (`src/ir.rs:894-928`) builds one `IrFunc` per linear struct,
linear enum, and *every* cell. `synthesize_struct_destructor` / `synthesize_enum_destructor`
(`src/ir.rs:1127`, `:1178`) dispatch on `recursive_disposal_path` (`:970`) and recurse into
fields; `synthesize_cell_destructor` (`:1217`) calls the fixed `FREE_SYMBOL` then drops a linear
payload. Nowhere does a user-written body enter this graph.

**2. Linearity is structural** (`is_copy`, `src/check.rs:173-193`): a struct/enum is linear iff a
field is, transitively; an array iff its element is; a cell always. **One exception already
exists**: `Type::Spy` is linear unconditionally with no fields at all, a scalar-shaped
always-linear marker whose drop is a hardcoded call to `SPY_DROP_SYMBOL` (`src/ir.rs:37`), wired
into the same generic drop dispatch as struct/enum/cell (`:2638`). That is the precedent for "a
type linear by declaration rather than by composition", and this slice is its general,
user-facing form.

**3. `drop` is an ordinary, explicit, user-callable word** `( T -- )` (`src/check.rs:3914`,
`src/ir.rs:1813`), not compiler-inserted. Nothing auto-drops; an unconsumed linear value at end
of body is the existing "forgot to dispose" error. So the *only* route to destructor
self-recursion is a user body explicitly writing `drop` on the type being destructed — there is
no implicit path, because there is no implicit drop.

**4. The existing non-`Copy` `dup` diagnostic does not carry a reason.** Live output for a
struct holding a `^`:

```
error: cannot `dup` a value of type `B` in `main` (line 4)
  `B` is linear: it owns a resource and has no `Copy` instance, so there are no bits to copy;
  thread the value through instead
```

That reads fine when the declaration visibly holds a `^`. For `type: File fd i64 ;` nothing in
the declaration looks linear, so this message would tell the user a type made of one `i64`
cannot be copied without saying why. D5 below.

## Decided (locked, one at a time)

**D1. No new declaration form. A user destructor is an overload of `drop` for a concrete type.**
The type is an ordinary `type:` struct; defining `: drop ( File -- ) ... ;` is what makes it a
resource:

```
type: File fd i64 ;

: drop ( File -- )
  | f | f File>fd close drop ;
```

An earlier draft added two keywords (`resource:` to force linearity, `destructor:` to attach a
body). Both are unnecessary: the *existence* of the `drop` overload is itself the marker that
forces linearity, and the body has an obvious home in ordinary `:` syntax under a name that is
already reserved. Disposal at use sites was always going to be the plain `drop` word, since
`drop` already dispatches by type to a struct/enum/cell's synthesized destructor (recon 1, 3);
for a resource it dispatches to the user body instead. Nothing new at the call site.

The one genuinely new mechanism this needs: **`drop` becomes overloaded by input type**, a
builtin generic fallback plus a user override for one concrete type. That is a miniature, early
instance of Phase 4's planned ad-hoc static overloading (`+` over `i64`/`f64`/`Vec2`) rather
than a parallel mechanism, which is the cheaper of the two directions.

**D2. Defining `drop` for a type forces it linear. `Copy` and a user destructor are mutually
exclusive.** A struct whose only field is `i64` would otherwise be `Copy` by recon 2's fold. If
it stayed `Copy`, `dup` would yield two values, each discarded independently, so the body would
run twice for one logical resource — a double-close. The compiler cannot distinguish an
idempotent log from a destructive `close`, so the safe rule is the only rule. **Rust enforces
exactly this** (E0184, "the trait `Copy` cannot be implemented for this type; the type has a
destructor"). Every apparent counterexample resolves the same way: a non-owning borrowed handle
should simply not define `drop` (Rust's own `OwnedFd` has `Drop` and is not `Copy`, while
`BorrowedFd` is `Copy` with no `Drop`), and `Rc` — custom drop plus cheap duplication — is
`Clone`, not `Copy`, precisely because each duplicate must run refcount logic. In Sooth that
would need an overloadable `dup`, which does not exist and is Phase 6's deferred RC problem.

**D3. The body runs *instead of* the synthesized field glue, never before or alongside it.** The
struct's field-disposal glue does not run at all for a type with a user `drop`. This is not a new
rule but the existing one ("nothing auto-drops; forgetting to dispose is a compile error")
applied to the destructor body, exactly as it already applies to every word body: if the resource
holds other linear fields, the existing move/must-consume checker already forces the body to
account for each of them by its end, with no new machinery. Running both would double-dispose
every field the body already consumed, and "before"/"after" would each need their own rule for
what the glue is allowed to touch. Note this is where Sooth diverges from Rust/C++/Ada, all of
which run the user body *and then* the field glue unconditionally — they can afford it because
they are affine (a value may be dropped implicitly, zero or one times), whereas Sooth is linear
and the body is answerable for its own fields.

**D4. Self-recursion is closed by whole-program call-graph reachability, not just a direct
self-call.** A narrower cut (reject only a literal `drop T` written inside `T`'s own `drop`
body) leaves a hole: a body that calls a helper word, which itself (perhaps transitively) calls
`drop` on a `T`, hits the same unbounded recursion without tripping a purely syntactic check.
Sooth has no generics, so every call site's target is resolved once, at compile time — this is
reachability over a fixed graph, not interprocedural type inference. `src/check.rs:1803`
(`check_tail_call_cycles`) already builds almost this: a whole-module adjacency graph, DFS cycle
detection (`find_tail_cycle`), a located error naming the cycle chain
(`mutual_tail_recursion_error`) — scoped to tail-position calls only, for an unrelated reason
(mutual tail-recursion, Slice 6's D3/X1). This slice needs a sibling pass, not a rewrite of that
one: an edge for *any* call (not just tail position), with a call to the overloaded `drop`
resolved to the concrete `drop@T` node the same way `check_term`'s dispatch already resolves it
per call site. For each user-defined `drop@T`, the question is just whether `drop@T` is
reachable from itself — one DFS per resource type, not a full SCC decomposition of the whole
program. This subsumes the direct-call case for free (a self-call is a cycle of length 1) and
generalizes it to any depth, through any number of helper words, with `T>` (the existing
full-destructure word) as the remedy either way.

**Known, accepted limitation: this is reachability, not data-flow, so it is context-insensitive.**
If a helper is called from `drop@T`'s body *and* is separately, legitimately reachable back to
`drop@T` only down some other branch never taken from there, the graph still sees a cycle and
rejects it — a false positive in principle, the standard cost of this class of check (the
existing tail-cycle pass has the identical shape). The remedy is the same one already given:
factor out a distinct helper. State this in the spec as expected behavior, not a bug to chase.

**D5. The non-`Copy` diagnostic must carry the reason.** Per recon 4, extend it to name the
cause when linearity came from a `drop` overload rather than from a `^`-holding field: "`File` is
linear because it defines `drop`". Without this the user is told a one-`i64` struct has no bits
to copy, with nothing pointing at the declaration responsible.

**D6. Composition is unaffected.** An ordinary struct holding a resource-typed field disposes it
like any other linear field, via the same `recursive_disposal_path` field recursion slices 3-4
built. A user destructor is just another kind of leaf, called the way
`synthesize_cell_destructor`'s `free` already is one.

**D7. `Type::Spy`'s hardcoded drop dispatch folds into this same mechanism, as its first builtin
entry, rather than remaining a second, parallel special case.** `Type::Spy`'s drop is currently a
hardcoded `IrType::Spy` match arm (`src/ir.rs:2719`) emitting `Instr::Call(None,
SPY_DROP_SYMBOL, [v])` — already shaped exactly like a resolved user overload's call, differing
only in that its symbol is a hardcoded constant instead of a resolved user-word name. Whatever
registration/lookup D1 introduces for user `drop` overloads gets one builtin entry, `Spy ->
sooth_spy_drop`, and the hardcoded `IrType::Spy` arm is deleted in favor of the same lookup every
other overload uses. Leaving Spy's dispatch as a separate hardcoded path beside a new general
mechanism for the same concern (a linear type with a destructor) is exactly the "different
sections pull different dependencies" signal CLAUDE.md's refactor list calls out. `is_copy`'s
`Type::Spy => false` special case (`src/check.rs:183`) is unaffected either way — it becomes the
same consequence D2 already derives for every other resource type, rather than a second,
independent hardcoded fact about Spy specifically.

**Not this slice: retiring `__spy` entirely.** D7 unifies the dispatch *mechanism*; it does not
remove `Type::Spy`/`IrType::Spy` or migrate the ~250 existing call sites across `tests/phase0.rs`,
`tests/phase1.rs`, `tests/phase3_locals.rs`, and in-crate unit tests in `check.rs`/`ir.rs` that
construct one. That is Slice 8c (ROADMAP.md), a deliberately separate follow-up once this slice's
mechanism is proven against a real resource (`File`) — do not fold it in here.

## Rejection inherited from 8a: multi-output `extern:`

**A multi-output `extern:` panics the compiler**, e.g. `extern: two ( i64 -- i64 i64 ) "two" ;` used at a call site dies at `panicked at src/ir.rs:1952: print: value`. Pre-existing, not a slice 8a regression: a multi-output *user* word (`: pair ( i64 -- i64 i64 ) dup ;`) panics at the identical line with the identical message. No C function returns two values, so rejecting a multi-output `extern:` at its declaration, alongside 8a's other R3 rejections, is cheap and belongs here. The general multi-output lowering hole (the user-word case) is the bigger, separate issue this does not fix.

## Open questions the spec should answer

- **What exactly counts as the `drop` overload.** Presumably a word literally named `drop` whose
  effect is `( T -- )` for a `type:`-declared `T`, with anything else (extra outputs, wrong
  arity, a non-struct `T`) rejected at the declaration. Pin the rule, and pin where the
  registration lives relative to `check_term`'s existing dispatch chain
  (`src/check.rs:2600-2665`), which is where `S>fi`-style intercepts already sit.
- **Whether an enum or array may carry a `drop` overload too, or structs only in this cut.**
  The mechanism generalises for free; the dogfood needs only structs. Recommend structs only,
  and say so, per the convention against pre-solving.
- **Whether the resource's fields may be non-`Copy` in this first cut.** D3 holds either way; a
  scalar-only first cut matches the dogfood (an fd is one `i64`) and is smaller.
- **Ordering against 8a's `extern:`.** The body calls `close`, so the spec should state the
  dependency rather than discover it.

## Dogfood

```
extern: open  ( cstr i64 -- i64 )              "open" ;
extern: read  ( i64 &![u8 64] usize -- isize ) "read" ;
extern: close ( i64 -- i64 )                   "close" ;

type: File fd i64 ;

: drop ( File -- )
  | f | f File>fd close drop ;

: main ( -- )
  "README.md" cstr 0 open | fd |
  fd File | f |
  f File|>fd 0u8 64 fill &!> 64 read .
  f drop ;
```

Exit criteria should include: a second `drop` of the same `File` is a use-after-move **compile**
error, not a runtime double-close; a `File` left unconsumed at end of `main` is a compile error
naming the forgotten resource; `dup` on a `File` is rejected with D5's reason-carrying message;
a `drop` body that drops its own receiver instead of destructuring it is rejected (D4); and the
emitted destructor for `File` contains the user body's `close` call and *no* synthesized field
glue (D3).
