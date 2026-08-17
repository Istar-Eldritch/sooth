# P7.S3 paper dogfood: `Map['K 'V]` and `sort` against the `trait:` sketch

Pressure-testing `docs/roadmap/P7/slice3-brief.md`'s opening surface
(`trait: Show 'T show ( 'T -- ) ;`, then `'T: Show` as a bound) by hand-writing a
`fixed`-layer `Map['K 'V]` and an array `sort`. The bodies below are Sooth-flavored
pseudocode and **do not compile** (no `trait:` keyword exists). Where a line fails to
type for a reason *other* than the missing keyword, it is tagged `[VERIFIED-BLOCKED]`
with the probe result, because those are the frictions that change the slice's budget.

Surface conventions taken from real files: `&f`/`&!f` receiver-directed field
projection, array element refs `&>`/`&!>`, `@`/`!`, `fill`, `len`, `| names |`
locals, clause dispatch `| Variant ... |`, bounds written inline at a variable's
first mention (`lib/core.sth`: `( 'T: Copy Ord 'T -- bool )`; `examples/poly_borrow_setat.sth`:
`( ['T: Copy 4] 'T -- ['T 4] )`). `Ordering` is `lib/binary_search.sth`'s
`Less | Equal | Greater`.

---

## Traits designed for this exercise

```sooth
\ Equality: ref-taking, because a collection must compare a stored key without
\ moving it out of the backing store (a non-Copy 'K cannot be @-read to a value).
trait: Eq 'T
  eq ( &'T &'T -- bool ) ;

\ Hashing: also ref-taking, same reason. Concrete usize output.
trait: Hash 'T
  hash ( &'T -- usize ) ;

\ Ordering for user types. NOT the builtin `Ord`: see friction #1.
trait: Order 'T
  cmp ( &'T &'T -- Ordering ) ;
```

---

## Program 1 — fixed-capacity open-addressing `Map['K 'V]`

```sooth
\ An entry cell. Empty is nullary, so `fill Empty` constructs the whole backing
\ array without a 'K/'V value in hand (see friction #6).
type: Entry 'K 'V | Empty | Full k 'K v 'V ;

type: Map 'K 'V 'N              \ 'N = capacity (a length parameter; friction #7)
  slots [Entry['K 'V] 'N]
  count usize ;

: empty ( -- Map['K 'V 'N] )
  Empty fill 0 >usize Map ;     \ every slot Empty, count 0

\ Linear-probe for the home slot of `k`, returning the index of either the
\ matching Full cell or the first Empty cell.
: probe ( &Map['K: Eq Hash 'V 'N] &'K -- usize )
  | m k |
  k hash  m &slots len rem  | i |                 \ home = hash(k) mod N
  \ walk while the slot is Full with a different key
  [ m &slots i &> ~[ Empty  false ]               \ [VERIFIED-BLOCKED] see #4, #5
                   ~[ Full   | ek ev | &ek k eq not ev drop ] Entry? ]
  [ i 1 + m &slots len rem  &m &k i! ]             \ i := (i+1) mod N ; loop
  while
  i ;

: get ( &Map['K: Eq Hash 'V 'N] &'K -- Option[&'V] )
  | m k |
  m k probe | i |
  m &slots i &> ~[ Empty          None ]
               ~[ Full | ek ev | ek drop ev Some ] Entry? ;

: insert ( Map['K: Eq Hash 'V 'N] 'K 'V -- Map['K 'V 'N] )
  | m k v |
  &m &k probe | i |
  &!m &!slots i &!> k v Full !                     \ overwrite: old Entry dropped
  m &count @ 1 + m &!count swap ! ;                \ (count++ only if was Empty; elided)

: remove ( Map['K: Eq Hash 'V 'N] 'K -- Map['K 'V 'N] )
  | m k |
  &m &k probe | i |
  &!m &!slots i &!> Empty !                         \ old Full dropped by `!`; see #8
  m ;                                               \ (back-shift/tombstone elided)
```

## Program 2 — array `sort` over a user ordering

```sooth
\ Insertion sort in place over a fixed array. Value-based swap forces `Copy`
\ ALONGSIDE the ordering trait (friction #9); the array form types today, the
\ Vec['T] form does not (friction #7).
: sort ( ['T: Copy Order 'N] -- ['T 'N] )
  | a |
  1 | i |
  [ i &a len u< ]                                   \ for i in 1..N
  [ &a i &> @ | key |                               \ Copy: @ duplicates safely
    i | j |
    [ j 0 u>  &a j 1 - &>  &key  cmp  Greater? ]     \ while a[j-1] > key
    [ &a j 1 - &> @  &!a j &!> swap !                \ a[j] := a[j-1]
      j 1 -  &j swap ! ]
    while
    &!a j &!> key !                                  \ a[j] := key
    i 1 +  &i swap ! ]
  while
  a ;
```

---

