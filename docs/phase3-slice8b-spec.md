# Phase 3 Slice 8b — Resources and user destructor bodies (spec)

Base: `main` @ `1b10005`, 892 tests green. Design input: [the brief](./phase3-slice8b-brief.md).
D1-D7 are the design; this spec applies them rather than re-arguing them, **except D1's
mechanism**, whose registration-site presumption checking against the actual source found
wrong (see "Grounding facts" below), **and D7**, which is cut entirely by explicit decision (see
"Out of scope") rather than implemented in any narrowed form. Slice 8c (retiring `__spy`
entirely) is a separate follow-up and out of scope.

## What the slice adds

A user-defined destructor body for a `type:` struct, spelled as an overload of `drop`
(`: drop ( T -- ) ... ;`), which forces `T` linear, working both in a native build and across
REPL lines. No new declaration form, no new keyword, no new runtime symbol beyond what the
user's own body calls. `extern:` gets one small addition inherited from 8a's review: a
multi-output declaration is rejected at the declaration.

## Grounding facts (confirmed against source)

The brief's D1 says `drop` "becomes overloaded by input type" without pinning how, and its own
open question presumed the answer would live "relative to `check_term`'s existing dispatch
chain (`src/check.rs:2600-2665`), which is where `S>fi`-style intercepts already sit." That
citation itself does not resolve to any dispatch chain in the current source (`:2600-2665` is
`leave_block`; the actual chain that matters is at `src/check.rs:2860-2900`), and the
presumption behind it is wrong regardless, for two independently verified reasons:

1. **`drop` is intercepted before any name lookup, at both check and lowering.**
   `check_term`'s dispatch chain (`src/check.rs:2876`) tries `check_shuffle` — whose `"drop"`
   arm (`src/check.rs:4217-4222`) unconditionally pops one value with no type check at all —
   *before* ever falling through to `env.get(name)` (`src/check.rs:2899`). `lower_call`'s
   `"drop"` arm (`src/ir.rs:1874`) is the same shape: it always calls `self.emit_drop(v)`
   (`src/ir.rs:2717-2739`), which dispatches on the popped value's `IrType`, and never reaches
   the final `_ =>` arm (`src/ir.rs:1988`) that would otherwise do an ordinary `env`-based
   `Instr::Call` — the `self.env.get(name)` lookup that arm is specifically about sits at
   `src/ir.rs:2045-2046`. **A user word registered into `env` under the literal name `"drop"`
   is dead on arrival at both call sites.**
2. **Every `:` word is compiled into an `IrFunc` named after itself, unconditionally.**
   `ir.rs::lower`'s `module.words.iter().map(|w| lower_word(w, &env, &resolve, regs))`
   (`src/ir.rs:922-926`) has no filter. A user's `: drop ( File -- ) ... ;`, unfiltered, would
   compile to a QBE function literally named `drop`; a second `: drop ( Widget -- ) ... ;` in
   the same module would collide with it under the identical symbol.

