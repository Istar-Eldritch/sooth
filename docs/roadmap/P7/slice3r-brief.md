# Phase 7 Slice 3r: `impl:` bodies instead of `impl:` bindings (brief)

Replace `impl:`'s binding form (`impl: Order for Point  cmp point-cmp ;`, a map from
member names to separately-declared concrete words) with a body form that declares the
implementing words inside the block:

```sooth
impl: Order for Point
  : cmp | a b | ... ;
;
```

A member's stack effect is **not restated**: it is the trait's member signature with the
trait variable substituted by the `for` type. This revisits `slice3e-spec.md` decision 1
("`impl:` is a binding, not a body"), which chose the binding form to avoid inventing an
impl-body lowering path. That objection does not apply to the design below, which
desugars at parse time and leaves check, dispatch, and lowering untouched.

## Motivation

1. **A restated signature is a diagnostic class that need not exist.** The binding form
   compares a trait member's signature against a separately declared word's, so
   `slice3e-spec.md` decision 2's mismatch error exists only because the signature is
   written twice. Under signature inheritance the comparison is vacuous, and a wrong
   implementation instead produces ordinary body-checking errors located *inside* the
   impl, where the mistake is.
2. **The binding form leaks a name the design does not want named.** `point-cmp` exists
   only to be pointed at: it occupies the module namespace, is importable, and
   participates in overload resolution. Meanwhile `slice3e-spec.md` decision 9 already
   says a bound-dispatched member is callable *without* importing the implementing
   word's name. The binding form therefore requires the user to invent and expose a name
   that bound dispatch never uses.

## Recon (probed against the built compiler at `5338c06`)

Eight probes, each a complete program built with `sooth build` under `examples/sooth.pkg`.

| # | shape | result |
| --- | --- | --- |
| P1 | a concrete impl member word calling itself by name | builds |
| P2 | a generic forwarder `: rcmp ( &'T: Order &'T -- Ordering ) cmp ;` called from `main` | builds |
| P3 | the monomorphization cycle `point-cmp -> rcmp['Point] -> point-cmp` | builds |
| P4 | bare `cmp` in a *concrete* body with `&Point &Point` on the stack | `error: unknown word cmp` |
| O1 | two words named `sz`, different signatures | builds (overload set) |
| O2 | two words named `sz`, identical signatures | `error: duplicate word sz` |
| O3 | traits `A` and `B` both declaring `f ( &'T -- i64 )`, both impl'd for `Point`, bound to two different words, called through `'T: A` and `'T: B` | builds |
| O4 | a module-private `lt ( i64 i64 -- Bool )` alongside a selective import of prelude's `lt` | `error: selective import of lt ... collides with a local definition` |

Consequences that shape the design:

- **P1** is the precedent that makes recursion a naming question, not a new resolution
  mechanism: a word can already call itself by name.
- **P4** rules out type-directed dispatch of a member name from a concrete body. Adding
  it would be a new global name-resolution path and contradicts decision 9.
- **O3 is the capability to preserve.** Two traits may share a member name *and* its
  substituted signature for one type, disambiguated by which bound the calling poly word
  declares. This is why the implementing word cannot simply be registered under its bare
  member name (that shape is O2, a duplicate-word error).
- **O4** establishes that same-name/same-signature shadowing is currently a located
  error, so the shadowing rule below is a deliberate, scoped exception, not a gap.

## Design

1. **Parse-time desugar.** Each `: member ... ;` inside an `impl:` block becomes a
   top-level word plus exactly the binding pair the current form writes by hand.
   `check::check_impl_decls`, the `(TraitId, Type)` impl registry, bound-directed
   dispatch, and lowering are unchanged. There is still no impl body after parsing, so
   there is still no impl-body lowering path.
2. **Signature inheritance.** The synthesized word's declared effect is the trait member
   signature with the trait's variable substituted by the `for` type. Writing an explicit
   signature inside an impl body is rejected (one spelling, not two).