## Friction points (each a fact about what the sketched surface can/cannot express)

1. **`sort` needs a *separate user trait*, not the existing `Bound::Ord`.**
   `is_ord` is *literally* `is_numeric` (`src/check/poly.rs:7-9`:
   `pub(super) fn is_ord(ty) -> bool { ty.is_numeric() }`), so `'T: Ord` today admits
   only the numeric tower and rejects any struct/enum key. Reusing `Bound::Ord` for a
   user `sort` would require making `is_ord` extensible, which *is* the hardcoded-predicate
   problem the slice exists to remove; and the builtin `Ord` buys the *primitive*
   comparisons `<`/`>` (they lower to `u<`/`u>`), which a user type has no analogue of.
   So a user-ordered `sort` must go through a `cmp`-word trait (`Order` above), distinct
   from `Bound::Ord`. Severity: high — it is the load-bearing reason the slice exists,
   and it must be stated in the spec, not assumed.

2. **A user trait cannot be named `Ord` or `Copy`.** `parse_capabilities`
   (`src/parser.rs:2158-2183`) matches the two string literals `"Copy"`/`"Ord"` *before*
   any trait-table lookup, so `'T: Ord` will always resolve to the builtin numeric bound
   and shadow a user trait of that name; `'T: Copy` likewise. Any trait table threaded
   into the parser (Recon 6) must decide precedence, and a user `Ord`/`Copy` is
   unreachable unless the builtins are demoted into the same table. Severity: medium
   (naming/namespacing), but it directly constrains Recon 6's parser change.

3. **The required-method signature must take `&'T`, not `'T`.** The brief's
   `show ( 'T -- )` *consumes* its receiver. Every collection method here inspects a
   value it must leave in place: `eq`/`hash`/`cmp` all take `&'T`. A value-consuming
   `eq ( 'T 'T -- bool )` would move both keys out of the map to compare them. Good news,
   probed: `&'T` in a top-level poly signature **parses today** — `: cmp2 ( &'T &'T -- bool )`
   gets *past* signature parsing (it fails later, in the body, on `@`, not on the sig).
   So the trait-declaration grammar must allow `&'T` in a member signature, and the
   body-side checker branch (Recon 4) must fire on "top of stack is a *ref to* a bounded
   var", not just "top is `PolyType::Var(v)`". Recon 4's sketch ("if the top of stack is
   `PolyType::Var(v)`") is too narrow. Severity: high — it enlarges the exact branch the
   brief says the slice's effort goes into.

