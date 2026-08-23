# P7.S3r paper dogfood + migration inventory

Pre-check for `docs/roadmap/P7/slice3r-brief.md` (replace the `impl:` *binding* form with a
*body* form, member signatures inherited from the trait). Nothing here compiles: the body
syntax does not exist yet. All findings are hand-traces and grep evidence against the tree
at the parent's `5338c06` baseline. No probe `.sth` files were built (there is nothing to
build the new syntax against); the existing-behaviour claims the brief relies on were
already probed by the parent and are not re-run here.

## Part A — paper dogfood: `examples/traits.sth` in the body form

The current file declares two standalone words (`point-cmp`, `point-show`) and binds them
with two `impl:` lines. Under the body form both words move *into* their impl block and
their signatures vanish (inherited from the trait plus `for Point`):

```sooth
import: intrinsics * ;
import: core::prelude | if Bool lt gt | ;

type: Ordering | Less | Equal | Greater ;

trait: Order 'T
  cmp ( &'T &'T -- Ordering )
;

trait: Show 'T
  show ( &'T -- )
;

type: Point x i64 y i64 ;

\ No `( &Point &Point -- Ordering )`: inherited from `Order`'s `cmp` with 'T = Point.
impl: Order for Point
  : cmp
    | a b |
    a &y @ | ay |
    b &y @ | by |
    ay by lt
    ~[ Less ]
    ~[
      ay by gt
      ~[ Greater ]
      ~[
        a &x @ | ax |
        b &x @ | bx |
        ax bx lt
        ~[ Less ]
        ~[ ax bx gt ~[ Greater ] ~[ Equal ] if ] if
      ] if
    ] if ;
;

\ No `( &Point -- )`: inherited from `Show`'s `show` with 'T = Point.
impl: Show for Point
  : show
    | p | "(" . p &x @ . "," . p &y @ . ")" . ;
;

: show_larger ( &'T: Order Show &'T -- )
  | b | | a |
  a b cmp
  ~[ ( Less ) drop b show ]
  ~[ ( Equal ) drop a show ]
  ~[ ( Greater ) drop a show ]
  Ordering? ;

: main ( -- )
  0 0 Point | origin |
  3 4 Point | corner |
  &origin &corner show_larger
  origin drop
  corner drop ;
```

### Hand-trace: does every construct survive?

- **Every construct survives.** `cmp`/`show` bodies, `show_larger` (bound-dispatched, so
  untouched), and `main` are unchanged. The migration is a net *removal* of two top-level
  names (`point-cmp`, `point-show`) plus two restated signatures.
- **No recursion in this example.** `point-cmp` does not call itself, so the brief's
  recursion/shadowing machinery (decisions 4/5) is not exercised by `traits.sth`. It has
  no witness here; its only witnesses live in the brief's own P1/P3 probes.
- **Signature inheritance loses nothing the *checker* needs.** For both members `'T`
  appears as `&'T` in the inputs, and the P7.S3p rule already forces every trait member to
  take `'T`/`&'T` as its last input
  (`tests/phase7_slice3e.rs::trait_member_with_a_zero_input_receiver_is_rejected`). So a
  member can never have a fully `'T`-free signature, and the `(trait, for-type)` pair
  always fully determines the grounded effect. Inheritance is never ambiguous.
- **Signature inheritance has a *reader* cost that scales with signature shape.** At the
  `impl:` site the concrete effect is no longer written. For a `&'T`-only member
  (`cmp`, `show`) the substitution is trivial (`&Point`). The cost grows when a member
  embeds `'T` in an array or generic-application shape, e.g. a hypothetical
  `clone ( &'T -- ['T 4] )` grounds to `( &Point -- [Point 4] )`; the reader must ground
  it themselves against the trait. `check_impl_decls`' `ground_member_type` already handles
  exactly "concrete / array / reference shapes over 'T" (`src/parser.rs:287-289`), so the
  checker copes, but the human loses at-a-glance the concrete signature. Mild for
  `traits.sth`, worth a spec note for richer members.