3. **The synthesized name is trait-qualified and unforgeable by construction.**
   Trait-qualified to preserve O3. Unforgeable by containing a lexer delimiter (e.g.
   `cmp;Order;Point`): words split only on whitespace and delimiters, so `$`-bearing
   names like `Order$Point$cmp` *are* legal user words (probed) and would be forgeable.
   No new name-rejection rule is needed if the delimiter approach is used.
   **Verified end-to-end** by a throwaway spike, `docs/roadmap/P7/slice3r-spike-mangling.md`:
   the name survives `resolve::mangle` with no new exemption, `qbe_name` escapes `;` to
   `.3b.` injectively at both definition and call site, and the binary links and runs.
   The spike also found the one real cost: the synthesized name **leaks verbatim into
   user diagnostics** (`demangle_word` strips only the `__m{n}` suffix), so a mis-called
   member reports a name the user never wrote and cannot type. Giving diagnostics a way
   to render a synthesized member name back to something readable is a **required spec
   ruling**, not polish.
4. **A member name inside its own impl body resolves to that member, before any
   module-scope lookup.** This is the recursion story. It is shadowing only in the
   same-signature case: a differently-signed in-scope word of the same name is already
   separated by ordinary overload resolution (O1).
5. **Shadowing is admitted here and nowhere else.** It is accepted for this construct
   because the shadow is visible at the point of use, the enclosing `impl: Trait for
   Type` header is the immediately enclosing form, and it is statically decided. General
   word shadowing is explicitly *not* adopted: a top-level word shadowing an import has
   no enclosing context at the call site, and O4's error is what makes such a collision
   loud. Any future general-shadowing rule needs its own consumer and its own visibility
   story, and must not be inherited from this slice.

## Rejected alternatives

- **Register the implementing word under its bare member name** and let ordinary
  overload resolution handle both recursion and dispatch. Costs nothing new and needs no
  shadowing rule, but breaks O3: two traits sharing a member name and signature for one
  type become undeclarable (O2).
- **Require a user-written generic forwarder over the bound** for the recursive call.
  Works today (P2/P3) and needs no new rules, but costs a top-level poly word per
  recursive member, reintroducing precisely the namespace leak this slice removes, and
  only for types that happen to be recursive.
- **Type-directed dispatch of member names in concrete bodies.** Ruled out by P4.

## Open questions

1. ~~Migration of the existing binding form.~~ **Resolved: delete it and migrate.** The
   body form is the only spelling; every existing `impl:` site (`.sth` sources and Rust
   test fixtures alike) is rewritten, and the binding form's parse path is removed rather
   than deprecated. Two spellings of one concept is the thing being avoided. Consequence
   to carry into the spec: the "one concrete word satisfies two traits' members"
   convenience is withdrawn and becomes a one-line forwarding body; a spec phase must
   own the migration, and its size is the inventory count.
2. ~~`export:` and the orphan rule.~~ **Resolved by recon: satisfied with no new rule.**
   Nothing requires the implementing word to be user-named, and a delimiter-bearing name
   is unnameable in an `export:` line by construction. Capability actually lost, to record
   rather than solve: a user *can* `export:` an implementing word (`point-cmp`) today, and
   the body form removes that.
3. ~~Does a member body see the other members of the same impl?~~ **Resolved: no.** No
   consumer in `lib/`, `examples/`, the `impl:` fixtures (`Eq`'s `eq`/`hash` members do
   not call each other), or `slice3-dogfood.md`'s planned `Map`/`sort` consumers needs it;
   in the planned consumers the cross-member calls come from the bound-carrying poly body,
   not from a sibling member. Lock the default. This is a ruling by absence of a consumer,
   not a proof that no future one wants it.
4. **The trailing `; ;`.** The block structurally parses as "loop until `Semicolon`, then
   expect `Semicolon`", but the doubled terminator is the form's one cosmetic wart. Sooth
   has no `end` to borrow. Accept, or spend a keyword? Note that `parse_worddef`
   (`src/parser.rs:1791`) *mandates* a `(` effect, so it cannot be reused unchanged
   anyway: signature inheritance needs its own body-member parse path.
5. **A body naming something that is not a trait member.** The binding form rejects
   `bogus int-show` (`impl_binding_an_unknown_member_is_rejected`). Under the body form,
   `: bogus ... ;` inside an `impl:` block has no member to bind to and decision 1 has no
   pair to synthesize. Rule it a located error ("`bogus` is not a member of trait
   `Show`"), preserving the current diagnostic's intent; the alternative (a free
   module-private word) silently swallows a typo.
