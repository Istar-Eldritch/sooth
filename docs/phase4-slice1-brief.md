# Phase 4 Slice 1 — Type variables + row variables + length variables + monomorphization, native only (brief)

Phase 3 closed the linear spine. Phase 4 opens minimal polymorphism: `'T` type variables and
a `..s` row variable so `dup`/`swap`/`over`/`rot`/`drop`, plus `max` and user words, gain
honest polymorphic signatures instead of the monomorphic Phase 0 shuffles they are today.
ROADMAP.md's Phase 4 section sliced this into eight dependency-ordered pieces; this brief
covers the first, deliberately the deepest and the one everything else depends on. It is
**native only** — REPL monomorphization is Slice 2, split off on purpose (below).

## Recon: what already exists (measured, not assumed)

**1. `Sig` is `Vec<Type>` in, `Vec<Type>` out, fully concrete** (`src/check.rs:22-25`,
`sig_of` at `:27-31`). No variable of any kind exists in it today. Every builtin shuffle
dispatches on the *concrete* operand type at check time (`check_shuffle`, `src/check.rs:4717`)
and at lowering time (`lower_call`'s `"dup"`/`"swap"`/`"over"`/`"rot"` arms, `src/ir.rs:2032`
onward) — there is no generic implementation to specialize, only per-concrete-type code paths
that already exist and already work. This is why the brief below treats "monomorphize the
core shuffles" as possibly a checker-only change with zero lowering work: confirm or refute
this directly rather than assuming either way.

**2. Array length is part of the type, and only the compiler is length-polymorphic.**
Confirmed directly:

```
: total ( [i64 8] -- i64 ) | a | &a 0 >usize &> @ ;
: main ( -- ) 0 4 fill total . ;
```

fails to typecheck (`` `total` expected `[i64 8]`, found `[i64 4]` ``), while

```
: main ( -- ) 0 4 fill len . drop 0 8 fill len . drop ;
```

prints `4` then `8` — the builtin `len` accepts both, `lower_array_word` (`src/ir.rs`) doesn't
care about `N`, only a user-declared signature does. `[T N]`'s `N` has no variable form for a
user word to declare, so `each`/`fold`/`map` — the whole Phase 4 combinator library, and thus
the phase's own exit criterion — cannot be written without a length variable. This is not
optional scope; it is required to make Phase 4's stated exit reachable at all, since the only
Phase 4 collection is the fixed-size array (`Vec` is Phase 6).

**3. A multi-output word panics the compiler, but only when called.** Defining one works fine
today — ten existing tests assert it, including `: w ( -- bool bool bool ) 1 2 <= 1 2 >=
1 2 <> ;` lowering successfully (`src/ir.rs:4560`, `src/check.rs:5765` and five more). The
break is isolated to `lower_call`'s fallthrough arm (`src/ir.rs`, around line 2230):

```rust
let (in_arity, out_arity, ret_ty) =
    *self.env.get(name).expect("checked user word exists");
let split = self.stack.len() - in_arity;
let args = self.stack.split_off(split);
let ret = if out_arity == 1 {
    Some(self.fresh_value(ret_ty.unwrap_or(IrType::I64)))
} else {
    None
};
let sym = (self.resolve)(name);
self.push_instr(Instr::Call(ret, sym, args));
if let Some(v) = ret {
    self.stack.push(v);
}
```

`out_arity >= 2` silently produces no result, desyncing the checker-verified stack from the
lowering-time stack; whatever consumes the missing value panics later (`print: value` from a
bare `.`, or a subtract-overflow from stack-depth arithmetic, depending on what runs next —
both reproduced directly against `: pair ( i64 -- i64 i64 ) dup ; : main ( -- ) 5 pair . . ;`
in the identical spot). `Instr::Call` itself carries only `Option<Value>` and `env` stores a
single `ret_ty`: there is no representation for a second output anywhere on this path. Noted
and passed over three times without a home (the Slice 6 spec's known-limitations note, and
8b's brief and spec, the latter rejecting multi-output `extern:` at the declaration precisely
to dodge it — see `docs/phase3-slice8b-brief.md`'s "Rejection inherited from 8a"). It has a
home now: **a `..s` in output position *is* a word with a statically unknown output count, so
a row variable cannot ship on a lowering path that panics on two.** How two values cross a
call boundary and how a row variable crosses one are the same question — do not answer it
twice.

**4. Aggregate returns already work and already ship.** `vm-pop ( Vm -- VmPop )` in
`examples/vm.sth` is a real, shipped word returning a struct today, because `out_arity == 1`
with a struct `ret_ty` already lowers correctly. This is the leading candidate for the
multi-output ABI: synthesize the same struct-bundling users currently do by hand (`VmPop`,
`Fetched` in `vm.sth`) rather than inventing out-parameters or a carried-stack convention.
Weigh it against those two alternatives explicitly; don't default to it silently.

**5. There is no inliner anywhere in the compiler, and nothing downstream adds one.**
Everything the source calls "inline" means "lowered straight to instructions instead of a
call" (builtins and generated struct/enum words are match arms in `lower_call`/`lower_struct_word`/
`lower_enum_word` that never emit `Instr::Call`); a user `:` word is always a real
`Instr::Call`. Verified directly against QBE itself: a one-instruction function called with a
constant argument still lowers to two `callq` sites in the emitted assembly (`qbe -o out.s
in.ssa` on a two-function `.ssa` file). `src/driver.rs` runs `cc` on the resulting `.s` with
no `-O` flags, so nothing downstream helps either. **This slice does not build an inliner.**
Combinators are specified as ordinary Sooth library words (Slice 4), so inlining them is
inlining monomorphized user words — one mechanism either way, assigned to Slice 5, designed
against its first real consumer instead of two slices ahead of one.