So `check_term`'s dispatch chain is not the registration site — it is the thing the mechanism
must route *around*. The right home already exists: `synthesize_aggregate_destructors`
(`src/ir.rs:945-979`) builds one `IrFunc` per linear struct, named `struct_drop_symbol(id)`
(`src/ir.rs:227-229`, keyed by `StructId`, not by name), and `emit_drop`'s `IrType::Struct(id) if
is_linear` arm already calls exactly that symbol (`src/ir.rs:2728-2730`). A `drop` overload's
natural implementation is: **the user's word body becomes the function synthesized under that
same, already-called symbol**, in place of the generic field-glue body
`synthesize_struct_destructor` (`src/ir.rs:1178-1211`) currently always produces.

One consequence worth stating up front, since D1 uses it as strategic justification and this
mechanism removes it: D1 calls this "a miniature, early instance of Phase 4's planned ad-hoc
static overloading (`+` over `i64`/`f64`/`Vec2`)". The mechanism below does **no name-based
overload resolution at all** — dispatch happens entirely through the existing symbol-keyed
destructor slot, never through a lookup on the string `"drop"`. Nothing here generalizes toward
Phase 4's `+`-over-several-types problem, which remains fully unstarted work.

## Requirements (final decisions only)

**R1 — A `drop` overload is recognized structurally, in its own pre-pass, not through the
ordinary word-registration path.** Before `check`'s call to `check_types` (`src/check.rs:844`
calls it at `845-850`), scan `module.words` for every word literally named `drop`. For each: its
effect must be exactly one input, zero outputs, and the input type must be `Type::Struct(id, _)`
naming a `type:`-declared struct — anything else (wrong arity, an output, an enum/array/scalar
input, `&T`/`&!T`) is a located error at the word's own declaration, modeled on
`check_main_effect`'s shape (`src/check.rs:1574-1598`): find the offending word by name, report
its span. This is also where an enum- or array-typed `: drop (...)` gets refused: the mechanism
generalizes to them for free in principle, but the dogfood needs only structs, and silently
ignoring a non-struct `drop` word would reproduce the exact dead-code/collision hazard described
above — so it is rejected with the same located error, not ignored.

At most one override per struct id: a second `: drop` naming the same struct is a located error
("`T` already defines its own `drop`"). Overloads for *different* struct types coexist with no
collision, since dispatch is by struct id, never by the shared literal name — this must hold
whatever data structure carries the registry; a `HashMap<&str, usize>` keyed on the word's name
(the shape `check_tail_call_cycles`'s own `name_to_idx` uses, `src/check.rs:1804-1808`) would
silently keep only the last `drop` word and must not be reused verbatim here.

The result is a small side-table, `StructId -> word index` (into `module.words`), threaded
wherever `structs`/`enums`/`arrays` already flow (`Registries` in `ir.rs`, the four discrete
parameters in `check.rs` — do not force a `Registries`-style bundle onto `check.rs`'s existing
`&`/`&mut` split, per the Slice 2 registry note in ROADMAP.md). **A `drop`-overload word is
excluded from `env`'s generic registration loop (`src/check.rs:876`) and from `ir.rs::lower`'s
generic `module.words.iter().map(lower_word)` pass (`src/ir.rs:922-926`)** — both would be either
dead or actively colliding, per the Grounding facts above. **This exclusion is the only thing
that changes about how the override's body is handled**: it is still checked by `check_word`
exactly like any other word body (`src/check.rs:902-904`'s ordinary loop), including calling
other words normally — R5 and R6 both depend on this.

The pre-pass must validate only the override's *declared shape* (arity, output count, input
type) and must not call `is_copy`/`is_linear` on that type itself: `is_copy`'s own termination
argument (`src/check.rs:178-180`) depends on `check_recursion` having already run, which happens
inside `check_types`, after this pre-pass. Calling it early turns a cyclic struct declaration
into a stack overflow instead of a diagnostic.

**R2 — The override body fills the existing destructor symbol, and the override must force the
IR's own, separately-computed linearity bit.** `synthesize_aggregate_destructors`, for a struct
id with a registered override, does not call `synthesize_struct_destructor` (which always
builds the generic field-glue body, taking either the plain per-field path or the fused-loop
path — see R7). Instead it compiles the override word's body via the same machinery `lower_word`
already uses, targeting the output `IrFunc.name` at `struct_drop_symbol(id)` instead of the
word's own name. This reaches `synthesize_aggregate_destructors`'s native call site
(`src/ir.rs:931`); its two REPL call sites (`src/repl.rs:490`, `src/repl.rs:620`) need R11's
additional fix, below.

This is **not** free of consequence at the two existing dispatch sites, and retracts an easy
mistake to make: `check.rs`'s `is_copy` (patched by R3) and `ir.rs`'s `StructLayout::is_linear`
are two independent computations. The latter is folded purely from declared field types,
memoized in `ensure_struct` (`src/ir.rs:624-642`, the fold itself at `:634` via
`layout_field_is_linear`, `:691-716`), inside `LayoutBuilder`, which is built by
`build_registries`/`build_registries_ww` (`src/ir.rs:459-500`) from struct/enum/array/cell
*declarations only* — it has no way to see `module.words` today. For the dogfood's scalar-only
`type: File fd i64 ;`, that fold alone yields `is_linear == false`. If nothing else changed:
`synthesize_aggregate_destructors`'s `.filter(|(_, layout)| layout.is_linear)` (`ir.rs:960`)
would skip `File` entirely (no `IrFunc` for the override to fill), and `emit_drop`'s
`IrType::Struct(id) if is_linear` guard (`ir.rs:2728`) would be false, so `f drop` would fall to
`_ => {}` (`ir.rs:2737`) and emit nothing — the dogfood would compile, run, print, and never
close the fd.

So `build_registries`/`build_registries_ww` must receive R1's override table as an additional
input (a new parameter, or threaded onto `LayoutBuilder`), and `ensure_struct`'s fold must become
`is_linear = has_override || fields.iter().any(|f| self.layout_field_is_linear(f.ty))`. Once that
is done, `emit_drop`'s guard and `synthesize_aggregate_destructors`'s filter both see the correct
bit with no further change, and `field_is_linear` (`src/ir.rs:214-224`, used by
`drop_level_fields` for ordinary field-composition) sees it too, closing R7's composition case for
free. Neither `check_shuffle`'s nor `lower_call`'s hardcoded `"drop"` arm changes shape — they
were always symbol-based, and stay so — but the layout fold that decides *whether that symbol
even gets emitted, and whether the arm's guard passes* does change, and must.

**R3 — Defining `drop` for a struct forces it linear; `Copy` and a user destructor are mutually
exclusive (D2).** `is_copy`'s `Type::Struct(id, _)` arm (`src/check.rs:184-187`) must return
`false` when `id` has a registered override, checked before (or in place of) the existing
structural fold over declared fields — a struct whose only field is `i64` would otherwise be
`Copy` by that fold alone. `is_linear` (`src/check.rs:208-215`, `!ty.is_ref() && !is_copy(...)`)
inherits the fix for free, so does the forgotten-disposal check
(`surplus_linear_value_error`, called at `src/check.rs:1701`) — body-checking runs last in
`check` (`check_word`, called at `src/check.rs:903`), after R1's pre-pass populates the
registry, so there is no ordering hazard on the `check.rs` side. Three call sites outside
`check_word`'s own path need the same fact: `src/repl.rs:551`'s `dispose_residual` (the `:quit`
LIFO-disposal path), `src/repl.rs:447`'s `check::check_def` call (a REPL-entered word body's own
move/must-consume checking — without the table, a `File` used inside a REPL word body stays
`Copy` there even though it is linear everywhere else), and `src/repl.rs:575`'s
`check::infer_line` (the per-line inference deciding whether a carried `File` on the session
stack is linear, which is where criterion 2's `dup` rejection would fire at the REPL). Whatever
data structure carries R1's side-table must be reachable at all three — threading a new
parameter through `is_copy`'s call sites versus annotating the fact directly onto the struct's
own registry entry (mirroring how the IR-side `Layout.is_linear` is already a separately
computed, not re-derived, bit) is an implementer choice; either must reach every one of
`repl.rs:447`, `:551`, `:575`, as well as every `check.rs` call site.

One consequence, stated rather than left as a surprise: at the REPL, a `File` value already
sitting on the carried stack from an earlier line becomes linear the moment a later line
registers `File`'s override, since every subsequent line re-derives linearity against the
session's *current* state, not a value's state at the time it was pushed. This is consistent
with how the REPL already treats every other structural fact about a type, not a new kind of
surprise this feature introduces.

**R4 — The non-`Copy` diagnostic must carry the reason (D5).** `cannot_copy_error`
(`src/check.rs:2378-2399`, both the `Ctx::Word` arm ending `:2396` and the `Ctx::Line` arm at
`:2397`) currently has exactly one linear-cause message: "`{found}` is linear: it owns a resource
and has no `Copy` instance...". Add a second cause, selected when `found`'s linearity traces to
a registered `drop` override rather than a `^`-holding field: "`{found}` is linear because it
defines `drop`". Without this, a one-`i64`-field resource struct gets told it has "no bits to
copy" with nothing pointing at the declaration responsible. Both `Ctx` arms need the new cause,
not just the more common `Ctx::Word` one.

**R5 — The override body runs instead of the synthesized field glue, never before or alongside
it (D3).** This is not a new check-time rule: the existing move/must-consume checker already
applies to every word body, including this one (R1's exclusion is from registration and generic
lowering only, never from body-checking), so a resource holding other linear fields is already
forced to account for each of them by the body's end. No new machinery; R2's "the override *is*
the destructor function" already makes this true by construction — there is no generic glue left
to run alongside it.

**R6 — Self-recursion is closed by whole-program call-graph reachability, computed after body
checking, not before it (D4).** The rule: for each registered override `drop@T`, reject it if
`drop@T`'s own word is reachable from itself through any sequence of calls, direct or indirect —
including a call that drops some *other* aggregate whose own disposal would, transitively through
ordinary (non-overridden) field composition, reach `drop@T` again. This subsumes a bare direct
self-call (a cycle of length one) and generalizes to any depth through any number of helper
words, with `T>` (the existing full-destructure word) as the remedy either way.

This cannot be "a sibling pass to `check_tail_call_cycles`, run where that pass runs" (before
body checking, `src/check.rs:882`): resolving *which* concrete override a `drop` call site refers
to needs the operand's static type, and nothing computes that before `check_word`'s own per-term
stack simulation runs. `check_shuffle`'s `"drop"` arm (`check.rs:4217-4222`) does `stack.pop()`
and nothing else today — there is no existing per-call-site type resolution to reuse, and a
purely syntactic pass (matching `check_tail_call_cycles`'s `Vec<&str>`-of-callee-names shape)
cannot tell `drop@File` from a `drop` of a plain `i64`. Under such a name-keyed graph, the
dogfood's own `: drop ( File -- ) | f | f File>fd close drop ;` would register a false-positive
cycle on its own last `drop` (of the `i64` `close` returns) and reject the dogfood outright.

So: during `check_word`'s existing walk, at the exact point `check_shuffle`'s `"drop"` arm runs,
record — as a side-observation, not a change to what it pops or how it type-checks — the pair
`(containing word, the popped operand's resolved type)` into a list threaded through checking.
After every word body has been checked, build the reachability graph as a **post-pass**: an edge
from word `A` to word `B` for an ordinary call anywhere in `A`'s body resolving to `B` (any
position, not just tail — `tail_position_calls`/`collect_tail_calls`, `check.rs:1749-1778`, only
look at `terms.last()`; a sibling walker must visit every term, including inside `if`/`else`
branches and every clause body); an edge from word `A` to the override word for `T` for any
`"drop"` call site in `A`'s body whose recorded resolved type is either (a) the registered-
override struct `T` itself, or (b) an aggregate type that does **not** itself have a registered
override, whose transitively linear fields reach `T` through ordinary, non-overridden
composition. Case (b) needs a check-side walk shaped like `is_copy`/`is_linear`'s own recursive
fold over `StructDecl.fields` (`src/check.rs:181-215`) rather than the ir-side
`field_is_linear`/`drop_level_fields` (those read `StructLayout`s built by `build_registries`
inside `ir::lower`, which runs *after* `check` entirely, `src/check.rs:844` — there is no layout
for a check-time pass to walk), stopped at any struct id in R1's registry exactly as R7's
`expand_path` fix stops at one. Case (b) must **not** run when the *directly dropped* type
itself has a registered override: dropping `B`, where `B` itself is overridden, already gets an
ordinary case-(a) edge `A -> drop@B`, and reachability continues from `drop@B`'s own recorded
call sites during that override's DFS — walking *into* `B`'s fields here as well would inspect
field glue that never runs (dropping an overridden `B` calls `B`'s override, not `B`'s generic
glue), producing a false edge. So case (b) only fires when the dropped type is a *non-overridden*
aggregate reaching an override transitively. Without case (b) at all, a `drop@File`'s body that
drops a plain, non-overridden struct merely *containing* a `File` field would go unrejected at
check time while still recursing unboundedly at runtime through that struct's own generic field
glue — case (b) closes exactly this route. Resolve both cases through R1's `StructId -> word
index` table, never through a name-keyed map, for the reason R1 states.

For each registered override, the question is only whether its own word is reachable from itself
in this graph — one DFS per resource type, not a full whole-program SCC decomposition, reusing
`find_tail_cycle`'s DFS shape and `mutual_tail_recursion_error`'s style of naming the full cycle
chain, including that the message names `T>` as the remedy (a message requirement, pinned by
criterion 8, not left implicit).

**Known, accepted limitation:** this is reachability, not data-flow, so it is context-
insensitive. If a helper is called from `drop@T`'s body *and* is separately, legitimately
reachable back to `drop@T` only down some other branch never taken from there, the graph still
sees a cycle and rejects it — a false positive in principle, the same cost the existing tail-
cycle pass already accepts. The remedy is the one D4 already gives: factor out a distinct helper.

**R7 — Composition is correct in both the ordinary case and the fused-recursive-disposal-cycle
case; the override always runs.** Two distinct paths, not one:

*Ordinary composition* (D6): an enclosing struct's per-field disposal
(`drop_level_fields`/`emit_field_level`, `src/ir.rs:2386-2394`/`2458-2475`) already calls each
field's own destructor via `emit_drop` rather than inlining it, so once R2's `is_linear` fix
lands, a field of a resource type is correctly disposed through the resource's own destructor
symbol — now the user's body for an overridden struct. No new mechanism here; this is R2's fix
taking effect. (*Correction from an earlier draft*: this requirement previously cited
`recursive_disposal_path` for ordinary composition; that function is the cycle-finder described
next, not the field-composition walk.)

*The disposal-cycle case, which needs an actual fix, not a documented limitation*: when a
struct's own fields loop back to itself through one or more intervening types (e.g. `type: Res
fd i64 next ^Chain ; type: Chain r Res ;`), `recursive_disposal_path` (`src/ir.rs:1021-1023`,
via `expand_path` at `:1050`) finds the cycle and `synthesize_struct_destructor` takes the
fused-loop branch (`src/ir.rs:1189-1196`), which *inlines* every intermediate type's field
projection directly into one iterative loop (`emit_field_level`) rather than calling each type's
own destructor — the whole point of the loop is constant-stack disposal for a long chain without
a stack of recursive destructor calls. Unmodified, this bypasses R2 entirely for any type on such
a cycle: the loop never calls `struct_drop_symbol(Res)`, so a registered override on `Res` would
simply never run, leaking its resource silently. This is a bug this slice's own mechanism
introduces (not pre-existing), and is latent rather than dogfood-visible, since the dogfood's
`File` is not on any cycle — worth fixing anyway, since it fires the moment any future resource
type is.

The fix: `expand_path`'s `IrType::Struct(id)` arm (`src/ir.rs:1057-1060`) must never continue
*into* a struct that has a registered `drop` override when searching on **another** type's
behalf — `if has_override(id) && current != target { return None; }`, exactly as a plain `Copy`
scalar field already is a dead end. The `current != target` guard is the "except as the search's
own root" carve-out: `Res`'s own destructor is always its override regardless of what
`recursive_disposal_path(Res, ...)` would otherwise find (R2 already guarantees this
unconditionally — `Res`'s own fused-loop-or-not decision is moot, since the override replaces
`synthesize_struct_destructor`'s call entirely), so the guard only needs to bite when some other
type's search reaches `Res` as an intermediate step. One guard in this one arm suffices for every
entry route, since the enum arm (`:1061-1078`) and the cell case both funnel back through
`find_path`/`expand_fields` into `expand_path`.

**Consequence, stated plainly:** a resource type that itself needs constant-stack disposal of its
own recursive cycle must write that iteration inside its own override body (via `T>`
destructuring or Slice 6's self-tail-recursion), since the compiler cannot auto-fuse an arbitrary
user body into a loop the way it can for the mechanical, compiler-synthesized field glue. R6's
self-recursion rejection is exactly what correctly forces this rather than silently accepting an
attempt at unbounded recursive `drop` calls.

**R8 — A multi-output `extern:` is rejected at the declaration (inherited from 8a's review).**
`extern: two ( i64 -- i64 i64 ) "two" ;`, when called, currently silently lowers to a discarded
result — `lower_call`'s `out_arity == 1` test (`src/ir.rs:2049-2053`) makes `ret` `None` and
pushes nothing for a 2-output call — and the *next* consumer of the (missing) value panics, e.g.
`.`'s `expect("print: value")` at `src/ir.rs:1951`. Pre-existing, not an 8a regression: a
multi-output *user* word panics the same way at the same site. No C function returns two values,
so reject a declared-output arity greater than one in `check_extern_decls`
(`src/check.rs:921-947`, alongside its existing rejections at `931-944`). The general
multi-output-lowering failure (the user-word case) is a separate, bigger issue this does not fix.

**R9 — Non-`Copy` fields are permitted in a resource struct; no scalar-only restriction (open
question, resolved, against the brief's own leaning).** The brief's own text guessed "a
scalar-only first cut … is smaller"; that intuition is backwards. R5 already makes the existing
must-consume checker responsible for any linear field a resource struct holds, at zero extra
implementation cost, whereas *restricting* to scalar-only fields would require writing a new
check that does not otherwise need to exist — the permissive reading is the one that needs less
code, not more. The dogfood's `File` happens to be scalar-only; nothing in the checker requires
it.

**R10 — Dependency on 8a is direct, not incidental (open question, resolved).** The dogfood's
`drop` body calls `close`, an `extern:`-declared word. This slice cannot be implemented or
tested standalone; it builds on `main` @ `1b10005`, after 8a's merge.

**R11 — A `drop` overload declared at the REPL must actually work across subsequent lines,
including redefinition, without the REPL supporting `extern:` calls in its own body.** `Session`
(`src/repl.rs:241-277`) already persists exactly this shape of fact across lines:
`structs`/`enums`/`arrays`/`owned_cells`/`refs` in stable declaration order, specifically so a
later line can still reference an earlier one. But `eval_def` (`src/repl.rs:442-521`) compiles
one line's `: drop ( File -- ) ... ;` into a `.so` and retains only the resulting `WordEntry`
(`sig`/`generation`/mangled `symbol`, `:511-518`) in `self.env`; the parsed `WordDef` itself is
discarded once that line finishes. Every subsequent line re-synthesizes destructors fresh into
its own freshly-`dlopen`ed module (`repl.rs:617-620`, whose own comment already anticipates this
class of bug: "`drop` on a linear struct/enum dies at `dlopen`"), and by then the override's body
is gone. Fixing this needs three coordinated pieces, not one:

1. **Retention.** Add a `drop_overloads: HashMap<StructId, (u64, WordDef)>` field to `Session`
   (the generation travels with the body, not through `self.env`, since the override is excluded
   from `env` per below and `next_generation` (`src/repl.rs:118-120`) reads `self.env.get(&name)`
   — with the override absent from `env`, that lookup would see nothing and every redefinition
   would silently mint generation 0, reopening exactly the collision point 2 exists to close, so
   the generation must be tracked in `drop_overloads` itself, bumped the same way `next_generation`
   bumps any other word's). Populated the same way `structs` already grows on a `type:` line, but
   storing the override's own body (not a word index — there is no persistent `module.words` at
   the REPL to index into). Every subsequent line's re-synthesis call (`eval_def`, `run_terms`)
   reconstructs the per-line override registry `ir::synthesize_aggregate_destructors` consumes
   from this session-level store by reference, rather than from a native module's
   `module.words[index]` — populated before that same line's own synthesis call if the line being
   evaluated is itself the `: drop` declaration, so the defining line's own re-synthesis already
   sees its own override rather than emitting generic glue for one line and the override from the
   next. The override is excluded from `self.env` (mirroring R1's env-exclusion) and validated
   with R1's declaration-shape rule at the point it is entered, exactly as a native `: drop` word
   would be — a REPL line does not get a laxer check than a compiled program.
2. **Symbol collision under redefinition.** `struct_drop_symbol` (`src/ir.rs:227-229`) emits the
   unmangled global `sooth_struct_drop_N`, and REPL libraries load with `RTLD_NOW | RTLD_GLOBAL`
   (`src/repl.rs:55`). The existing doc comment on `synthesize_aggregate_destructors`
   (`src/ir.rs:941-944`) already names this exact hazard and its fix: "The REPL redefines these
   per line; safe because type redefinition is rejected, so every generation's glue is identical.
   If type redefinition is ever allowed, add a generation suffix, matching word symbols." A user
   override breaks the "every generation's glue is identical" premise directly: redefining
   `: drop ( File -- )` a second time would otherwise emit a *second* `.so` defining the *same*
   global symbol with a *different* body — ambiguous linkage under `RTLD_GLOBAL`, not merely an
   edge case (re-emitting an *identical* generic-glue body across successive `.so`s, as happens
   today for every non-overridden linear struct, is harmless under `RTLD_GLOBAL` and needs no
   fix; only a *differing* body is the hazard). `struct_drop_symbol` has exactly two callers
   repo-wide, both of which need the fix, not just the synthesis site: the synthesis call
   (`src/ir.rs:1205`, inside `synthesize_struct_destructor`) and the emission call
   (`src/ir.rs:2729`, inside `emit_drop`, which `drop_level_fields` also reaches through for
   nested field disposal, `src/ir.rs:2386-2394`). Fix: `struct_drop_symbol` takes an optional
   generation, appended to the symbol only when `Some`, and a `StructId -> generation` map (empty
   on the native path) must be reachable from both call sites — threaded onto `Registries` or
   passed alongside the override table — so `emit_drop`/`drop_level_fields` mint the *same*
   suffixed symbol name the synthesis site used for that struct at that generation. Every native
   call and every REPL call for a non-overridden struct pass `None` throughout (byte-identical
   output, preserving the "every generation's glue is identical" invariant those cases still
   satisfy); only an overridden struct's REPL calls pass `Some(generation)`, read from
   `drop_overloads`, mirroring `mangled_symbol(name, generation)`'s existing pattern for ordinary
   words (`src/repl.rs:114-116`) — so redefining `: drop` at the REPL mints a distinct global
   symbol per generation at *both* call sites consistently, rather than colliding. Redefinition
   itself follows the session's ordinary generation-bump rule (described at `repl.rs:90-95`,
   pinned by the `redefinition_bumps_generation` test at `repl.rs:892`), not R1's per-module
   "duplicate override" rejection, which only applies within a single compiled program.
3. **No `extern:` at the REPL.** The REPL has no `extern:` evaluation path at all (confirmed:
   `grep -n extern src/repl.rs` finds only Rust `extern "C"` items; 8a's own Out-of-scope already
   records this gap). The dogfood's `drop` body calls `close`, an `extern:` word, so it cannot be
   the body used to test REPL retention. The REPL-retention test needs an extern-free override
   body instead — e.g. a resource wrapping a plain `i64` whose `drop` just destructures and
   drops the scalar — proving retention and redefinition without depending on REPL `extern:`
   support this slice does not add.

## Criterion → test map

| # | Criterion | Test |
|---|---|---|
| 1 | a struct with a `drop` overload is linear even if every field is `Copy` | `check_struct_with_drop_overload_is_linear` |
| 2 | `dup` on such a struct names the overload as the reason | `check_dup_of_drop_overload_type_names_the_cause` |
| 3 | a forgotten (unconsumed) resource, whose fields are all `Copy`, at end of body is a compile error naming the forgotten resource | `check_unconsumed_all_copy_resource_at_word_end_is_error` |
| 4 | a second `drop` of the same value, whose fields are all `Copy`, is a use-after-move compile error | `check_double_drop_of_all_copy_resource_is_use_after_move_error` |
| 5 | a `drop` overload with a non-struct input (enum, array, or scalar) is a located declaration error | `check_drop_overload_on_non_struct_input_is_error` |
| 6 | a `drop` overload with extra outputs is a located declaration error | `check_drop_overload_with_output_is_error` |
| 7 | two `drop` overloads for the same struct is a located declaration error | `check_duplicate_drop_overload_for_one_struct_is_error` |
| 8 | a direct self-call to `drop T` inside `T`'s own `drop` body is a located error naming `T>` in the message text | `check_drop_body_direct_self_recursion_is_error` |
| 9 | an indirect self-call (through one helper word) is caught the same way | `check_drop_body_indirect_self_recursion_through_helper_is_error` |
| 10 | a `drop` of a `Copy` scalar (e.g. the dogfood's own `close` result) inside a `drop` body is legal, not a false-positive cycle | `check_drop_of_copy_scalar_inside_drop_body_is_not_a_cycle` |
| 11 | a `drop` of a *different* resource `B` inside `drop@A`'s body is legal | `check_drop_of_different_resource_inside_another_drop_body_is_ok` |
| 12 | a resource holding another linear field must consume it in the `drop` body | `check_drop_body_must_consume_linear_fields` |
| 13 | composition: an ordinary struct holding a resource field disposes it via the resource's own `drop`, verified by the emitted destructor calling the resource's destructor symbol (not inlining its fields) | `resource_field_disposed_via_its_own_drop_symbol` |
| 14 | a resource on a disposal cycle still has its override called, not bypassed by the fused loop | `synthesize_destructor_excludes_override_structs_from_a_fused_disposal_path` |
| 15 | the emitted destructor for a resource with a linear field contains the user body's own disposal of that field and no synthesized field glue | `synthesize_destructor_of_resource_with_a_linear_field_uses_user_body_not_field_glue` |
| 16 | two `drop` overloads for different structs coexist with no symbol collision, and neither compiles to an `IrFunc` literally named `drop` | `two_drop_overloads_for_different_structs_do_not_collide` |
| 17 | a `drop` overload declared at one REPL line is still the destructor two lines later, using an extern-free body | `repl_drop_overload_still_runs_on_a_later_line` |
| 18 | a multi-output `extern:` is rejected at the declaration | `check_extern_multi_output_is_error` |
| 19 | the dogfood runs with the documented output | `slice8b_dogfood_compiles_and_runs` |
| 20 | an overridden, scalar-only struct's `StructLayout::is_linear` is `true` at the IR level | `ir_registers_overridden_struct_as_linear_despite_all_copy_fields` |
| 21 | dropping an aggregate that merely *contains* a resource field, from inside a *different* resource's own `drop` body, still closes the self-recursion cycle if it reaches back to that same resource | `check_drop_body_recursion_through_a_containing_aggregate_is_error` |
| 22 | redefining a REPL `drop` overload mints a distinct destructor symbol per generation, not a collision | `repl_redefining_drop_overload_does_not_collide_under_rtld_global` |

Rows 2, 3, 4, 5, 6, 7, 8, 9, 18 assert the specific message, not merely that compilation failed.

## Stage unit-test obligations

- **check**: R1's declaration-shape validation (each rejection independently, including the
  ordering-hazard caveat — a self-recursive struct with a malformed `drop` override still gets a
  diagnostic, not a stack overflow), R1's collision-freedom (criterion 16's check-side half: two
  overrides for different structs both land in the `StructId -> word index` side-table under
  distinct keys, and neither is in `env`), R3's `is_copy` interaction, R4's diagnostic message
  (both `Ctx` arms), R6's cycle detector — direct, indirect through one helper, a non-cyclic
  helper reused from two call sites (no false positive), a `drop` of a `Copy` scalar inside a
  `drop` body (criterion 10, the dogfood's own shape), a `drop` of a different resource inside
  another's `drop` body (criterion 11), and a `drop` of a containing aggregate that reaches back
  to the same override through its own generic field glue (criterion 21) — R8's extern arity
  rejection.
- **ir**: R2's destructor-symbol substitution (the synthesized `IrFunc` for the resource's
  struct id is the lowered user body, not `synthesize_struct_destructor`'s output — assert on
  the emitted function's instructions, not a substring match), R2's `is_linear`-forcing fix
  (criterion 20: a scalar-only overridden struct's `StructLayout::is_linear` is `true`), R1's
  lowering exclusion (no emitted `IrFunc` is literally named `drop`; two overrides for different
  structs produce two distinct `struct_drop_symbol` bodies, criterion 16's ir-side half), R7's
  ordinary-composition dispatch (an enclosing struct's field-level disposal calls the resource's
  destructor symbol rather than inlining its fields), R7's cycle-boundary fix (construct the
  `Res`/`Chain` shape from R7's own text and assert the override's call appears in the emitted
  destructor reachable from the cycle, not bypassed by a fused loop — criterion 14).
- **repl**: R11's retention (a `drop` overload declared on one line is still the destructor two
  lines later, using an extern-free body, criterion 17), its declaration-shape validation and
  `env`-exclusion at the REPL line, and its generation-suffixed symbol under redefinition
  (criterion 22 — construct two generations of the same struct's override and assert the emitted
  symbols differ, not merely that no crash occurs).
- **backend**: none beyond the existing `Instr::Call` emission path; no new QBE-level case.

## Dogfood

Reading a real file whose length varies with repo state makes for a non-deterministic golden, so
the dogfood reads a small dedicated fixture with a fixed, known size instead of a project
document: `examples/resource_fixture.txt`, containing exactly `hi\n` (3 bytes, no other content).
`examples/resources.sth`:

```
extern: open  ( cstr i64 -- i64 )              "open" ;
extern: read  ( i64 &![u8 64] usize -- isize ) "read" ;
extern: close ( i64 -- i64 )                   "close" ;