6. **An operator-spelled member name, which sharpens decision 4.** Two existing fixtures
   pin an operator-named implementing word reached only through a bound
   (`src/ir/driver.rs:1622`, `tests/phase7_slice3e.rs:559`). Under the body form the
   collision moves from the *implementing word* to the *member name*: a trait member named
   `max` or `add` means decision 4's self-binding shadows a **builtin operator**, not just
   a user word. That is a wider exception than decision 5 signed up for, and it needs its
   own ruling: either members may not be operator-spelled, or the shadow is admitted with
   the builtin explicitly losing inside the impl body.

## Pre-check findings (three workers, verified against `5338c06`)

Companion documents: `slice3r-spike-mangling.md` (the mangling spike) and
`slice3r-paper-dogfood.md` (paper dogfood plus migration inventory).

1. **The migration is small in source and concentrated in fixtures.** Two real `.sth`
   declarations (`examples/traits.sth:55,56`), zero in `lib/`, and ~55 Rust test-fixture
   declarations across 6 files. The count must be classified by test subject, not
   rewritten wholesale.
2. **Five `check_impl_decls` guards go vacuous, not one.** Beyond signature mismatch:
   unknown-word, cross-module binding, polymorphic-member (two tests), and drop-overload
   all key on a hand-declared word that the desugar now guarantees
   (`src/check/declarations.rs:3744/3767/3778/3859/3881`). Each is a test that would
   silently become a placebo, so the spec must delete them with the binding form rather
   than migrate them.
3. **The O3 convenience is unused.** No program in the tree binds one concrete word to two
   different `(Trait, Type)` members, so deleting the binding form strands no existing
   consumer of that shape. OQ1's stated cost is hypothetical here.
4. **The dogfood's "sharpest hazard" is smaller than reported, and was re-probed.** It
   claimed the body form cannot express binding a member to an existing named word, citing
   the operator-named fixtures. Probed directly: the two-input shape migrates fine (a
   forwarding body builds, runs, and keeps `max__m0` in the symbol table), and the
   one-input shape needs no forwarding at all, because a one-input word named `max` is
   **unreachable by name anywhere today** (`max` resolves to the two-input builtin:
   ``error: `max` needs 2 values, but the stack holds 1``). The body form reaches it by
   *being* it. So no capability is lost; what changes is that the scenario must be re-pinned
   as an operator-spelled *member name*, which is OQ6.
5. **The REPL needs its own located rejection.** `impl:` and `trait:` are unsupported
   there and produce no located error today (`impl:` falls into the term loop and reports
   `unexpected token Semicolon`), matching this project's standing hazard that anything
   wired only into `assemble_module` is unenforced at the REPL. Mirror the `export:` /
   `global:` guards at `src/repl.rs:1596/1700`.
6. **Signature inheritance is never ambiguous.** Every trait member must take `'T`/`&'T`
   as some input (`member_binds_trait_var`; a member binding it in none was rejected at
   `trait:` time, deferred as P7.S3t — **superseded: P7.S3t relaxed the gate to admit an
   empty input list**), so a member can never have a `'T`-free signature
   and the `(trait, for-type)` pair always determines the grounded effect. The cost is to
   the *reader*, not the checker, and it grows with signature shape (a member like
   `clone ( &'T -- ['T 4] )` must be grounded by hand at the impl site).

## Out of scope

- General word shadowing (see decision 5).
- Any change to bound-directed dispatch, the impl registry, monomorphization, or
  lowering.
- P7.S3o (a bound on a poly combinator's own type variable) and P7.S3n
  (`Map['K 'V]`).

## Ready to spec?

**Yes, with two rulings the spec must carry rather than discover.** OQ1 (delete and
migrate), OQ2 (export), and OQ3 (no sibling access) are resolved; design decision 3 is
verified end-to-end by spike, including linking. The mechanism has no remaining unknown.

The two rulings to make explicit in the spec, both surfaced by the pre-check rather than
by the original design:

- **Diagnostic rendering of a synthesized member name** (decision 3's measured defect).
  Without it, a routine mistake shows the user a name they cannot type.
- **Operator-spelled member names** (OQ6), which decides how far decision 5's scoped
  shadowing actually reaches. This is the one place the design's shadowing exception could
  quietly widen from "shadows a user word" to "shadows a builtin".

OQ4 (the `; ;` wart) and OQ5 (a non-member body) are small enough to settle in the spec.
The spec's phase plan should treat the fixture migration as its own phase, sized by the
inventory, with the five vacuous guards deleted rather than rewritten.