**Part A verdict:** clean translation, no lost checker information, one reader-locality
tradeoff (no concrete signature at the impl site) that is mild here and grows with member
signature complexity.

## Part B — migration inventory (decides OQ1: delete vs keep both)

### Counts by category (baseline `5338c06`, worktrees excluded)

| Category | Sites | Where |
| --- | --- | --- |
| Real `.sth` source declarations | **2** | `examples/traits.sth:55,56` |
| `.sth` `impl:` in comments (no migration) | 4 | `examples/traits.sth` prose |
| `impl:` anywhere in `lib/` | **0** | — |
| Rust test-fixture declaration occurrences | **~55** | 6 files (below) |
| Rust doc-comment / error-string mentions (no migration) | rest of the 122 `grep -o` hits | prose/asserts |

Rust fixture declaration occurrences, by file (excluding `///` docs, `//` comments, and
`"error: ..."` assertion strings):

```text
17  tests/phase7_slice3e.rs
12  src/check/poly.rs
11  src/check/declarations.rs
 7  src/driver.rs
 5  src/parser.rs
 3  src/ir/driver.rs
```

**The count IS the slice size, and it is almost entirely test fixtures.** Two real source
lines; ~55 fixture declarations. But a raw count overstates a mechanical migration, because
a meaningful subset are *negative tests of the binding form's own validation surface*, and
several test a diagnostic class the brief says disappears (see below). Classify by test
subject, not by grep count.

### The O3 "one word satisfies two traits" convenience: unused anywhere

Checked every `(member -> word)` binding pair across all fixtures. **No single program binds
one concrete word to two different `(Trait, Type)` members.** The apparent reuse of `p-b`
(bound in `B for PB`, `B for PA`, `A for PB`) is across *three independent test fixtures*,
never within one program. So the convenience OQ1 worries about losing is **hypothetical in
this codebase**: deleting the binding form strands no existing consumer of that shape.

### The real capability the body form removes: bind a member to an *existing named* word

The body form can only *create* a fresh (synthesised-name) word; it cannot point a member
at a word that already exists under its own name. Two fixtures exercise exactly that, both
deliberately binding a member to an **operator-named** word:

- `src/ir/driver.rs:1627` — `impl: Getter for Pt  get max ;` with `: max ( &Pt &Pt -- i64 )`.
  The test `an_operator_named_impl_member_reached_only_by_a_bound_is_not_pruned` asserts the
  emitted call symbol is literally `"max"` and that a func named `max` survives pruning.
- `tests/phase7_slice3e.rs:562` — `impl: Getter for Pt  show max ;`, same shape.

Under the body form there is no way to make the member *be* `max`; you would write a
forwarding body `: get | a b | a b max ;`, which emits a call to `max` from inside the
synthesised `get;Getter;Pt`. The pruning *concern* (does a bound-reached operator overload
survive?) can still be probed, but the test's current assertion (`call_symbols == ["max"]`)
no longer holds and the scenario "an operator overload IS a trait member" becomes
inexpressible. **This is the sharpest migration hazard and the strongest concrete argument
against a clean delete.** It is a capability, not just the namespace leak the brief frames it
as: binding a member to a pre-existing, independently-useful word (an operator overload, a
shared helper) is only expressible via a forwarding body under the new form.

### Fixtures whose diagnostic class DISAPPEARS under the body form (delete/replace, not migrate)

- **Signature mismatch.** `tests::impl_binding_with_a_mismatched_signature_is_rejected`
  (+ `src/check/declarations.rs:3752` sig-mismatch fixture). Brief motivation 1 explicitly
  retires this class; replace with an ordinary in-body stack-effect-error test.