type: File fd i64 ;

: drop ( File -- )
  | f | f File>fd close drop ;

: main ( -- )
  "examples/resource_fixture.txt" cstr 0 open | fd |
  fd File | f |
  0 >u8 64 fill | buf |
  f File|>fd &!buf 64 >usize read .
  f drop ;
```

(Verified against the syntax `tests/phase3_refs.rs` already exercises and passes: `0 >u8 N fill`
for a `u8`-element array, `&!name` as the prefix mutable borrow of a bare array-typed local
— confirmed at `mutable_element_projection_through_mutable_reference_is_accepted`,
`0 4 fill | a | &!a 0 &!> 99 !` — and `File|>fd` as the non-consuming peek of a `Copy` field,
legal regardless of the enclosing struct's own linearity since `check_struct_peek_word`,
`src/check.rs:4095-4131`, gates only on the *field*'s `is_copy`, never the struct's. `buf` needs
no explicit `drop`: it is `Copy`, and the surplus-value check `check_outputs`
(`src/check.rs:1682-1691`) inspects only the final *stack* (`final_stack: &[Slot]`), not bound
locals, so a `Copy` local left unused simply goes out of scope.)

Expected output: exactly `"3\n"` (the fixture's fixed byte count, from `read`'s return value,
printed with the trailing newline every other golden asserts) — deterministic regardless of repo
state, unlike reading `README.md` would be. The golden test invokes the built binary with the
working directory at the repo root. Note this is a **new** dependency this slice introduces, not
one inherited from existing goldens: no prior `examples/*` golden opens a file at runtime (every
existing one only uses a relative `.sth` path as *compiler input*), so `run_binary`'s lack of an
explicit `current_dir` has never mattered before this dogfood.

Exit criteria: a second `drop` of the same `File` is a compile error, not a runtime
double-close (criterion 4); a `File` left unconsumed at end of `main` is a compile error naming
the forgotten resource (criterion 3); `dup` on a `File` is rejected with R4's reason-carrying
message (criterion 2); a `drop` body that calls `drop` on its own receiver (directly or through
a helper) is rejected (criteria 8, 9), while a `drop` of the `Copy` scalar `close` returns, in
the same body, is not (criterion 10); the emitted destructor for `File` contains the user body's
`close` call and no synthesized field glue (criterion 15, tested against a separate fixture with
a linear field, since scalar-only `File` cannot observe the *absence* of glue that was never
going to be there); a `drop` overload declared at the REPL still runs correctly on a later line,
including across redefinition (criteria 17, 22).

## Out of scope

Enum- or array-typed `drop` overloads (R1; rejected with a located error, not silently accepted).
Retiring `Type::Spy`/`IrType::Spy` (Slice 8c). **Unifying `Type::Spy`'s hardcoded drop dispatch
with this slice's override mechanism (D7): cut from this slice entirely, per explicit decision.**
`Type::Spy` has no `StructId` to join R1's per-struct registry as a real entry, so the only thing
D7's unification could still deliver here is a behavior-preserving refactor of `emit_drop`'s
match arms with no user-visible consequence — and Slice 8c deletes `Type::Spy`/`IrType::Spy`
outright one slice later, which deletes the refactor's target along with it. `Type::Spy`'s
existing dispatch (`src/ir.rs:2717-2739`) is untouched by this slice. `drop` becoming fully
polymorphic open dispatch (Phase 4; this slice is one builtin fallback plus exactly one user
override per struct, not a general multimethod, and per the Grounding facts section, does not
even lay groundwork toward Phase 4's overloading — that remains fully unstarted). An
overloadable `dup` / opt-in reference-counting (D2's own justification turns on `Rc` needing
`Clone` rather than `Copy`, which needs an overloadable `dup` that does not exist; Phase 6's
deferred RC problem, untouched here). `^T`/cell destructor dispatch joining any shared resolution
step (moot, since D7's unification is cut, not R9 — R9 in this document is the non-`Copy`-fields
decision). **`extern:` at the REPL, generally**: R11 makes a `drop` overload work at the REPL,
but the REPL still cannot evaluate an `extern:` declaration or call an extern-declared word at
all (unchanged from 8a's own Out-of-scope note on this gap) — R11's own test works around this by
using an extern-free override body, not by adding REPL `extern:` support. The general
multi-output-lowering panic for ordinary user words (R8 fixes only the `extern:` declaration
case). A symbol-existence check for `close` (unchanged from 8a's R14). Any change to `str`/`cstr`
or the `.`-separator question (8a, DESIGN.md Open/deferred).

```json
{
  "phases": [
    {
      "phase": 1,
      "focus": "R1's drop-overload recognition pre-pass and registry: find every word literally named drop, validate its effect shape (one struct input, zero outputs), reject a duplicate override for one struct id and any non-struct-input or output-bearing drop declaration with a located error, guard against the is_copy/check_recursion ordering hazard, and exclude a recognized override from both the ordinary env registration loop and ir.rs::lower's generic per-word lowering pass (while leaving it checked normally by check_word), asserting only that neither override lands in env and that no emitted IrFunc is literally named drop -- the two-distinct-destructor-bodies half of criterion 16 needs R2's is_linear fix and belongs to phase 2, since an overridden struct's layout still folds to non-linear until then and no destructor is synthesized for it at all. Covers R1 and criteria 5, 6, 7, and criterion 16's check-side half only.",
      "effort": "M",
      "difficulty": "standard"
    },
    {
      "phase": 2,
      "focus": "R2's lowering substitution, including forcing StructLayout::is_linear for an overridden struct by threading the override table into build_registries/LayoutBuilder's ensure_struct fold (the fix that makes the dogfood's scalar-only File actually linear and actually dispatch, criterion 20), which also unblocks criterion 16's ir-side half (two overrides for different structs now each get a distinct struct_drop_symbol body, not just distinct env-absence). R3's is_copy interaction reaching check.rs plus repl.rs:447/551/575, including the retroactive-linearity note. R4's reason-carrying diagnostic (same site as R3, hence co-located here). R5 and R9 (both true by construction once R2/R3 land, R9 needing no new check). R7's two composition cases: ordinary field disposal calling the resource's own destructor, and the fused-disposal-cycle boundary fix in expand_path so a resource on a recursive-type cycle still has its override called instead of bypassed. Covers R2, R3, R4, R5, R7, R9 and criteria 1, 2, 3, 4, 12, 13, 14, 15, 16 (ir-side half), 20.",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 3,
      "focus": "R6's whole-program call-graph reachability pass for drop self-recursion: recording each drop call site's resolved operand type as a side-observation during check_word's existing stack simulation, then a post-pass reachability query per registered override, covering both a direct/indirect self-call and a call that drops a containing aggregate whose own non-overridden field composition reaches the same override (reusing the boundary rule R7 applies to the fused-loop search). Explicitly verify no false positive on a drop of a Copy scalar or of a different, unrelated resource inside another's drop body. Covers R6 and criteria 8, 9, 10, 11, 21.",
      "effort": "L",
      "difficulty": "hard"
    },
    {
      "phase": 4,
      "focus": "R8's extern: multi-output rejection at the declaration. R11's REPL support in full: a drop_overloads field on Session storing the override's WordDef (populated like structs already is, but keyed to a body rather than a module.words index), threaded into eval_def/run_terms's re-synthesis calls; a generation-suffixed struct_drop_symbol for an overridden struct only (mirroring mangled_symbol's existing pattern) so redefinition does not collide under RTLD_GLOBAL; declaration-shape validation and env-exclusion applied to a REPL-entered drop line exactly as R1 applies them natively; and an extern-free test fixture proving retention and redefinition without relying on REPL extern: support, which this slice still does not add. R10 (dependency ordering, no code). The dogfood: examples/resource_fixture.txt plus examples/resources.sth opening/reading/closing a file through File's drop overload, run both natively and checked as a golden with the documented, deterministic expected output \"3\\n\". Covers R8, R10, R11 and criteria 17, 18, 19, 22.",
      "effort": "L",
      "difficulty": "hard"
    }
  ]
}
```