4. **Calling a bound method on a key buried in an enum cell needs variant-field
   borrowing that does not exist.** To compare a stored key I need `&'K` out of a
   `Full k 'K v 'V` cell. A variant's field is only reachable *after* clause dispatch
   (`Entry?`), and dispatch consumes the cell rather than lending a `&'K`. There is no
   "borrow the `k` field of the `Full` arm" projection. So `probe`'s comparison step is
   not expressible as written; the natural body is blocked independent of traits.
   Severity: high — it means the brief's forcing consumer can't be written even once
   `trait:` lands, unless the Map representation avoids enum-in-array (see #6).

5. **`[VERIFIED-BLOCKED]` A polymorphic word cannot even *name* a generic type applied to
   a type variable in its signature.** Probed against the built compiler:
   - `: unbox ( Box['T] -- 'T )` → `error: unknown type 'T` (`/tmp/genstruct.sth`)
   - `: or-default ( 'T Option['T] -- 'T )` → `error: unknown type 'T` (`/tmp/genpoly.sth`)
   - contrast: `: setat ( ['T: Copy 4] 'T -- ['T 4] )` (`examples/poly_borrow_setat.sth`)
     **builds green** — an *array* carrying a type var in a poly sig works; a *named
     generic* carrying one does not.
   Every signature in Program 1 (`Map['K 'V 'N]`, `Entry['K 'V]`, `Option[&'V]`) and the
   `Vec['T]` form of Program 2 depend on this and are unparseable today. This is a
   Phase-5-shaped gap (generic instantiation with a variable argument in a word
   signature), and it is a hard prerequisite for S3's stated consumers — the bounds
   feature has *no typeable consumer* without it. Severity: **blocker for the slice's
   own dogfood.**

6. **Constructing generic fixed-capacity storage forces either an `Option`/nullary-variant
   wrapper or a `Default`-style bound.** A `['K 'N]` backing array cannot be built without
   a `'K` value to `fill` with, and there is no uninitialized memory in the language. The
   dogfood sidesteps this with `Entry ... | Empty | ...` (nullary `Empty` is constructible
   with no `'K`), but that reintroduces #4/#7. The parallel-array alternative
   (`keys ['K 'N]` + `used [bool 'N]`) is *not* constructible at all without a third bound
   like `'T: Default`. So the fixed layer's Map has a construction constraint the trait
   sketch does not mention. Severity: medium (fixed-layer, adjacent to bounds), but it
   adds a candidate *third* required capability (`Default`) the brief lists nowhere.

7. **`Map` needs a capacity parameter `'N`, and `type:` header support for a length-var
   parameter is unverified.** `type: Map 'K 'V 'N` puts a length variable in a header
   position that today only holds type variables (`type: Result 'T 'E`); whether the
   header grammar admits `'N` and threads it to the `[Entry 'N]` field is Phase-5
   territory and was not probed. Severity: medium, adjacent.

8. **Removal does *not* need the value type bounded.** `remove` overwrites a `Full` cell
   with `Empty`; the old `Full k v` is disposed by `!`'s in-place drop, and disposal is
   whole-program and type-directed (`drop` needs no bound). So neither `'V` nor `'K` needs
   an `Eq`/`Ord`/anything bound *to be dropped* — only `'K: Eq Hash` is needed *to locate*
   the cell. This confirms the sketch does not need a "droppable" bound; the linear spine
   already covers it. Severity: none (a *non*-friction worth recording so the spec does not
   invent a `Drop` bound).

9. **`sort` needs `Copy` AND the ordering trait on the same `'T`, and the composition is
   clean — but only because of the swap, not the compare.** The *compare* step needs only
   `Order` (ref-based `cmp`). The *swap* step, written with `@`/`!`, duplicates elements
   and so needs `'T: Copy`; a non-Copy swap would need a dedicated array-swap primitive that
   moves both slots without a transient hole (not expressible under the exclusivity/linear
   rules today — two live `@`s from one array can't coexist). So `( ['T: Copy Order 'N] -- ... )`
   mixes a *builtin* bound (`Copy`) and a *user* trait (`Order`) on one variable. That
   composition is structurally identical to `lib/core.sth`'s live `'T: Copy Ord`, so OQ4's
   "does a bound compose" is answered yes *provided* the parser change (#2) keeps the
   greedy capability list and does not special-case "exactly one trait name". Severity:
   low, but it is a concrete OQ4 confirmation with a real consumer.

10. **Multi-bound method-name resolution is underspecified.** `probe`/`get`/`insert` all
    carry `'K: Eq Hash` and call both `eq` and `hash`. The body checker must, per call,
    walk the variable's *bound list* and find which trait supplies the called name. If two
    bounds on one variable declared a same-named method, the sketch has no disambiguation
    rule. Recon 3/4 assume a unique name. Severity: medium — a rule ("a member name must be
    unique across a variable's bound set, else a located error") should be in the spec.

---

## Verdicts

- **OQ3 (is single-type-variable trait scope enough?): YES for these two consumers.**
  `Map`'s `'K`/`'V` relationship is carried by the *struct* `Map['K 'V]`, not by any trait;
  every trait it needs (`Eq`, `Hash`) closes over one variable (`'K`) and never mentions
  `'V`. `sort`'s `Order` closes over one `'T`. No spot forced a two-variable trait (a
  `Zip`/`Coerce`-style relation between two variables). The multi-variable relationship
  lives in the collection type and in *composition of several single-variable bounds on the
  same variable* (`'K: Eq Hash`, `'T: Copy Order`), which is the OQ4 mechanism, not OQ3.
  Recommend locking single-type-variable scope and naming multi-variable traits out of
  scope, exactly as the brief proposes.

- **Does `Map` need more than one required-method trait? YES — an open-addressing map needs
  two required methods (`eq` AND `hash`).** A single-required-method trait is insufficient
  for the hashed representation. This is satisfied either by two single-method traits
  (`'K: Eq Hash`) or by one two-method trait; both are fine because bound composition
  already parses (#9). Caveat: an *ordered* map representation (sorted array + binary
  search, cf. `lib/binary_search.sth`) needs only one method (`cmp`), so "more than one
  required method" is a property of the *hashed* representation, not of maps in general.
  The construction constraint (#6) can add a *third* (`Default`) depending on the backing
  choice.

## Bottom line for the spec

The trait *surface* itself (`trait:`, `'T: TraitName`, single variable, composed bounds,
ref-taking members) survives the dogfood with three required amendments: members must take
`&'T` (#3), the parser must not collide user names with `Copy`/`Ord` (#2), and multi-bound
member-name uniqueness needs a rule (#10). But the dogfood surfaces **two prerequisites that
sit outside S3 and block its own forcing consumers**: named-generic-applied-to-a-variable in
a poly signature (#5, VERIFIED) and variant-field borrowing / generic-enum handling in a poly
body (#4). Until those exist, `Map['K 'V]` is not typeable and the bounds feature has no
consumer that compiles. Recommend the spec either (a) depend explicitly on the Phase-5
generic-instantiation gap being closed first, or (b) restrict S3's dogfood to the *array*
form (`sort ( ['T: Copy Order 'N] -- ['T 'N] )`, which types structurally today), and defer
the named-`Map` consumer to S4/S5 where the generic-collection machinery lands.
