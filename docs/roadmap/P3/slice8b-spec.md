# Phase 3 Slice 8b — Resources and user destructor bodies (condensed, as implemented)

Base: `main` @ `1b10005`. Delivered in four phases; all 22 criteria green. Design input:
[the brief](./phase3-slice8b-brief.md). D7 (unifying `Type::Spy`'s hardcoded dispatch with this
mechanism) was cut by explicit decision, and D1's presumed registration site was found wrong
against the source (see "Grounding facts"). Slice 8c (retiring `__spy`) remains separate.

## What the slice adds

A user destructor body for a `type:` struct, spelled as an overload of `drop`
(`: drop ( T -- ) ... ;`), which forces `T` linear, working in a native build and across REPL
lines including redefinition. No new declaration form, no new keyword, no new runtime symbol
beyond what the user's own body calls. `extern:` gains one rejection inherited from 8a's review:
a multi-output declaration is refused at the declaration.

## Grounding facts (confirmed against source)

`drop` is intercepted *before* any name lookup at both check (`check_shuffle`'s `"drop"` arm,
before the fall-through to `env.get(name)`) and lowering (`lower_call`'s `"drop"` arm always calls
`emit_drop`, never the generic `env`-based `Instr::Call`), so a word registered into `env` under
the literal name `"drop"` is dead on arrival; and every `:` word is otherwise lowered to an
`IrFunc` named after itself, so two `: drop` words in one module would collide under one QBE
symbol. The dispatch chain is therefore not the registration site, it is the thing to route
*around*. The existing home is `synthesize_aggregate_destructors`, which already builds one
`IrFunc` per linear struct named `struct_drop_symbol(id)` (keyed by `StructId`, not by name), and
which `emit_drop` already calls. The override simply *becomes* the function synthesized under that
symbol, in place of `synthesize_struct_destructor`'s generic field glue.

Consequence, stated because D1 used the opposite as justification: there is **no name-based
overload resolution anywhere** in this mechanism. Nothing here lays groundwork for Phase 4's
ad-hoc overloading of `+`, which remains fully unstarted.

## Requirements (final decisions only)

