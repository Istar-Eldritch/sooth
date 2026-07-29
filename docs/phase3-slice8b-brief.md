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

**D4. Self-recursion is closed by rejecting `drop` on a `T` inside `T`'s own `drop` body.** Per
recon 3 the only route in is an explicit call, so the guard is exactly that narrow: while
checking the `drop` overload for `T`, a `drop` whose operand type is `T` is a located compile
error pointing at `T>` (the existing full-destructure word) as the remedy. No liveness or
call-graph analysis: it needs only "am I checking `T`'s overload" and "is this operand a `T`",
both available at any word-body check. This must be an error rather than a tail-call-optimised
loop, since it has no base case.

**D5. The non-`Copy` diagnostic must carry the reason.** Per recon 4, extend it to name the
cause when linearity came from a `drop` overload rather than from a `^`-holding field: "`File` is
linear because it defines `drop`". Without this the user is told a one-`i64` struct has no bits
to copy, with nothing pointing at the declaration responsible.

**D6. Composition is unaffected.** An ordinary struct holding a resource-typed field disposes it
like any other linear field, via the same `recursive_disposal_path` field recursion slices 3-4
built. A user destructor is just another kind of leaf, called the way
`synthesize_cell_destructor`'s `free` already is one.

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