**6. `Copy` cannot be a single global gate on every type variable.** `each`'s planned
signature reads an element through `&> @` (Phase 3 Slice 6 restricts `@` to a `Copy`
referent), so its element variable needs a `Copy` bound. `fold`'s accumulator needs no such
bound and may legitimately be linear (an accumulator can be a resource being built up). Any
constraint mechanism this slice adds must be attachable per type-variable, not phase-wide.

## Decided (locked, one at a time)

**D1. Type variables (`'T`), the row variable (`..s`), and length variables (`'N`) land
together, in this slice, as one change to what a `Sig` is.** Not three separate slices: `Sig`
stops being purely concrete exactly once, and reopening that representation a second or third
time (once for `'T`, again for `'N`) risks designing the substitution/unification machinery
against an incomplete variable set. The cost is size — this is deliberately the phase's
largest slice — accepted because Phase 3's Slice 8 (splitting into 8a/8b/8c mid-flight) and
Slice 3/4 (a follow-on slice needed to generalize what Slice 3 shipped) are both examples of a
boundary discovered late, under load, when a phase's foundational representation was touched
more than once.

**D2. Native monomorphization only. REPL monomorphization is its own slice (Slice 2), placed
immediately after.** `Session` (`src/repl.rs`) retains signatures in `env` but discards
ordinary word *bodies* once a line compiles to a `.so`; a polymorphic word has no concrete
instantiation to compile at its defining line, so REPL support needs a retention scheme this
slice does not build. Splitting mirrors why 8b needed three review rounds: it built the native
and REPL halves of `drop` overloading together, and that combination was Phase 3's largest,
riskiest single slice. Don't repeat that shape here.

**D3. No inliner in this slice** (recon 5). If recon's checker-only hypothesis for "force-
inline the small core words" holds, this slice may add no lowering machinery for the
shuffles at all — confirm this explicitly in the spec rather than building speculative
lowering changes against it.

**D4. The multi-output call-boundary ABI is decided here, not deferred** (recon 3, 4). Pick
one of: synthesized aggregate return (recon 4's leading candidate), out-parameters, or a
carried stack. Whichever is chosen must also be the mechanism a `..s` in output position
lowers through — one decision, not two.

**D5. `Copy` (and any other required-operation constraint) is a per-type-variable attachment,
not a phase-wide rule** (recon 6). `dup`'s own signature is the sharpest forcing case:
`dup ( 'T -- 'T 'T )` is only sound when `'T: Copy`, so this slice must decide whether `Copy`
is an ordinary required-operation constraint (resolved Kitten-style at the concrete
instantiation, same as `>` for `max`) or a privileged one baked into the variable-binding
machinery itself. The signature of a polymorphic `drop ( 'T -- )` is the same question pointed
at 8b's per-type `drop` overloads (parked for Slice 6); this slice's answer must not foreclose
it.

**D6. The float total-ordering decision, deferred from the floats slice, lands here.** Float
`<`/`=` are IEEE-partial, so a `max`/sort over floats needs an explicit total order
(Rust-`total_cmp`-style) surfaced at the call site rather than pretending IEEE ordering is
total. `max` does not exist as a word anywhere in the source today; this slice both introduces
it and settles how it handles floats.

**D7. Dogfood is rewriting existing examples, not a new program.** The exit criterion
("polymorphic `dup`/`swap`/`max`") is a test, not a program, and Sooth's existing example
corpus (`examples/*.sth`) is the honest measure of whether the generics are usable: if a
polymorphic `dup`/`swap` genuinely simplifies something already checked in, that's the
dogfood; if it doesn't touch any existing example, that is itself informative and should be
said plainly rather than papered over with a contrived new program.

## Open questions the spec must answer

- **The concrete `Sig`/environment representation change**: how unification and substitution
  work, what "monomorphise per concrete stack shape" means mechanically (a generated concrete
  `WordDef` per instantiation? A substitution map consulted at each call site?), and how deep
  the required-operation resolution goes (Kitten-style, no formal trait system, per ROADMAP.md
  — spell out exactly what that means for `>`/`Copy`/anything else a generic body calls).
- **The multi-output ABI choice itself** (D4) — pick one, with the recon-4 struct-bundling
  precedent as the candidate to beat.
- **Whether `Copy` is an ordinary constraint or privileged** (D5) — decide the mechanism, not
  just the principle.
- **Whether any lowering change is needed for the core shuffles at all**, or whether this is a
  checker-side-only slice for them (D3/recon 1) — confirm by reading `check_shuffle` and
  `lower_call`'s shuffle arms directly, don't assume.
- **Concrete surface syntax**: the exact spelling of a type variable, the row variable, and a
  length variable in a stack-effect declaration (e.g. `( 'T 'N -- ['T 'N] )`-shaped), and
  whether/how a required operation or `Copy` bound is written on one.
- **`max`'s float surface**: the actual word or convention for a total order over floats
  (D6) — name it.
- **Which existing example(s), if any, are the dogfood** (D7) — the spec should look, not
  assume one exists.

## Out of scope

Quotations, the combinator library, static overloading, open multimethods, `if`-as-combinator,
`Bool`-as-enum (Slices 3–7 respectively). REPL monomorphization (Slice 2). Generic struct
declarations (Slice 3) — this slice's variables are consumed by word signatures only, not by
`type:` declarations, even though Slice 3 will parameterize `type:` with the same variables
this slice introduces.