**R1 — Structural recognition in its own pre-pass** (`find_drop_overloads` /
`drop_overload_struct_id`, `src/check.rs`), before `check_types`. Every word literally named
`drop` must have exactly one input, zero outputs, and a `Type::Struct` input naming a
`type:`-declared struct. Wrong arity, any output, and a non-struct input (enum, array, scalar,
`&T`/`&!T`) are three separate located errors at the word's own declaration, modeled on
`check_main_effect`. A second override for the same struct id is a fourth ("`T` already defines its
own `drop`"). Overloads for *different* structs coexist, since the registry is `StructId -> word
index`, never a name-keyed map (a `HashMap<&str, _>` would silently keep only the last `drop`).

An override is excluded from `env`'s registration loop and from `ir::lower`'s generic per-word
lowering pass, and from nothing else: its body is still checked by `check_word` like any other
word, which is what R5 and R6 rest on. The pre-pass validates *declared shape* only and must not
call `is_copy`/`is_linear`, whose termination argument depends on `check_recursion` having already
run inside `check_types`; calling them early turns a cyclic struct into a stack overflow instead of
a diagnostic (pinned by
`check_drop_overload_with_self_recursive_struct_is_still_a_declaration_error_not_overflow`).

**R2 — The override body fills the existing destructor symbol, and forces the IR's separately
computed linearity bit.** `synthesize_aggregate_destructors` routes an overridden struct to
`synthesize_struct_destructor_override`, lowering the user body with `lower_word`'s machinery but
naming the output `struct_drop_symbol(id)`.

The load-bearing half is that `check.rs`'s `is_copy` and `ir.rs`'s `StructLayout::is_linear` are
two independent computations, and the latter folds from declared field types only. For the
dogfood's `type: File fd i64 ;` that fold yields `is_linear == false`, which would make
`synthesize_aggregate_destructors`'s filter skip `File` entirely *and* `emit_drop`'s
`Struct(id) if is_linear` guard fall through to `_ => {}`: the dogfood would compile, run, print,
and never close the fd. So `ensure_struct`'s fold became
`has_drop_overload || fields.iter().any(layout_field_is_linear)`. Both dispatch sites then see the
right bit with no further change, and `field_is_linear` sees it too, which closes R7's ordinary
composition case for free.

**R3 — Defining `drop` forces linearity; `Copy` and a user destructor are mutually exclusive
(D2).** `is_copy`'s `Type::Struct` arm returns `false` for an overridden struct, checked before
the structural fold (an all-`i64` resource would otherwise be `Copy`). `is_linear` and the
forgotten-disposal check inherit this for free.

*As shipped, this is carried as a bit on the declaration, not a threaded table:*
`StructDecl::has_drop_overload`, set by `check`'s pre-pass, read by `is_copy`, the IR layout fold,
`expand_path`, and the diagnostic. That is what makes R3 reach the three REPL sites
(`check_def`, `dispose_residual`, `infer_line`) without new parameters: `eval_drop_overload` sets
the bit on `Session::structs` when an override is entered and restores it if compilation of that
line fails.

Consequence, not a surprise this feature invents: a `File` already on the REPL's carried stack
becomes linear the moment a later line registers `File`'s override, since every line re-derives
linearity against the session's current state.

**R4 — The non-`Copy` diagnostic carries the reason (D5),** in both `Ctx` arms: "`File` is linear
because it defines `drop`: its own destructor runs exactly once, so a copy would run it twice;
thread the value through instead". Without it, a one-`i64`-field resource is told it has "no bits
to copy" with nothing pointing at the declaration responsible.

**R5 — The override runs instead of the field glue, never alongside it (D3).** True by
construction once R2 lands: the override *is* the destructor function, so there is no glue left to
run. A resource holding other linear fields is already forced to account for each of them by the
existing must-consume checker, since R1's exclusion never touched body checking.

**R6 — Self-recursion is closed by whole-program call-graph reachability, after body checking**
(`check_drop_overload_recursion` / `drop_reachability_graph` / `collect_drop_targets` /
`all_calls`). It cannot be a sibling of `check_tail_call_cycles`: resolving which override a
`drop` site dispatches to needs the operand's static type, and a name-keyed pass cannot tell
`drop@File` from the `drop` of the `i64` that `close` returns, so it would reject the dogfood
outright. So each `drop` site's resolved operand type is recorded during `check_word`'s existing
walk (onto the per-body `Provenance` arena the walk already carries `&mut`, since an `if` arm
clones `Scope` and an observation kept there would die with the arm), and the graph is built as a
post-pass: name edges for ordinary calls in *any* position (`all_calls` visits both `if` arms and
every clause body, unlike `tail_position_calls`), plus a typed edge to `drop@T` for every recorded
`drop` operand type reaching `T`. One DFS per override, reported over sorted struct ids so a
program with two offending overrides always names the same one, reusing `find_tail_cycle`'s DFS
shape and `mutual_tail_recursion_error`'s full-chain wording, and naming `T>` as the remedy.

*Cases (a) and (b) collapsed into one rule during implementation.* `collect_drop_targets`'s field
walk stops at the first struct id in the registry, whether that is the walk's own root (case (a):
the edge goes to that override, and the DFS continues from its recorded call sites) or reached
below it (case (b)'s boundary: a non-overridden aggregate's generic glue does reach every override
below it, but stops at the first). "Must not descend into an overridden type's fields" is then not
a separate guard to remember but the same stop, which is also exactly R7's `expand_path` boundary.
The walk covers enum variants, array elements and `^T` payloads, not just struct fields, with a
monotone `seen` set because a payload can close a type cycle.

*Pre-existing bug this made reachable, fixed in phase 2 ahead of R6:* a word named `drop`
poisoned two name-keyed passes. `has_self_tail_call` would read the dogfood's trailing `drop` as a
self tail call and lower the body to a back-edge loop instead of closing the fd; and
`check_tail_call_cycles`'s `name_to_idx` gave any tail-position `drop` of *anything* an edge to the
override, so `: f ( -- ) 1 drop ; : drop ( A -- ) ... f ;` was rejected as mutual tail recursion
though nothing recurses. Both now exclude a `drop`-named word, and R6's typed graph owns every
`drop` edge.

**Known, accepted limitation:** reachability is not data-flow, so it is context-insensitive. A
helper reachable back to `drop@T` only down a branch never taken from `drop@T` still reads as a
cycle, the same false positive the tail-cycle pass already accepts, with the same remedy.

**R7 — Composition is correct in both paths; the override always runs.** *Ordinary composition*
(D6) needs no new mechanism: `drop_level_fields`/`emit_field_level` already dispose each field
through `emit_drop` rather than inlining it, so R2's `is_linear` fix is the whole fix. *The
disposal-cycle case does need one:* when a struct's fields loop back to itself through
intervening types (`type: Res fd i64 next ^Chain ; type: Chain r Res ;`),
`recursive_disposal_path` makes `synthesize_struct_destructor` take the fused-loop branch, which
inlines every intermediate type's field projection into one iterative loop for constant-stack
disposal, and therefore never calls `struct_drop_symbol(Res)`: the override would silently leak.
Fix: `expand_path`'s `Struct(id)` arm returns `None` for an overridden struct unless it is the
search's own root (`current != target`), exactly as a `Copy` scalar field is already a dead end.
One guard suffices, since the enum and cell routes funnel back through it.

**Consequence:** a resource needing constant-stack disposal of its own recursive cycle must write
that iteration inside its override body via `T>`; the compiler cannot auto-fuse an arbitrary user
body the way it fuses mechanical glue. R6's rejection is what forces that instead of accepting
unbounded recursive `drop`.

**R8 — A multi-output `extern:` is rejected at the declaration.** Unrejected, `lower_call`'s
`out_arity == 1` test discarded the result and the *next* consumer panicked, naming the wrong term.
No C function returns two values, so `check_extern_decls` refuses `outputs.len() > 1`. The general
multi-output-lowering panic for ordinary user words is untouched.

**R9 — Non-`Copy` fields are permitted; no scalar-only restriction.** The brief's guess that
scalar-only would be smaller is backwards: R5 already makes the existing must-consume checker
responsible for a linear field at zero cost, whereas restricting to scalars would need a new check
that otherwise does not exist. `File` is scalar-only by accident, not by rule.

**R10 — Direct dependency on 8a:** the dogfood's `drop` body calls the `extern:` word `close`.

**R11 — A REPL-declared override works across later lines, including redefinition.** Three pieces:

1. **Retention.** `Session::drop_overloads: HashMap<StructId, (u64, WordDef)>` keeps the body
   (there is no persistent `module.words` to index into) with its epoch travelling alongside it,
   since the override is absent from `env` and `next_generation` reads `env`. `eval_drop_overload`
   is a separate `Line::Def` route that validates R1's declaration shape, sets
   `has_drop_overload` on `Session::structs`, keeps the override out of `env`, and rolls all of
   that back if the line fails to compile. Later lines rebuild the per-line
   `ir::DropOverrides` from the session store (`drop_override_bodies`), including on the
   declaring line itself, so the defining line already emits its own override rather than glue.
2. **Symbol collision under redefinition, revised twice.** `struct_drop_symbol` gained an
   optional epoch suffix, mirroring `mangled_symbol`. A *per-struct* generation turned out to be
   insufficient: an enclosing aggregate's glue `Call`s the overridden symbol, so its own body
   changes across an override event while its symbol would not move, and `RTLD_GLOBAL` keeps the
   first definition loaded forever. So the suffix is one session-wide `Session::override_epoch`,
   bumped on every override define/redefine of any struct and stamped onto every linear
   struct's, enum's and cell's symbol once the session holds any override
   (`apply_drop_generations`, `StructLayout`/`EnumLayout::drop_generation`,
   `Cells::drop_generations`). Deliberately coarse: some glue is re-emitted under a fresh name
   needlessly, which is free.
   The second revision: with a session-wide epoch, an override's own symbol moved on every later
   override event, forcing its retained body to be re-lowered against a *later* line's env than it
   was checked against, which panicked lowering outright when a callee had been redefined at a
   different arity (and silently did nothing at the same arity, since the first-loaded body wins).
   Fix: an overridden struct's symbol carries the epoch its override was **defined** at, so it
   never moves while that override stands; the body is lowered exactly once, on its declaring
   line, and every later line emits nothing for it (`DropOverride::AlreadyLoaded`) and resolves
   the pinned symbol through `RTLD_GLOBAL`. Every other symbol still moves per event, so composing
   glue is refreshed and re-resolves to whichever epoch each override is pinned at. This makes
   the REPL's snapshot semantics uniform: an override's callees bind the generations visible when
   it was defined, exactly as an ordinary word's body already does. It also collapsed the two
   counters into one, and they cannot collide (a struct emits glue only at epochs strictly before
   its override exists).
3. **No `extern:` at the REPL,** unchanged from 8a. The REPL cannot evaluate an `extern:`
   declaration or call an extern word at all, so the retention tests use an extern-free override
   body (a resource wrapping a plain `i64`).

## Delivery, as shipped

**Phase 1 — recognition pre-pass and registry** (`3d7377cc`). R1 end to end: the four located
errors, the `StructId`-keyed registry, `env`-exclusion and generic-lowering exclusion. Exit:
criteria 5, 6, 7 and criterion 16's check-side half.

**Phase 2 — lowering substitution, linearity, diagnostics, composition** (`5c00b381`,
`098447e8`). R2, R3, R4, R5, R7, R9. Exit: criteria 1, 2, 3, 4, 12, 13, 14, 15, 16 (ir half), 20.

*Deltas.* Recognition was **unified rather than threaded**: `ir::lower` calls
`check::find_drop_overloads` itself and forces the bit on its own copy of the struct decls, so
lowering is correct even when `check` never ran (`lower_forces_drop_overload_linearity_even_when_check_never_ran`),
instead of receiving a table from `check`. R3's fact is a decl bit
(`StructDecl::has_drop_overload`), not a parameter threaded through `is_copy`'s call sites, which
is why the REPL half of R3 needed the bit set on `Session::structs` and not just a map passed to
re-synthesis. The two name-keyed false positives described under R6 were fixed here, ahead of R6.

**Phase 3 — self-recursion reachability** (`6d760401`). R6. Exit: criteria 8, 9, 10, 11, 21.

*Deltas.* Cases (a)/(b) collapsed into one stop rule; the walk covers enums, arrays and cells;
three tests beyond the spec's list: `check_drop_body_recursion_inside_an_if_arm_is_error`,
`check_drop_of_an_overridden_aggregate_does_not_walk_its_fields`,
`check_drop_body_sharing_a_helper_with_another_word_is_not_a_cycle`.

**Phase 4 — extern arity, REPL support, dogfood** (`47696e1b`, `123e2d1b`, `6add95ec`). R8, R10,
R11. Exit: criteria 17, 18, 19, 22.

*Deltas.* R11.2 and R11.3 above (session-wide epoch, then pinning an override's symbol to its
defining epoch) were both found in phase 4's own reviews and rewrote R11.2's original per-struct
generation scheme. The dogfood's `main` as specified did not compile: `File|>fd` is non-consuming
of the *stack slot*, not of the *local*, so the bare `f` term moves the local and the trailing
`f drop` was a use-after-move; the leftover struct is now bound under a fresh name
(`... read . | file | file drop ;`). Four extra REPL goldens beyond the spec's list, pinning the
two revisions and the residual-disposal path.

## Criterion → test map

Goldens and REPL sessions in `tests/phase3_resources.rs` (new file); unit tests beside their stage
in `src/check.rs`, `src/ir.rs`, `src/repl.rs`.

| # | Criterion | Test | Phase |
|---|---|---|---|
| 1 | an overloaded struct is linear even if every field is `Copy` | `check_struct_with_drop_overload_is_linear` | 2 |
| 2 | `dup` on it names the overload as the reason | `check_dup_of_drop_overload_type_names_the_cause` (both `Ctx` arms) | 2 |
| 3 | an unconsumed all-`Copy` resource at end of body is an error naming it | `check_unconsumed_all_copy_resource_at_word_end_is_error` | 2 |
| 4 | a second `drop` of it is use-after-move | `check_double_drop_of_all_copy_resource_is_use_after_move_error` | 2 |
| 5 | non-struct input is a located declaration error | `check_drop_overload_on_non_struct_input_is_error` | 1 |
| 6 | an output is a located declaration error | `check_drop_overload_with_output_is_error` | 1 |
| 7 | two overloads for one struct is a located error | `check_duplicate_drop_overload_for_one_struct_is_error` | 1 |
| 8 | direct self-`drop` is a located error naming `T>` | `check_drop_body_direct_self_recursion_is_error` | 3 |
| 9 | indirect through a helper, likewise | `check_drop_body_indirect_self_recursion_through_helper_is_error` | 3 |
| 10 | `drop` of a `Copy` scalar in a `drop` body is not a cycle | `check_drop_of_copy_scalar_inside_drop_body_is_not_a_cycle` | 3 |
| 11 | `drop` of a different resource in `drop@A` is legal | `check_drop_of_different_resource_inside_another_drop_body_is_ok` | 3 |
| 12 | a linear field must be consumed by the body | `check_drop_body_must_consume_linear_fields` | 2 |
| 13 | an enclosing struct disposes a resource field via its destructor symbol | `resource_field_disposed_via_its_own_drop_symbol` | 2 |
| 14 | a resource on a disposal cycle is not bypassed by the fused loop | `synthesize_destructor_excludes_override_structs_from_a_fused_disposal_path` | 2 |
| 15 | the emitted destructor is the user body, with no field glue | `synthesize_destructor_of_resource_with_a_linear_field_uses_user_body_not_field_glue` | 2 |
| 16 | two overloads coexist; no `IrFunc` named `drop` | `two_drop_overloads_for_different_structs_do_not_collide`, `check_drop_overloads_for_different_structs_both_land_in_the_registry`, `check_drop_overloads_are_excluded_from_env` | 1, 2 |
| 17 | a REPL override is still the destructor two lines later | `repl_drop_overload_still_runs_on_a_later_line` | 4 |
| 18 | a multi-output `extern:` is rejected at the declaration | `check_extern_multi_output_is_error` | 4 |
| 19 | the dogfood runs with the documented output | `slice8b_dogfood_compiles_and_runs` | 4 |
| 20 | an overridden scalar-only struct's `StructLayout::is_linear` is `true` | `ir_registers_overridden_struct_as_linear_despite_all_copy_fields` | 2 |
| 21 | recursion through a containing aggregate is caught | `check_drop_body_recursion_through_a_containing_aggregate_is_error` | 3 |
| 22 | redefinition mints a distinct symbol, not a collision | `repl_redefining_drop_overload_does_not_collide_under_rtld_global` | 4 |

Rows 2, 3, 4, 5, 6, 7, 8, 9, 18 assert the specific message, not merely that compilation failed.
Added beyond the map: `repl_drop_overload_is_kept_by_struct_id_and_out_of_env`,
`repl_drop_overload_declaration_shape_is_validated`,
`repl_generic_glue_symbol_stays_unsuffixed_while_session_has_no_override`,
`repl_unrelated_overload_suffixes_a_composing_structs_glue_too`,
`repl_redefined_drop_overload_runs_the_new_body`,
`repl_redefining_an_overrides_callee_leaves_the_override_alone`,
`repl_declaring_a_second_override_leaves_the_first_alone`,
`repl_quit_disposes_a_residual_resource_through_its_overload`,
`repl_redefining_drop_overload_refreshes_a_composing_structs_glue`,
`repl_composing_structs_glue_is_correct_when_override_postdates_it`,
`repl_resource_field_is_disposed_through_the_overload`.

## Dogfood (`examples/resources.sth`, as shipped)

Reading a real file whose length varies with repo state makes for a non-deterministic golden, so
the dogfood reads a small dedicated fixture with a fixed, known size instead of a project
document: `examples/resource_fixture.txt`, containing exactly `hi\n` (3 bytes, no other content).

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
  f File|>fd &!buf 64 >usize read . | file |
  file drop ;
```

(`main` is the corrected body — see "Delivery, as shipped", Phase 4's deltas, for why it differs
from the original spec's `f drop`. `File|>fd` is verified against the syntax `tests/phase3_refs.rs`
already exercises and passes: `0 >u8 N fill` for a `u8`-element array, `&!name` as the prefix
mutable borrow of a bare array-typed local, and `File|>fd` as the non-consuming peek of a `Copy`
field, legal regardless of the enclosing struct's own linearity since `check_struct_peek_word`
gates only on the *field*'s `is_copy`, never the struct's. `buf` needs no explicit `drop`: it is
`Copy`, and the surplus-value check inspects only the final stack, not bound locals, so a `Copy`
local left unused simply goes out of scope.)

Expected output: exactly `"3\n"` (the fixture's fixed byte count, from `read`'s return value,
printed with the trailing newline every other golden asserts) — deterministic regardless of repo
state, unlike reading a real project document would be. The golden test
(`slice8b_dogfood_compiles_and_runs`) invokes the built binary with the working directory pinned
at the repo root (`Command::current_dir`): this is the first example that opens a file at *run
time* rather than only using a relative `.sth` path as compiler input, so no prior golden ever
needed to set it.

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
`Type::Spy`/`IrType::Spy` untouched (`emit_drop`'s `IrType::Spy` arm, `src/ir.rs:2936`), deferred
to Slice 8c along with the D7 unification cut above. The general multi-output-lowering panic for
ordinary user words, as distinct from the `extern:` case R8 fixes. A symbol-existence check for
`close` (unchanged from 8a's R14). `extern:` at the REPL, in general: R11 makes a `drop` overload
work at the REPL, but the REPL still cannot evaluate an `extern:` declaration or call an
extern-declared word at all (unchanged from 8a's own Out-of-scope note); R11's own tests work
around this with an extern-free override body. `^T`/cell destructor dispatch joining any shared
resolution step (moot, since D7's unification is cut, not R9 — R9 in this document is the
non-`Copy`-fields decision). An overloadable `dup` / opt-in reference-counting, and `drop`
becoming fully polymorphic open dispatch (Phase 4's ad-hoc overloading, and Phase 6's deferred RC
problem): this
slice does not lay groundwork toward either. Any change to `str`/`cstr` or the `.`-separator
question (8a, DESIGN.md Open/deferred).

**Accepted limitations found and documented, not fixed, during post-implementation review:**

- A REPL word body compiled *before* a struct gains a `drop` override (while the struct was
  still all-`Copy`) is not retroactively re-lowered once the override makes it linear — calling
  that stale-compiled word on a later, now-linear value of that type silently runs no destructor
  at all. This is a linear-spine hole in principle, but matches the REPL's existing "word bodies
  are snapshots at definition time" semantics used everywhere else, and re-lowering every
  already-compiled word on every later type-linearity change is out of scope for this slice.
- An override's body that calls a *different* struct's override binds that callee to the callee's
  epoch at the point the caller override was defined — redefining the callee later does not
  retroactively update an already-lowered caller. This is asymmetric with generated
  (non-override) composition glue, which *does* refresh on redefinition; the asymmetry is a
  decision (R11.2/R11.3 above), not an accident.
- R6's whole-program reachability check now runs at the REPL too (`check_drop_overload_reachability`,
  against the session's live override registry), so both a direct and a same-body indirect
  self-recursive `drop` override are rejected there exactly as natively. It still cannot catch
  recursion that only closes through an *ordinary* REPL helper word defined on an earlier,
  separate line: the REPL retains no bodies for ordinary words across lines, so there is nothing
  to walk for that case, the same residual gap R6's own "known, accepted limitation" paragraph
  above already accepts for the native pass.