- **Polymorphic implementing word.** `tests::impl_binding_a_polymorphic_word_is_rejected`
  (tests:302) + `declarations.rs:3771` (`poly-show`) + `declarations.rs:3792`
  (`nothing p`). A body has an inherited *concrete* signature and cannot be polymorphic, so
  the "concrete word" check (spec3e decision 2) has nothing to fire on. Disappears.
- **Member bound to `drop`.** `declarations.rs:3869` `impl: Eat for Spy  eat drop`. There is
  no binding-to-`drop`; a body that *calls* `drop` is legal. Disappears.
- **Odd binding-token count parse error.** `src/parser.rs:2124-2126`. The body form has no
  `member word` pairs, so this parse error is deleted outright.

### Fixtures that migrate mechanically (rewrite the syntax, same intent)

- Every positive dispatch/lowering fixture (`impl: Show for Point  show point-show`,
  `get pt-get`/`get qt-get`, `eq point-eq  hash point-hash`, the `sort3` golden's
  `Order for i64`/`Order for Pair`, the cross-module `a::A`/`b::B` pair, etc.) across
  `poly.rs`, `driver.rs`, `ir/driver.rs`, `tests/phase7_slice3e.rs`.
- Structural rejections that live at the `trait:` decl or the registry, not the binding:
  `duplicate_impl_for_the_same_trait_and_type`, `impl_binding_missing_a_required_member`
  (body omits a member), orphan-rule, export/selective-import, P7.S3p `sandwiched_receiver`
  and zero-input-receiver.

### A NEW ruling the body form forces (not in the brief's OQ list)

`tests::impl_binding_an_unknown_member_is_rejected` binds `bogus int-show` where `bogus` is
not a member of `Show`. Under the body form this becomes **a `: bogus ... ;` inside an
`impl:` block naming a word that is not a trait member**. Brief decision 1 ("each
`: member ... ;` becomes a top-level word *plus the binding pair*") has no binding pair to
synthesise for a non-member, so the spec must rule: is a non-member body an error, or a free
top-level word? Recommend: located error ("`bogus` is not a member of trait `Show`"),
preserving the current diagnostic's intent. Flag this in the spec explicitly.

## Part C — OQ3: does any consumer need sibling-member access?

**No.** A member body calling *another member of the same impl* is needed by nothing in the
tree. Sites checked:

- `examples/traits.sth`: `Order`/`Show` are single-member traits in separate impls; no
  cross-call.
- Every multi-member impl fixture: `impl: Eq for Point  eq point-eq  hash point-hash`
  (`src/check/poly.rs:5738`) and the `Eq eq/hash` fixtures — `point-eq` (`drop drop 1`) and
  `point-hash` (`drop 7`) are independent; neither calls the other.
- The planned consumers in `docs/roadmap/P7/slice3-dogfood.md` (`Map['K 'V]` needing
  `Eq` + `Hash`, `sort` needing `Order`): the *bound-carrying word* (`probe`) calls both
  `eq` and `hash`, but that is bound dispatch from a poly body, not one impl member's body
  calling a sibling. Within a concrete `Eq`/`Hash` impl, `eq` and `hash` do not call each
  other.

So OQ3's default (a member body sees only its own name, not its siblings) has **no consumer
that would break**. Lock the default; note it is a ruling by absence of a consumer, not a
proof that no future consumer wants it.

## Bottom line for OQ1

The migration is small in real source (2 lines, one file, a net simplification) and
concentrated in ~55 test-fixture declarations that must be re-classified, not blindly
rewritten: ~4 diagnostic-class tests are deleted/replaced (the brief retires their class),
1 new ruling is forced (non-member body), and **2 fixtures encode a capability the body form
cannot express** (binding a member to an existing operator-named word), which is the one real
cost of a clean delete. The "one word, two traits" convenience the brief cites as the delete's
cost is unused anywhere and can be discounted; the operator-overload-as-member capability is
the argument that actually deserves a ruling before locking OQ1.
