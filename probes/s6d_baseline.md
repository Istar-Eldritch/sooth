# Non-regression baseline: S6d probe round (S6d-7..S6d-10)

Captured 260911, HEAD `390b1b2` (post-S12 main tip; worktree `soo-38`,
`src/` and `lib/` untouched by the round). Byte-exact
`sooth build probes/<name>.sth --manifest tests/fixtures/sooth.pkg` stderr
per fixture on the CLEAN tree, plus run stdout where the build succeeds.
Invocation: run from the repo root; the shared manifest
`tests/fixtures/sooth.pkg` resolves `core`/`hosted`. Compiled binaries are
removed from `probes/` after each accepted build (the runner writes them
beside the source).

Patched-tree behavior (the S6d-8 two-half sentinel patch in a scratch copy)
is in the round report, not here — except where noted, every fixture below
contains an `impl: Iterator for Slice[i64]` (or `!Slice[i64]`) block, so its
clean-tree behavior is the S2-6 member-app fence, byte-identical in shape to
the old S6d-1/S6d-2 captures, at the member's own line/col.

### probes/s6d_a_fence_baseline.sth

The shared-target fence baseline (S6d-7.1). Also the exact spelling that
builds and runs on the patched tree (one mono `next` step: prints `3`, then
the remainder's length `4`).

```text
error: trait member `next` of `Iterator` (line 37, col 3) applies the trait variable `'It`, but the impl target `Slice[i64]` is concrete
  an application-headed member has no monomorphic representation (its applied arguments are member locals); implement the trait for a constructor target with a type variable instead
```

exit=1. The mutable twin (`!Slice[i64]`, one word different, run from /tmp)
differs only in the target display:

```text
error: trait member `next` of `Iterator` (line 37, col 3) applies the trait variable `'It`, but the impl target `!Slice[i64]` is concrete
  an application-headed member has no monomorphic representation ...
```

### probes/s6d_b_step_shared_inline.sth

S6d-7.2a + 7.2d — PREREQ admissions isolated from the fence. Builds and runs
on the CLEAN tree (no patch needed): the inline `mk` packs the shared slice
into the declared `Step` payload, the mono consumer destructures it, and
`dup-check` (`More dup drop drop`) proves the packed container is `Copy`.

```text
41
5
```

build exit=0, run exit=0.

### probes/s6d_b2_noninline_out.sth

S6d-7.2b — the non-inline twin. `check_reference_free_signature`'s output
ban at the declared signature:

```text
error: a reference cannot be stored: `mk` declares the output `Step[i64 Slice[i64]]`
  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead
```

exit=1.

### probes/s6d_b3_mut_payload.sth

S6d-7.2c — the `!Slice` payload twin. The enum-payload sweep of
`check_no_stored_references` rejects the parse-time-minted monomorph (same
sweep/wording shape as the old S6d-2 rejection, naming `!Slice`):

```text
error: a reference cannot be stored: payload field 1 of variant `More[i64 !Slice[i64]]` of type `Step[i64 !Slice[i64]]` has type `!Slice[i64]` (line 10, col 3)
  a `&T`/`&!T` borrows a local and may not outlive it, so it cannot be put anywhere that survives the borrow
```

exit=1.

### probes/s6d_c_impl_consumer.sth

S6d-8.3 — the full monomorphic consumer (non-tail recursive `drain`,
5-element then 1-element view). Clean tree: the fence, at the member:

```text
error: trait member `next` of `Iterator` (line 27, col 5) applies the trait variable `'It`, but the impl target `Slice[i64]` is concrete
  an application-headed member has no monomorphic representation ...
```

exit=1.

### probes/s6d_d_selftail.sth

S6d-10.1 — the self-tail drain. Clean tree: the fence at the member.

```text
error: trait member `next` of `Iterator` (line 17, col 5) applies the trait variable `'It`, but the impl target `Slice[i64]` is concrete
  an application-headed member has no monomorphic representation ...
```

exit=1. (On the patched tree this BUILD AND RUNS — the 10.1 prediction is
falsified for non-inline words; see the round report.)

### probes/s6d_d2_selftail_inline.sth

S6d-10.1's sharper twin — the same drain declared `inline`, so the spliced
copy sees the real frame root. Clean tree: the fence at the member (line 17).
On the patched tree:

```text
error: a reference to a local cannot cross a loop in `main` (line 37)
  a reference derived from `buf`, a local of this frame, crosses the self-tail-call back-edge to `drain`: that local's storage does not survive to the next iteration
  note: declared ( -- )
```

exit=1 — `check_reference_across_back_edge` fires exactly when the deriv's
root is a visible frame local.

### probes/s6d_e_nontail_drain.sth

S6d-10.2 — the non-tail drain. Clean tree: the fence at the member.

```text
error: trait member `next` of `Iterator` (line 17, col 5) applies the trait variable `'It`, but the impl target `Slice[i64]` is concrete
  an application-headed member has no monomorphic representation ...
```

exit=1.

### probes/s6d_f0_libiter_gate.sth

(Not in the original name list; added to pin the gate.) A slice impl over the
IMPORTED `core::iterator` trait in the entry file. Clean tree: the fence at
the member (line 9). On the patched tree the impl parses and a DIFFERENT,
tree-independent gate fires:

```text
error: `impl: Iterator for Slice[i64]` at line 8, col 1 must live in the module declaring `Iterator` (`Slice[i64]` declares no module of its own)
```

exit=1.

### probes/s6d_f_for_each_slice.sth

S6d-8.6 — the lib's `for_each` body verbatim over a probe-local trait with
the slice impl. Clean tree: the fence at the member (line 17). On the
patched tree the bound-slot unification rejects the call site before any
App-fence or back-edge check:

```text
error: type mismatch in `main` (line 41)
  `for_each` expected `'It['T]`, found `Slice[i64]`
  note: declared ( -- )
```

exit=1.

### probes/s6d_f2_fold_slice.sth

S6d-8.6 — the lib's `fold` twin. Clean tree: the fence at the member (line
17). On the patched tree, the same unification rejection:

```text
error: type mismatch in `main` (line 49)
  `fold` expected `'It['T]`, found `Slice[i64]`
  note: declared ( -- )
```

exit=1.

### probes/s6d_g_mut_impl.sth

S6d-9 — the mutable impl (`!Slice[i64]`, linear mentions, `&!>` read). Clean
tree: the fence with the mutable target display:

```text
error: trait member `next` of `Iterator` (line 17, col 5) applies the trait variable `'It`, but the impl target `!Slice[i64]` is concrete
  an application-headed member has no monomorphic representation ...
```

exit=1. On the patched tree the Ruling A enum-payload sweep fires first
(see the round report for the verbatim text).

### probes/s6d_h_while_drain.sth

S6d-10.3 — the while-threaded drain. Clean tree: the fence at the member
(line 18). On the patched tree the while combinator's OWN internal join
rejects (see the round report for the mechanism):

```text
error: borrow state disagrees at the branch join in `drain` (line 80)
  the first arm leaves no live borrow, the second arm leaves a borrow with no local root: both arms must agree on which place, if any, stays borrowed past the join
  note: declared ( Slice[i64] -- )
```

exit=1.

### probes/s6d_i_times_drain.sth

S6d-10.4 — the `times`-bounded drain. Clean tree: the fence at the member
(line 18). On the patched tree the times quotation's standalone check leaves
`next` no concrete operand:

```text
error: `next` in `drain3` (line 43, col 7) is a trait member of Iterator, but no `impl:` in this program dispatches on these operands
  the operand types here are ``; declare an impl of one of those traits for the operand's type, or import a word that claims this name
```

exit=1.

### probes/s6d_next_noninline.sth

S6d-8.4(iii) pinned as a fixture — the working impl with the member
re-spelled non-inline. Clean tree: the fence (line 37). On the patched tree
the member word's own output ban:

```text
error: a reference cannot be stored: `next` (member of trait `Iterator` for `Slice[i64]`) declares the output `Step[i64 Slice[i64]]`
  a `&T`/`&!T` borrows a local of the callee's own frame, which is gone by the time the caller reads it; take the reference as an input instead
```

exit=1.

### Cheap sanity (S6d-7.3)

`sooth build examples/slices.sth` (manifest discovery via
`examples/sooth.pkg`), clean HEAD: build exit=0; run prints

```text
15
6
6
```

run exit=0 — bare-slice input to non-inline words (`sum`/`double`)
admissible, unchanged post-PREREQ.
